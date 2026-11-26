//! The Learn tab: lessons, ear-training games and the theory overlay
//! ("what you're hearing"), which also shows on the Play tab.
//!
//! Lessons and games never hard-code keys: targets are notes and chords,
//! and this file finds the keys in the current keymap that play them, then
//! lights those keys up on the drawn keyboard.

use super::*;
use crate::learn::Rng;
use crate::learn::analysis::{self, Explanation, Family, Heard, shape_for};
use crate::learn::ear::{self, Answer, ChordKey, Game, Played, Question, Score};
use crate::learn::intervals::interval;
use crate::learn::lessons::{self, CHAPTERS, Lesson, Step, Target};
use crate::theory::{Quality, Scale, pc_name};
use crate::ui::circle::circle_of_fifths;
use crate::ui::widgets::{Hint, HintKind, key_position};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LearnView {
    Lessons,
    Ear,
}

/// Plays lesson examples and ear-training questions. Timed from the UI
/// thread (frame accuracy is plenty for examples).
#[derive(Default)]
pub(super) struct Player {
    /// (when, note, on?)
    queue: Vec<(Instant, i32, bool)>,
    /// Notes this player started that haven't been stopped yet.
    pub sounding: Vec<i32>,
}

pub(super) struct LearnUi {
    pub view: LearnView,
    // Lessons.
    lesson: &'static Lesson,
    step: usize,
    /// How many targets of the current step are done.
    progress: usize,
    /// A new note was pressed since the last "hold these together" target,
    /// so the same held keys can't complete two targets in a row.
    armed: bool,
    step_done: bool,
    lesson_done: bool,
    auto_played: Option<(&'static str, usize)>,
    /// "That was E, try the lit key" (shown briefly).
    wrong: Option<(String, Instant)>,
    // Ear training.
    game: Game,
    started: bool,
    question: Option<Question>,
    q_error: Option<String>,
    missed: bool,
    answered: Option<Instant>,
    score: Score,
    rng: Rng,
    /// Notes / chords played on the Learn tab since the last frame.
    pub events: Vec<Played>,
    pub player: Player,
    // Theory overlay.
    pub overlay: Option<Explanation>,
    /// The overlay is about something sounding right now (vs. the last thing).
    pub overlay_live: bool,
    overlay_cur: Option<(i32, Family)>,
    pub overlay_motion: Option<String>,
    /// What was sounding last frame, when it changed, and how many notes
    /// the explanation on screen is about. Letting go of a chord one key
    /// at a time shouldn't replace "C major" with "C + E".
    overlay_notes: Vec<i32>,
    overlay_changed: Option<Instant>,
    overlay_shown_n: usize,
    /// Width of the lesson card column (for wrapping key chips).
    mid_w: f32,
}

impl LearnUi {
    pub fn new(progress: &crate::config::LearnProgress) -> LearnUi {
        let lesson = lessons::find(&progress.lesson).unwrap_or(&CHAPTERS[0].lessons[0]);
        let step = progress.step.min(lesson.steps.len() - 1);
        LearnUi {
            view: LearnView::Lessons,
            lesson,
            step,
            progress: 0,
            armed: true,
            step_done: matches!(lesson.steps[step], Step::Read(_) | Step::Listen { .. }),
            lesson_done: false,
            auto_played: None,
            wrong: None,
            game: Game::FindNote,
            started: false,
            question: None,
            q_error: None,
            missed: false,
            answered: None,
            score: Score::default(),
            rng: Rng::from_time(),
            events: Vec::new(),
            player: Player::default(),
            overlay: None,
            overlay_live: false,
            overlay_cur: None,
            overlay_motion: None,
            overlay_notes: Vec::new(),
            overlay_changed: None,
            overlay_shown_n: 0,
            mid_w: 500.0,
        }
    }
}

/// What a lesson target needs, worked out for the current keymap.
#[derive(Clone, Debug)]
enum Want {
    Note {
        note: i32,
        exact: bool,
    },
    /// These pitch classes held together.
    Pcs(Vec<i32>),
    Chord {
        root_pc: i32,
        family: Family,
    },
}

#[derive(Clone, Debug)]
struct Resolved {
    want: Want,
    /// Keys to light.
    keys: Vec<KeyId>,
    /// False if this layout can't play it.
    ok: bool,
    /// Short name for tags ("E", "G7", "Am").
    name: String,
}

fn family_of(q: Quality) -> Family {
    match q {
        Quality::Major => Family::Major,
        Quality::Minor => Family::Minor,
        Quality::Dim => Family::Dim,
        Quality::Aug => Family::Aug,
    }
}

fn triad(family: Family) -> [i32; 3] {
    match family {
        Family::Minor => [0, 3, 7],
        Family::Dim => [0, 3, 6],
        Family::Aug => [0, 4, 8],
        _ => [0, 4, 7],
    }
}

fn chord_label(root_pc: i32, family: Family) -> String {
    let suffix = match family {
        Family::Minor => "m",
        Family::Dim => "dim",
        Family::Aug => "aug",
        Family::Sus => "sus",
        Family::Power => "5",
        Family::Major => "",
    };
    format!("{}{suffix}", analysis::spell(root_pc, 0, Scale::Major))
}

impl QwertoneApp {
    // ------------------------------------------------------------ helpers

    /// The C at the bottom of the Piano layout (C3 at octave 0). Lesson
    /// notes are counted from here.
    fn lesson_c(&self) -> i32 {
        BASE_NOTE + 12 * self.cfg.octave
    }

    /// Every (note, key) the layout plays right now.
    pub(super) fn note_keys(&self) -> Vec<(i32, KeyId)> {
        let mut v: Vec<(i32, KeyId)> = self
            .keymap
            .bindings
            .iter()
            .filter_map(|(k, a)| self.action_note(*a).map(|n| (n, *k)))
            .collect();
        v.sort_by_key(|(n, k)| (*n, key_position(*k).map(|(r, c)| (4 - r, c))));
        v
    }

