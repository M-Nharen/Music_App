//! Part 4a: recording what you play and saving it as a MIDI file, plus tap
//! tempo. Pure logic (no audio, no egui) so it can be tested.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

/// Something the audio thread heard you play. Times are in samples since the
/// recording started (after the count-in).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecEvent {
    /// `vel == 0` = note off.
    Note {
        ch: u8,
        note: u8,
        vel: u8,
    },
    Program {
        ch: u8,
        program: u8,
    },
    /// Everything stopped (Escape / panic): close all open notes.
    AllOff,
}

/// One recording.
#[derive(Clone, Debug, Default)]
pub struct Take {
    pub events: Vec<(u64, RecEvent)>,
}

const TPB: u16 = 480;

impl Take {
    pub fn note_count(&self) -> usize {
        self.events
            .iter()
            .filter(|(_, e)| matches!(e, RecEvent::Note { vel: 1.., .. }))
            .count()
    }

    /// A type-0 MIDI file at `bpm` in 4/4, so the bars line up with the
    /// metronome. Notes still held at the end are closed there.
    pub fn to_midi(&self, sample_rate: u32, bpm: f32) -> Vec<u8> {
        let bpm = bpm.clamp(20.0, 400.0) as f64;
        let ticks_per_sample = bpm / 60.0 * TPB as f64 / sample_rate.max(1) as f64;
        let tick = |t: u64| (t as f64 * ticks_per_sample).round() as u64;
        let mut evs: Vec<(u64, TrackEventKind<'static>)> = vec![
            (
                0,
                TrackEventKind::Meta(MetaMessage::TrackName(b"Qwertone take")),
            ),
            (
                0,
                TrackEventKind::Meta(MetaMessage::Tempo(u24::new((60e6 / bpm).round() as u32))),
            ),
            (
                0,
                TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
            ),
        ];
        let msg = |ch: u8, m: MidiMessage| TrackEventKind::Midi {
            channel: u4::new(ch & 15),
            message: m,
        };
        let off = |ch: u8, n: u8| {
            msg(
                ch,
                MidiMessage::NoteOff {
                    key: u7::new(n),
                    vel: u7::new(0),
                },
            )
        };
        // Open notes per (channel, key), so stray offs are dropped and
        // re-strikes close the old note first.
        let mut open = [[false; 128]; 16];
        let mut end = 0;
        for &(t, e) in &self.events {
            let tk = tick(t);
            end = end.max(tk);
            match e {
                RecEvent::Note { ch, note, vel } => {
                    let (c, n) = ((ch & 15) as usize, (note & 127) as usize);
                    if open[c][n] {
                        evs.push((tk, off(ch, note)));
                        open[c][n] = false;
                    }
                    if vel > 0 {
                        evs.push((
                            tk,
                            msg(
                                ch,
                                MidiMessage::NoteOn {
                                    key: u7::new(note & 127),
                                    vel: u7::new(vel.min(127)),
                                },
                            ),
                        ));
                        open[c][n] = true;
                    }
                }
                RecEvent::Program { ch, program } => evs.push((
                    tk,
                    msg(
                        ch,
                        MidiMessage::ProgramChange {
                            program: u7::new(program & 127),
                        },
                    ),
                )),
                RecEvent::AllOff => close_all(&mut open, tk, &mut evs, off),
            }
        }
        // Drum hits have no off: give them a short one so every player is happy.
        let drum_len = TPB as u64 / 8;
        close_all(&mut open, end + drum_len, &mut evs, off);
        // Stable sort keeps same-tick order (off before the re-strike).
        evs.sort_by_key(|(t, _)| *t);
        let mut track: Vec<TrackEvent> = Vec::with_capacity(evs.len() + 1);
        let mut last = 0;
        for (t, kind) in evs {
            track.push(TrackEvent {
                delta: u28::new((t - last) as u32),
                kind,
            });
            last = t;
        }
        track.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(TPB)),
        ));
        smf.tracks.push(track);
        let mut out = Vec::new();
        smf.write_std(&mut out).expect("writing to memory");
        out
    }
}

