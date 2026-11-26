//! Practising a song: what to play (targets), matching key presses to them,
//! timing windows, wait mode, one-key mode, loops and scoring.
//!
//! All times here are **song seconds** as you *hear* them: the app takes the
//! audio clock and subtracts the output delay (Bluetooth!) before asking, so
//! the falling notes, the scoring and the sound you hear all line up.

use super::{ChordSpan, Song};
use std::collections::HashSet;

/// What one target asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Want {
    /// A note (MIDI pitch, already moved by the part's octave shift).
    Note(i32),
    /// A chord: root pitch class and its pitch classes (sorted).
    Chord {
        root_pc: i32,
        pcs: Vec<i32>,
        symbol: String,
    },
    /// A drum hit (General MIDI drum note).
    Drum(u8),
}

/// Which lane of the falling-notes view a target is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Notes,
    Chords,
}

#[derive(Clone, Debug)]
pub struct Target {
    // (`end` and `part` are kept for the note editor / later parts.)
    pub start: f64,
    #[allow(dead_code)]
    pub end: f64,
    pub start_beat: f64,
    pub end_beat: f64,
    pub want: Want,
    pub lane: Lane,
    pub low_confidence: bool,
    /// Index into the song's parts (None = the chord sheet).
    #[allow(dead_code)]
    pub part: Option<usize>,
    pub velocity: u8,
    /// The current keymap has a key for it (else it's shown but not asked).
    pub playable: bool,
}

/// How well a target was hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Judgement {
    Perfect,
    Good,
    Miss,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub judgement: Judgement,
    /// Seconds early (−) or late (+), in real time.
    pub offset: f64,
}

/// What the user just played.
#[derive(Clone, Debug, PartialEq)]
pub enum Played {
    Note(i32),
    /// A chord key: root and pitch classes.
    Chord {
        root_pc: i32,
        pcs: Vec<i32>,
    },
    Drum(u8),
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Stop and wait for the right keys instead of moving on.
    pub wait: bool,
    /// A note in any octave counts (the laptop keyboard only has ~2 octaves).
    pub any_octave: bool,
    /// G counts for G7 (the plain chord under a fancier one).
    pub lenient_chords: bool,
    /// Perfect if within this many seconds (real time).
    pub perfect: f64,
    /// Good if within this.
    pub good: f64,
    /// Extra time allowed for late presses: Part 2's Bluetooth grace window
    /// (you react to sound that reaches you late).
    pub late_extra: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wait: false,
            any_octave: true,
            lenient_chords: true,
            perfect: 0.06,
            good: 0.15,
            late_extra: 0.0,
        }
    }
}