    /// Chord keys (no modifiers), one per distinct chord.
    fn chord_keys_list(&self) -> Vec<(KeyId, ChordKey)> {
        let mut keys: Vec<(KeyId, ChordRef)> = self
            .keymap
            .bindings
            .iter()
            .filter_map(|(k, a)| match a {
                Action::Chord(c) => Some((*k, *c)),
                _ => None,
            })
            .collect();
        keys.sort_by_key(|(k, _)| key_position(*k).map(|(r, c)| (4 - r, c)));
        let mut out: Vec<(KeyId, ChordKey)> = Vec::new();
        for (k, c) in keys {
            let ch = build_chord(c, 0, self.tonic_pc(), self.cfg.scale);
            let pcs: Vec<i32> = ch
                .tones
                .iter()
                .map(|t| (ch.root_pc + t).rem_euclid(12))
                .collect();
            let family = shape_for(ch.root_pc, &pcs)
                .map(|s| s.family)
                .unwrap_or(Family::Major);
            if out
                .iter()
                .any(|(_, o)| o.root_pc == ch.root_pc && o.family == family)
            {
                continue;
            }
            let notes = voice_chord(&ch, &[], self.chord_center(), false);
            out.push((
                k,
                ChordKey {
                    root_pc: ch.root_pc,
                    family,
                    name: ch.name.clone(),
                    numeral: ch.numeral.clone(),
                    degree: match c {
                        ChordRef::Diatonic(d) => Some(d),
                        _ => None,
                    },
                    notes,
                },
            ));
        }
        out
    }

    /// Of several keys, the one in the same row as `prev`, else the lowest row.
    pub(super) fn pick_key(keys: &[KeyId], prev: Option<KeyId>) -> Option<KeyId> {
        let prev_row = prev.and_then(key_position).map(|(r, _)| r);
        keys.iter().copied().min_by_key(|k| {
            let pos = key_position(*k);
            let same = pos.map(|(r, _)| r) == prev_row && prev_row.is_some();
            (!same, pos.map(|(r, c)| (4 - r, c)).unwrap_or((9, 0)))
        })
    }

    /// Keys for a note: exact if possible, otherwise the same note name in
    /// the nearest octave. Returns (keys, exact?).
    pub(super) fn keys_for_note(note_keys: &[(i32, KeyId)], note: i32) -> (Vec<KeyId>, bool) {
        let exact: Vec<KeyId> = note_keys
            .iter()
            .filter(|(n, _)| *n == note)
            .map(|(_, k)| *k)
            .collect();
        if !exact.is_empty() {
            return (exact, true);
        }
        let best = note_keys
            .iter()
            .filter(|(n, _)| (n - note).rem_euclid(12) == 0)
            .map(|(n, _)| (n - note).abs())
            .min();
        let keys = match best {
            Some(d) => note_keys
                .iter()
                .filter(|(n, _)| (n - note).rem_euclid(12) == 0 && (n - note).abs() == d)
                .map(|(_, k)| *k)
                .collect(),
            None => Vec::new(),
        };
        (keys, false)
    }

    fn step_targets(step: &Step) -> Vec<Target> {
        match *step {
            Step::Play { targets, .. } => targets.to_vec(),
            Step::Interval { root, semitones } => {
                vec![Target::Note(root), Target::Note(root + semitones)]
            }
            _ => Vec::new(),
        }
    }

    /// Work out every target of a step for the current keymap.
    fn resolve_targets(&self, targets: &[Target]) -> Vec<Resolved> {
        let note_keys = self.note_keys();
        let chord_keys = self.chord_keys_list();
        let c = self.lesson_c();
        let mut prev: Option<KeyId> = None;
        let mut out = Vec::new();
        let as_notes = |offs: &[i32], prev: &mut Option<KeyId>| -> (Vec<KeyId>, bool) {
            let mut keys = Vec::new();
            let mut ok = true;
            for o in offs {
                let (ks, _) = Self::keys_for_note(&note_keys, c + o);
                match Self::pick_key(&ks, *prev) {
                    Some(k) => {
                        keys.push(k);
                        *prev = Some(k);
                    }
                    None => ok = false,
                }
            }
            (keys, ok)
        };
        for t in targets {
            let r = match *t {
                Target::Note(off) => {
                    let note = c + off;
                    let (ks, exact) = Self::keys_for_note(&note_keys, note);
                    let key = Self::pick_key(&ks, prev);
                    prev = key.or(prev);
                    Resolved {
                        want: Want::Note { note, exact },
                        keys: key.into_iter().collect(),
                        ok: key.is_some(),
                        name: analysis::spell(note, 0, Scale::Major).to_string(),
                    }
                }
                Target::Together(offs) => {
                    let (keys, ok) = as_notes(offs, &mut prev);
                    let names: Vec<&str> = offs
                        .iter()
                        .map(|o| analysis::spell(c + o, 0, Scale::Major))
                        .collect();
                    Resolved {
                        want: Want::Pcs(offs.iter().map(|o| (c + o).rem_euclid(12)).collect()),
                        keys,
                        ok,
                        name: names.join(" "),
                    }
                }
                Target::Chord(numeral) => {
                    let (root_pc, family) = match ChordRef::parse(numeral) {
                        Some(ChordRef::Fixed { root, quality }) => {
                            (root as i32, family_of(quality))
                        }
                        _ => (0, Family::Major),
                    };
                    let name = chord_label(root_pc, family);
                    let ck: Vec<KeyId> = chord_keys
                        .iter()
                        .filter(|(_, ck)| ck.root_pc == root_pc && ck.family == family)
                        .map(|(k, _)| *k)
                        .collect();
                    if let Some(k) = Self::pick_key(&ck, None) {
                        Resolved {
                            want: Want::Chord { root_pc, family },
                            keys: vec![k],
                            ok: true,
                            name,
                        }
                    } else {
                        // No chord key: play its notes. Keep chords around
                        // lesson C (roots above A drop an octave).
                        let base = if root_pc > 7 { root_pc - 12 } else { root_pc };
                        let offs: Vec<i32> = triad(family).iter().map(|i| base + i + 12).collect();
                        let (keys, ok) = as_notes(&offs, &mut prev);
                        Resolved {
                            want: Want::Pcs(offs.iter().map(|o| (c + o).rem_euclid(12)).collect()),
                            keys,
                            ok,
                            name,
                        }
                    }
                }
            };
            out.push(r);
        }
        out
    }

