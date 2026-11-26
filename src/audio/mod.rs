//! The real-time audio engine.
//!
//! The UI thread sends [`Command`]s over a lock-free channel; the audio
//! callback drains them at the start of every buffer, so a key press is
//! heard on the very next buffer (~1.3 ms at 64 frames / 48 kHz).

pub mod builtin;
pub mod looper;
pub mod patterns;
pub mod sequencer;
pub mod songplay;

use crate::midiout::MidiMsg;
use crate::record::RecEvent;
use anyhow::{Context, Result, anyhow};
use builtin::BuiltinSynth;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SizedSample};
use crossbeam_channel::{Receiver, Sender, unbounded};
use looper::{LoopCommand, LoopState, LoopStatus, Looper};
use patterns::Pattern;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use sequencer::{CH_DRUMS, ChordNotes, SeqEvent, Sequencer, StyleParams};
use songplay::{SongClock, SongCommand, SongPlayer};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum Command {
    NoteOn {
        note: i32,
        velocity: i32,
    },
    NoteOff {
        note: i32,
    },
    Sustain(bool),
    Program(u8),
    Volume(f32),
    AllOff,
    /// Swap in a SoundFont synth that was built on a background thread.
    UseSoundFont(Box<Synthesizer>),
    UseBuiltin,
    /// Start/stop the latency-test metronome.
    Metronome {
        bpm: f32,
        on: bool,
    },
    /// Instrument for one channel (1 = chords, 2 = bass).
    ChannelProgram {
        channel: u8,
        program: u8,
    },
    /// Chord key pressed (`Some`) or released (`None`). `retrigger` is
    /// false when only a modifier changed the chord.
    Chord {
        chord: Option<ChordNotes>,
        retrigger: bool,
        velocity: u8,
    },
    Style(StyleParams),
    /// Patterns are owned by the UI too, so the audio thread never frees one.
    Pattern(Option<Arc<Pattern>>),
    Tempo(f32),
    /// Start / stop the accompaniment rhythm.
    Rhythm(bool),
    /// One drum hit (General MIDI drum note).
    Drum {
        note: u8,
        velocity: u8,
    },
    /// Song mode playback.
    Song(SongCommand),
    /// Note on / off on any channel (one-key mode plays the song's notes).
    ChannelNote {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    /// Your metronome on / off (follows `Tempo`, accent every 4 beats).
    Click(bool),
    /// Start / stop recording what you play. `count_in` beats of clicks
    /// first (the metronome plays them even when it's off).
    Record {
        on: bool,
        count_in: u32,
    },
    /// The looper (Part 4b).
    Loop(LoopCommand),
    /// Virtual MIDI out (Part 4b): where to send (None = off), whether the
    /// song backing goes out too, and whether to still play the sound here.
    MidiOut {
        tx: Option<Sender<MidiMsg>>,
        song: bool,
        local: bool,
    },
}

/// Numbers the UI shows about the engine. Written by the audio thread.
#[derive(Default)]
pub struct Stats {
    pub sample_rate: AtomicU32,
    pub channels: AtomicU32,
    pub buffer_frames: AtomicU32,
    /// Latency the audio system reports between callback and playback, in µs.
    pub reported_latency_us: AtomicU64,
    pub active_voices: AtomicU32,
    pub callbacks: AtomicU64,
    pub errors: AtomicU32,
    pub soundfont_active: AtomicBool,
    /// Accompaniment clock, for the beat display.
    pub seq_running: AtomicBool,
    pub seq_step: AtomicU32,
    pub seq_steps_per_bar: AtomicU32,
    pub seq_steps_per_beat: AtomicU32,
    pub host: Mutex<String>,
    pub device: Mutex<String>,
    /// Song mode clock (updated with try_lock, never blocks audio).
    pub song_clock: Mutex<SongClock>,
    /// Looper state for the UI (same: try_lock only).
    pub looper: Mutex<LoopStatus>,
}

impl Stats {
    pub fn buffer_ms(&self) -> f32 {
        let sr = self.sample_rate.load(Ordering::Relaxed).max(1) as f32;
        self.buffer_frames.load(Ordering::Relaxed) as f32 * 1000.0 / sr
    }
    pub fn reported_ms(&self) -> f32 {
        self.reported_latency_us.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

#[allow(clippy::large_enum_variant)] // only one exists, never moved in the hot path
enum Backend {
    Builtin(BuiltinSynth),
    SoundFont(Box<Synthesizer>),
}

struct Metronome {
    on: bool,
    interval: u64,
    next_at: u64,
    beat: u32,
    click_pos: Option<(usize, bool)>,
    /// Started by `Click`: follows tempo changes.
    user: bool,
    /// Count-in only: stop clicking here.
    off_at: Option<u64>,
}

/// Recording tap: copies what you play (not song playback) to the UI.
struct Rec {
    on: bool,
    start: u64,
    tx: Sender<(u64, RecEvent)>,
}

impl Rec {
    fn tap(&self, t: u64, e: RecEvent) {
        if self.on && t >= self.start {
            let _ = self.tx.try_send((t - self.start, e));
        }
    }
    fn seq(&self, t: u64, e: SeqEvent) {
        self.tap(
            t,
            match e {
                SeqEvent::On { ch, note, vel } => RecEvent::Note { ch, note, vel },
                SeqEvent::Off { ch, note } => RecEvent::Note { ch, note, vel: 0 },
            },
        );
    }
}

/// Where sound events go: the synth, and MIDI out when it's on.
struct Out {
    backend: Backend,
    midi: Option<Sender<MidiMsg>>,
    /// When the audio being rendered right now starts (MIDI time stamps).
    at: Instant,
    /// Make sound here too (off = MIDI out only).
    local: bool,
    /// Send the song backing as MIDI too.
    song_midi: bool,
}

impl Out {
    fn midi(&self, m: MidiMsg) {
        if let Some(tx) = &self.midi {
            let _ = tx.try_send(m);
        }
    }
    fn midi_event(&self, e: SeqEvent) {
        if self.midi.is_some() {
            self.midi(match e {
                SeqEvent::On { ch, note, vel } => MidiMsg::note_on(self.at, ch, note, vel),
                SeqEvent::Off { ch, note } => MidiMsg::note_off(self.at, ch, note),
            });
        }
    }
}

/// Something you (or the looper) play: the synth + MIDI out.
fn out_event(out: &mut Out, e: SeqEvent) {
    if out.local {
        backend_event(&mut out.backend, e);
    }
    out.midi_event(e);
}

/// Song playback: MIDI out only if asked; always heard here unless it goes out.
fn song_event(out: &mut Out, e: SeqEvent) {
    if out.local || !out.song_midi {
        backend_event(&mut out.backend, e);
    }
    if out.song_midi {
        out.midi_event(e);
    }
}

/// What you play is seen by the recorder and by the looper.
fn tap(rec: &Rec, looper: &mut Looper, programs: &[u8; 16], t: u64, e: SeqEvent) {
    rec.seq(t, e);
    looper.record(t, e, programs);
}

/// State that lives inside the audio callback.
struct Renderer {
    out: Out,
    builtin_fallback: BuiltinSynth,
    rx: Receiver<Command>,
    click_tx: Sender<Instant>,
    stats: Arc<Stats>,
    sample_rate: u32,
    /// Program per channel: 0 melody, 1 chords, 2 bass, 3.. song parts.
    programs: [u8; 16],
    volume: f32,
    sustain: bool,
    held: [bool; 128],
    sustained: [bool; 128],
    left: Vec<f32>,
    right: Vec<f32>,
    sample_clock: u64,
    metro: Metronome,
    seq: Sequencer,
    song: SongPlayer,
    tempo: f32,
    rec: Rec,
    pub rec_rx: Receiver<(u64, RecEvent)>,
    looper: Looper,
    /// The looper switched the clicks on for its first layer.
    loop_click: bool,
}

const CLICK_LEN: usize = 1200;

impl Renderer {
    fn new(
        sample_rate: u32,
        program: u8,
        volume: f32,
        rx: Receiver<Command>,
        click_tx: Sender<Instant>,
        stats: Arc<Stats>,
    ) -> Self {
        let (rec_tx, rec_rx) = crossbeam_channel::bounded(8192);
        let mut builtin = BuiltinSynth::new(sample_rate);
        for ch in 0..3 {
            builtin.set_program(ch, program);
        }
        Renderer {
            out: Out {
                backend: Backend::Builtin(builtin),
                midi: None,
                at: Instant::now(),
                local: true,
                song_midi: false,
            },
            builtin_fallback: BuiltinSynth::new(sample_rate),
            rx,
            click_tx,
            stats,
            sample_rate,
            programs: [program; 16],
            volume,
            sustain: false,
            held: [false; 128],
            sustained: [false; 128],
            left: vec![0.0; 8192],
            right: vec![0.0; 8192],
            sample_clock: 0,
            metro: Metronome {
                on: false,
                interval: 1,
                next_at: 0,
                beat: 0,
                click_pos: None,
                user: false,
                off_at: None,
            },
            seq: Sequencer::new(sample_rate),
            song: SongPlayer::new(sample_rate),
            tempo: 100.0,
            rec: Rec {
                on: false,
                start: 0,
                tx: rec_tx,
            },
            rec_rx,
            looper: Looper::new(sample_rate),
            loop_click: false,
        }
    }

    /// Something you played right now: recorder, looper, synth, MIDI.
    fn play(&mut self, e: SeqEvent) {
        tap(
            &self.rec,
            &mut self.looper,
            &self.programs,
            self.sample_clock,
            e,
        );
        out_event(&mut self.out, e);
    }

    fn handle(&mut self, cmd: Command, now: Instant) {
        self.out.at = now;
        match cmd {
            Command::Song(c) => {
                let (song, out) = (&mut self.song, &mut self.out);
                song.handle(c, now, &mut |e| song_event(out, e));
            }
            Command::Loop(c) => self.loop_command(c),
            Command::MidiOut { tx, song, local } => {
                self.out.song_midi = song;
                self.out.local = local || tx.is_none();
                // Swapping the sender never frees the channel here: the
                // MIDI thread owns the other end.
                self.out.midi = tx;
                for ch in 0..16u8 {
                    if ch != CH_DRUMS {
                        self.out
                            .midi(MidiMsg::program(now, ch, self.programs[ch as usize]));
                    }
                }
            }
            Command::ChannelNote {
                channel,
                note,
                velocity,
            } => out_event(
                &mut self.out,
                if velocity > 0 {
                    SeqEvent::On {
                        ch: channel & 15,
                        note: note & 127,
                        vel: velocity.min(127),
                    }
                } else {
                    SeqEvent::Off {
                        ch: channel & 15,
                        note: note & 127,
                    }
                },
            ),
            Command::NoteOn { note, velocity } => {
                if !(0..128).contains(&note) {
                    return;
                }
                let n = note as usize;
                // Re-striking a sustained note: release the old one first.
                if self.held[n] || self.sustained[n] {
                    self.backend_off(note);
                }
                self.held[n] = true;
                self.sustained[n] = false;
                self.play(SeqEvent::On {
                    ch: 0,
                    note: n as u8,
                    vel: velocity.clamp(1, 127) as u8,
                });
            }
            Command::NoteOff { note } => {
                if !(0..128).contains(&note) {
                    return;
                }
                let n = note as usize;
                self.held[n] = false;
                if self.sustain {
                    self.sustained[n] = true;
                } else {
                    self.backend_off(note);
                }
            }
            Command::Sustain(on) => {
                self.sustain = on;
                let (seq, out, rec, looper, programs, t) = (
                    &mut self.seq,
                    &mut self.out,
                    &self.rec,
                    &mut self.looper,
                    &self.programs,
                    self.sample_clock,
                );
                seq.set_pedal(on, t, &mut |e| {
                    tap(rec, looper, programs, t, e);
                    out_event(out, e)
                });
                if !on {
                    for n in 0..128 {
                        if self.sustained[n] && !self.held[n] {
                            self.sustained[n] = false;
                            self.backend_off(n as i32);
                        }
                    }
                }
            }
            Command::Program(p) => {
                self.rec
                    .tap(self.sample_clock, RecEvent::Program { ch: 0, program: p });
                self.programs[0] = p;
                self.out.midi(MidiMsg::program(now, 0, p));
                self.apply_program();
            }
            Command::ChannelProgram { channel, program } => {
                if let Some(slot) = self.programs.get_mut(channel as usize) {
                    if channel < 3 {
                        self.rec.tap(
                            self.sample_clock,
                            RecEvent::Program {
                                ch: channel,
                                program,
                            },
                        );
                    }
                    *slot = program;
                    self.out.midi(MidiMsg::program(now, channel, program));
                    self.apply_program();
                }
            }
            Command::Volume(v) => self.volume = v.clamp(0.0, 2.0),
            Command::AllOff => {
                self.rec.tap(self.sample_clock, RecEvent::AllOff);
                // The loop keeps going (losing window focus sends this too);
                // a layer being recorded closes its held notes.
                self.looper.all_off(self.sample_clock);
                self.held = [false; 128];
                self.sustained = [false; 128];
                self.seq.reset();
                self.song.handle(SongCommand::Play(false), now, &mut |_| {});
                for ch in 0..16 {
                    self.out.midi(MidiMsg::control(now, ch, 123, 0));
                }
                match &mut self.out.backend {
                    Backend::Builtin(s) => s.all_off(),
                    Backend::SoundFont(s) => s.note_off_all(true),
                }
            }
            Command::UseSoundFont(synth) => {
                self.seq.reset();
                self.out.backend = Backend::SoundFont(synth);
                self.stats.soundfont_active.store(true, Ordering::Relaxed);
                self.apply_program();
            }
            Command::UseBuiltin => {
                self.seq.reset();
                let mut b = BuiltinSynth::new(self.sample_rate);
                std::mem::swap(&mut b, &mut self.builtin_fallback);
                self.out.backend = Backend::Builtin(b);
                self.stats.soundfont_active.store(false, Ordering::Relaxed);
                self.apply_program();
            }
            Command::Metronome { bpm, on } => {
                self.metro.on = on;
                self.metro.interval =
                    (self.sample_rate as f64 * 60.0 / bpm.max(20.0) as f64) as u64;
                self.metro.next_at = self.sample_clock + self.sample_rate as u64 / 4;
                self.metro.beat = 0;
                self.metro.user = false;
                self.metro.off_at = None;
            }
            Command::Click(on) => {
                self.metro.on = on;
                self.metro.user = on;
                self.metro.off_at = None;
                self.loop_click = false;
                if on {
                    match self.looper.next_beat(self.sample_clock) {
                        // Click on the loop's beats.
                        Some((at, beat)) => {
                            self.metro.interval = self.beat_samples();
                            self.metro.next_at = at;
                            self.metro.beat = beat;
                        }
                        None => self.restart_clicks(),
                    }
                }
            }
            Command::Record { on, count_in } => {
                self.rec.on = on;
                if !on {
                    if self.metro.off_at.take().is_some() {
                        self.metro.on = false;
                    }
                    return;
                }
                self.rec.start = self.sample_clock;
                if let Some(top) = self.looper.next_pass_start() {
                    // A loop is playing: the take starts at its top.
                    self.rec.start = top;
                } else if count_in > 0 {
                    if !self.metro.on {
                        self.metro.on = true;
                        self.metro.user = true;
                        self.metro.off_at = Some(0);
                    }
                    self.restart_clicks();
                    self.rec.start = self.metro.next_at + count_in as u64 * self.metro.interval;
                    if self.metro.off_at.is_some() {
                        // Stop just before the first recorded beat's click.
                        self.metro.off_at = Some(self.rec.start);
                    }
                }
                let lent = self.looper.channels_in_use();
                for ch in 0..16u8 {
                    if ch < 3 || (lent & (1 << ch) != 0 && ch != CH_DRUMS) {
                        let program = self.programs[ch as usize];
                        self.rec
                            .tap(self.rec.start, RecEvent::Program { ch, program });
                    }
                }
            }
            Command::Drum { note, velocity } => self.play(SeqEvent::On {
                ch: CH_DRUMS,
                note,
                vel: velocity,
            }),
            cmd => {
                let now = self.sample_clock;
                if let Command::Tempo(bpm) = &cmd {
                    let bpm = *bpm;
                    self.tempo = bpm;
                    self.looper.set_tempo(bpm, now);
                    if self.metro.user {
                        self.metro.interval = self.beat_samples();
                        // Keep clicking on the loop's beats.
                        if let Some((at, beat)) = self.looper.next_beat(now) {
                            self.metro.next_at = at;
                            self.metro.beat = beat;
                        }
                    }
                }
                let (seq, o, rec, looper, programs) = (
                    &mut self.seq,
                    &mut self.out,
                    &self.rec,
                    &mut self.looper,
                    &self.programs,
                );
                let mut out = |e| {
                    tap(rec, looper, programs, now, e);
                    out_event(o, e)
                };
                match cmd {
                    Command::Chord {
                        chord,
                        retrigger,
                        velocity,
                    } => seq.set_chord(chord, retrigger, velocity, now, &mut out),
                    Command::Style(p) => seq.set_style(p, now, &mut out),
                    Command::Pattern(p) => seq.set_pattern(p, now, &mut out),
                    Command::Tempo(bpm) => seq.set_tempo(bpm),
                    Command::Rhythm(on) => seq.set_running(on, now, &mut out),
                    _ => {}
                }
            }
        }
    }

    fn loop_command(&mut self, c: LoopCommand) {
        let now = self.sample_clock;
        let mut start = now;
        if let LoopCommand::Rec { count_in, .. } = c
            && self.looper.state() == LoopState::Empty
        {
            // The loop starts on a click, after the count-in. If the
            // metronome is off it clicks until the first layer is done.
            if !(self.metro.on && self.metro.user) {
                self.metro.on = true;
                self.metro.user = true;
                self.metro.off_at = None;
                self.loop_click = true;
                self.restart_clicks();
            }
            self.metro.beat = 0;
            start = self.metro.next_at + count_in as u64 * self.metro.interval;
        }
        let (looper, out, rec) = (&mut self.looper, &mut self.out, &self.rec);
        if let Some(g) = looper.command(c, now, start, &mut |e| {
            rec.seq(now, e);
            out_event(out, e)
        }) {
            self.align_clicks(g);
        }
        self.end_loop_click();
    }

    /// The loop (re)started at sample `g`: put the clicks on its beats.
    fn align_clicks(&mut self, g: u64) {
        if self.metro.on && self.metro.user {
            self.metro.interval = self.beat_samples();
            self.metro.next_at = g;
            self.metro.beat = 0;
        }
    }

    /// Clicks the looper started stop once the loop plays (or is dropped).
    fn end_loop_click(&mut self) {
        if self.loop_click && !matches!(self.looper.state(), LoopState::CountIn | LoopState::First)
        {
            self.loop_click = false;
            self.metro.on = false;
            self.metro.user = false;
        }
    }

    /// Lent channels get the instrument their layer was recorded with.
    fn loop_programs(&mut self) {
        while let Some((ch, program)) = self.looper.take_program() {
            self.programs[ch as usize] = program;
            match &mut self.out.backend {
                Backend::Builtin(s) => s.set_program(ch, program),
                Backend::SoundFont(s) => s.process_midi_message(ch as i32, 0xC0, program as i32, 0),
            }
            self.out.midi(MidiMsg::program(self.out.at, ch, program));
            self.rec
                .tap(self.sample_clock, RecEvent::Program { ch, program });
        }
    }

    fn beat_samples(&self) -> u64 {
        (self.sample_rate as f64 * 60.0 / self.tempo.clamp(20.0, 400.0) as f64) as u64
    }

    /// Clicks at the current tempo, first one (an accent) in ~50 ms.
    fn restart_clicks(&mut self) {
        self.metro.interval = self.beat_samples();
        self.metro.next_at = self.sample_clock + self.sample_rate as u64 / 20;
        self.metro.beat = 0;
    }

    fn backend_off(&mut self, note: i32) {
        self.play(SeqEvent::Off {
            ch: 0,
            note: note.clamp(0, 127) as u8,
        });
    }

    fn apply_program(&mut self) {
        for (ch, &prog) in self.programs.iter().enumerate() {
            if ch == CH_DRUMS as usize {
                continue;
            }
            match &mut self.out.backend {
                Backend::Builtin(s) => s.set_program(ch as u8, prog),
                // 0xC0 = program change.
                Backend::SoundFont(s) => s.process_midi_message(ch as i32, 0xC0, prog as i32, 0),
            }
        }
    }

    /// Render `frames` of stereo audio into `self.left` / `self.right`.
    /// Separate from the cpal callback so it can be tested offline.
    fn render_block(&mut self, frames: usize, started: Instant) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.handle(cmd, started);
        }
        let sr = self.sample_rate as f64;
        let at = |frame: usize| started + Duration::from_secs_f64(frame as f64 / sr);
        if self.left.len() < frames {
            // Only happens if the host changes buffer size; allocation is rare.
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
        }
        // Render in pieces, split wherever the sequencer has something to
        // play, so accompaniment timing is exact to the sample.
        if let Ok(mut s) = self.stats.looper.try_lock() {
            *s = self.looper.status(self.sample_clock);
        }
        let mut pos = 0;
        while pos < frames {
            let now = self.sample_clock + pos as u64;
            self.out.at = at(pos);
            {
                let (seq, out, rec, looper, programs) = (
                    &mut self.seq,
                    &mut self.out,
                    &self.rec,
                    &mut self.looper,
                    &self.programs,
                );
                seq.process(now, &mut |e| {
                    tap(rec, looper, programs, now, e);
                    out_event(out, e)
                });
                let grid = looper.process(now, &mut |e| {
                    rec.seq(now, e);
                    out_event(out, e)
                });
                let (song, out) = (&mut self.song, &mut self.out);
                song.process(at(pos), &mut |e| song_event(out, e));
                if let Some(g) = grid {
                    self.align_clicks(g);
                }
            }
            let next = match (self.seq.next_time(), self.looper.next_time()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let mut end = match next {
                Some(t) => ((t.max(now + 1) - self.sample_clock) as usize).min(frames),
                None => frames,
            };
            if let Some(n) = self.song.samples_until_next() {
                end = end.min(pos + (n as usize).max(1));
            }
            let (l, r) = (&mut self.left[pos..end], &mut self.right[pos..end]);
            match &mut self.out.backend {
                Backend::Builtin(s) => s.render(l, r),
                Backend::SoundFont(s) => s.render(l, r),
            }
            self.song.advance(end - pos);
            self.song.after_render();
            pos = end;
        }
        self.end_loop_click();
        self.loop_programs();
        if let Ok(mut c) = self.stats.song_clock.try_lock() {
            *c = self.song.clock(at(frames));
        }
        if let Backend::Builtin(s) = &self.out.backend {
            self.stats
                .active_voices
                .store(s.active_voices() as u32, Ordering::Relaxed);
        }
        self.stats
            .seq_running
            .store(self.seq.running(), Ordering::Relaxed);
        self.stats
            .seq_step
            .store(self.seq.current_step(), Ordering::Relaxed);
        self.stats
            .seq_steps_per_bar
            .store(self.seq.steps_per_bar(), Ordering::Relaxed);
        self.stats
            .seq_steps_per_beat
            .store(self.seq.steps_per_beat(), Ordering::Relaxed);
        let (l, r) = (&mut self.left[..frames], &mut self.right[..frames]);

        // Metronome clicks (latency test).
        if self.metro.on {
            for i in 0..frames {
                let t = self.sample_clock + i as u64;
                if self.metro.off_at.is_some_and(|o| t >= o) {
                    self.metro.off_at = None;
                    self.metro.on = false;
                    self.metro.user = false;
                    break;
                }
                if t >= self.metro.next_at {
                    let accent = self.metro.beat.is_multiple_of(4);
                    self.metro.click_pos = Some((0, accent));
                    self.metro.next_at += self.metro.interval;
                    self.metro.beat += 1;
                    let at = started + Duration::from_secs_f64(i as f64 / self.sample_rate as f64);
                    let _ = self.click_tx.try_send(at);
                }
                if let Some((pos, accent)) = self.metro.click_pos {
                    let f = if accent { 1760.0 } else { 1320.0 };
                    let env = 1.0 - pos as f32 / CLICK_LEN as f32;
                    let s = (pos as f32 * f * std::f32::consts::TAU / self.sample_rate as f32)
                        .sin()
                        * env
                        * env
                        * 0.5;
                    l[i] += s;
                    r[i] += s;
                    self.metro.click_pos = if pos + 1 < CLICK_LEN {
                        Some((pos + 1, accent))
                    } else {
                        None
                    };
                }
            }
        }
        let vol = self.volume;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            *a = (*a * vol).clamp(-1.0, 1.0);
            *b = (*b * vol).clamp(-1.0, 1.0);
        }
        self.sample_clock += frames as u64;
    }

    fn render<T: SizedSample + FromSample<f32>>(
        &mut self,
        data: &mut [T],
        channels: usize,
        info: &OutputCallbackInfo,
    ) {
        let started = Instant::now();
        let frames = data.len() / channels.max(1);
        self.render_block(frames, started);
        for (i, frame) in data.chunks_mut(channels).enumerate() {
            let (a, b) = (self.left[i], self.right[i]);
            for (c, out) in frame.iter_mut().enumerate() {
                *out = T::from_sample(if c % 2 == 0 { a } else { b });
            }
        }
        let ts = info.timestamp();
        let lat = ts.playback.duration_since(ts.callback);
        self.stats
            .reported_latency_us
            .store(lat.as_micros() as u64, Ordering::Relaxed);
        self.stats
            .buffer_frames
            .store(frames as u32, Ordering::Relaxed);
        self.stats.callbacks.fetch_add(1, Ordering::Relaxed);
    }
}

/// Send one note event to whichever synth is active.
fn backend_event(backend: &mut Backend, e: SeqEvent) {
    match (backend, e) {
        (
            Backend::Builtin(s),
            SeqEvent::On {
                ch: CH_DRUMS,
                note,
                vel,
            },
        ) => s.drum(note, vel),
        (Backend::Builtin(_), SeqEvent::Off { ch: CH_DRUMS, .. }) => {}
        (Backend::Builtin(s), SeqEvent::On { ch, note, vel }) => {
            s.note_on(ch, note as i32, vel as i32)
        }
        (Backend::Builtin(s), SeqEvent::Off { ch, note }) => s.note_off(ch, note as i32),
        (Backend::SoundFont(s), SeqEvent::On { ch, note, vel }) => {
            s.note_on(ch as i32, note as i32, vel as i32)
        }
        (Backend::SoundFont(s), SeqEvent::Off { ch, note }) => s.note_off(ch as i32, note as i32),
    }
}

/// Handle owned by the UI. Dropping it stops audio.
pub struct Engine {
    tx: Sender<Command>,
    pub stats: Arc<Stats>,
    pub clicks: Receiver<Instant>,
    /// What you played while recording (see `Command::Record`).
    pub recorded: Receiver<(u64, RecEvent)>,
    _stream: cpal::Stream,
}

impl Engine {
    /// Open the default output with the requested buffer size (frames).
    pub fn start(buffer_frames: u32, program: u8, volume: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no audio output device found"))?;
        let default = device
            .default_output_config()
            .context("reading output config")?;
        let sample_rate = default.sample_rate();
        let channels = default.channels() as usize;

        let stats = Arc::new(Stats::default());
        stats.sample_rate.store(sample_rate, Ordering::Relaxed);
        stats.channels.store(channels as u32, Ordering::Relaxed);
        *stats.host.lock().unwrap() = format!("{:?}", host.id());
        *stats.device.lock().unwrap() = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "default".into());

        let (tx, rx) = unbounded();
        let (click_tx, clicks) = crossbeam_channel::bounded(64);
        let renderer = Renderer::new(sample_rate, program, volume, rx, click_tx, stats.clone());
        let recorded = renderer.rec_rx.clone();

        let mut config: cpal::StreamConfig = default.config();
        config.buffer_size = cpal::BufferSize::Fixed(buffer_frames);
        let fmt = default.sample_format();
        let renderer = Arc::new(Mutex::new(Some(renderer)));

        // Try the requested small buffer first, then let the system choose.
        let stream =
            build_stream(&device, config, fmt, renderer.clone(), stats.clone()).or_else(|e| {
                eprintln!(
                    "qwertone: fixed buffer of {buffer_frames} frames failed ({e}), using default"
                );
                config.buffer_size = cpal::BufferSize::Default;
                build_stream(&device, config, fmt, renderer, stats.clone())
            })?;
        stream.play().context("starting audio stream")?;

        Ok(Self {
            tx,
            stats,
            clicks,
            recorded,
            _stream: stream,
        })
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub fn note_on(&self, note: i32, velocity: i32) {
        self.send(Command::NoteOn { note, velocity });
    }

    pub fn note_off(&self, note: i32) {
        self.send(Command::NoteOff { note });
    }

    /// Load a SoundFont on a background thread, then hand it to the audio
    /// thread. `done` receives the result message for the UI.
    pub fn load_soundfont_async(&self, path: &Path, done: Sender<Result<String, String>>) {
        let path = path.to_path_buf();
        let tx = self.tx.clone();
        let sr = self.stats.sample_rate.load(Ordering::Relaxed) as i32;
        std::thread::spawn(move || {
            let res = (|| -> Result<String> {
                let mut f = std::io::BufReader::new(
                    std::fs::File::open(&path)
                        .with_context(|| format!("opening {}", path.display()))?,
                );
                let sf = Arc::new(SoundFont::new(&mut f).map_err(|e| anyhow!("{e:?}"))?);
                let mut settings = SynthesizerSettings::new(sr);
                settings.block_size = 32;
                settings.maximum_polyphony = 96;
                let synth = Synthesizer::new(&sf, &settings).map_err(|e| anyhow!("{e:?}"))?;
                let name = sf.get_info().get_bank_name().to_string();
                let _ = tx.send(Command::UseSoundFont(Box::new(synth)));
                Ok(if name.trim().is_empty() {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    name
                })
            })();
            let _ = done.send(res.map_err(|e| format!("{e:#}")));
        });
    }
}

fn build_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    fmt: cpal::SampleFormat,
    renderer: Arc<Mutex<Option<Renderer>>>,
    stats: Arc<Stats>,
) -> Result<cpal::Stream> {
    use cpal::SampleFormat as F;
    match fmt {
        F::I16 => build_typed::<i16>(device, config, renderer, stats),
        F::I32 => build_typed::<i32>(device, config, renderer, stats),
        F::U16 => build_typed::<u16>(device, config, renderer, stats),
        _ => build_typed::<f32>(device, config, renderer, stats),
    }
}