impl Settings {
    /// Windows for an output delay (ms) and Part 2's grace window (ms). The
    /// delay itself is already taken off the clock; what's left is how
    /// unsure that measurement is, so the windows widen a little with it.
    pub fn with_latency(mut self, delay_ms: f32, grace_ms: f32) -> Settings {
        let d = delay_ms.max(0.0) as f64 / 1000.0;
        self.perfect = 0.06 + d * 0.1;
        self.good = 0.15 + d * 0.25;
        // Grace = 80 ms + 0.8 × delay (max 320 ms); the 80 ms base is
        // already inside `good`.
        self.late_extra = ((grace_ms as f64 - 80.0) / 1000.0).max(0.0);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Score {
    pub perfect: u32,
    pub good: u32,
    pub missed: u32,
    /// Keys pressed that matched nothing.
    pub wrong: u32,
    pub streak: u32,
    pub best_streak: u32,
    pub points: u32,
}

impl Score {
    fn add(&mut self, j: Judgement) {
        match j {
            Judgement::Perfect => {
                self.perfect += 1;
                self.streak += 1;
                self.points += 100 + 2 * self.streak.min(50);
            }
            Judgement::Good => {
                self.good += 1;
                self.streak += 1;
                self.points += 60 + self.streak.min(50);
            }
            Judgement::Miss => {
                self.missed += 1;
                self.streak = 0;
            }
        }
        self.best_streak = self.best_streak.max(self.streak);
    }

    pub fn judged(&self) -> u32 {
        self.perfect + self.good + self.missed
    }

    /// 0–100. Good hits count 70 %, wrong keys cost a little.
    pub fn accuracy(&self) -> f32 {
        let n = self.judged();
        if n == 0 {
            return 0.0;
        }
        let got = self.perfect as f32 + 0.7 * self.good as f32;
        let penalty = 0.25 * self.wrong as f32;
        ((got - penalty).max(0.0) / n as f32 * 100.0).clamp(0.0, 100.0)
    }

    /// A word for the result screen.
    pub fn grade(&self) -> &'static str {
        match self.accuracy() {
            a if a >= 95.0 => "Perfect run!",
            a if a >= 85.0 => "Great",
            a if a >= 70.0 => "Good",
            a if a >= 50.0 => "Getting there",
            _ => "Keep practising",
        }
    }
}

/// Targets that start together (a chord written as notes, melody + chord).
#[derive(Clone, Debug)]
pub struct Group {
    pub start: f64,
    pub targets: Vec<usize>,
}

/// Notes closer than this start "together".
const TOGETHER: f64 = 0.045;

pub struct Practice {
    pub targets: Vec<Target>,
    pub groups: Vec<Group>,
    pub results: Vec<Option<Hit>>,
    pub settings: Settings,
    /// The whole run so far.
    pub score: Score,
    /// The current pass of the loop (or the whole song without a loop).
    pub pass: Score,
    /// Earliest group that isn't finished (wait / one-key mode).
    cursor: usize,
}

/// What a key press did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Hit(Vec<(usize, Hit)>),
    /// Didn't match anything near now.
    Wrong,
    /// Part of a chord-of-notes that isn't complete yet.
    Partial,
}

impl Practice {
    pub fn new(mut targets: Vec<Target>, settings: Settings) -> Practice {
        targets.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        let mut groups: Vec<Group> = Vec::new();
        for (i, t) in targets.iter().enumerate() {
            if !t.playable {
                continue;
            }
            match groups.last_mut() {
                Some(g) if t.start - g.start < TOGETHER => g.targets.push(i),
                _ => groups.push(Group {
                    start: t.start,
                    targets: vec![i],
                }),
            }
        }
        let n = targets.len();
        Practice {
            targets,
            groups,
            results: vec![None; n],
            settings,
            score: Score::default(),
            pass: Score::default(),
            cursor: 0,
        }
    }

    pub fn playable_count(&self) -> usize {
        self.targets.iter().filter(|t| t.playable).count()
    }

    fn group_done(&self, g: usize) -> bool {
        self.groups[g]
            .targets
            .iter()
            .all(|&i| self.results[i].is_some())
    }

    fn advance_cursor(&mut self) {
        while self.cursor < self.groups.len() && self.group_done(self.cursor) {
            self.cursor += 1;
        }
    }

    /// Where wait mode should stop the song: the start of the first group
    /// that still needs playing.
    pub fn wait_point(&self) -> Option<f64> {
        if !self.settings.wait {
            return None;
        }
        self.groups.get(self.cursor).map(|g| g.start)
    }

    /// The group one-key mode would play now: the waiting group in wait mode,
    /// else the earliest unfinished group that isn't past its window.
    pub fn next_group(&self, now: f64, speed: f64) -> Option<usize> {
        let late = (self.settings.good + self.settings.late_extra) * speed;
        (self.cursor..self.groups.len()).find(|&g| {
            !self.group_done(g) && (self.settings.wait || self.groups[g].start + late >= now)
        })
    }

    /// Targets due soon, for lighting keys: (target, seconds until it starts).
    pub fn upcoming(&self, now: f64, horizon: f64) -> Vec<(usize, f64)> {
        let from = self.cursor;
        let mut out = Vec::new();
        for g in from..self.groups.len() {
            let gs = self.groups[g].start;
            if gs > now + horizon {
                break;
            }
            for &i in &self.groups[g].targets {
                if self.results[i].is_none() {
                    out.push((i, gs - now));
                }
            }
        }
        out
    }