    /// Fill `{n:4}`, `{c:V}` and `{a:sustain}` with the user's keys.
    fn expand_text(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let Some(end) = rest[i..].find('}').map(|e| e + i) else {
                break;
            };
            let inner = &rest[i + 1..end];
            let rep = match inner.split_once(':') {
                Some(("n", n)) => n
                    .parse::<i32>()
                    .ok()
                    .and_then(|o| {
                        let (ks, _) = Self::keys_for_note(&self.note_keys(), self.lesson_c() + o);
                        Self::pick_key(&ks, None)
                    })
                    .map(|k| k.label().to_string())
                    .unwrap_or_else(|| "(no key)".into()),
                Some(("c", n)) => {
                    let r = self.resolve_targets(&[Target::Chord(leak_numeral(n))]);
                    match r.first() {
                        Some(r) if r.ok => {
                            let keys: Vec<&str> = r.keys.iter().map(|k| k.label()).collect();
                            format!("{} ({})", keys.join(" "), r.name)
                        }
                        _ => "(no key)".into(),
                    }
                }
                Some(("a", a)) => Action::parse(a)
                    .map(|a| self.keys_text(a))
                    .unwrap_or_default(),
                _ => inner.to_string(),
            };
            out.push_str(&rep);
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Record a note / chord played while the Learn tab is open.
    pub(super) fn learn_event(&mut self, ev: Played) {
        if self.tab == Tab::Learn {
            self.learn.events.push(ev);
        }
    }

    pub(super) fn chord_event(&self) -> Option<Played> {
        let ch = self.chord.as_ref()?;
        let pcs: Vec<i32> = ch
            .tones
            .iter()
            .map(|t| (ch.root_pc + t).rem_euclid(12))
            .collect();
        let family = shape_for(ch.root_pc, &pcs)
            .map(|s| s.family)
            .unwrap_or(Family::Major);
        Some(Played::Chord {
            root_pc: ch.root_pc,
            family,
        })
    }

    fn played_name(ev: Played) -> String {
        match ev {
            Played::Note(n) => pc_name(n, false).to_string(),
            Played::Chord { root_pc, family } => chord_label(root_pc, family),
        }
    }

    // ------------------------------------------------------------ player

    /// Play moments (notes that start together) `gap_ms` apart.
    fn play_moments(&mut self, moments: &[Vec<i32>], gap_ms: u32) {
        self.stop_player();
        let now = Instant::now() + Duration::from_millis(30);
        let gap = Duration::from_millis(gap_ms as u64);
        let last = moments.len().saturating_sub(1);
        for (i, m) in moments.iter().enumerate() {
            let at = now + gap * i as u32;
            let hold = if i == last {
                gap.max(Duration::from_millis(1100))
            } else {
                gap.mul_f32(0.92)
            };
            for n in m {
                self.learn.player.queue.push((at, *n, true));
                self.learn.player.queue.push((at + hold, *n, false));
            }
        }
        self.learn.player.queue.sort_by_key(|(t, _, on)| (*t, *on));
    }

    pub(super) fn stop_player(&mut self) {
        self.learn.player.queue.clear();
        for n in std::mem::take(&mut self.learn.player.sounding) {
            if let Some(e) = &self.engine {
                e.note_off(n);
            }
        }
    }

    /// Send due player notes. Returns true while something is queued.
    pub(super) fn tick_player(&mut self) -> bool {
        let now = Instant::now();
        let due: Vec<(Instant, i32, bool)> = {
            let q = &mut self.learn.player.queue;
            let split = q.iter().position(|(t, ..)| *t > now).unwrap_or(q.len());
            q.drain(..split).collect()
        };
        for (_, n, on) in due {
            let n = n.clamp(0, 127);
            if let Some(e) = &self.engine {
                if on {
                    e.note_on(n, 92);
                } else {
                    e.note_off(n);
                }
            }
            if on {
                self.learn.player.sounding.push(n);
            } else if let Some(i) = self.learn.player.sounding.iter().position(|x| *x == n) {
                self.learn.player.sounding.remove(i);
            }
        }
        !self.learn.player.queue.is_empty()
    }

    /// The example sound for a lesson step, as absolute notes.
    fn step_sound(&self, step: &Step) -> Option<(Vec<Vec<i32>>, u32)> {
        let c = self.lesson_c();
        match *step {
            Step::Listen { sound, gap_ms, .. } => Some((
                sound
                    .iter()
                    .map(|m| m.iter().map(|o| c + o).collect())
                    .collect(),
                gap_ms,
            )),
            Step::Interval { root, semitones } => {
                let (a, b) = (c + root, c + root + semitones);
                Some((vec![vec![a], vec![b], vec![], vec![a, b]], 650))
            }
            Step::Play { targets, .. } => {
                // "Show me": play the targets.
                let r = self.resolve_targets(targets);
                let moments = r
                    .iter()
                    .map(|r| match &r.want {
                        Want::Note { note, .. } => vec![*note],
                        Want::Pcs(pcs) => pcs
                            .iter()
                            .map(|p| c + 12 + (p - c).rem_euclid(12))
                            .collect(),
                        Want::Chord { root_pc, family } => {
                            let base = c + 12 + if *root_pc > 7 { root_pc - 12 } else { *root_pc };
                            triad(*family).iter().map(|i| base + i).collect()
                        }
                    })
                    .collect();
                Some((moments, 550))
            }
            Step::Read(_) => None,
        }
    }

    // ------------------------------------------------------------ overlay

    /// Re-work the "what you're hearing" explanation from what's sounding.
    /// Returns true while waiting to see if a released chord settles.
    pub(super) fn update_overlay(&mut self) -> bool {
        let mut melody: Vec<i32> = self.sounding.values().copied().collect();
        // Lesson examples get explained too; ear-training questions don't
        // (that would give the answer away).
        if self.learn.view == LearnView::Lessons {
            melody.extend(self.learn.player.sounding.iter().copied());
        }
        melody.sort();
        melody.dedup();
        let mut all = melody.clone();
        if self.chord.is_some() {
            all.extend(self.voicing.iter().copied());
        }
        if all != self.learn.overlay_notes {
            self.learn.overlay_notes = all.clone();
            self.learn.overlay_changed = Some(Instant::now());
        }
        let n = all.len();
        self.learn.overlay_live = n > 0;
        if n == 0 {
            self.learn.overlay_shown_n = 0;
            return false;
        }
        // Fewer notes than what's explained: probably letting go of a chord.
        // Only switch if it stays like that for a moment.
        let settled = self
            .learn
            .overlay_changed
            .is_none_or(|t| t.elapsed() > Duration::from_millis(350));
        if n < self.learn.overlay_shown_n && !settled {
            return true;
        }
        let e = analysis::explain(&Heard {
            melody: &melody,
            chord: self.chord.as_ref(),
            tonic_pc: self.tonic_pc(),
            scale: self.cfg.scale,
        });
        self.learn.overlay_shown_n = n;
        if let Some(e) = e {
            if let Some(step) = e.step
                && Some(step) != self.learn.overlay_cur
            {
                self.learn.overlay_motion = self
                    .learn
                    .overlay_cur
                    .and_then(|prev| analysis::motion(prev, step, self.cfg.scale));
                self.learn.overlay_cur = Some(step);
            }
            self.learn.overlay = Some(e);
        }
        false
    }

    /// The overlay card. `compact` = fewer lines (Play tab).
    pub(super) fn overlay_card(&self, ui: &mut egui::Ui, compact: bool) {
        let p = self.palette;
        card(ui, &p, "", |ui| {
            // A steady height so the keyboard below doesn't jump around.
            ui.set_min_height(if compact { 150.0 } else { 250.0 });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("What you're hearing")
                        .size(15.0)
                        .strong()
                        .color(p.primary),
                );
                if !self.learn.overlay_live && self.learn.overlay.is_some() {
                    ui.label(
                        RichText::new("· last thing you played")
                            .small()
                            .color(p.outline),
                    );
                }
            });
            let Some(e) = &self.learn.overlay else {
                ui.label(
                    RichText::new(
                        "Play a note, two notes or a chord and this panel explains it: its name, \
                         its job in the key, and why it sounds happy, sad or tense.",
                    )
                    .color(p.on_surface_variant),
                );
                return;
            };
            let fade = |c: Color32| {
                if self.learn.overlay_live {
                    c
                } else {
                    c.gamma_multiply(0.6)
                }
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&e.title)
                        .size(if compact { 30.0 } else { 36.0 })
                        .strong()
                        .color(fade(p.primary)),
                );
                ui.vertical(|ui| {
                    if let Some(n) = &e.numeral {
                        ui.label(RichText::new(n).size(16.0).color(fade(p.on_surface)));
                    }
                    ui.horizontal_wrapped(|ui| {
                        chip(
                            ui,
                            p.secondary_container,
                            p.on_secondary_container,
                            &e.label,
                        );
                        ui.label(RichText::new(&e.feel).color(fade(p.tertiary)));
                    });
                });
            });
            tension_bar(ui, &p, e.tension);
            if let Some(m) = &self.learn.overlay_motion {
                ui.label(RichText::new(format!("↪ {m}")).color(p.tertiary));
            }
            let lines = if compact { 2 } else { 3 };
            for w in e.why.iter().take(lines) {
                ui.label(RichText::new(format!("• {w}")).color(fade(p.on_surface_variant)));
            }
            if !e.songs.is_empty() {
                ui.label(
                    RichText::new(format!("Sounds like: {}", e.songs.join(" · ")))
                        .color(fade(p.on_surface)),
                );
            }
            if !e.outside.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Outside {}: {}",
                        self.key_name(),
                        e.outside.join(", ")
                    ))
                    .small()
                    .color(p.error),
                );
            }
        });
    }

    // ------------------------------------------------------------ keys F2/F3

    /// F2 on the Learn tab: hear the example / question again.
    pub(super) fn learn_replay(&mut self) {
        match self.learn.view {
            LearnView::Lessons => {
                let step = self.learn.lesson.steps[self.learn.step];
                if let Some((m, gap)) = self.step_sound(&step) {
                    self.play_moments(&m, gap);
                }
            }
            LearnView::Ear => {
                if !self.learn.started {
                    self.learn.started = true;
                    self.new_question();
                } else if let Some(q) = self.learn.question.clone() {
                    self.play_moments(&q.sound, q.gap_ms);
                }
            }
        }
    }

    /// F3 on the Learn tab: next step / next question.
    pub(super) fn learn_next(&mut self) {
        match self.learn.view {
            LearnView::Lessons => {
                if self.learn.lesson_done {
                    if let Some(n) = lessons::next_after(self.learn.lesson.id) {
                        self.open_lesson(n);
                    }
                } else if self.can_go_next() {
                    self.go_step(self.learn.step + 1);
                }
            }
            LearnView::Ear => {
                if self.learn.started {
                    if self.learn.answered.is_none() && self.learn.question.is_some() {
                        self.learn.score.record(false);
                    }
                    self.new_question();
                }
            }
        }
    }

    // ------------------------------------------------------------ lessons

    fn open_lesson(&mut self, lesson: &'static Lesson) {
        self.stop_player();
        self.learn.lesson = lesson;
        self.cfg.learn.lesson = lesson.id.to_string();
        self.learn.lesson_done = false;
        self.go_step(0);
    }

    fn go_step(&mut self, step: usize) {
        let steps = self.learn.lesson.steps;
        if step >= steps.len() {
            self.finish_lesson();
            return;
        }
        self.stop_player();
        self.learn.step = step;
        self.learn.progress = 0;
        self.learn.armed = true;
        self.learn.wrong = None;
        self.learn.events.clear();
        self.learn.step_done = matches!(steps[step], Step::Read(_) | Step::Listen { .. });
        self.cfg.learn.step = step;
        self.mark_dirty();
    }

    fn finish_lesson(&mut self) {
        self.learn.lesson_done = true;
        self.cfg.learn.done.insert(self.learn.lesson.id.to_string());
        self.mark_dirty();
    }

    fn can_go_next(&self) -> bool {
        self.learn.step_done
    }

    /// Feed played notes into the current lesson step.
    fn lesson_input(&mut self) {
        let events = std::mem::take(&mut self.learn.events);
        let step = self.learn.lesson.steps[self.learn.step];
        let targets = Self::step_targets(&step);
        if targets.is_empty() || self.learn.step_done || self.learn.lesson_done {
            return;
        }
        let resolved = self.resolve_targets(&targets);
        for ev in events {
            if matches!(ev, Played::Note(_) | Played::Chord { .. }) {
                self.learn.armed = true;
            }
            let Some(r) = resolved.get(self.learn.progress) else {
                break;
            };
            let ok = match (&r.want, ev) {
                (Want::Note { note, exact: true }, Played::Note(m)) => m == *note,
                (Want::Note { note, exact: false }, Played::Note(m)) => {
                    (m - note).rem_euclid(12) == 0
                }
                (
                    Want::Chord { root_pc, family },
                    Played::Chord {
                        root_pc: r2,
                        family: f2,
                    },
                ) => *root_pc == r2 && *family == f2,
                (Want::Pcs(_), _) => continue,
                _ => false,
            };
            if ok {
                self.advance_target(targets.len());
            } else {
                self.learn.wrong = Some((
                    format!(
                        "That was {}. Try the lit key ({}).",
                        Self::played_name(ev),
                        r.name
                    ),
                    Instant::now(),
                ));
            }
        }
        // "Hold these together" targets are checked against what's held.
        if let Some(r) = resolved.get(self.learn.progress)
            && let Want::Pcs(pcs) = &r.want
            && self.learn.armed
        {
            let mut held: HashSet<i32> = self.sounding.values().map(|n| n.rem_euclid(12)).collect();
            if !self.chord_keys.is_empty() {
                held.extend(self.voicing.iter().map(|n| n.rem_euclid(12)));
            }
            if pcs.iter().all(|pc| held.contains(pc)) {
                self.learn.armed = false;
                self.advance_target(targets.len());
            }
        }
    }

    fn advance_target(&mut self, total: usize) {
        self.learn.progress += 1;
        self.learn.wrong = None;
        if self.learn.progress >= total {
            self.learn.step_done = true;
            if self.learn.step + 1 >= self.learn.lesson.steps.len() {
                self.finish_lesson();
            }
        }
    }

    /// Keys to light for the current lesson step.
    fn lesson_hints(&self, resolved: &[Resolved]) -> HashMap<KeyId, Hint> {
        let mut hints = HashMap::new();
        if self.learn.step_done {
            return hints;
        }
        let many = resolved.len() > 1;
        // Later targets first so the current one wins on shared keys.
        for (i, r) in resolved.iter().enumerate().skip(self.learn.progress).rev() {
            let now = i == self.learn.progress;
            for k in &r.keys {
                let tag = if many {
                    (i + 1).to_string()
                } else {
                    String::new()
                };
                if !now && hints.contains_key(k) {
                    continue;
                }
                hints.insert(
                    *k,
                    Hint {
                        kind: if now { HintKind::Now } else { HintKind::Next },
                        tag,
                    },
                );
            }
        }
        hints
    }

    // ------------------------------------------------------------ ear games

    fn ear_level(&self, game: Game) -> usize {
        self.cfg
            .learn
            .levels
            .get(game.code())
            .copied()
            .unwrap_or(0)
            .min(2)
    }

    fn new_question(&mut self) {
        let notes: Vec<i32> = {
            let mut v: Vec<i32> = self.note_keys().iter().map(|(n, _)| *n).collect();
            v.dedup();
            v
        };
        let chords: Vec<ChordKey> = self.chord_keys_list().into_iter().map(|(_, c)| c).collect();
        let game = self.learn.game;
        let level = self.ear_level(game);
        let ctx = ear::Context {
            notes: &notes,
            chords: &chords,
            c: self.lesson_c(),
            tonic_pc: self.tonic_pc(),
            scale: self.cfg.scale,
        };
        // Don't ask the same thing twice in a row if we can help it.
        let mut q = ear::question(game, level, &ctx, &mut self.learn.rng);
        for _ in 0..4 {
            match (&q, &self.learn.question) {
                (Ok(a), Some(b)) if a.answer == b.answer && a.sound == b.sound => {
                    q = ear::question(game, level, &ctx, &mut self.learn.rng);
                }
                _ => break,
            }
        }
        self.learn.missed = false;
        self.learn.answered = None;
        self.learn.wrong = None;
        self.learn.events.clear();
        match q {
            Ok(q) => {
                self.learn.q_error = None;
                self.play_moments(&q.sound, q.gap_ms);
                self.learn.question = Some(q);
            }
            Err(e) => {
                self.learn.q_error = Some(e);
                self.learn.question = None;
            }
        }
    }

    fn set_game(&mut self, game: Game, level: usize) {
        self.learn.game = game;
        self.cfg
            .learn
            .levels
            .insert(game.code().to_string(), level.min(2));
        self.mark_dirty();
        self.learn.score = Score::default();
        self.learn.question = None;
        if self.learn.started {
            self.new_question();
        }
    }

    fn ear_input(&mut self) {
        let events = std::mem::take(&mut self.learn.events);
        let Some(q) = self.learn.question.clone() else {
            return;
        };
        if self.learn.answered.is_some() {
            return;
        }
        for ev in events {
            if q.answer.accepts(ev) {
                self.learn.answered = Some(Instant::now());
                self.learn.wrong = None;
                self.learn.score.record(!self.learn.missed);
                let code = self.learn.game.code().to_string();
                let best = self.cfg.learn.best.get(&code).copied().unwrap_or(0);
                if self.learn.score.best_streak > best {
                    self.cfg
                        .learn
                        .best
                        .insert(code, self.learn.score.best_streak);
                    self.mark_dirty();
                }
                break;
            }
            // Pressing a reference key to check it isn't a wrong answer.
            if let Played::Note(n) = ev
                && q.home.iter().any(|(h, _)| *h == n)
            {
                continue;
            }
            self.learn.missed = true;
            self.learn.wrong = Some((
                format!(
                    "Not quite: that was {}. Press F2 to hear it again.",
                    Self::played_name(ev)
                ),
                Instant::now(),
            ));
        }
    }

    fn ear_hints(&self) -> HashMap<KeyId, Hint> {
        let mut hints = HashMap::new();
        let Some(q) = &self.learn.question else {
            return hints;
        };
        let note_keys = self.note_keys();
        let chord_keys = self.chord_keys_list();
        let mut add = |keys: Vec<KeyId>, kind: HintKind, tag: &str| {
            if let Some(k) = Self::pick_key(&keys, None) {
                hints.insert(
                    k,
                    Hint {
                        kind,
                        tag: tag.to_string(),
                    },
                );
            }
        };
        let exact = |n: i32| -> Vec<KeyId> {
            note_keys
                .iter()
                .filter(|(m, _)| *m == n)
                .map(|(_, k)| *k)
                .collect()
        };
        for (n, tag) in &q.options {
            add(exact(*n), HintKind::Option, tag);
        }
        for (root, fam) in &q.chord_options {
            let keys: Vec<KeyId> = chord_keys
                .iter()
                .filter(|(_, c)| c.root_pc == *root && c.family == *fam)
                .map(|(k, _)| *k)
                .collect();
            let tag = match fam {
                Family::Minor => "sad",
                _ if q.chord_options.len() == 2 => "happy",
                _ => "",
            };
            add(keys, HintKind::Option, tag);
        }
        for (n, tag) in &q.home {
            add(exact(*n), HintKind::Home, tag);
        }
        if self.learn.answered.is_some() {
            let keys: Vec<KeyId> = match &q.answer {
                Answer::Note(n) => exact(*n),
                Answer::Pc(pc) => Self::keys_for_note(&note_keys, self.lesson_c() + pc).0,
                Answer::Chord { root_pc, family } => chord_keys
                    .iter()
                    .filter(|(_, c)| c.root_pc == *root_pc && c.family == *family)
                    .map(|(k, _)| *k)
                    .collect(),
            };
            add(keys, HintKind::Correct, "✓");
        }
        hints
    }

    // ------------------------------------------------------------ the tab

    pub(super) fn learn_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        match self.learn.view {
            LearnView::Lessons => self.lesson_input(),
            LearnView::Ear => self.ear_input(),
        }
        // Next ear question a moment after a right answer.
        if self.learn.view == LearnView::Ear
            && self
                .learn
                .answered
                .is_some_and(|t| t.elapsed() > Duration::from_millis(2600))
        {
            self.new_question();
        }
        if self
            .learn
            .wrong
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(4))
        {
            self.learn.wrong = None;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (view, label) in [
                        (LearnView::Lessons, "Lessons"),
                        (LearnView::Ear, "Ear training"),
                    ] {
                        let sel = self.learn.view == view;
                        let btn = egui::Button::new(RichText::new(label).color(if sel {
                            p.on_secondary_container
                        } else {
                            p.on_surface
                        }))
                        .fill(if sel {
                            p.secondary_container
                        } else {
                            p.surface_container_high
                        })
                        .corner_radius(egui::CornerRadius::same(255));
                        if ui.add(btn).clicked() && !sel {
                            self.stop_player();
                            self.learn.view = view;
                            self.learn.events.clear();
                            self.learn.wrong = None;
                        }
                    }
                    let total = lessons::all().count();
                    let done = lessons::all()
                        .filter(|l| self.cfg.learn.done.contains(l.id))
                        .count();
                    chip(
                        ui,
                        p.surface_container_high,
                        p.on_surface,
                        format!("{done}/{total} lessons done"),
                    );
                    ui.label(
                        RichText::new("F2 hear again · F3 next")
                            .small()
                            .color(p.on_surface_variant),
                    );
                });
                ui.add_space(6.0);

                let total_w = ui.available_width();
                let gap = ui.spacing().item_spacing.x;
                let side_w = 214.0;
                let main_w = (total_w - side_w - gap).max(420.0);
                let right_w = (main_w * 0.36).clamp(270.0, 360.0);
                let mid_w = (main_w - right_w - gap).max(240.0);
                self.learn.mid_w = mid_w;
                ui.horizontal_top(|ui| {
                    // Left: lesson / game list, scrolling on its own.
                    ui.vertical(|ui| {
                        ui.set_width(side_w);
                        egui::ScrollArea::vertical()
                            .id_salt("learn-list")
                            .max_height(ui.ctx().content_rect().height() - 150.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| match self.learn.view {
                                LearnView::Lessons => self.lesson_list(ui),
                                LearnView::Ear => self.game_list(ui),
                            });
                    });
                    // Right: lesson card + overlay, keyboard underneath.
                    ui.vertical(|ui| {
                        ui.set_width(main_w);
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(mid_w);
                                match self.learn.view {
                                    LearnView::Lessons => self.lesson_card(ui),
                                    LearnView::Ear => self.game_card(ui),
                                }
                            });
                            ui.vertical(|ui| {
                                ui.set_width(right_w);
                                self.overlay_card(ui, false);
                                if !(self.learn.view == LearnView::Lessons
                                    && self.learn.lesson.circle)
                                {
                                    ui.add_space(6.0);
                                    egui::CollapsingHeader::new("Circle of fifths")
                                        .id_salt("circle")
                                        .show(ui, |ui| {
                                            self.circle_widget(ui, (right_w - 40.0).min(290.0))
                                        });
                                }
                            });
                        });
                        ui.add_space(8.0);
                        let hints = match self.learn.view {
                            LearnView::Lessons => {
                                let step = self.learn.lesson.steps[self.learn.step];
                                let r = self.resolve_targets(&Self::step_targets(&step));
                                self.lesson_hints(&r)
                            }
                            LearnView::Ear => self.ear_hints(),
                        };
                        let label = self.cap_label();
                        KeyboardView {
                            keymap: &self.keymap,
                            palette: &p,
                            held: &self.held,
                            selected: None,
                            show_notes: true,
                            label: &label,
                            hints: &hints,
                        }
                        .show(ui);
                    });
                });
            });
    }

    fn lesson_list(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut open: Option<&'static Lesson> = None;
        card(ui, &p, "", |ui| {
            for ch in CHAPTERS {
                ui.label(RichText::new(ch.title).strong().color(p.primary));
                for l in ch.lessons {
                    let current = l.id == self.learn.lesson.id;
                    let done = self.cfg.learn.done.contains(l.id);
                    let mark = if done { "✓ " } else { "○ " };
                    let text = RichText::new(format!("{mark}{}", l.title)).color(if current {
                        p.on_secondary_container
                    } else if done {
                        p.on_surface_variant
                    } else {
                        p.on_surface
                    });
                    let btn = egui::Button::new(text)
                        .fill(if current {
                            p.secondary_container
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(egui::CornerRadius::same(10))
                        .min_size(egui::vec2(ui.available_width(), 0.0));
                    if ui.add(btn).on_hover_text(l.summary).clicked() && !current {
                        open = Some(l);
                    }
                }
                ui.add_space(4.0);
            }
        });
        if let Some(l) = open {
            self.open_lesson(l);
        }
    }

    fn lesson_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let lesson = self.learn.lesson;
        let step = lesson.steps[self.learn.step];
        let targets = Self::step_targets(&step);
        let resolved = self.resolve_targets(&targets);

        // Auto-play examples when a Listen / Interval step opens.
        let here = (lesson.id, self.learn.step);
        if self.learn.auto_played != Some(here) {
            self.learn.auto_played = Some(here);
            if matches!(step, Step::Listen { .. } | Step::Interval { .. })
                && let Some((m, gap)) = self.step_sound(&step)
            {
                self.play_moments(&m, gap);
            }
        }

        let mut action: Option<&str> = None;
        card(ui, &p, "", |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(lesson.title)
                        .size(20.0)
                        .strong()
                        .color(p.primary),
                );
                ui.label(RichText::new(lesson.summary).color(p.on_surface_variant));
            });
            let n = lesson.steps.len();
            ui.add(
                egui::ProgressBar::new(if self.learn.lesson_done {
                    1.0
                } else {
                    self.learn.step as f32 / n as f32
                })
                .desired_height(6.0)
                .fill(p.primary),
            );
            ui.label(
                RichText::new(format!("Step {} of {n}", self.learn.step + 1))
                    .small()
                    .color(p.on_surface_variant),
            );
            ui.add_space(4.0);

            // Banners: wrong key / layout / key.
            let tonic = self.tonic_pc();
            let off_key = tonic != 0 || self.cfg.scale != Scale::Major;
            let has_chord_keys = self.keymap.has_kind(crate::keymap::ActionKind::Chord);
            if resolved.iter().any(|r| !r.ok) {
                banner(
                    ui,
                    &p,
                    p.error,
                    "Your current layout can't play this step.",
                    |ui| {
                        let (file, name) = BUILTIN_LAYOUTS
                            .iter()
                            .find(|(f, _)| *f == lesson.layout.file())
                            .copied()
                            .unwrap_or(BUILTIN_LAYOUTS[0]);
                        if ui.button(format!("Switch to {name}")).clicked() {
                            action = Some(file);
                        }
                    },
                );
            } else if lesson.layout == lessons::Layout::Chords
                && !has_chord_keys
                && !targets.is_empty()
                && targets.iter().any(|t| matches!(t, Target::Chord(_)))
            {
                banner(
                    ui,
                    &p,
                    p.tertiary,
                    "Tip: in Chords + Melody each chord is one key. Here you play its notes.",
                    |ui| {
                        if ui.button("Switch to Chords + Melody").clicked() {
                            action = Some("chords");
                        }
                    },
                );
            }
            if off_key {
                banner(
                    ui,
                    &p,
                    p.tertiary,
                    &format!(
                        "Lessons are written in C major; you're in {}.",
                        self.key_name()
                    ),
                    |ui| {
                        if ui.button("Set key to C major").clicked() {
                            action = Some("key");
                        }
                    },
                );
            }

            if self.learn.lesson_done {
                ui.label(
                    RichText::new("Lesson complete!")
                        .size(22.0)
                        .strong()
                        .color(p.tertiary),
                );
                if let Some(next) = lessons::next_after(lesson.id) {
                    ui.label(format!("Next up: {} · {}", next.title, next.summary));
                    if ui.button(format!("▶  {}  (F3)", next.title)).clicked() {
                        action = Some("next-lesson");
                    }
                } else {
                    ui.label("That was the last lesson. Practise with the ear-training games, or go and play!");
                }
                if ui.button("Do it again").clicked() {
                    action = Some("restart");
                }
                return;
            }

            // The step itself, with the circle of fifths beside it in circle
            // lessons (so the keyboard stays on screen).
            let circle_w = if lesson.circle { 210.0 } else { 0.0 };
            let text_w = self.learn.mid_w - 40.0 - circle_w;
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    match step {
                        Step::Read(t) => {
                            ui.label(RichText::new(self.expand_text(t)).size(15.0));
                        }
                        Step::Listen { text, .. } => {
                            ui.label(RichText::new(self.expand_text(text)).size(15.0));
                        }
                        Step::Play { text, done, .. } => {
                            ui.label(RichText::new(self.expand_text(text)).size(15.0));
                            self.target_line(ui, &resolved, text_w);
                            if self.learn.step_done {
                                ui.label(
                                    RichText::new(format!("✓ {}", self.expand_text(done)))
                                        .color(p.tertiary),
                                );
                            }
                        }
                        Step::Interval { semitones, .. } => {
                            let iv = interval(semitones);
                            ui.label(RichText::new(iv.name).size(22.0).strong());
                            ui.label(
                                RichText::new(format!(
                                    "{} half step{} · {}",
                                    iv.semitones,
                                    if iv.semitones == 1 { "" } else { "s" },
                                    iv.mood
                                ))
                                .color(p.tertiary),
                            );
                            ui.label(format!(
                                "Sounds like the start of: {}",
                                iv.songs.join(" · ")
                            ));
                            ui.label(
                                RichText::new("Now play it: the first note, then the second.")
                                    .size(15.0),
                            );
                            self.target_line(ui, &resolved, text_w);
                            if self.learn.step_done {
                                ui.label(
                                    RichText::new(format!(
                                        "✓ That's a {}.",
                                        iv.name.to_lowercase()
                                    ))
                                    .color(p.tertiary),
                                );
                            }
                        }
                    }
                    if let Some((w, _)) = &self.learn.wrong {
                        ui.label(RichText::new(w).color(p.error));
                    }
                });
                if lesson.circle {
                    self.circle_widget(ui, circle_w - 10.0);
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.learn.step > 0, egui::Button::new("◀ Back"))
                    .clicked()
                {
                    action = Some("back");
                }
                let hear = match step {
                    Step::Read(_) => None,
                    Step::Play { .. } => Some("▶ Show me"),
                    _ => Some("▶ Hear it (F2)"),
                };
                if let Some(h) = hear
                    && ui.button(h).clicked()
                {
                    action = Some("hear");
                }
                let next_ok = self.can_go_next();
                let next = egui::Button::new(RichText::new("Next ▶ (F3)").color(if next_ok {
                    p.on_primary
                } else {
                    p.on_surface_variant
                }))
                .fill(if next_ok {
                    p.primary
                } else {
                    p.surface_container_high
                })
                .corner_radius(egui::CornerRadius::same(255));
                if ui.add_enabled(next_ok, next).clicked() {
                    action = Some("next");
                }
                if !next_ok
                    && ui
                        .small_button("Skip")
                        .on_hover_text("Skip this step")
                        .clicked()
                {
                    action = Some("skip");
                }
            });
        });

        match action {
            Some("back") => self.go_step(self.learn.step.saturating_sub(1)),
            Some("next") | Some("skip") => self.go_step(self.learn.step + 1),
            Some("hear") => {
                if let Some((m, gap)) = self.step_sound(&step) {
                    self.play_moments(&m, gap);
                }
            }
            Some("next-lesson") => {
                if let Some(n) = lessons::next_after(lesson.id) {
                    self.open_lesson(n);
                }
            }
            Some("restart") => self.open_lesson(lesson),
            Some("key") => {
                self.cfg.transpose = 0;
                self.cfg.scale = Scale::Major;
                self.mark_dirty();
                self.refresh_chord();
            }
            Some(file) => {
                self.switch_keymap(file);
                self.notify(format!("Layout: {}", self.keymap.name));
            }
            None => {}
        }
    }

    /// The circle of fifths for the current key and chord.
    fn circle_widget(&self, ui: &mut egui::Ui, size: f32) {
        let chord_root = self
            .learn
            .overlay
            .as_ref()
            .filter(|_| self.learn.overlay_live)
            .and_then(|e| e.step)
            .map(|(off, f)| ((off + self.tonic_pc()).rem_euclid(12), f));
        circle_of_fifths(
            ui,
            &self.palette,
            size,
            self.tonic_pc(),
            analysis::minor_home(self.cfg.scale),
            chord_root,
        );
    }

    /// "Keys: Z → X → C" with the done ones ticked.
    fn target_line(&self, ui: &mut egui::Ui, resolved: &[Resolved], width: f32) {
        let p = self.palette;
        // egui can't size a chip before drawing it, so track the row width.
        let budget = width;
        let mut line_w = 50.0;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Keys:").color(p.on_surface_variant));
            for (i, r) in resolved.iter().enumerate() {
                // Chips can't be measured before they're drawn, so wrap by hand.
                let keys_w: Vec<&str> = r.keys.iter().map(|k| k.label()).collect();
                let w = ui
                    .painter()
                    .layout_no_wrap(
                        format!("✓ {} {}", keys_w.join("+"), r.name),
                        egui::FontId::proportional(13.5),
                        p.on_surface,
                    )
                    .size()
                    .x
                    + 48.0;
                if line_w + w > budget && i > 0 {
                    ui.end_row();
                    line_w = 0.0;
                }
                line_w += w + 22.0;
                if i > 0 {
                    ui.label(RichText::new("→").color(p.outline));
                }
                let keys: Vec<&str> = r.keys.iter().map(|k| k.label()).collect();
                let keys = if keys.is_empty() {
                    "?".to_string()
                } else {
                    keys.join("+")
                };
                let text = format!("{keys} {}", r.name);
                let done = i < self.learn.progress || self.learn.step_done;
                let now = i == self.learn.progress && !self.learn.step_done;
                let (fill, fg) = if done {
                    (p.surface_container_high, p.outline)
                } else if now {
                    (p.tertiary_container, p.on_surface)
                } else {
                    (p.surface_container_high, p.on_surface_variant)
                };
                chip(ui, fill, fg, if done { format!("✓ {text}") } else { text });
            }
        });
    }

    fn game_list(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut pick: Option<(Game, usize)> = None;
        card(ui, &p, "", |ui| {
            for g in Game::ALL {
                let sel = self.learn.game == g;
                let best = self.cfg.learn.best.get(g.code()).copied().unwrap_or(0);
                let btn = egui::Button::new(
                    RichText::new(format!(
                        "{}{}",
                        g.name(),
                        if best > 0 {
                            format!("   best {best}")
                        } else {
                            String::new()
                        }
                    ))
                    .color(if sel {
                        p.on_secondary_container
                    } else {
                        p.on_surface
                    }),
                )
                .fill(if sel {
                    p.secondary_container
                } else {
                    Color32::TRANSPARENT
                })
                .corner_radius(egui::CornerRadius::same(10))
                .min_size(egui::vec2(ui.available_width(), 0.0));
                if ui.add(btn).on_hover_text(g.blurb()).clicked() && !sel {
                    pick = Some((g, self.ear_level(g)));
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Level").strong().color(p.primary));
            let g = self.learn.game;
            let cur = self.ear_level(g);
            for (i, name) in g.level_names().iter().enumerate() {
                if ui.radio(cur == i, *name).clicked() && cur != i {
                    pick = Some((g, i));
                }
            }
        });
        if let Some((g, l)) = pick {
            self.stop_player();
            self.set_game(g, l);
        }
    }

    fn game_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let game = self.learn.game;
        let mut action: Option<&str> = None;
        card(ui, &p, "", |ui| {
            ui.label(
                RichText::new(game.name())
                    .size(20.0)
                    .strong()
                    .color(p.primary),
            );
            ui.label(RichText::new(game.blurb()).color(p.on_surface_variant));
            ui.add_space(4.0);
            if !self.learn.started {
                ui.label("Answer by playing on your keyboard. Lit keys show where to look.");
                let btn = egui::Button::new(
                    RichText::new("▶  Start (F2)")
                        .size(16.0)
                        .color(p.on_primary),
                )
                .fill(p.primary)
                .corner_radius(egui::CornerRadius::same(255));
                if ui.add(btn).clicked() {
                    action = Some("start");
                }
                return;
            }
            if let Some(e) = &self.learn.q_error {
                banner(ui, &p, p.error, e, |ui| {
                    if ui.button("Switch to Piano").clicked() {
                        action = Some("piano");
                    }
                });
                return;
            }
            if let Some(q) = &self.learn.question {
                ui.label(RichText::new(&q.prompt).size(15.0));
                if self.learn.answered.is_some() {
                    let first = !self.learn.missed;
                    ui.label(
                        RichText::new(if first { "✓ Right!" } else { "✓ Got it." })
                            .size(20.0)
                            .strong()
                            .color(p.tertiary),
                    );
                    ui.label(&q.reveal);
                } else if let Some((w, _)) = &self.learn.wrong {
                    ui.label(RichText::new(w).color(p.error));
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("▶ Hear again (F2)").clicked() {
                    action = Some("hear");
                }
                let label = if self.learn.answered.is_some() {
                    "Next ▶ (F3)"
                } else {
                    "Skip (F3)"
                };
                if ui.button(label).clicked() {
                    action = Some("next");
                }
            });
            ui.add_space(4.0);
            let s = &self.learn.score;
            ui.horizontal_wrapped(|ui| {
                chip(
                    ui,
                    p.surface_container_high,
                    p.on_surface,
                    format!("Right first try: {}/{}", s.right_first_try, s.asked),
                );
                chip(
                    ui,
                    p.primary_container,
                    p.on_primary_container,
                    format!("Streak {}", s.streak),
                );
                let best = self.cfg.learn.best.get(game.code()).copied().unwrap_or(0);
                chip(
                    ui,
                    p.surface_container_high,
                    p.on_surface,
                    format!("Best streak {best}"),
                );
            });
        });
        match action {
            Some("start") => {
                self.learn.started = true;
                self.new_question();
            }
            Some("hear") => self.learn_replay(),
            Some("next") => self.learn_next(),
            Some("piano") => {
                self.switch_keymap("piano");
                self.notify("Layout: Piano");
                self.new_question();
            }
            _ => {}
        }
    }
}

