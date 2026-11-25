//! The real-time audio engine.
//!
//! The UI thread sends [`Command`]s over a lock-free channel; the audio
//! callback drains them at the start of every buffer, so a key press is
//! heard on the very next buffer (~1.3 ms at 64 frames / 48 kHz).

pub mod builtin;

use anyhow::{Context, Result, anyhow};
use builtin::BuiltinSynth;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SizedSample};
use crossbeam_channel::{Receiver, Sender, unbounded};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
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
    pub host: Mutex<String>,
    pub device: Mutex<String>,
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
}

/// State that lives inside the audio callback.
struct Renderer {
    backend: Backend,
    builtin_fallback: BuiltinSynth,
    rx: Receiver<Command>,
    click_tx: Sender<Instant>,
    stats: Arc<Stats>,
    sample_rate: u32,
    program: u8,
    volume: f32,
    sustain: bool,
    held: [bool; 128],
    sustained: [bool; 128],
    left: Vec<f32>,
    right: Vec<f32>,
    sample_clock: u64,
    metro: Metronome,
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
        let mut builtin = BuiltinSynth::new(sample_rate);
        builtin.set_program(program);
        Renderer {
            backend: Backend::Builtin(builtin),
            builtin_fallback: BuiltinSynth::new(sample_rate),
            rx,
            click_tx,
            stats,
            sample_rate,
            program,
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
            },
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
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
                match &mut self.backend {
                    Backend::Builtin(s) => s.note_on(note, velocity),
                    Backend::SoundFont(s) => s.note_on(0, note, velocity),
                }
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
                self.program = p;
                self.apply_program();
            }
            Command::Volume(v) => self.volume = v.clamp(0.0, 2.0),
            Command::AllOff => {
                self.held = [false; 128];
                self.sustained = [false; 128];
                match &mut self.backend {
                    Backend::Builtin(s) => s.all_off(),
                    Backend::SoundFont(s) => s.note_off_all(true),
                }
            }
            Command::UseSoundFont(synth) => {
                self.backend = Backend::SoundFont(synth);
                self.stats.soundfont_active.store(true, Ordering::Relaxed);
                self.apply_program();
            }
            Command::UseBuiltin => {
                let mut b = BuiltinSynth::new(self.sample_rate);
                std::mem::swap(&mut b, &mut self.builtin_fallback);
                self.backend = Backend::Builtin(b);
                self.stats.soundfont_active.store(false, Ordering::Relaxed);
                self.apply_program();
            }
            Command::Metronome { bpm, on } => {
                self.metro.on = on;
                self.metro.interval =
                    (self.sample_rate as f64 * 60.0 / bpm.max(20.0) as f64) as u64;
                self.metro.next_at = self.sample_clock + self.sample_rate as u64 / 4;
                self.metro.beat = 0;
            }
        }
    }

    fn backend_off(&mut self, note: i32) {
        match &mut self.backend {
            Backend::Builtin(s) => s.note_off(note),
            Backend::SoundFont(s) => s.note_off(0, note),
        }
    }

    fn apply_program(&mut self) {
        match &mut self.backend {
            Backend::Builtin(s) => s.set_program(self.program),
            // 0xC0 = program change.
            Backend::SoundFont(s) => s.process_midi_message(0, 0xC0, self.program as i32, 0),
        }
    }

    /// Render `frames` of stereo audio into `self.left` / `self.right`.
    /// Separate from the cpal callback so it can be tested offline.
    fn render_block(&mut self, frames: usize, started: Instant) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.handle(cmd);
        }
        if self.left.len() < frames {
            // Only happens if the host changes buffer size; allocation is rare.
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
        }
        let (l, r) = (&mut self.left[..frames], &mut self.right[..frames]);
        match &mut self.backend {
            Backend::Builtin(s) => {
                s.render(l, r);
                self.stats
                    .active_voices
                    .store(s.active_voices() as u32, Ordering::Relaxed);
            }
            Backend::SoundFont(s) => s.render(l, r),
        }

        // Metronome clicks (latency test).
        if self.metro.on {
            for i in 0..frames {
                let t = self.sample_clock + i as u64;
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

/// Handle owned by the UI. Dropping it stops audio.
pub struct Engine {
    tx: Sender<Command>,
    pub stats: Arc<Stats>,
    pub clicks: Receiver<Instant>,
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