    fn judge(&self, offset_real: f64) -> Option<Judgement> {
        let s = &self.settings;
        let late_ok = s.good + s.late_extra;
        if offset_real.abs() <= s.perfect {
            Some(Judgement::Perfect)
        } else if (-s.good..=late_ok).contains(&offset_real) {
            Some(Judgement::Good)
        } else {
            None
        }
    }

    fn matches(&self, want: &Want, played: &Played, held_pcs: &[i32]) -> bool {
        match (want, played) {
            (Want::Note(p), Played::Note(n)) => {
                *p == *n || self.settings.any_octave && (p - n).rem_euclid(12) == 0
            }
            (Want::Drum(d), Played::Drum(x)) => d == x,
            (
                Want::Chord { root_pc, pcs, .. },
                Played::Chord {
                    root_pc: r,
                    pcs: got,
                },
            ) => *root_pc == *r && chord_fits(pcs, got, self.settings.lenient_chords),
            // A chord played as separate notes: the held notes cover it.
            (Want::Chord { pcs, .. }, Played::Note(_)) => {
                chord_fits(pcs, held_pcs, self.settings.lenient_chords)
            }
            _ => false,
        }
    }

    /// A key press at song time `now` (as heard). `held_pcs` are the pitch
    /// classes of every note held right now (for chords played note by
    /// note). `halted`: wait mode is holding the song at the next notes.
    pub fn press(
        &mut self,
        played: &Played,
        now: f64,
        speed: f64,
        held_pcs: &[i32],
        halted: bool,
    ) -> Outcome {
        let speed = speed.max(0.05);
        let s = &self.settings;
        let reach_early = s.good.max(if s.wait { 0.35 } else { 0.0 });
        let reach_late = s.good + s.late_extra;
        // Candidate: unhit, playable, matching, closest in time.
        let mut best: Option<(usize, f64)> = None;
        for (i, t) in self.targets.iter().enumerate() {
            if self.results[i].is_some() || !t.playable {
                continue;
            }
            let off = (now - t.start) / speed;
            if off > reach_late + 0.5 {
                continue;
            }
            if t.start > now + 4.0 {
                break;
            }
            let waiting_here = s.wait
                && self
                    .groups
                    .get(self.cursor)
                    .is_some_and(|g| g.targets.contains(&i));
            let in_reach = (-reach_early..=reach_late).contains(&off) || waiting_here && halted;
            if !in_reach || !self.matches(&t.want, played, held_pcs) {
                continue;
            }
            if best.is_none_or(|(_, b)| off.abs() < b.abs()) {
                best = Some((i, off));
            }
        }
        let Some((i, off)) = best else {
            // Holding part of a chord isn't "wrong".
            let partial = matches!(played, Played::Note(_))
                && self.targets.iter().enumerate().any(|(i, t)| {
                    self.results[i].is_none()
                        && matches!(&t.want, Want::Chord { pcs, .. } if pcs.iter().any(|p| held_pcs.contains(p)))
                        && ((now - t.start) / speed).abs() < reach_late.max(0.35)
                });
            if partial {
                return Outcome::Partial;
            }
            self.score.wrong += 1;
            self.pass.wrong += 1;
            return Outcome::Wrong;
        };
        let judgement = if self.settings.wait && halted {
            // Waiting: no rush, getting it right is what counts.
            Judgement::Perfect
        } else {
            self.judge(off).unwrap_or(Judgement::Good)
        };
        let hit = Hit {
            judgement,
            offset: off,
        };
        self.record(i, hit);
        Outcome::Hit(vec![(i, hit)])
    }

    fn record(&mut self, i: usize, hit: Hit) {
        if self.results[i].is_some() {
            return;
        }
        self.results[i] = Some(hit);
        self.score.add(hit.judgement);
        self.pass.add(hit.judgement);
        self.advance_cursor();
    }

