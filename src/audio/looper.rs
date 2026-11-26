//! Part 4b: the looper. Runs inside the audio thread, next to the
//! sequencer, so loops are sample-accurate and stay locked to the metronome.
//!
//! A loop is a stack of *layers*. Each layer is a list of note events whose
//! positions are stored in **beats**, so changing the tempo speeds the loop
//! up or slows it down instead of breaking it. The first layer sets the
//! loop's length (a fixed number of bars, or "free": press again and it
//! closes at the nearest bar line); every later layer is an overdub that is
//! laid on top while the loop keeps going round.
//!
//! Nothing here allocates or frees memory after [`Looper::new`]: every layer
//! slot gets its event buffer up front, and "removing" a layer only clears
//! it. That keeps the audio thread glitch-free.
//!
//! Each layer plays on channels of its own (borrowed from a pool), with the
//! instrument that was chosen when it was recorded. So you can record a
//! piano layer, switch to strings and overdub, and a loop note never cuts a
//! note you're holding on the same key.

use super::sequencer::{CH_DRUMS, SeqEvent};

pub const MAX_LAYERS: usize = 8;
/// Events per layer (an on + an off per note: about 2000 notes).
const LAYER_CAP: usize = 4096;
/// Room kept for note-offs once a layer is nearly full, so nothing sticks.
const OFF_RESERVE: usize = 128;
pub const BEATS_PER_BAR: f64 = 4.0;
/// A free-length first layer closes itself after this many bars.
pub const MAX_FREE_BARS: u32 = 64;
/// Channels lent to layers. Song parts take 3, 4, … upward, so the looper
/// takes them from the top down. Channel 9 is drums and is never lent.
const POOL: [u8; 12] = [15, 14, 13, 12, 11, 10, 8, 7, 6, 5, 4, 3];
const NONE: u8 = 255;
const EPS: f64 = 1e-9;

#[derive(Clone, Copy, Debug)]
struct Ev {
    /// Beats from the start of the loop.
    pos: f64,
    /// The pass it was recorded in; it plays from the next pass on.
    pass: u32,
    /// Insertion order, so events at the same position keep their order.
    seq: u32,
    /// Source channel (0 melody, 1 chords, 2 bass, 9 drums).
    ch: u8,
    note: u8,
    /// 0 = note off.
    vel: u8,
}

struct Layer {
    events: Vec<Ev>,
    used: bool,
    muted: bool,
    full: bool,
    /// Next event to play in the current pass.
    cursor: usize,
    /// Output channel per source channel.
    out: [u8; 16],
    /// Notes this layer's playback has sounding, by source channel.
    sounding: [u128; 16],
    /// Notes held while this layer records, by source channel.
    open: [u128; 16],
    notes: u16,
}

impl Layer {
    fn new() -> Self {
        Layer {
            events: Vec::with_capacity(LAYER_CAP),
            used: false,
            muted: false,
            full: false,
            cursor: 0,
            out: [NONE; 16],
            sounding: [0; 16],
            open: [0; 16],
            notes: 0,
        }
    }

    fn reset(&mut self) {
        self.events.clear();
        self.used = false;
        self.muted = false;
        self.full = false;
        self.cursor = 0;
        self.out = [NONE; 16];
        self.sounding = [0; 16];
        self.open = [0; 16];
        self.notes = 0;
    }

    /// Note-offs for everything this layer's playback is sounding.
    fn silence(&mut self, out: &mut impl FnMut(SeqEvent)) {
        for src in 0..16 {
            let mut bits = self.sounding[src];
            while bits != 0 {
                let note = bits.trailing_zeros() as u8;
                bits &= bits - 1;
                out(SeqEvent::Off {
                    ch: self.out_ch(src as u8),
                    note,
                });
            }
            self.sounding[src] = 0;
        }
    }

    fn out_ch(&self, src: u8) -> u8 {
        match self.out[src as usize & 15] {
            NONE => src,
            c => c,
        }
    }

