//! Plays chords in the audio thread: block chords, strums, the arpeggiator
//! and auto-accompaniment patterns. Running here (not in the UI thread)
//! keeps the rhythm sample-accurate no matter what the window is doing.
//!
//! Channels: 1 = chords, 2 = bass, 9 = drums (General MIDI percussion).

use super::patterns::{BassNote, Part, Pattern};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const CH_CHORD: u8 = 1;
pub const CH_BASS: u8 = 2;
pub const CH_DRUMS: u8 = 9;

/// A voiced chord, fixed-size so it can be sent to the audio thread
/// without allocating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChordNotes {
    notes: [u8; 8],
    len: u8,
    /// Bass note, 0 = none.
    pub bass: u8,
}

impl ChordNotes {
    pub fn new(notes: &[i32], bass: Option<i32>) -> Self {
        let mut c = ChordNotes::default();
        for &n in notes.iter().take(8) {
            c.notes[c.len as usize] = n.clamp(1, 127) as u8;
            c.len += 1;
        }
        c.bass = bass.map(|b| b.clamp(1, 127) as u8).unwrap_or(0);
        c
    }
    pub fn notes(&self) -> &[u8] {
        &self.notes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArpMode {
    #[default]
    Up,
    Down,
    UpDown,
    Random,
}

impl ArpMode {
    pub const ALL: [ArpMode; 4] = [ArpMode::Up, ArpMode::Down, ArpMode::UpDown, ArpMode::Random];
    pub fn name(self) -> &'static str {
        match self {
            ArpMode::Up => "Up",
            ArpMode::Down => "Down",
            ArpMode::UpDown => "Up & down",
            ArpMode::Random => "Random",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    /// Notes sound while the key is held.
    Block,
    /// Like Block but the notes roll in low → high, like a guitar.
    Strum,
    Arp,
    Pattern,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleParams {
    pub kind: StyleKind,
    pub strum_ms: f32,
    pub arp_mode: ArpMode,
    pub arp_steps_per_beat: u32,
    pub arp_octaves: u32,
    /// Arp note length as a fraction of a step.
    pub gate: f32,
    pub bass: bool,
    pub drums: bool,
    /// Arp / pattern keep going after the chord key is released.
    pub latch: bool,
    /// A chord change this soon after a beat replaces that beat's chord, so
    /// pressing slightly late (e.g. reacting to Bluetooth-delayed sound)
    /// still lands on the beat.
    pub grace_ms: f32,
}

impl Default for StyleParams {
    fn default() -> Self {
        StyleParams {
            kind: StyleKind::Block,
            strum_ms: 30.0,
            arp_mode: ArpMode::Up,
            arp_steps_per_beat: 4,
            arp_octaves: 1,
            gate: 0.8,
            bass: true,
            drums: true,
            latch: true,
            grace_ms: 80.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqEvent {
    On { ch: u8, note: u8, vel: u8 },
    Off { ch: u8, note: u8 },
}

#[derive(Clone, Copy)]
struct Pending {
    at: u64,
    ch: u8,
    note: u8,
    /// 0 = note off.
    vel: u8,
    generation: u16,
}

const QUEUE_CAP: usize = 1024;

pub struct Sequencer {
    sample_rate: f64,
    tempo: f64,
    params: StyleParams,
    pattern: Option<Arc<Pattern>>,
    chord: Option<ChordNotes>,
    key_held: bool,
    pedal: bool,
    velocity: u8,
    running: bool,
    /// Next step to play (counts up forever; wrap with `% steps_per_bar`).
    step: u64,
    next_step_at: u64,
    /// The last step played, and when.
    last_step: Option<(u64, u64)>,
    queue: Vec<Pending>,
    /// Sounding notes on the chord (0) and bass (1) channels.
    sounding: [[bool; 128]; 2],
    generation: [[u16; 128]; 2],
    rng: u32,
}

fn slot(ch: u8) -> Option<usize> {
    match ch {
        CH_CHORD => Some(0),
        CH_BASS => Some(1),
        _ => None,
    }
}

impl Sequencer {
    pub fn new(sample_rate: u32) -> Self {
        Sequencer {
            sample_rate: sample_rate as f64,
            tempo: 100.0,
            params: StyleParams::default(),
            pattern: None,
            chord: None,
            key_held: false,
            pedal: false,
            velocity: 95,
            running: false,
            step: 0,
            next_step_at: 0,
            last_step: None,
            queue: Vec::with_capacity(QUEUE_CAP),
            sounding: [[false; 128]; 2],
            generation: [[0; 128]; 2],
            rng: 0x9E37_79B9,
        }
    }

    // ------------------------------------------------------------ queries

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn steps_per_beat(&self) -> u32 {
        match (self.params.kind, &self.pattern) {
            (StyleKind::Pattern, Some(p)) => p.steps_per_beat,
            (StyleKind::Arp, _) => self.params.arp_steps_per_beat.max(1),
            _ => 4,
        }
    }

    pub fn steps_per_bar(&self) -> u32 {
        match (self.params.kind, &self.pattern) {
            (StyleKind::Pattern, Some(p)) => p.steps_per_bar(),
            _ => 4 * self.steps_per_beat(),
        }
    }

    /// Step currently playing within the bar (for the beat display).
    pub fn current_step(&self) -> u32 {
        let spb = self.steps_per_bar() as u64;
        ((self.step + spb - 1) % spb) as u32
    }

    /// When the next thing will happen, so the renderer can split its
    /// buffer there.
    pub fn next_time(&self) -> Option<u64> {
        let q = self.queue.iter().map(|p| p.at).min();
        let s = self.running.then_some(self.next_step_at);
        match (q, s) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn samples_per_step(&self) -> u64 {
        let beat = self.sample_rate * 60.0 / self.tempo.clamp(30.0, 300.0);
        (beat / self.steps_per_beat() as f64).max(1.0) as u64
    }

    fn drums_active(&self) -> bool {
        self.params.kind == StyleKind::Pattern
            && self.params.drums
            && self.pattern.as_ref().is_some_and(|p| p.has_drums)
    }

    fn rhythmic(&self) -> bool {
        matches!(self.params.kind, StyleKind::Arp | StyleKind::Pattern)
    }

    // ----------------------------------------------------------- commands

    pub fn set_tempo(&mut self, bpm: f32) {
        self.tempo = bpm as f64;
    }

    pub fn set_pattern(
        &mut self,
        p: Option<Arc<Pattern>>,
        now: u64,
        out: &mut impl FnMut(SeqEvent),
    ) {
        self.pattern = p;
        if self.params.kind == StyleKind::Pattern && self.pattern.is_none() {
            self.stop(now, out);
        }
    }

    pub fn set_style(&mut self, p: StyleParams, now: u64, out: &mut impl FnMut(SeqEvent)) {
        let old = self.params;
        self.params = p;
        if old.kind != p.kind {
            let chord = self.chord;
            let keep = self.key_held || self.pedal;
            self.release_tonal(now, out);
            self.running = false;
            self.chord = None;
            if let Some(c) = chord
                && keep
            {
                let held = self.key_held;
                self.set_chord(Some(c), true, self.velocity, now, out);
                self.key_held = held;
            }
        } else if old.drums && !p.drums && self.chord.is_none() {
            self.running = false;
        }
        if !p.bass {
            self.release_channel(CH_BASS, out);
        }
    }

    pub fn set_pedal(&mut self, on: bool, now: u64, out: &mut impl FnMut(SeqEvent)) {
        self.pedal = on;
        if !on && !self.key_held {
            self.chord_released(now, out);
        }
    }

    /// Start or stop the rhythm (drums can run before any chord).
    pub fn set_running(&mut self, on: bool, now: u64, out: &mut impl FnMut(SeqEvent)) {
        if on {
            if !self.running && self.rhythmic() {
                self.start_clock(now);
            }
        } else {
            self.stop(now, out);
        }
    }

    /// Stop everything the sequencer is playing.
    pub fn stop(&mut self, _now: u64, out: &mut impl FnMut(SeqEvent)) {
        self.running = false;
        self.chord = None;
        self.key_held = false;
        self.last_step = None;
        self.release_channel(CH_CHORD, out);
        self.release_channel(CH_BASS, out);
        self.queue.clear();
    }

    /// Forget all state without sending note-offs (the synth was reset).
    pub fn reset(&mut self) {
        self.running = false;
        self.chord = None;
        self.key_held = false;
        self.last_step = None;
        self.queue.clear();
        self.sounding = [[false; 128]; 2];
    }

    fn start_clock(&mut self, now: u64) {
        self.running = true;
        self.step = 0;
        self.next_step_at = now;
        self.last_step = None;
    }

    /// A chord key was pressed (`Some`) or released (`None`). `retrigger`
    /// is true for a fresh key press, false when only a modifier changed.
    pub fn set_chord(
        &mut self,
        chord: Option<ChordNotes>,
        retrigger: bool,
        velocity: u8,
        now: u64,
        out: &mut impl FnMut(SeqEvent),
    ) {
        let Some(new) = chord else {
            self.key_held = false;
            if !self.pedal {
                self.chord_released(now, out);
            }
            return;
        };
        let old = self.chord;
        self.key_held = true;
        self.chord = Some(new);
        self.velocity = velocity;
        match self.params.kind {
            StyleKind::Block | StyleKind::Strum => {
                if retrigger || old.is_none() {
                    self.release_tonal(now, out);
                    let strum = if self.params.kind == StyleKind::Strum {
                        (self.params.strum_ms as f64 * self.sample_rate / 1000.0) as u64
                    } else {
                        0
                    };
                    if self.params.bass && new.bass > 0 {
                        self.note_on(CH_BASS, new.bass, self.vel(100), None, now, out);
                    }
                    for (i, &n) in new.notes().iter().enumerate() {
                        let v = self.vel(88);
                        if strum == 0 || i == 0 {
                            self.note_on(CH_CHORD, n, v, None, now, out);
                        } else {
                            self.push(Pending {
                                at: now + strum * i as u64,
                                ch: CH_CHORD,
                                note: n,
                                vel: v,
                                generation: 0,
                            });
                        }
                    }
                } else {
                    // Only the shape changed: move just the notes that differ.
                    let keep = |n: u8| new.notes().contains(&n);
                    for n in 0..128u8 {
                        if self.sounding[0][n as usize] && !keep(n) {
                            self.note_off(CH_CHORD, n, out);
                        }
                    }
                    self.queue
                        .retain(|p| !(p.ch == CH_CHORD && p.vel > 0 && !keep(p.note)));
                    for &n in new.notes() {
                        let pending = self.queue.iter().any(|p| p.ch == CH_CHORD && p.note == n);
                        if !self.sounding[0][n as usize] && !pending {
                            self.note_on(CH_CHORD, n, self.vel(88), None, now, out);
                        }
                    }
                    if self.params.bass && old.is_some_and(|o| o.bass != new.bass) && new.bass > 0 {
                        self.release_channel(CH_BASS, out);
                        self.note_on(CH_BASS, new.bass, self.vel(100), None, now, out);
                    }
                }
            }
            StyleKind::Arp | StyleKind::Pattern => {
                if self.params.kind == StyleKind::Pattern && self.pattern.is_none() {
                    return;
                }
                if !self.running || (old.is_none() && !self.drums_active()) {
                    // First chord: start right now, on beat 1.
                    self.release_tonal(now, out);
                    self.start_clock(now);
                } else if retrigger
                    && old != Some(new)
                    && let Some((st, at)) = self.last_step
                {
                    // Pressed just after a beat (e.g. reacting to delayed
                    // Bluetooth sound): replay the most recent step that had
                    // chord notes, with the new chord.
                    let grace = (self.params.grace_ms as f64 * self.sample_rate / 1000.0) as u64;
                    let sps = self.samples_per_step();
                    let mut k = 0u64;
                    while k <= st {
                        let at_k = at.saturating_sub(k * sps);
                        if now.saturating_sub(at_k) > grace {
                            break;
                        }
                        if self.step_has_tonal(st - k) {
                            self.release_tonal(now, out);
                            self.play_step(st - k, now, now - at_k, true, out);
                            break;
                        }
                        k += 1;
                    }
                }
            }
        }
    }

    fn step_has_tonal(&self, st: u64) -> bool {
        match (self.params.kind, &self.pattern) {
            (StyleKind::Arp, _) => true,
            (StyleKind::Pattern, Some(p)) => p.steps[(st % p.steps.len() as u64) as usize]
                .iter()
                .any(|h| !matches!(h.part, Part::Drum(_))),
            _ => false,
        }
    }

    fn chord_released(&mut self, now: u64, out: &mut impl FnMut(SeqEvent)) {
        if self.rhythmic() {
            if !self.params.latch {
                self.chord = None;
                if !self.drums_active() {
                    self.running = false;
                }
            }
        } else {
            self.chord = None;
            self.release_tonal(now, out);
        }
    }

    // ------------------------------------------------------------ playing

    fn vel(&self, base: u8) -> u8 {
        ((base as u32 * self.velocity as u32) / 95).clamp(1, 127) as u8
    }

    fn push(&mut self, p: Pending) {
        if self.queue.len() < QUEUE_CAP {
            self.queue.push(p);
        }
    }

    fn note_on(
        &mut self,
        ch: u8,
        note: u8,
        vel: u8,
        gate: Option<u64>,
        now: u64,
        out: &mut impl FnMut(SeqEvent),
    ) {
        let note = note.min(127);
        let Some(s) = slot(ch) else {
            out(SeqEvent::On { ch, note, vel });
            return;
        };
        if self.sounding[s][note as usize] {
            out(SeqEvent::Off { ch, note });
        }
        out(SeqEvent::On { ch, note, vel });
        self.sounding[s][note as usize] = true;
        let g = &mut self.generation[s][note as usize];
        *g = g.wrapping_add(1);
        let generation = *g;
        if let Some(len) = gate {
            self.push(Pending {
                at: now + len.max(1),
                ch,
                note,
                vel: 0,
                generation,
            });
        }
    }

    fn note_off(&mut self, ch: u8, note: u8, out: &mut impl FnMut(SeqEvent)) {
        if let Some(s) = slot(ch)
            && self.sounding[s][note as usize]
        {
            self.sounding[s][note as usize] = false;
            out(SeqEvent::Off { ch, note });
        }
    }

    fn release_channel(&mut self, ch: u8, out: &mut impl FnMut(SeqEvent)) {
        for n in 0..128u8 {
            self.note_off(ch, n, out);
        }
        self.queue.retain(|p| p.ch != ch);
    }

    fn release_tonal(&mut self, _now: u64, out: &mut impl FnMut(SeqEvent)) {
        self.release_channel(CH_CHORD, out);
        self.release_channel(CH_BASS, out);
    }

    /// Fire everything due at or before `now`.
    pub fn process(&mut self, now: u64, out: &mut impl FnMut(SeqEvent)) {
        // Scheduled offs / strum notes.
        let mut i = 0;
        while i < self.queue.len() {
            if self.queue[i].at <= now {
                let p = self.queue.swap_remove(i);
                if p.vel == 0 {
                    if let Some(s) = slot(p.ch)
                        && self.generation[s][p.note as usize] == p.generation
                    {
                        self.note_off(p.ch, p.note, out);
                    }
                } else {
                    self.note_on(p.ch, p.note, p.vel, None, now, out);
                }
            } else {
                i += 1;
            }
        }
        // Clock steps.
        let mut guard = 0;
        while self.running && self.next_step_at <= now && guard < 8 {
            let st = self.step;
            self.play_step(st, now, 0, false, out);
            self.step += 1;
            self.next_step_at += self.samples_per_step();
            guard += 1;
        }
        if guard == 8 {
            // Fell far behind (tempo change, device hiccup): resync.
            self.next_step_at = now + self.samples_per_step();
        }
    }

    /// Play step `st`. `elapsed` > 0 when replaying a step late (grace).
    fn play_step(
        &mut self,
        st: u64,
        now: u64,
        elapsed: u64,
        tonal_only: bool,
        out: &mut impl FnMut(SeqEvent),
    ) {
        let sps = self.samples_per_step();
        let gate_of = |steps: u64, frac: f64| -> u64 { ((steps * sps) as f64 * frac) as u64 };
        if !tonal_only {
            self.last_step = Some((st, now));
        }
        match self.params.kind {
            StyleKind::Arp => {
                let Some(c) = self.chord else { return };
                let spbar = self.steps_per_bar() as u64;
                if st.is_multiple_of(spbar) && self.params.bass && c.bass > 0 {
                    let g = gate_of(spbar, 0.95).saturating_sub(elapsed);
                    self.note_on(CH_BASS, c.bass, self.vel(100), Some(g), now, out);
                }
                let notes = c.notes();
                if notes.is_empty() {
                    return;
                }
                let oct = self.params.arp_octaves.clamp(1, 4) as usize;
                let len = notes.len() * oct;
                let pos = st as usize;
                let idx = match self.params.arp_mode {
                    ArpMode::Up => pos % len,
                    ArpMode::Down => len - 1 - pos % len,
                    ArpMode::UpDown => {
                        if len < 2 {
                            0
                        } else {
                            let cyc = 2 * len - 2;
                            let p = pos % cyc;
                            if p < len { p } else { cyc - p }
                        }
                    }
                    ArpMode::Random => {
                        self.rng ^= self.rng << 13;
                        self.rng ^= self.rng >> 17;
                        self.rng ^= self.rng << 5;
                        self.rng as usize % len
                    }
                };
                let note = notes[idx % notes.len()] as usize + 12 * (idx / notes.len());
                let accent = if st.is_multiple_of(self.steps_per_beat() as u64) {
                    96
                } else {
                    80
                };
                let g =
                    gate_of(1, self.params.gate.clamp(0.05, 1.0) as f64).saturating_sub(elapsed);
                self.note_on(
                    CH_CHORD,
                    note.min(127) as u8,
                    self.vel(accent),
                    Some(g.max(1)),
                    now,
                    out,
                );
            }
            StyleKind::Pattern => {
                let Some(p) = self.pattern.clone() else {
                    return;
                };
                let steps = &p.steps[(st % p.steps.len() as u64) as usize];
                for h in steps {
                    let g = gate_of(h.len as u64, if h.len > 1 { 0.97 } else { 0.85 })
                        .saturating_sub(elapsed)
                        .max(1);
                    match h.part {
                        Part::Drum(n) => {
                            if !tonal_only && self.params.drums {
                                out(SeqEvent::On {
                                    ch: CH_DRUMS,
                                    note: n,
                                    vel: self.vel(h.vel),
                                });
                            }
                        }
                        Part::Bass(b) => {
                            let Some(c) = self.chord else { continue };
                            if !self.params.bass || c.bass == 0 {
                                continue;
                            }
                            let n = c.bass
                                + match b {
                                    BassNote::Root => 0,
                                    BassNote::Fifth => 7,
                                    BassNote::Octave => 12,
                                };
                            self.note_on(CH_BASS, n, self.vel(h.vel), Some(g), now, out);
                        }
                        Part::Chord => {
                            let Some(c) = self.chord else { continue };
                            for &n in c.notes() {
                                self.note_on(
                                    CH_CHORD,
                                    n,
                                    self.vel((h.vel as u32 * 7 / 8) as u8),
                                    Some(g),
                                    now,
                                    out,
                                );
                            }
                        }
                        Part::Tone(i) => {
                            let Some(c) = self.chord else { continue };
                            let notes = c.notes();
                            if notes.is_empty() {
                                continue;
                            }
                            let i = i as usize;
                            let n = notes[i % notes.len()] as usize + 12 * (i / notes.len());
                            self.note_on(
                                CH_CHORD,
                                n.min(127) as u8,
                                self.vel((h.vel as u32 * 7 / 8) as u8),
                                Some(g),
                                now,
                                out,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::patterns::builtin;

    const SR: u32 = 48_000;

    fn run(seq: &mut Sequencer, from: u64, to: u64) -> Vec<(u64, SeqEvent)> {
        let mut evs = Vec::new();
        let mut now = from;
        while now < to {
            seq.process(now, &mut |e| evs.push((now, e)));
            now = seq
                .next_time()
                .map(|t| t.max(now + 1))
                .unwrap_or(to)
                .min(to);
        }
        evs
    }

    fn ons(evs: &[(u64, SeqEvent)], ch: u8) -> Vec<(u64, u8)> {
        evs.iter()
            .filter_map(|(t, e)| match e {
                SeqEvent::On { ch: c, note, .. } if *c == ch => Some((*t, *note)),
                _ => None,
            })
            .collect()
    }

    fn c_major() -> ChordNotes {
        ChordNotes::new(&[48, 52, 55], Some(36))
    }

    #[test]
    fn block_chord_on_and_off() {
        let mut s = Sequencer::new(SR);
        let mut evs = Vec::new();
        s.set_chord(Some(c_major()), true, 95, 0, &mut |e| evs.push(e));
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, SeqEvent::On { .. }))
                .count(),
            4
        );
        evs.clear();
        s.set_chord(None, false, 95, 100, &mut |e| evs.push(e));
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, SeqEvent::Off { .. }))
                .count(),
            4
        );
    }

    #[test]
    fn modifier_change_moves_only_changed_notes() {
        let mut s = Sequencer::new(SR);
        s.set_chord(Some(c_major()), true, 95, 0, &mut |_| {});
        let mut evs = Vec::new();
        // C -> C7: only Bb is added.
        s.set_chord(
            Some(ChordNotes::new(&[48, 52, 55, 58], Some(36))),
            false,
            95,
            10,
            &mut |e| evs.push(e),
        );
        assert_eq!(
            evs,
            vec![SeqEvent::On {
                ch: CH_CHORD,
                note: 58,
                vel: 88
            }]
        );
    }

    #[test]
    fn pedal_holds_block_chord() {
        let mut s = Sequencer::new(SR);
        let mut evs = Vec::new();
        s.set_pedal(true, 0, &mut |e| evs.push(e));
        s.set_chord(Some(c_major()), true, 95, 0, &mut |e| evs.push(e));
        evs.clear();
        s.set_chord(None, false, 95, 10, &mut |e| evs.push(e));
        assert!(evs.is_empty(), "pedal keeps it");
        s.set_pedal(false, 20, &mut |e| evs.push(e));
        assert_eq!(evs.len(), 4);
    }

    #[test]
    fn strum_rolls_notes_in() {
        let mut s = Sequencer::new(SR);
        s.set_style(
            StyleParams {
                kind: StyleKind::Strum,
                strum_ms: 20.0,
                ..Default::default()
            },
            0,
            &mut |_| {},
        );
        let mut evs = Vec::new();
        s.set_chord(Some(c_major()), true, 95, 0, &mut |e| evs.push((0, e)));
        evs.extend(run(&mut s, 0, SR as u64));
        let chord = ons(&evs, CH_CHORD);
        assert_eq!(
            chord.iter().map(|x| x.1).collect::<Vec<_>>(),
            vec![48, 52, 55]
        );
        assert_eq!(chord[1].0, 960); // 20 ms
        assert_eq!(chord[2].0, 1920);
    }

    #[test]
    fn arpeggio_plays_in_time() {
        let mut s = Sequencer::new(SR);
        s.set_tempo(120.0);
        s.set_style(
            StyleParams {
                kind: StyleKind::Arp,
                arp_steps_per_beat: 2,
                bass: false,
                ..Default::default()
            },
            0,
            &mut |_| {},
        );
        s.set_chord(Some(c_major()), true, 95, 1000, &mut |_| {});
        // One second at 120 bpm in 8ths = 4 notes.
        let evs = run(&mut s, 1000, 1000 + SR as u64);
        let notes = ons(&evs, CH_CHORD);
        assert_eq!(
            notes.iter().map(|x| x.1).collect::<Vec<_>>(),
            vec![48, 52, 55, 48]
        );
        assert_eq!(notes[0].0, 1000);
        assert_eq!(notes[1].0 - notes[0].0, 12_000);
        // Each note is released before the next starts.
        let offs = evs
            .iter()
            .filter(|(_, e)| matches!(e, SeqEvent::Off { .. }))
            .count();
        assert!(offs >= 3);
    }

    #[test]
    fn pattern_with_drums_and_grace() {
        let mut s = Sequencer::new(SR);
        s.set_tempo(120.0);
        let pop = Arc::new(builtin().into_iter().find(|p| p.name == "Pop").unwrap());
        s.set_pattern(Some(pop), 0, &mut |_| {});
        s.set_style(
            StyleParams {
                kind: StyleKind::Pattern,
                grace_ms: 100.0,
                ..Default::default()
            },
            0,
            &mut |_| {},
        );
        s.set_running(true, 0, &mut |_| {});
        // Drums alone for one bar (2 s at 120 bpm).
        let evs = run(&mut s, 0, 2 * SR as u64);
        assert!(!ons(&evs, CH_DRUMS).is_empty());
        assert!(ons(&evs, CH_CHORD).is_empty(), "no chord yet");
        // Press a chord 40 ms after the downbeat of bar 2: it should play now.
        let t = 2 * SR as u64 + 1920;
        let mut evs = run(&mut s, 2 * SR as u64, t);
        s.set_chord(Some(c_major()), true, 95, t, &mut |e| evs.push((t, e)));
        assert!(
            ons(&evs, CH_CHORD).iter().any(|(at, _)| *at == t),
            "grace replays beat 1"
        );
        assert!(
            ons(&evs, CH_BASS)
                .iter()
                .any(|(at, n)| *at == t && *n == 36)
        );
        // Change chord way after the beat: waits for the next step.
        let t2 = t + 4000;
        let mut evs = run(&mut s, t, t2);
        evs.clear();
        s.set_chord(
            Some(ChordNotes::new(&[53, 57, 60], Some(41))),
            true,
            95,
            t2,
            &mut |e| evs.push((t2, e)),
        );
        assert!(ons(&evs, CH_CHORD).is_empty());
        // Stop silences everything.
        let mut offs = Vec::new();
        s.stop(t2, &mut |e| offs.push(e));
        assert!(!s.running());
    }
}