    /// One-key mode: the whole group counts as played at `now`.
    pub fn hit_group(&mut self, g: usize, now: f64, speed: f64, halted: bool) -> Vec<(usize, Hit)> {
        let Some(group) = self.groups.get(g).cloned() else {
            return vec![];
        };
        let off = (now - group.start) / speed.max(0.05);
        let judgement = if self.settings.wait && halted {
            Judgement::Perfect
        } else {
            self.judge(off).unwrap_or(Judgement::Good)
        };
        let mut out = Vec::new();
        for i in group.targets {
            if self.results[i].is_none() {
                let hit = Hit {
                    judgement,
                    offset: off,
                };
                self.record(i, hit);
                out.push((i, hit));
            }
        }
        out
    }

    /// Mark targets whose time has passed as missed. Not in wait mode.
    pub fn update(&mut self, now: f64, speed: f64) -> Vec<usize> {
        if self.settings.wait {
            return vec![];
        }
        let late = (self.settings.good + self.settings.late_extra) * speed.max(0.05);
        let mut missed = Vec::new();
        for i in 0..self.targets.len() {
            let t = &self.targets[i];
            if t.start > now {
                break;
            }
            if t.playable && self.results[i].is_none() && t.start + late < now {
                missed.push(i);
            }
        }
        for &i in &missed {
            self.record(
                i,
                Hit {
                    judgement: Judgement::Miss,
                    offset: 0.0,
                },
            );
        }
        missed
    }

    /// Start again from `t0` (loop restart, rewind): forget results from
    /// there on and start a new pass. Returns the pass that just ended.
    pub fn restart_from(&mut self, t0: f64, t1: Option<f64>) -> Score {
        for (i, t) in self.targets.iter().enumerate() {
            if t.start >= t0 - 1e-6 && t1.is_none_or(|e| t.start < e) {
                self.results[i] = None;
            }
        }
        self.cursor = self
            .groups
            .iter()
            .position(|g| g.start >= t0 - 1e-6)
            .unwrap_or(self.groups.len());
        // Groups before t0 that were never played don't hold wait mode.
        std::mem::take(&mut self.pass)
    }

    /// Fresh scores (new song, settings changed).
    pub fn reset(&mut self) {
        self.results.iter_mut().for_each(|r| *r = None);
        self.score = Score::default();
        self.pass = Score::default();
        self.cursor = 0;
    }
}

/// Does `got` (pitch classes played) cover the chord `want`?
pub fn chord_fits(want: &[i32], got: &[i32], lenient: bool) -> bool {
    if want.is_empty() {
        return false;
    }
    let got: HashSet<i32> = got.iter().map(|p| p.rem_euclid(12)).collect();
    if lenient {
        // The root, third and fifth are enough (G for G7).
        want.iter().take(3).all(|p| got.contains(p))
    } else {
        want.iter().all(|p| got.contains(p)) && got.len() <= want.len() + 1
    }
}

/// Targets for the chord sheet (skipping "no chord").
pub fn chord_targets(chords: &[ChordSpan], playable: impl Fn(i32, &[i32]) -> bool) -> Vec<Target> {
    chords
        .iter()
        .filter_map(|c| {
            let (root, kind) = super::chords::span_kind(c)?;
            let pcs: Vec<i32> = kind.intervals().iter().map(|t| (root + t) % 12).collect();
            Some(Target {
                start: c.start,
                end: c.end,
                start_beat: c.start_beat,
                end_beat: c.end_beat,
                playable: playable(root, &pcs),
                want: Want::Chord {
                    root_pc: root,
                    pcs,
                    symbol: c.symbol.clone(),
                },
                lane: Lane::Chords,
                low_confidence: c.low_confidence,
                part: None,
                velocity: 90,
            })
        })
        .collect()
}

