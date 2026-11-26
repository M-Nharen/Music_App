//! Song playback in the audio thread (Song mode, Part 5): the backing parts
//! of a song, with its own clock that can slow down, loop, and stop at the
//! next note in wait mode. Events are sample-accurate like the sequencer.

use super::sequencer::SeqEvent;
use super::stems::{MAX_STEMS, SongCtx, StemPlayer, StemSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One note on / off at a song time (seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongEvent {
    pub time: f64,
    pub ch: u8,
    pub note: u8,
    /// 0 = note off.
    pub vel: u8,
    /// Which track it belongs to (for muting), 0..63.
    pub track: u8,
}

/// Everything the audio thread needs, built by the UI and shared (the UI
/// keeps a clone of the Arc so the audio thread never frees it).
#[derive(Debug, Default)]
pub struct SongData {
    /// Sorted by time, offs before ons at the same time.
    pub events: Vec<SongEvent>,
    /// The song ends here (seconds).
    pub length: f64,
}

impl SongData {
    pub fn new(mut events: Vec<SongEvent>, length: f64) -> SongData {
        events.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then((a.vel > 0).cmp(&(b.vel > 0)))
        });
        SongData { events, length }
    }

    fn first_at(&self, t: f64) -> usize {
        self.events.partition_point(|e| e.time < t)
    }
}

pub enum SongCommand {
    Load(Option<Arc<SongData>>),
    Play(bool),
    /// Jump to a song time.
    Seek(f64),
    Speed(f64),
    /// Bit per track: muted.
    Mute(u64),
    /// Stop at this time until it changes (wait mode).
    Wait(Option<f64>),
    Loop(Option<(f64, f64)>),
    /// Backing volume 0..1.5.
    Gain(f32),
    /// Jam mode (Part 7): the stems to play (the UI keeps the Arc too).
    Stems(Option<Arc<StemSet>>),
    /// Play the stems (or keep them quiet).
    StemsOn(bool),
    /// Gain per stem (index into the set), mute = 0.
    StemGains([f32; MAX_STEMS]),
    /// Pitch shift of the stems in semitones (easy parts in another key).
    StemPitch(i32),
}

/// What the UI reads to draw and score: where the song was at `at`.
#[derive(Clone, Copy, Debug)]
pub struct SongClock {
    pub at: Instant,
    pub pos: f64,
    pub speed: f64,
    pub moving: bool,
    pub playing: bool,
    /// Bumped on every seek (the UI resets its smoothing).
    pub seeks: u32,
    /// Bumped every time the loop jumps back.
    pub loops: u32,
    pub ended: bool,
}

impl Default for SongClock {
    fn default() -> Self {
        SongClock {
            at: Instant::now(),
            pos: 0.0,
            speed: 1.0,
            moving: false,
            playing: false,
            seeks: 0,
            loops: 0,
            ended: false,
        }
    }
}

const MAX_ACTIVE: usize = 512;

pub struct SongPlayer {
    data: Option<Arc<SongData>>,
    sample_rate: f64,
    playing: bool,
    pos: f64,
    speed: f64,
    idx: usize,
    mute: u64,
    wait: Option<f64>,
    looping: Option<(f64, f64)>,
    gain: f32,
    /// Notes this player has on: (channel, note, track).
    active: Vec<(u8, u8, u8)>,
    seeks: u32,
    loops: u32,
    ended: bool,
    /// When it last stopped moving (pos, wall time): kept while stopped so
    /// the UI can tell how much sound is still on its way.
    stopped_at: Option<(f64, Instant)>,
    stems: StemPlayer,
}

impl SongPlayer {
    pub fn new(sample_rate: u32) -> SongPlayer {
        SongPlayer {
            data: None,
            sample_rate: sample_rate.max(1) as f64,
            playing: false,
            pos: 0.0,
            speed: 1.0,
            idx: 0,
            mute: 0,
            wait: None,
            looping: None,
            gain: 1.0,
            active: Vec::with_capacity(MAX_ACTIVE),
            seeks: 0,
            loops: 0,
            ended: false,
            stopped_at: Some((0.0, Instant::now() - Duration::from_secs(60))),
            stems: StemPlayer::new(sample_rate),
        }
    }

    #[cfg(test)]
    pub fn pos(&self) -> f64 {
        self.pos
    }

    fn halted(&self) -> bool {
        self.wait.is_some_and(|w| self.pos >= w - 1e-9)
    }