/// `{c:…}` placeholders need a `&'static str` numeral; the few numerals in
/// lesson text are matched against a fixed list.
fn leak_numeral(n: &str) -> &'static str {
    const ALL: [&str; 19] = [
        "I", "ii", "iii", "IV", "V", "vi", "vii°", "i", "II", "bII", "bIII", "III", "iv", "v",
        "bVI", "VI", "bVII", "VII", "vii",
    ];
    ALL.iter().find(|x| **x == n).copied().unwrap_or("I")
}

/// A coloured note with a button area.
fn banner(
    ui: &mut egui::Ui,
    p: &Palette,
    color: Color32,
    text: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(p.surface_container_high)
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(p.on_surface));
                add(ui);
            });
        });
    ui.add_space(4.0);
}

/// Rest ←→ Tension meter.
pub(super) fn tension_bar(ui: &mut egui::Ui, p: &Palette, t: f32) {
    let t = t.clamp(0.0, 1.0);
    let width = ui.available_width().min(320.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 44.0, rect.center().y - 3.0),
        egui::pos2(rect.right() - 56.0, rect.center().y + 3.0),
    );
    let font = egui::FontId::proportional(11.0);
    painter.text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Rest",
        font.clone(),
        p.on_surface_variant,
    );
    painter.text(
        egui::pos2(rect.right(), rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "Tension",
        font,
        p.on_surface_variant,
    );
    painter.rect_filled(
        bar,
        egui::CornerRadius::same(3),
        p.surface_container_highest,
    );
    let mix = |a: Color32, b: Color32, k: f32| {
        let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
        Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
    };
    let filled =
        egui::Rect::from_min_max(bar.min, egui::pos2(bar.left() + bar.width() * t, bar.max.y));
    let color = mix(p.primary, p.error, t);
    painter.rect_filled(filled, egui::CornerRadius::same(3), color);
    painter.circle_filled(egui::pos2(filled.right(), bar.center().y), 6.0, color);
}
