//! Opening an audio input and running the [`Analyzer`] on its own thread.

use super::{Analyzer, Heard};
use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// An input the user can pick.
#[derive(Clone, Debug, PartialEq)]
pub struct InputInfo {
    pub id: String,
    pub name: String,
    /// A "monitor" input: what the computer is playing.
    pub monitor: bool,
    /// Probably a Bluetooth headset mic (switches the earbuds to call mode).
    pub bluetooth: bool,
}

/// Every input device cpal can see.
pub fn list_inputs() -> Vec<InputInfo> {
    let host = cpal::default_host();
    let Ok(devs) = host.input_devices() else {
        return vec![];
    };
    devs.filter_map(|d| {
        let name = d
            .description()
            .map(|x| x.name().to_string())
            .unwrap_or_default();
        let id = d
            .id()
            .map(|i| i.to_string())
            .unwrap_or_else(|_| name.clone());
        if name.is_empty() && id.is_empty() {
            return None;
        }
        let lower = format!("{name} {id}").to_lowercase();
        Some(InputInfo {
            monitor: lower.contains("monitor"),
            bluetooth: lower.contains("bluez")
                || lower.contains("bluetooth")
                || lower.contains("headset"),
            name: if name.is_empty() { id.clone() } else { name },
            id,
        })
    })
    .collect()
}

/// A running input: the stream (dropping it stops listening) and the
/// latest analysis.
pub struct Listener {
    _stream: cpal::Stream,
    pub name: String,
    pub sample_rate: u32,
    pub latest: Arc<Mutex<Heard>>,
    /// Chord changes heard: (time, chord name).
    pub history: Arc<Mutex<Vec<(f64, String)>>>,
    /// Recent pitches (time, midi) for the melody trail.
    pub trail: Arc<Mutex<Vec<(f64, f32)>>>,
    stop: Arc<AtomicBool>,
    pub settings: Arc<Mutex<(f32, super::Vocab)>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn find_device(id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(id) = id {
        if let Ok(devs) = host.input_devices() {
            for d in devs {
                let did = d.id().map(|i| i.to_string()).unwrap_or_default();
                if did == id {
                    return Ok(d);
                }
            }
        }
        return Err(anyhow!("input {id} isn't there any more"));
    }
    host.default_input_device()
        .ok_or_else(|| anyhow!("no audio input found (no microphone?)"))
}

/// Start listening. `on_new` is called (from the analysis thread) after
/// every result, e.g. to repaint the window.
pub fn start(id: Option<&str>, on_new: impl Fn() + Send + 'static) -> Result<Listener> {
    let device = find_device(id)?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "input".into());
    let cfg = device
        .default_input_config()
        .context("reading the input's settings")?;
    let sr = cfg.sample_rate();
    let channels = cfg.channels() as usize;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.config();
    let (tx, rx) = bounded::<Vec<f32>>(64);
    let stream = match fmt {
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, channels, tx)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, channels, tx)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, channels, tx)?,
        _ => build::<f32>(&device, &config, channels, tx)?,
    };
    stream.play().context("starting the input")?;

    let latest = Arc::new(Mutex::new(Heard::default()));
    let history = Arc::new(Mutex::new(Vec::new()));
    let trail = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let settings = Arc::new(Mutex::new((-50.0f32, super::Vocab::Extended)));
    {
        let (latest, history, trail, stop, settings) = (
            latest.clone(),
            history.clone(),
            trail.clone(),
            stop.clone(),
            settings.clone(),
        );
        std::thread::Builder::new()
            .name("qwertone-listen".into())
            .spawn(move || run(rx, sr, latest, history, trail, stop, settings, on_new))
            .context("starting the analysis thread")?;
    }
    Ok(Listener {
        _stream: stream,
        name,
        sample_rate: sr,
        latest,
        history,
        trail,
        stop,
        settings,
    })
}

#[allow(clippy::too_many_arguments)]
fn run(
    rx: Receiver<Vec<f32>>,
    sr: u32,
    latest: Arc<Mutex<Heard>>,
    history: Arc<Mutex<Vec<(f64, String)>>>,
    trail: Arc<Mutex<Vec<(f64, f32)>>>,
    stop: Arc<AtomicBool>,
    settings: Arc<Mutex<(f32, super::Vocab)>>,
    on_new: impl Fn(),
) {
    let mut a = Analyzer::new(sr);
    let mut last_chord: Option<String> = None;
    while !stop.load(Ordering::Relaxed) {
        let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_millis(200)) else {
            continue;
        };
        {
            let s = settings.lock().unwrap();
            a.gate_db = s.0;
            a.vocab = s.1;
        }
        for h in a.push(&chunk) {
            let name = h.chord_name();
            if name.is_some() && name != last_chord {
                let mut hist = history.lock().unwrap();
                hist.push((h.time, name.clone().unwrap()));
                if hist.len() > 400 {
                    hist.remove(0);
                }
            }
            if name.is_some() {
                last_chord = name;
            }
            if let Some((p, clarity)) = h.pitch
                && clarity > 0.75
            {
                let mut t = trail.lock().unwrap();
                t.push((h.time, p));
                let cutoff = h.time - 12.0;
                t.retain(|(at, _)| *at >= cutoff);
            }
            *latest.lock().unwrap() = h;
            on_new();
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    tx: Sender<Vec<f32>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let ch = channels.max(1);
    let stream = device.build_input_stream::<T, _, _>(
        *config,
        move |data: &[T], _| {
            // Mono, as f32. Dropped if the analysis falls behind.
            let mono: Vec<f32> = data
                .chunks(ch)
                .map(|f| {
                    f.iter()
                        .map(|s| <f32 as FromSample<T>>::from_sample_(*s))
                        .sum::<f32>()
                        / ch as f32
                })
                .collect();
            let _ = tx.try_send(mono);
        },
        |e| eprintln!("qwertone listen: {e}"),
        None,
    )?;
    Ok(stream)
}