    /// Moving forward right now.
    pub fn moving(&self) -> bool {
        self.playing && self.data.is_some() && !self.halted()
    }

    fn all_off(&mut self, out: &mut impl FnMut(SeqEvent)) {
        for (ch, note, _) in self.active.drain(..) {
            out(SeqEvent::Off { ch, note });
        }
    }

    fn mark_stopped(&mut self, now: Instant) {
        if self.stopped_at.is_none() {
            self.stopped_at = Some((self.pos, now));
        }
    }

    pub fn handle(&mut self, cmd: SongCommand, now: Instant, out: &mut impl FnMut(SeqEvent)) {
        match cmd {
            SongCommand::Load(d) => {
                self.all_off(out);
                self.data = d;
                self.playing = false;
                self.pos = 0.0;
                self.idx = 0;
                self.wait = None;
                self.looping = None;
                self.ended = false;
                self.seeks += 1;
                self.stopped_at = Some((0.0, now - Duration::from_secs(60)));
            }
            SongCommand::Play(on) => {
                if on && self.data.is_some() {
                    if self.ended {
                        self.seek(0.0, now, out);
                    }
                    self.playing = true;
                    if self.moving() {
                        self.stopped_at = None;
                    }
                } else {
                    self.playing = false;
                    self.all_off(out);
                    self.mark_stopped(now);
                }
            }
            SongCommand::Seek(t) => self.seek(t, now, out),
            SongCommand::Speed(s) => self.speed = s.clamp(0.1, 2.0),
            SongCommand::Mute(m) => {
                self.mute = m;
                let mut i = 0;
                while i < self.active.len() {
                    let (ch, note, track) = self.active[i];
                    if m & (1u64 << (track & 63)) != 0 {
                        out(SeqEvent::Off { ch, note });
                        self.active.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
            SongCommand::Wait(w) => {
                self.wait = w;
                if self.moving() {
                    self.stopped_at = None;
                } else if self.halted() {
                    self.mark_stopped(now);
                }
            }
            SongCommand::Loop(l) => {
                self.looping = l.filter(|(a, b)| b > a);
            }
            SongCommand::Gain(g) => self.gain = g.clamp(0.0, 1.5),
            SongCommand::Stems(s) => self.stems.set_stems(s),
            SongCommand::StemsOn(on) => self.stems.set_on(on),
            SongCommand::StemGains(g) => self.stems.set_gains(g),
            SongCommand::StemPitch(p) => self.stems.set_semitones(p),
        }
    }

    /// Add the stems for the next `l.len()` samples (call before `advance`).
    pub fn render_stems(&mut self, l: &mut [f32], r: &mut [f32]) {
        let ctx = SongCtx {
            pos: self.pos,
            speed: self.speed,
            playing: self.playing && self.data.is_some(),
            moving: self.moving(),
            wait: self.wait,
            jumps: self.seeks.wrapping_add(self.loops),
        };
        self.stems.render(l, r, ctx);
    }

    fn seek(&mut self, t: f64, now: Instant, out: &mut impl FnMut(SeqEvent)) {
        self.all_off(out);
        self.pos = t;
        self.idx = self.data.as_ref().map(|d| d.first_at(t)).unwrap_or(0);
        self.ended = false;
        self.seeks += 1;
        // Nothing is "on its way" after a jump.
        self.stopped_at = Some((t, now - Duration::from_secs(60)));
        if self.moving() {
            self.stopped_at = None;
        }
    }

    /// Send every event due at the current position.
    pub fn process(&mut self, now: Instant, out: &mut impl FnMut(SeqEvent)) {
        let Some(data) = self.data.clone() else {
            return;
        };
        if !self.playing {
            return;
        }
        // Loop end: jump back.
        if let Some((a, b)) = self.looping
            && self.pos >= b - 1e-9
        {
            self.all_off(out);
            self.pos = a;
            self.idx = data.first_at(a);
            self.loops += 1;
        }
        let limit = self.wait.unwrap_or(f64::INFINITY);
        while let Some(e) = data.events.get(self.idx) {
            if e.time > self.pos + 1e-9 || e.time >= limit && e.vel > 0 {
                break;
            }
            self.idx += 1;
            if self.mute & (1u64 << (e.track & 63)) != 0 {
                continue;
            }
            if e.vel == 0 {
                if let Some(i) = self
                    .active
                    .iter()
                    .position(|a| a.0 == e.ch && a.1 == e.note && a.2 == e.track)
                {
                    self.active.swap_remove(i);
                    out(SeqEvent::Off {
                        ch: e.ch,
                        note: e.note,
                    });
                }
            } else {
                if self.active.len() >= MAX_ACTIVE {
                    let (ch, note, _) = self.active.remove(0);
                    out(SeqEvent::Off { ch, note });
                }
                // Re-striking the same note: end the old one first.
                if let Some(i) = self
                    .active
                    .iter()
                    .position(|a| a.0 == e.ch && a.1 == e.note)
                {
                    self.active.swap_remove(i);
                    out(SeqEvent::Off {
                        ch: e.ch,
                        note: e.note,
                    });
                }
                let vel = ((e.vel as f32 * self.gain).round() as i32).clamp(1, 127) as u8;
                if self.gain > 0.01 {
                    self.active.push((e.ch, e.note, e.track));
                    out(SeqEvent::On {
                        ch: e.ch,
                        note: e.note,
                        vel,
                    });
                }
            }
        }
        if self.looping.is_none() && self.pos >= data.length {
            self.all_off(out);
            self.playing = false;
            self.ended = true;
            self.mark_stopped(now);
        }
        if self.halted() {
            self.mark_stopped(now);
        }
    }

    /// Samples until the next thing happens (event, loop end, wait point),
    /// or None when nothing is scheduled.
    pub fn samples_until_next(&self) -> Option<u64> {
        let data = self.data.as_ref()?;
        if !self.moving() {
            return None;
        }
        let mut next = data
            .events
            .get(self.idx)
            .map(|e| e.time)
            .unwrap_or(f64::INFINITY);
        if let Some(w) = self.wait {
            next = next.min(w);
        }
        if let Some((_, b)) = self.looping {
            next = next.min(b);
        } else {
            next = next.min(data.length);
        }
        if !next.is_finite() {
            return None;
        }
        let secs = ((next - self.pos) / self.speed).max(0.0);
        Some((secs * self.sample_rate).ceil() as u64)
    }

    /// Move the clock on by `frames` samples (stops exactly at a wait point).
    pub fn advance(&mut self, frames: usize) {
        if !self.moving() {
            return;
        }
        self.pos += frames as f64 / self.sample_rate * self.speed;
        if let Some(w) = self.wait
            && self.pos > w
        {
            self.pos = w;
        }
    }

    /// The clock for the UI. `at` = when the sample at the current position
    /// started (the callback's start time, plus the offset inside it).
    pub fn clock(&self, at: Instant) -> SongClock {
        let moving = self.moving();
        let (pos, at) = match (moving, self.stopped_at) {
            (false, Some((p, t))) => (p, t),
            _ => (self.pos, at),
        };
        SongClock {
            at,
            pos,
            speed: self.speed,
            moving,
            playing: self.playing,
            seeks: self.seeks,
            loops: self.loops,
            ended: self.ended,
        }
    }

    /// The moment the song starts moving again, forget the stop.
    pub fn after_render(&mut self) {
        if self.moving() {
            self.stopped_at = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Arc<SongData> {
        let mut ev = Vec::new();
        for (i, n) in [60u8, 62, 64, 65].iter().enumerate() {
            let t = i as f64 * 0.5;
            ev.push(SongEvent {
                time: t,
                ch: 3,
                note: *n,
                vel: 100,
                track: (i % 2) as u8,
            });
            ev.push(SongEvent {
                time: t + 0.4,
                ch: 3,
                note: *n,
                vel: 0,
                track: (i % 2) as u8,
            });
        }
        Arc::new(SongData::new(ev, 2.0))
    }

    /// Run the player like the renderer does; returns (sample, event) pairs.
    fn run(p: &mut SongPlayer, frames: usize) -> Vec<(u64, SeqEvent)> {
        let mut out = Vec::new();
        let mut done = 0u64;
        let now = Instant::now();
        while (done as usize) < frames {
            p.process(now, &mut |e| out.push((done, e)));
            let step = p
                .samples_until_next()
                .map(|s| s.max(1))
                .unwrap_or(u64::MAX)
                .min(frames as u64 - done)
                .min(128);
            p.advance(step as usize);
            p.after_render();
            done += step;
        }
        out
    }

    #[test]
    fn plays_events_on_time_and_at_half_speed() {
        let mut p = SongPlayer::new(1000);
        let mut sink = |_| {};
        let now = Instant::now();
        p.handle(SongCommand::Load(Some(data())), now, &mut sink);
        p.handle(SongCommand::Play(true), now, &mut sink);
        let ev = run(&mut p, 2500);
        let ons: Vec<u64> = ev
            .iter()
            .filter(|(_, e)| matches!(e, SeqEvent::On { .. }))
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(ons, vec![0, 500, 1000, 1500]);
        assert!(!p.playing && p.ended, "stops at the end");

        let mut p = SongPlayer::new(1000);
        p.handle(SongCommand::Load(Some(data())), now, &mut sink);
        p.handle(SongCommand::Speed(0.5), now, &mut sink);
        p.handle(SongCommand::Play(true), now, &mut sink);
        let ons: Vec<u64> = run(&mut p, 2500)
            .iter()
            .filter(|(_, e)| matches!(e, SeqEvent::On { .. }))
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(ons, vec![0, 1000, 2000]);
    }

    #[test]
    fn wait_mode_stops_before_the_next_notes() {
        let mut p = SongPlayer::new(1000);
        let now = Instant::now();
        let mut sink = |_| {};
        p.handle(SongCommand::Load(Some(data())), now, &mut sink);
        p.handle(SongCommand::Wait(Some(1.0)), now, &mut sink);
        p.handle(SongCommand::Play(true), now, &mut sink);
        let ev = run(&mut p, 3000);
        assert!((p.pos() - 1.0).abs() < 1e-9, "held at the wait point");
        let ons = ev
            .iter()
            .filter(|(_, e)| matches!(e, SeqEvent::On { .. }))
            .count();
        assert_eq!(ons, 2, "the notes at 1.0 wait too");
        let c = p.clock(Instant::now());
        assert!(!c.moving && c.playing);
        assert!((c.pos - 1.0).abs() < 1e-9);
        // Released: the next notes play straight away.
        p.handle(SongCommand::Wait(Some(1.5)), now, &mut sink);
        let ev = run(&mut p, 100);
        assert!(matches!(ev[0].1, SeqEvent::On { note: 64, .. }));
    }

    #[test]
    fn loops_and_mutes() {
        let mut p = SongPlayer::new(1000);
        let now = Instant::now();
        let mut sink = |_| {};
        p.handle(SongCommand::Load(Some(data())), now, &mut sink);
        p.handle(SongCommand::Loop(Some((0.5, 1.5))), now, &mut sink);
        p.handle(SongCommand::Mute(0b10), now, &mut sink);
        p.handle(SongCommand::Seek(0.5), now, &mut sink);
        p.handle(SongCommand::Play(true), now, &mut sink);
        let ev = run(&mut p, 3000);
        // Track 1 (notes 62, 65) muted: only 64 plays, once per loop pass.
        let ons: Vec<u8> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                SeqEvent::On { note, .. } => Some(*note),
                _ => None,
            })
            .collect();
        assert_eq!(ons, vec![64, 64, 64]);
        assert_eq!(p.clock(now).loops, 2);
        // Every note that went on came off (loops end notes).
        let offs = ev
            .iter()
            .filter(|(_, e)| matches!(e, SeqEvent::Off { .. }))
            .count();
        assert!(offs >= 2);
    }

    #[test]
    fn seeking_resets_and_reports() {
        let mut p = SongPlayer::new(1000);
        let now = Instant::now();
        let mut out = Vec::new();
        p.handle(SongCommand::Load(Some(data())), now, &mut |e| out.push(e));
        p.handle(SongCommand::Play(true), now, &mut |e| out.push(e));
        run(&mut p, 200);
        let seeks = p.clock(now).seeks;
        p.handle(SongCommand::Seek(1.2), now, &mut |e| out.push(e));
        assert!(matches!(out.last(), Some(SeqEvent::Off { note: 60, .. })));
        assert_eq!(p.clock(now).seeks, seeks + 1);
        // Negative positions are fine (count-in before the song).
        p.handle(SongCommand::Seek(-1.0), now, &mut |e| out.push(e));
        let ev = run(&mut p, 1100);
        assert_eq!(
            ev.iter()
                .find(|(_, e)| matches!(e, SeqEvent::On { .. }))
                .unwrap()
                .0,
            1000
        );
    }
}