fn close_all(
    open: &mut [[bool; 128]; 16],
    tk: u64,
    evs: &mut Vec<(u64, TrackEventKind<'static>)>,
    off: impl Fn(u8, u8) -> TrackEventKind<'static>,
) {
    for (c, keys) in open.iter_mut().enumerate() {
        for (n, o) in keys.iter_mut().enumerate() {
            if *o {
                evs.push((tk, off(c as u8, n as u8)));
                *o = false;
            }
        }
    }
}

/// `~/Music/qwertone/`.
pub fn takes_dir() -> PathBuf {
    dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qwertone")
}

/// Write `bytes` as the next free `take-NNN.mid` in `dir`.
pub fn save(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for i in 1..10_000 {
        let p = dir.join(format!("take-{i:03}.mid"));
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
        {
            use std::io::Write;
            f.write_all(bytes)?;
            return Ok(p);
        }
    }
    Err(std::io::Error::other("too many takes in the folder"))
}

/// Tap tempo: remembers recent taps and gives the tempo once there are two.
/// A pause of more than 2 seconds starts over.
#[derive(Default)]
pub struct TapTempo {
    taps: Vec<Instant>,
}

impl TapTempo {
    pub fn tap(&mut self, now: Instant) -> Option<f32> {
        if self
            .taps
            .last()
            .is_some_and(|&l| now.duration_since(l) > Duration::from_secs(2))
        {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        let n = self.taps.len();
        if n < 2 {
            return None;
        }
        let span = self.taps[n - 1].duration_since(self.taps[0]).as_secs_f32();
        let beat = span / (n - 1) as f32;
        (beat > 0.0).then(|| (60.0 / beat).clamp(40.0, 240.0).round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(ch: u8, note: u8) -> RecEvent {
        RecEvent::Note { ch, note, vel: 100 }
    }
    fn off(ch: u8, note: u8) -> RecEvent {
        RecEvent::Note { ch, note, vel: 0 }
    }

    #[test]
    fn take_round_trips_through_the_song_reader() {
        // 120 bpm at 48 kHz: one beat = 24 000 samples.
        let take = Take {
            events: vec![
                (0, RecEvent::Program { ch: 0, program: 0 }),
                (0, on(0, 60)),
                (24_000, off(0, 60)),
                (24_000, on(1, 64)),
                (24_000, on(1, 67)),
                (48_000, RecEvent::AllOff),
                (48_000, on(9, 36)),
                (60_000, off(2, 40)), // stray off: dropped
            ],
        };
        assert_eq!(take.note_count(), 4);
        let bytes = take.to_midi(48_000, 120.0);
        let smf = Smf::parse(&bytes).unwrap();
        let mut ons = vec![];
        let (mut tick, mut offs) = (0u64, 0);
        for e in &smf.tracks[0] {
            tick += e.delta.as_int() as u64;
            if let TrackEventKind::Midi { channel, message } = e.kind {
                match message {
                    MidiMessage::NoteOn { key, .. } => {
                        ons.push((tick, channel.as_int(), key.as_int()))
                    }
                    MidiMessage::NoteOff { .. } => offs += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(
            ons,
            vec![(0, 0, 60), (480, 1, 64), (480, 1, 67), (960, 9, 36)]
        );
        assert_eq!(offs, 4, "every note closed, stray off dropped");
        // The app's own MIDI reader agrees on tempo.
        let dir = std::env::temp_dir().join(format!("qwertone-take-{}", std::process::id()));
        let p = save(&dir, &bytes).unwrap();
        assert!(p.ends_with("take-001.mid"));
        assert!(save(&dir, &bytes).unwrap().ends_with("take-002.mid"));
        let s = crate::song::Song::load(&p).unwrap();
        assert!((s.tempo.bpm - 120.0).abs() < 0.5);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tap_tempo() {
        let t0 = Instant::now();
        let mut t = TapTempo::default();
        assert_eq!(t.tap(t0), None);
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(t.tap(ms(500)), Some(120.0));
        assert_eq!(t.tap(ms(1000)), Some(120.0));
        assert_eq!(t.tap(ms(1650)), Some(109.0)); // averages: 1650/3 ms
        // Long pause starts over.
        assert_eq!(t.tap(ms(5000)), None);
        assert_eq!(t.tap(ms(5750)), Some(80.0));
    }
}