/// Targets for one part. `shift` moves notes by octaves to fit the keys.
pub fn part_targets(
    song: &Song,
    part: usize,
    shift: i32,
    playable: impl Fn(i32) -> bool,
) -> Vec<Target> {
    let p = &song.parts[part];
    p.notes
        .iter()
        .map(|n| {
            let want = if p.is_drum {
                Want::Drum(n.pitch)
            } else {
                Want::Note(n.pitch as i32 + shift)
            };
            Target {
                start: n.start,
                end: n.end,
                start_beat: n.start_beat,
                end_beat: n.end_beat,
                playable: match want {
                    Want::Note(x) => playable(x),
                    Want::Drum(d) => playable(d as i32),
                    _ => false,
                },
                want,
                lane: Lane::Notes,
                low_confidence: n.low_confidence,
                part: Some(part),
                velocity: n.velocity,
            }
        })
        .collect()
}

/// The octave shift (multiple of 12) that puts most of `pitches` on keys
/// the layout has; ties go to the smallest shift.
pub fn best_octave_shift(pitches: &[i32], has_key: impl Fn(i32) -> bool) -> i32 {
    if pitches.is_empty() {
        return 0;
    }
    let mut best = (0usize, 0i32);
    for k in [0, -1, 1, -2, 2, -3, 3] {
        let shift = k * 12;
        let n = pitches.iter().filter(|p| has_key(*p + shift)).count();
        if n > best.0 {
            best = (n, shift);
        }
    }
    best.1
}

/// Song clock as heard: turns the audio thread's position into what you're
/// hearing right now, using the output delay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockSnapshot {
    /// Song position at `at` (seconds).
    pub pos: f64,
    /// Seconds since `at` (wall clock).
    pub since: f64,
    pub speed: f64,
    /// The song is moving (not paused / waiting / ended).
    pub moving: bool,
}