fn build_typed<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    renderer: Arc<Mutex<Option<Renderer>>>,
    stats: Arc<Stats>,
) -> Result<cpal::Stream> {
    let channels = config.channels as usize;
    // Take the renderer out of the shared slot only once the stream is built
    // so a failed attempt leaves it for the retry. The mutex is never
    // touched from the callback after that.
    let slot = renderer.clone();
    let mut local: Option<Renderer> = None;
    let stream = device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &OutputCallbackInfo| {
            if local.is_none() {
                local = slot.lock().ok().and_then(|mut g| g.take());
            }
            match local.as_mut() {
                Some(r) => r.render(data, channels, info),
                None => data.iter_mut().for_each(|s| *s = T::from_sample(0.0)),
            }
        },
        move |err| {
            stats.errors.fetch_add(1, Ordering::Relaxed);
            eprintln!("qwertone audio: {err}");
        },
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> (Renderer, Sender<Command>, Receiver<Instant>) {
        let (tx, rx) = unbounded();
        let (ctx, crx) = crossbeam_channel::bounded(64);
        (
            Renderer::new(48_000, 0, 1.0, rx, ctx, Arc::new(Stats::default())),
            tx,
            crx,
        )
    }

    fn peak(r: &Renderer, n: usize) -> f32 {
        r.left[..n].iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    #[test]
    fn note_plays_and_sustain_holds_it() {
        let (mut r, tx, _) = renderer();
        r.render_block(256, Instant::now());
        assert_eq!(peak(&r, 256), 0.0, "silent before any note");

        tx.send(Command::NoteOn {
            note: 60,
            velocity: 100,
        })
        .unwrap();
        r.render_block(256, Instant::now());
        assert!(
            peak(&r, 256) > 0.01,
            "note should sound on the very next buffer"
        );

        // Pedal down, release key: the note keeps sounding.
        tx.send(Command::Sustain(true)).unwrap();
        tx.send(Command::NoteOff { note: 60 }).unwrap();
        r.render_block(256, Instant::now());
        assert!(r.sustained[60]);
        // Pedal up: it gets released.
        tx.send(Command::Sustain(false)).unwrap();
        r.render_block(256, Instant::now());
        assert!(!r.sustained[60]);
    }

    #[test]
    fn accompaniment_plays_through_the_renderer() {
        let (mut r, tx, _) = renderer();
        tx.send(Command::Tempo(120.0)).unwrap();
        tx.send(Command::Style(StyleParams {
            kind: sequencer::StyleKind::Arp,
            ..Default::default()
        }))
        .unwrap();
        tx.send(Command::Chord {
            chord: Some(ChordNotes::new(&[60, 64, 67], Some(48))),
            retrigger: true,
            velocity: 100,
        })
        .unwrap();
        let mut loud = 0.0f32;
        for _ in 0..100 {
            r.render_block(128, Instant::now());
            loud = loud.max(peak(&r, 128));
        }
        assert!(loud > 0.01);
        assert!(r.stats.seq_running.load(Ordering::Relaxed));
        tx.send(Command::Drum {
            note: 38,
            velocity: 120,
        })
        .unwrap();
        tx.send(Command::AllOff).unwrap();
        r.render_block(128, Instant::now());
        assert!(!r.seq.running());
    }

    #[test]
    fn records_after_count_in() {
        let (mut r, tx, clicks) = renderer();
        tx.send(Command::Tempo(120.0)).unwrap();
        tx.send(Command::Record {
            on: true,
            count_in: 4,
        })
        .unwrap();
        // Played during the count-in: not recorded.
        tx.send(Command::NoteOn {
            note: 60,
            velocity: 100,
        })
        .unwrap();
        r.render_block(128, Instant::now());
        // 50 ms lead + 4 beats of 0.5 s = 2.05 s; play at ~2.2 s.
        for _ in 0..(105_600 / 128) {
            r.render_block(128, Instant::now());
        }
        tx.send(Command::NoteOn {
            note: 64,
            velocity: 90,
        })
        .unwrap();
        tx.send(Command::Drum {
            note: 36,
            velocity: 100,
        })
        .unwrap();
        r.render_block(128, Instant::now());
        tx.send(Command::NoteOff { note: 64 }).unwrap();
        tx.send(Command::Record {
            on: false,
            count_in: 0,
        })
        .unwrap();
        for _ in 0..400 {
            r.render_block(128, Instant::now());
        }
        assert_eq!(
            clicks.try_iter().count(),
            4,
            "count-in clicks, then silence"
        );
        let evs: Vec<_> = r.rec_rx.try_iter().collect();
        let notes: Vec<_> = evs
            .iter()
            .filter(|(_, e)| matches!(e, RecEvent::Note { vel: 1.., .. }))
            .collect();
        assert_eq!(notes.len(), 2, "{evs:?}");
        let t = notes[0].0 as i64;
        assert!((t - 7_200).abs() < 200, "first note ~0.15 s in, got {t}");
        assert!(evs.iter().any(|(_, e)| *e
            == RecEvent::Note {
                ch: 0,
                note: 64,
                vel: 0
            }));
        assert!(evs.iter().any(|(_, e)| *e
            == RecEvent::Note {
                ch: CH_DRUMS,
                note: 36,
                vel: 100
            }));
    }

    #[test]
    fn looper_plays_in_time_with_the_clicks_and_goes_out_as_midi() {
        let (mut r, tx, clicks) = renderer();
        let (mtx, mrx) = crossbeam_channel::bounded(1024);
        tx.send(Command::MidiOut {
            tx: Some(mtx),
            song: false,
            local: true,
        })
        .unwrap();
        tx.send(Command::Tempo(120.0)).unwrap();
        // One bar, two beats of count-in; the metronome is off, so the
        // looper clicks the count-in and the first layer itself.
        tx.send(Command::Loop(LoopCommand::Rec {
            count_in: 2,
            bars: 1,
        }))
        .unwrap();
        let block = |r: &mut Renderer| r.render_block(128, Instant::now());
        block(&mut r);
        // 50 ms lead + 2 beats = 1.05 s = 50 400 samples; play on beat 2
        // of the loop (+0.5 s).
        while r.sample_clock < 50_400 + 24_000 {
            block(&mut r);
        }
        tx.send(Command::NoteOn {
            note: 64,
            velocity: 100,
        })
        .unwrap();
        block(&mut r);
        tx.send(Command::NoteOff { note: 64 }).unwrap();
        // Through the first layer (ends at 50 400 + 96 000) and one pass.
        while r.sample_clock < 50_400 + 2 * 96_000 + 1_000 {
            block(&mut r);
        }
        let st = *r.stats.looper.lock().unwrap();
        assert_eq!(st.state, LoopState::Playing);
        assert_eq!(st.len_beats, 4.0);
        assert!(!r.metro.on, "the looper's clicks stop once the loop plays");
        // 2 count-in + 4 first-layer clicks.
        assert_eq!(clicks.try_iter().count(), 6);
        let msgs: Vec<[u8; 3]> = mrx.try_iter().map(|m| m.data).collect();
        // Played live on channel 0, then replayed by the loop on 15 with
        // the same instrument set up first.
        assert!(msgs.contains(&[0x90, 64, 100]), "{msgs:?}");
        assert!(msgs.contains(&[0xCF, 0, 0]), "{msgs:?}");
        assert!(msgs.contains(&[0x9F, 64, 100]), "{msgs:?}");
        assert_eq!(r.programs[15], 0);

        // MIDI only: the synth stays silent.
        tx.send(Command::AllOff).unwrap();
        tx.send(Command::Loop(LoopCommand::Clear)).unwrap();
        tx.send(Command::MidiOut {
            tx: Some(crossbeam_channel::bounded(1024).0),
            song: false,
            local: false,
        })
        .unwrap();
        for _ in 0..40 {
            block(&mut r);
        }
        tx.send(Command::NoteOn {
            note: 60,
            velocity: 100,
        })
        .unwrap();
        block(&mut r);
        assert_eq!(peak(&r, 128), 0.0);
    }

    #[test]
    fn metronome_reports_clicks() {
        let (mut r, tx, clicks) = renderer();
        tx.send(Command::Metronome {
            bpm: 120.0,
            on: true,
        })
        .unwrap();
        // 2 seconds of audio at 120 bpm -> about 3-4 clicks after the 250 ms lead-in.
        for _ in 0..(96_000 / 128) {
            r.render_block(128, Instant::now());
        }
        let n = clicks.try_iter().count();
        assert!((3..=4).contains(&n), "got {n} clicks");
    }

    #[test]
    fn soundfont_renders_if_installed() {
        let Some(path) = crate::config::find_soundfont(None) else {
            return;
        };
        let mut f = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        let sf = Arc::new(SoundFont::new(&mut f).unwrap());
        let synth = Synthesizer::new(&sf, &SynthesizerSettings::new(48_000)).unwrap();
        let (mut r, tx, _) = renderer();
        tx.send(Command::UseSoundFont(Box::new(synth))).unwrap();
        tx.send(Command::NoteOn {
            note: 60,
            velocity: 100,
        })
        .unwrap();
        let mut loud = 0.0f32;
        for _ in 0..20 {
            r.render_block(256, Instant::now());
            loud = loud.max(peak(&r, 256));
        }
        assert!(
            loud > 0.01,
            "SoundFont piano should be audible, peak {loud}"
        );
    }
}