    /// Sorted insert (by position, then insertion order). Keeps the play
    /// cursor pointing at the same event.
    fn insert(&mut self, ev: Ev) {
        let idx = self
            .events
            .partition_point(|e| (e.pos, e.seq) <= (ev.pos, ev.seq));
        self.events.insert(idx, ev);
        if idx < self.cursor {
            self.cursor += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopState {
    /// No loop.
    #[default]
    Empty,
    /// Clicking in before the first layer.
    CountIn,
    /// Recording the first layer (this sets the length).
    First,
    Playing,
    /// Has layers, not playing.
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopCommand {
    /// The loop key: start the first layer (after `count_in` beats; `bars`
    /// long, 0 = until pressed again), close the first layer at the nearest
    /// bar line, start an overdub, or finish the overdub.
    Rec { count_in: u32, bars: u32 },
    /// Start / stop playback (layers are kept).
    TogglePlay,
    /// Stop playback (panic).
    Stop,
    /// Cancel the layer being recorded, else remove the newest layer.
    Undo,
    /// Remove every layer.
    Clear,
    /// Mute / unmute a layer (0-based).
    Mute(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayerStatus {
    pub notes: u16,
    pub muted: bool,
    /// Ran out of room; later notes weren't kept.
    pub full: bool,
}

/// What the UI shows. Copied out of the audio thread every buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoopStatus {
    pub state: LoopState,
    /// Layer being recorded (the first layer or an overdub).
    pub rec_layer: Option<u8>,
    /// Loop length in beats (0 until the first layer closes).
    pub len_beats: f32,
    /// Where the loop is: beats into the current pass. While the first
    /// layer records: beats since it started (negative during the count-in).
    pub pos_beats: f32,
    /// First layer: where it will close, in beats (0 = not decided yet).
    pub target_beats: f32,
    pub count: u8,
    pub layers: [LayerStatus; MAX_LAYERS],
}

pub struct Looper {
    layers: Vec<Layer>,
    count: usize,
    state: LoopState,
    /// Length in beats; 0 until the first layer closes.
    len: f64,
    /// First layer: close here (beats).
    target: Option<f64>,
    rec: Option<usize>,
    beat_samples: f64,
    sample_rate: f64,
    /// The loop clock: `anchor_beats` at `anchor_sample`.
    anchor_sample: u64,
    anchor_beats: f64,
    /// Pass being played (counts up forever).
    pass: u32,
    seq: u32,
    /// Instruments to set on lent channels: (channel, program), NONE = none.
    program_req: [u8; 16],
}

impl Looper {
    pub fn new(sample_rate: u32) -> Self {
        Looper {
            layers: (0..MAX_LAYERS).map(|_| Layer::new()).collect(),
            count: 0,
            state: LoopState::Empty,
            len: 0.0,
            target: None,
            rec: None,
            beat_samples: sample_rate as f64 * 0.6,
            sample_rate: sample_rate as f64,
            anchor_sample: 0,
            anchor_beats: 0.0,
            pass: 0,
            seq: 0,
            program_req: [NONE; 16],
        }
    }

    pub fn state(&self) -> LoopState {
        self.state
    }

    /// True while a layer is being recorded (or counted in).
    #[cfg(test)]
    pub fn recording(&self) -> bool {
        self.rec.is_some()
    }

    fn beats(&self, now: u64) -> f64 {
        self.anchor_beats + (now as f64 - self.anchor_sample as f64) / self.beat_samples
    }

    fn sample_at(&self, beats: f64) -> u64 {
        let s = self.anchor_sample as f64 + (beats - self.anchor_beats) * self.beat_samples;
        s.max(0.0).ceil() as u64
    }

    pub fn set_tempo(&mut self, bpm: f32, now: u64) {
        let t = self.beats(now);
        self.anchor_sample = now;
        self.anchor_beats = t;
        self.beat_samples = self.sample_rate * 60.0 / bpm.clamp(20.0, 400.0) as f64;
    }

    /// The loop's next whole beat: (sample, beat within the bar), so the
    /// metronome can click on it.
    pub fn next_beat(&self, now: u64) -> Option<(u64, u32)> {
        if !matches!(
            self.state,
            LoopState::CountIn | LoopState::First | LoopState::Playing
        ) {
            return None;
        }
        let b = self.beats(now).ceil();
        Some((self.sample_at(b), (b as i64).rem_euclid(4) as u32))
    }

    /// Start of the next pass while playing (a take started now begins there).
    pub fn next_pass_start(&self) -> Option<u64> {
        (self.state == LoopState::Playing && self.len > 0.0)
            .then(|| self.sample_at((self.pass + 1) as f64 * self.len))
    }

    /// Channels the layers have borrowed (bit per channel).
    pub fn channels_in_use(&self) -> u16 {
        let mut m = 0u16;
        for l in self.layers.iter().filter(|l| l.used) {
            for &c in &l.out {
                if c != NONE {
                    m |= 1 << c;
                }
            }
        }
        m
    }

    /// An instrument to set up for a lent channel, if any is waiting.
    pub fn take_program(&mut self) -> Option<(u8, u8)> {
        let ch = self.program_req.iter().position(|&p| p != NONE)?;
        let p = std::mem::replace(&mut self.program_req[ch], NONE);
        Some((ch as u8, p))
    }

    /// Handle a command. `start` = sample where a first layer would begin
    /// (the renderer works that out from the metronome and count-in).
    /// Returns a sample to restart the clicks from, when the loop's grid moved.
    pub fn command(
        &mut self,
        cmd: LoopCommand,
        now: u64,
        start: u64,
        out: &mut impl FnMut(SeqEvent),
    ) -> Option<u64> {
        match cmd {
            LoopCommand::Rec { bars, .. } => match self.state {
                LoopState::Empty => {
                    self.clear(out);
                    self.state = LoopState::CountIn;
                    self.anchor_sample = start;
                    self.anchor_beats = 0.0;
                    self.target = (bars > 0).then_some(bars as f64 * BEATS_PER_BAR);
                    self.layers[0].used = true;
                    self.count = 1;
                    self.rec = Some(0);
                    if start <= now {
                        self.state = LoopState::First;
                    }
                }
                // Pressed again during the count-in: never mind.
                LoopState::CountIn => self.clear(out),
                LoopState::First => {
                    // Close at the nearest bar line (at least one bar).
                    let t = self.beats(now);
                    let bars = (t / BEATS_PER_BAR).round().max(1.0);
                    self.target = Some(bars * BEATS_PER_BAR);
                }
                LoopState::Playing | LoopState::Stopped => {
                    if self.rec.is_some() {
                        self.finish_overdub(now);
                    } else if self.count < MAX_LAYERS {
                        let mut grid = None;
                        if self.state == LoopState::Stopped {
                            grid = self.play(now);
                        }
                        let i = self.count;
                        self.layers[i].reset();
                        self.layers[i].used = true;
                        self.count += 1;
                        self.rec = Some(i);
                        return grid;
                    }
                }
            },
            LoopCommand::TogglePlay => match self.state {
                LoopState::Playing => self.stop(now, out),
                LoopState::Stopped => return self.play(now),
                _ => {}
            },
            LoopCommand::Stop => match self.state {
                LoopState::Playing => self.stop(now, out),
                LoopState::CountIn | LoopState::First => self.clear(out),
                _ => {}
            },
            LoopCommand::Undo => match (self.state, self.rec) {
                (LoopState::CountIn | LoopState::First, _) => self.clear(out),
                (_, Some(i)) => {
                    self.rec = None;
                    self.remove_last(i, out);
                }
                (LoopState::Playing | LoopState::Stopped, None) if self.count > 0 => {
                    self.remove_last(self.count - 1, out);
                }
                _ => {}
            },
            LoopCommand::Clear => self.clear(out),
            LoopCommand::Mute(i) => {
                let i = i as usize;
                if i < self.count && self.state != LoopState::First {
                    let l = &mut self.layers[i];
                    l.muted = !l.muted;
                    if l.muted {
                        l.silence(out);
                    }
                }
            }
        }
        None
    }

    fn remove_last(&mut self, i: usize, out: &mut impl FnMut(SeqEvent)) {
        self.layers[i].silence(out);
        self.layers[i].reset();
        self.count = i;
        if self.count == 0 {
            self.clear(out);
        }
    }

    fn clear(&mut self, out: &mut impl FnMut(SeqEvent)) {
        for l in &mut self.layers {
            l.silence(out);
            l.reset();
        }
        self.count = 0;
        self.state = LoopState::Empty;
        self.len = 0.0;
        self.target = None;
        self.rec = None;
        self.pass = 0;
        self.program_req = [NONE; 16];
    }

    fn stop(&mut self, now: u64, out: &mut impl FnMut(SeqEvent)) {
        if self.rec.is_some() {
            self.finish_overdub(now);
        }
        for l in &mut self.layers {
            l.silence(out);
        }
        self.state = LoopState::Stopped;
    }

    /// Start from the top of the loop, now. Returns the start sample.
    fn play(&mut self, now: u64) -> Option<u64> {
        if self.count == 0 || self.len <= 0.0 {
            return None;
        }
        self.pass += 1;
        self.anchor_sample = now;
        self.anchor_beats = self.pass as f64 * self.len;
        for l in &mut self.layers {
            l.cursor = 0;
        }
        self.state = LoopState::Playing;
        Some(now)
    }

    /// End the overdub: close notes still held, drop the layer if it's empty.
    fn finish_overdub(&mut self, now: u64) {
        let Some(i) = self.rec.take() else { return };
        let t = self.beats(now);
        let (pass, pos) = self.split(t);
        self.close_open(i, pos, pass);
        if self.layers[i].notes == 0 {
            self.layers[i].reset();
            if i + 1 == self.count {
                self.count = i;
            }
        }
    }

    fn close_open(&mut self, i: usize, pos: f64, pass: u32) {
        for src in 0..16u8 {
            let mut bits = self.layers[i].open[src as usize];
            while bits != 0 {
                let note = bits.trailing_zeros() as u8;
                bits &= bits - 1;
                self.seq += 1;
                let ev = Ev {
                    pos,
                    pass,
                    seq: self.seq,
                    ch: src,
                    note,
                    vel: 0,
                };
                let l = &mut self.layers[i];
                if l.events.len() < LAYER_CAP {
                    l.insert(ev);
                }
            }
            self.layers[i].open[src as usize] = 0;
        }
    }

    /// (pass, position in the pass) for a loop-clock time.
    fn split(&self, t: f64) -> (u32, f64) {
        if self.len <= 0.0 {
            return (0, t.max(0.0));
        }
        let pass = (t / self.len + EPS).floor().max(0.0);
        (pass as u32, (t - pass * self.len).max(0.0))
    }

    /// Something you played (the same events the recorder sees).
    pub fn record(&mut self, now: u64, e: SeqEvent, programs: &[u8; 16]) {
        let Some(i) = self.rec else { return };
        if !matches!(self.state, LoopState::First | LoopState::Playing) {
            return;
        }
        let t = self.beats(now);
        if t < 0.0 {
            return;
        }
        let (ch, note, vel) = match e {
            SeqEvent::On { ch, note, vel } => (ch & 15, note & 127, vel.max(1)),
            SeqEvent::Off { ch, note } => (ch & 15, note & 127, 0),
        };
        // The first layer keeps raw times (it doesn't know its length yet);
        // they're folded into the loop when it closes.
        let (pass, pos) = if self.state == LoopState::First {
            (0, t)
        } else {
            self.split(t)
        };
        let bit = 1u128 << note;
        if vel > 0 {
            if self.layers[i].events.len() >= LAYER_CAP - OFF_RESERVE {
                self.layers[i].full = true;
                return;
            }
            if ch != CH_DRUMS && self.layers[i].out[ch as usize] == NONE {
                self.lend_channel(i, ch, programs[ch as usize]);
            }
            if self.layers[i].open[ch as usize] & bit != 0 {
                // Re-struck while held: close the old one first.
                self.push(i, pos, pass, ch, note, 0);
            }
            if ch != CH_DRUMS {
                self.layers[i].open[ch as usize] |= bit;
            }
            self.layers[i].notes = self.layers[i].notes.saturating_add(1);
            self.push(i, pos, pass, ch, note, vel);
        } else if self.layers[i].open[ch as usize] & bit != 0 {
            self.layers[i].open[ch as usize] &= !bit;
            self.push(i, pos, pass, ch, note, 0);
        }
    }

    /// Everything stopped (Escape, window lost focus): close held notes.
    pub fn all_off(&mut self, now: u64) {
        let Some(i) = self.rec else { return };
        if !matches!(self.state, LoopState::First | LoopState::Playing) {
            return;
        }
        let t = self.beats(now).max(0.0);
        let (pass, pos) = if self.state == LoopState::First {
            (0, t)
        } else {
            self.split(t)
        };
        self.close_open(i, pos, pass);
    }

    fn push(&mut self, i: usize, pos: f64, pass: u32, ch: u8, note: u8, vel: u8) {
        self.seq += 1;
        let l = &mut self.layers[i];
        if l.events.len() < LAYER_CAP {
            l.insert(Ev {
                pos,
                pass,
                seq: self.seq,
                ch,
                note,
                vel,
            });
        }
    }

    fn lend_channel(&mut self, i: usize, src: u8, program: u8) {
        let used = self.channels_in_use();
        match POOL.iter().find(|&&c| used & (1 << c) == 0) {
            Some(&c) => {
                self.layers[i].out[src as usize] = c;
                self.program_req[c as usize] = program;
            }
            // Out of channels: share yours.
            None => self.layers[i].out[src as usize] = src,
        }
    }

    /// The first layer is done: fold late notes into the loop and start it.
    fn close_first(&mut self, len: f64) {
        self.len = len;
        self.target = None;
        // Notes still held stop at the loop's end.
        let end = (len - 1e-6).max(0.0);
        self.close_open(0, end, 0);
        let l = &mut self.layers[0];
        for e in &mut l.events {
            if e.pos >= len - EPS {
                let k = (e.pos / len + EPS).floor();
                e.pos = (e.pos - k * len).max(0.0);
                e.pass = k as u32;
            }
        }
        l.events
            .sort_unstable_by(|a, b| a.pos.total_cmp(&b.pos).then(a.seq.cmp(&b.seq)));
        self.rec = None;
        if l.notes == 0 {
            // Nothing played: no loop.
            let mut drop = |_| {};
            self.clear(&mut drop);
            return;
        }
        self.state = LoopState::Playing;
        self.pass = 0;
        for l in &mut self.layers {
            l.cursor = 0;
        }
    }

    /// Play everything due up to `now`. Returns the sample where a new pass
    /// (or the loop itself) started during this call, for the metronome.
    pub fn process(&mut self, now: u64, out: &mut impl FnMut(SeqEvent)) -> Option<u64> {
        let mut grid = None;
        if self.state == LoopState::CountIn {
            if now < self.anchor_sample {
                return None;
            }
            self.state = LoopState::First;
        }
        if self.state == LoopState::First {
            let t = self.beats(now);
            let target = self
                .target
                .filter(|&g| t + EPS >= g)
                .or_else(|| (t >= MAX_FREE_BARS as f64 * BEATS_PER_BAR).then(|| t.floor()));
            let len = (target? / BEATS_PER_BAR).round().max(1.0) * BEATS_PER_BAR;
            grid = Some(self.sample_at(len.min(t)));
            self.close_first(len);
        }
        if self.state != LoopState::Playing {
            return grid;
        }
        let t = self.beats(now);
        loop {
            let (pass, pos) = self.split(t);
            if pass > self.pass {
                self.play_range(f64::INFINITY, out);
                self.pass += 1;
                for l in &mut self.layers {
                    l.cursor = 0;
                }
                grid = Some(self.sample_at(self.pass as f64 * self.len));
            } else {
                self.play_range(pos + EPS, out);
                break;
            }
        }
        grid
    }

    fn play_range(&mut self, upto: f64, out: &mut impl FnMut(SeqEvent)) {
        let pass = self.pass;
        for l in self.layers.iter_mut().filter(|l| l.used) {
            while let Some(&e) = l.events.get(l.cursor) {
                if e.pos > upto {
                    break;
                }
                l.cursor += 1;
                if e.pass >= pass || l.muted {
                    continue;
                }
                let bit = 1u128 << e.note;
                let s = &mut l.sounding[e.ch as usize];
                let ch = match l.out[e.ch as usize] {
                    NONE => e.ch,
                    c => c,
                };
                if e.vel > 0 {
                    if *s & bit != 0 && e.ch != CH_DRUMS {
                        out(SeqEvent::Off { ch, note: e.note });
                    }
                    if e.ch != CH_DRUMS {
                        *s |= bit;
                    }
                    out(SeqEvent::On {
                        ch,
                        note: e.note,
                        vel: e.vel,
                    });
                } else if *s & bit != 0 {
                    *s &= !bit;
                    out(SeqEvent::Off { ch, note: e.note });
                }
            }
        }
    }

    /// When the looper next needs to run (so the renderer can split there).
    pub fn next_time(&self) -> Option<u64> {
        match self.state {
            LoopState::CountIn => Some(self.anchor_sample),
            LoopState::First => self.target.map(|g| self.sample_at(g)),
            LoopState::Playing => {
                let base = self.pass as f64 * self.len;
                let mut next = base + self.len;
                for l in self.layers.iter().filter(|l| l.used && !l.muted) {
                    if let Some(e) = l.events.get(l.cursor) {
                        next = next.min(base + e.pos);
                    }
                }
                Some(self.sample_at(next))
            }
            _ => None,
        }
    }

    pub fn status(&self, now: u64) -> LoopStatus {
        let mut s = LoopStatus {
            state: self.state,
            rec_layer: self.rec.map(|i| i as u8),
            len_beats: self.len as f32,
            count: self.count as u8,
            target_beats: self.target.unwrap_or(0.0) as f32,
            ..Default::default()
        };
        s.pos_beats = match self.state {
            LoopState::CountIn | LoopState::First => self.beats(now) as f32,
            LoopState::Playing => self.split(self.beats(now)).1 as f32,
            _ => 0.0,
        };
        for (i, l) in self.layers.iter().enumerate().take(self.count) {
            s.layers[i] = LayerStatus {
                notes: l.notes,
                muted: l.muted,
                full: l.full,
            };
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;
    /// 120 BPM: one beat = 24 000 samples, a 4/4 bar = 96 000.
    const BEAT: u64 = 24_000;
    const BAR: u64 = 4 * BEAT;

    fn looper() -> Looper {
        let mut l = Looper::new(SR);
        l.set_tempo(120.0, 0);
        l
    }

    fn on(ch: u8, note: u8) -> SeqEvent {
        SeqEvent::On { ch, note, vel: 100 }
    }
    fn off(ch: u8, note: u8) -> SeqEvent {
        SeqEvent::Off { ch, note }
    }

    /// Run the looper from `from` to `to` the way the renderer does
    /// (stopping at every `next_time`), collecting (sample, event).
    fn run(l: &mut Looper, from: u64, to: u64) -> Vec<(u64, SeqEvent)> {
        let mut got = vec![];
        let mut now = from;
        while now < to {
            l.process(now, &mut |e| got.push((now, e)));
            now = match l.next_time() {
                Some(t) => t.max(now + 1).min(to),
                None => to,
            };
        }
        got
    }

    fn rec(bars: u32) -> LoopCommand {
        LoopCommand::Rec { count_in: 0, bars }
    }

    fn ons(evs: &[(u64, SeqEvent)]) -> Vec<(u64, u8)> {
        evs.iter()
            .filter_map(|(t, e)| match e {
                SeqEvent::On { note, .. } => Some((*t, *note)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fixed_length_first_layer_loops() {
        let mut l = looper();
        let progs = [0u8; 16];
        l.command(rec(1), 0, 0, &mut |_| {});
        assert_eq!(l.state(), LoopState::First);
        l.record(BEAT, on(0, 60), &progs);
        l.record(BEAT + 100, off(0, 60), &progs);
        l.record(3 * BEAT, on(9, 36), &progs);
        // The first pass plays nothing back (you just played it).
        let first = run(&mut l, 0, BAR);
        assert!(ons(&first).is_empty(), "{first:?}");
        // Then it repeats every bar, on its own channel.
        let evs = run(&mut l, BAR, 3 * BAR);
        assert_eq!(l.state(), LoopState::Playing);
        assert_eq!(
            ons(&evs),
            vec![
                (BAR + BEAT, 60),
                (BAR + 3 * BEAT, 36),
                (2 * BAR + BEAT, 60),
                (2 * BAR + 3 * BEAT, 36)
            ]
        );
        let melody_ch = evs
            .iter()
            .find_map(|(_, e)| match e {
                SeqEvent::On { ch, note: 60, .. } => Some(*ch),
                _ => None,
            })
            .unwrap();
        assert_eq!(melody_ch, 15, "first lent channel");
        assert!(
            evs.iter().any(|(_, e)| matches!(
                e,
                SeqEvent::On {
                    ch: 9,
                    note: 36,
                    ..
                }
            )),
            "drums stay on 9"
        );
        assert_eq!(l.take_program(), Some((15, 0)));
        assert_eq!(l.take_program(), None);
    }

    #[test]
    fn free_length_closes_at_nearest_bar() {
        let progs = [0u8; 16];
        // Pressed a little after bar 2: the loop is 2 bars, and the note
        // played just after the line lands at the start of the loop.
        let mut l = looper();
        l.command(rec(0), 0, 0, &mut |_| {});
        l.record(BEAT, on(0, 60), &progs);
        l.record(2 * BEAT, off(0, 60), &progs);
        run(&mut l, 0, 2 * BAR + 2_000);
        l.record(2 * BAR + 1_000, on(0, 62), &progs);
        l.record(2 * BAR + 1_500, off(0, 62), &progs);
        l.command(rec(0), 2 * BAR + 2_000, 0, &mut |_| {});
        let evs = run(&mut l, 2 * BAR + 2_000, 6 * BAR);
        assert_eq!(l.status(0).len_beats, 8.0);
        // 62 was played in pass 1 (just after the line), so it's heard from
        // pass 2; 60 from pass 1.
        let o = ons(&evs);
        assert_eq!(o[0], (2 * BAR + BEAT, 60), "{o:?}");
        assert!(o.contains(&(4 * BAR + 1_000, 62)), "{o:?}");
        assert!(o.contains(&(4 * BAR + BEAT, 60)), "{o:?}");

        // Pressed a little before bar 1: it keeps recording to the line.
        let mut l = looper();
        l.command(rec(0), 0, 0, &mut |_| {});
        l.record(BEAT, on(0, 60), &progs);
        l.record(BEAT + 10, off(0, 60), &progs);
        run(&mut l, 0, BAR - 3_000);
        l.command(rec(0), BAR - 3_000, 0, &mut |_| {});
        assert_eq!(l.state(), LoopState::First);
        l.record(BAR - 1_000, on(0, 64), &progs);
        let evs = run(&mut l, BAR - 3_000, 2 * BAR + 10);
        assert_eq!(l.state(), LoopState::Playing);
        assert_eq!(l.status(0).len_beats, 4.0);
        // 64 was still held at the loop end: closed there, plays next pass.
        assert_eq!(ons(&evs), vec![(BAR + BEAT, 60), (2 * BAR - 1_000, 64)]);
        assert!(
            evs.iter()
                .any(|(t, e)| *t == 2 * BAR && matches!(e, SeqEvent::Off { note: 64, .. }))
        );
    }

    #[test]
    fn count_in_drops_early_notes() {
        let progs = [0u8; 16];
        let mut l = looper();
        l.command(rec(1), 0, BAR, &mut |_| {});
        assert_eq!(l.state(), LoopState::CountIn);
        l.record(1_000, on(0, 50), &progs); // during the count-in
        run(&mut l, 0, BAR + 1);
        assert_eq!(l.state(), LoopState::First);
        l.record(BAR + 1, on(0, 60), &progs);
        l.record(BAR + 10, off(0, 60), &progs);
        let evs = run(&mut l, BAR + 1, 3 * BAR + 10);
        assert_eq!(ons(&evs), vec![(2 * BAR + 1, 60), (3 * BAR + 1, 60)]);
    }

    #[test]
    fn overdub_undo_mute_clear() {
        let progs = [0, 48, 33, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut l = looper();
        l.command(rec(1), 0, 0, &mut |_| {});
        l.record(0, on(0, 60), &progs);
        l.record(10, off(0, 60), &progs);
        run(&mut l, 0, BAR + 1);
        // Overdub chords in pass 1, starting mid-bar; hold one across the wrap.
        l.command(rec(0), BAR + 1, 0, &mut |_| {});
        assert!(l.recording());
        l.record(BAR + BEAT, on(1, 64), &progs);
        l.record(BAR + 2 * BEAT, off(1, 64), &progs);
        l.record(BAR + 3 * BEAT, on(1, 67), &progs);
        run(&mut l, BAR + 1, 2 * BAR + BEAT / 2);
        l.record(2 * BAR + BEAT / 2, off(1, 67), &progs);
        l.command(rec(0), 2 * BAR + BEAT / 2, 0, &mut |_| {});
        assert!(!l.recording());
        assert_eq!(l.status(0).count, 2);
        let evs = run(&mut l, 2 * BAR + BEAT / 2, 4 * BAR);
        let o = ons(&evs);
        // Pass 2: 64 and 67 (recorded in pass 1) come round; pass 3 too.
        assert!(o.contains(&(2 * BAR + BEAT, 64)), "{o:?}");
        assert!(o.contains(&(3 * BAR + BEAT, 64)), "{o:?}");
        assert!(o.contains(&(3 * BAR + 3 * BEAT, 67)), "{o:?}");
        // The chord layer got its own channel with the chord instrument.
        assert_eq!(l.take_program(), Some((14, 48)));
        assert_eq!(l.take_program(), Some((15, 0)));
        // 67 was held across the wrap: it's released early in the next pass.
        assert!(
            evs.iter()
                .any(|(t, e)| *t == 3 * BAR + BEAT / 2 && *e == off(14, 67)),
            "{evs:?}"
        );

        // Mute layer 2: no chords; its sounding notes are released.
        let mut cut = vec![];
        l.command(LoopCommand::Mute(1), 4 * BAR, 0, &mut |e| cut.push(e));
        let evs = run(&mut l, 4 * BAR, 5 * BAR);
        assert_eq!(ons(&evs), vec![(4 * BAR, 60)]);
        assert!(l.status(0).layers[1].muted);
        l.command(LoopCommand::Mute(1), 5 * BAR, 0, &mut |_| {});

        // Undo removes the chord layer; a second undo clears the loop.
        l.command(LoopCommand::Undo, 5 * BAR, 0, &mut |_| {});
        assert_eq!(l.status(0).count, 1);
        assert_eq!(l.channels_in_use(), 1 << 15);
        let evs = run(&mut l, 5 * BAR, 6 * BAR);
        assert_eq!(ons(&evs), vec![(5 * BAR, 60)]);
        l.command(LoopCommand::Undo, 6 * BAR, 0, &mut |_| {});
        assert_eq!(l.state(), LoopState::Empty);
        assert_eq!(l.channels_in_use(), 0);
    }

    #[test]
    fn stop_and_play_restart_from_the_top() {
        let progs = [0u8; 16];
        let mut l = looper();
        l.command(rec(1), 0, 0, &mut |_| {});
        l.record(BEAT, on(0, 60), &progs);
        run(&mut l, 0, BAR + BEAT + 100);
        // 60 is still sounding from pass 1 (held through the end): stopping
        // releases it.
        let mut cut = vec![];
        l.command(LoopCommand::TogglePlay, BAR + BEAT + 100, 0, &mut |e| {
            cut.push(e)
        });
        assert_eq!(l.state(), LoopState::Stopped);
        assert_eq!(cut, vec![off(15, 60)]);
        assert!(run(&mut l, BAR + BEAT + 100, 5 * BAR).is_empty());
        let start = 5 * BAR + 7;
        assert_eq!(
            l.command(LoopCommand::TogglePlay, start, 0, &mut |_| {}),
            Some(start)
        );
        let evs = run(&mut l, start, start + BAR);
        assert_eq!(ons(&evs), vec![(start + BEAT, 60)]);
        assert_eq!(l.next_pass_start(), Some(start + BAR));
    }

    #[test]
    fn tempo_change_stretches_the_loop() {
        let progs = [0u8; 16];
        let mut l = looper();
        l.command(rec(1), 0, 0, &mut |_| {});
        l.record(2 * BEAT, on(0, 60), &progs);
        l.record(2 * BEAT + 10, off(0, 60), &progs);
        run(&mut l, 0, BAR);
        // Half speed from the start of pass 1: beat 2 is now 2 × 48 000 away.
        l.set_tempo(60.0, BAR);
        let evs = run(&mut l, BAR, BAR + 2 * BAR);
        assert_eq!(ons(&evs)[0], (BAR + 4 * BEAT, 60));
        assert_eq!(l.status(BAR).len_beats, 4.0);
    }

    #[test]
    fn empty_first_layer_means_no_loop_and_full_layers_stop_taking_notes() {
        let mut l = looper();
        l.command(rec(1), 0, 0, &mut |_| {});
        run(&mut l, 0, 2 * BAR);
        assert_eq!(l.state(), LoopState::Empty);

        let progs = [0u8; 16];
        let mut l = looper();
        l.command(rec(1), 0, 0, &mut |_| {});
        for i in 0..3_000u64 {
            l.record(i * 10, on(0, (i % 100) as u8), &progs);
            l.record(i * 10 + 5, off(0, (i % 100) as u8), &progs);
        }
        let s = l.status(0);
        assert!(s.layers[0].full);
        assert!(l.layers[0].events.len() <= LAYER_CAP);
        assert_eq!(l.layers[0].events.capacity(), LAYER_CAP, "never grew");
    }
}