/// Song time you hear now. While moving: the position minus the delay. When
/// it has just stopped (wait mode), the sound before the stop is still on
/// its way, so the time keeps moving until it reaches the stop.
pub fn heard_time(c: ClockSnapshot, delay_s: f64) -> f64 {
    let d = c.since - delay_s;
    if c.moving {
        c.pos + d * c.speed
    } else {
        c.pos + d.min(0.0) * c.speed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f64, pitch: i32) -> Target {
        Target {
            start,
            end: start + 0.4,
            start_beat: start * 2.0,
            end_beat: start * 2.0 + 0.8,
            want: Want::Note(pitch),
            lane: Lane::Notes,
            low_confidence: false,
            part: Some(0),
            velocity: 90,
            playable: true,
        }
    }

    fn chord(start: f64, root: i32, pcs: &[i32]) -> Target {
        Target {
            want: Want::Chord {
                root_pc: root,
                pcs: pcs.to_vec(),
                symbol: "X".into(),
            },
            lane: Lane::Chords,
            part: None,
            ..note(start, 0)
        }
    }

    #[test]
    fn timing_windows() {
        let mut p = Practice::new(
            vec![note(1.0, 60), note(2.0, 62), note(3.0, 64)],
            Settings::default(),
        );
        assert!(matches!(
            p.press(&Played::Note(60), 1.02, 1.0, &[], false),
            Outcome::Hit(ref h) if h[0].1.judgement == Judgement::Perfect
        ));
        assert!(matches!(
            p.press(&Played::Note(62), 2.12, 1.0, &[], false),
            Outcome::Hit(ref h) if h[0].1.judgement == Judgement::Good
        ));
        // Wrong note.
        assert_eq!(
            p.press(&Played::Note(61), 2.9, 1.0, &[], false),
            Outcome::Wrong
        );
        // The third is missed once its window passes.
        assert!(p.update(3.1, 1.0).is_empty());
        assert_eq!(p.update(3.5, 1.0), vec![2]);
        assert_eq!(p.score.perfect, 1);
        assert_eq!(p.score.good, 1);
        assert_eq!(p.score.missed, 1);
        assert_eq!(p.score.wrong, 1);
        assert_eq!(p.score.best_streak, 2);
        assert!(p.score.accuracy() > 40.0 && p.score.accuracy() < 60.0);
    }

    #[test]
    fn slower_speed_widens_the_window_in_song_time() {
        // At half speed 0.1 song-seconds late is 0.2 s real time: only Good.
        let mut p = Practice::new(vec![note(1.0, 60)], Settings::default());
        let Outcome::Hit(h) = p.press(&Played::Note(60), 1.05, 0.5, &[], false) else {
            panic!()
        };
        assert_eq!(h[0].1.judgement, Judgement::Good);
        assert!((h[0].1.offset - 0.1).abs() < 1e-9);
    }

    #[test]
    fn bluetooth_grace_lets_late_presses_count() {
        let s = Settings::default().with_latency(200.0, 80.0 + 0.8 * 200.0);
        assert!((s.late_extra - 0.16).abs() < 1e-6);
        let mut p = Practice::new(vec![note(1.0, 60)], s.clone());
        // 0.3 s late: outside the wired window, inside with the grace.
        assert!(matches!(
            p.press(&Played::Note(60), 1.3, 1.0, &[], false),
            Outcome::Hit(_)
        ));
        let mut wired = Practice::new(vec![note(1.0, 60)], Settings::default());
        assert_eq!(
            wired.press(&Played::Note(60), 1.3, 1.0, &[], false),
            Outcome::Wrong
        );
    }

    #[test]
    fn any_octave() {
        let mut p = Practice::new(vec![note(1.0, 72)], Settings::default());
        assert!(matches!(
            p.press(&Played::Note(60), 1.0, 1.0, &[], false),
            Outcome::Hit(_)
        ));
        let strict = Settings {
            any_octave: false,
            ..Default::default()
        };
        let mut p = Practice::new(vec![note(1.0, 72)], strict);
        assert_eq!(
            p.press(&Played::Note(60), 1.0, 1.0, &[], false),
            Outcome::Wrong
        );
    }

    #[test]
    fn wait_mode_waits_for_every_note_of_a_group() {
        let s = Settings {
            wait: true,
            ..Default::default()
        };
        let mut p = Practice::new(vec![note(1.0, 60), note(1.01, 64), note(2.0, 67)], s);
        assert_eq!(p.wait_point(), Some(1.0));
        // Nothing is ever missed while waiting.
        assert!(p.update(10.0, 1.0).is_empty());
        // Halted at 1.0: pressing late is fine.
        assert!(matches!(
            p.press(&Played::Note(60), 1.0, 1.0, &[], true),
            Outcome::Hit(_)
        ));
        assert_eq!(p.wait_point(), Some(1.0), "still needs the E");
        assert!(matches!(
            p.press(&Played::Note(64), 1.0, 1.0, &[], true),
            Outcome::Hit(_)
        ));
        assert_eq!(p.wait_point(), Some(2.0));
        assert_eq!(p.score.perfect, 2);
        // Pressing ahead (within reach) is allowed too.
        assert!(matches!(
            p.press(&Played::Note(67), 1.75, 1.0, &[], false),
            Outcome::Hit(_)
        ));
        assert_eq!(p.wait_point(), None);
    }

    #[test]
    fn chords_by_key_or_by_notes() {
        let g7 = [7, 11, 2, 5];
        let mut p = Practice::new(
            vec![chord(1.0, 7, &g7), chord(2.0, 0, &[0, 4, 7])],
            Settings::default(),
        );
        // A G chord key counts for G7 (lenient).
        assert!(matches!(
            p.press(
                &Played::Chord {
                    root_pc: 7,
                    pcs: vec![7, 11, 2]
                },
                1.0,
                1.0,
                &[],
                false
            ),
            Outcome::Hit(_)
        ));
        // C played as three notes: the first two are partial, the third hits.
        assert_eq!(
            p.press(&Played::Note(60), 2.0, 1.0, &[0], false),
            Outcome::Partial
        );
        assert_eq!(
            p.press(&Played::Note(64), 2.0, 1.0, &[0, 4], false),
            Outcome::Partial
        );
        assert!(matches!(
            p.press(&Played::Note(67), 2.0, 1.0, &[0, 4, 7], false),
            Outcome::Hit(_)
        ));
        assert!(chord_fits(&g7, &[7, 11, 2], true));
        assert!(!chord_fits(&g7, &[7, 11, 2], false));
    }

    #[test]
    fn one_key_mode_plays_the_next_group() {
        let mut p = Practice::new(
            vec![note(1.0, 60), note(1.0, 64), note(2.0, 67)],
            Settings::default(),
        );
        assert_eq!(p.next_group(0.5, 1.0), Some(0));
        let hits = p.hit_group(0, 1.01, 1.0, false);
        assert_eq!(hits.len(), 2);
        assert_eq!(p.next_group(1.1, 1.0), Some(1));
        // Too late for the second one: one-key skips to nothing.
        assert_eq!(p.next_group(3.0, 1.0), None);
    }

    #[test]
    fn loops_start_a_new_pass() {
        let mut p = Practice::new(vec![note(1.0, 60), note(2.0, 62)], Settings::default());
        p.press(&Played::Note(60), 1.0, 1.0, &[], false);
        p.update(5.0, 1.0);
        let pass = p.restart_from(0.5, Some(3.0));
        assert_eq!((pass.perfect, pass.missed), (1, 1));
        assert!(p.results.iter().all(|r| r.is_none()));
        assert_eq!(p.pass, Score::default());
        assert_eq!(p.score.perfect, 1, "the run total is kept");
    }

    #[test]
    fn unplayable_targets_are_skipped() {
        let mut t = note(1.0, 30);
        t.playable = false;
        let s = Settings {
            wait: true,
            ..Default::default()
        };
        let p = Practice::new(vec![t, note(2.0, 60)], s);
        assert_eq!(p.wait_point(), Some(2.0));
        assert_eq!(p.playable_count(), 1);
    }

    #[test]
    fn octave_shift_fits_the_keys() {
        // Keys for C3..G5 (48..79); a bass line around C2 moves up an octave.
        let has = |p: i32| (48..=79).contains(&p);
        assert_eq!(best_octave_shift(&[36, 38, 40, 43], has), 12);
        assert_eq!(best_octave_shift(&[60, 62], has), 0);
        assert_eq!(best_octave_shift(&[96, 98], has), -24);
    }

    #[test]
    fn heard_clock_follows_the_delay() {
        let c = ClockSnapshot {
            pos: 10.0,
            since: 0.5,
            speed: 1.0,
            moving: true,
        };
        assert!((heard_time(c, 0.2) - 10.3).abs() < 1e-9);
        // Half speed.
        assert!((heard_time(ClockSnapshot { speed: 0.5, ..c }, 0.2) - 10.15).abs() < 1e-9);
        // Stopped at 10.0 0.1 s ago with 0.25 s delay: still 0.15 s to go.
        let stopped = ClockSnapshot {
            pos: 10.0,
            since: 0.1,
            speed: 1.0,
            moving: false,
        };
        assert!((heard_time(stopped, 0.25) - 9.85).abs() < 1e-9);
        assert_eq!(
            heard_time(
                ClockSnapshot {
                    since: 2.0,
                    ..stopped
                },
                0.25
            ),
            10.0
        );
    }

    #[test]
    fn targets_from_the_clip() {
        let s = crate::song::tests::clip_song();
        let mel = s.parts.iter().position(|p| p.id == "melody").unwrap();
        let t = part_targets(&s, mel, -12, |p| p < 60);
        assert_eq!(t.len(), 48);
        assert!(t.iter().any(|x| x.playable) && t.iter().any(|x| !x.playable));
        assert!(matches!(t[0].want, Want::Note(p) if p == s.parts[mel].notes[0].pitch as i32 - 12));
        let c = chord_targets(&s.chords, |_, _| true);
        assert!(c.len() >= 10);
        assert!(c.iter().all(|x| x.lane == Lane::Chords));
    }
}
