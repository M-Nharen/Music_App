//! Live listen mode: hear any audio input (microphone, or the "monitor" of
//! what the computer is playing) and show the chord, the notes and the
//! key in real time.
//!
//! [`Analyzer`] is pure DSP (tested with synthesised sound); [`input`] opens
//! the audio input and runs the analyzer on its own thread.

pub mod input;

pub use crate::song::chords::Vocab;
use crate::song::chords::{best_chord, estimate_key};
use crate::theory::{ChordKind, chord_symbol};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub const FFT_SIZE: usize = 8192;
pub const HOP: usize = 2048;
const YIN_WINDOW: usize = 1024;

/// What was heard in one analysis step.
#[derive(Clone, Debug, Default)]
pub struct Heard {
    /// Seconds since listening started.
    pub time: f64,
    /// Input level, dBFS (−100 = silence).
    pub level_db: f32,
    /// Smoothed pitch-class weights, max = 1.
    pub chroma: [f32; 12],
    /// The chord, after smoothing: (root pc, kind, confidence 0..1).
    pub chord: Option<(i32, ChordKind, f32)>,
    /// Notes that seem to be sounding, strongest first (MIDI, strength 0..1).
    pub notes: Vec<(i32, f32)>,
    /// One clear pitch (singing, a solo line): (MIDI as a float, clarity 0..1).
    pub pitch: Option<(f32, f32)>,
    /// Key so far: (tonic pc, minor?, confidence).
    pub key: Option<(i32, bool, f32)>,
}

impl Heard {
    pub fn chord_name(&self) -> Option<String> {
        self.chord.map(|(r, k, _)| chord_symbol(r, k))
    }
}

pub struct Analyzer {
    sr: f32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// The last FFT_SIZE samples.
    ring: Vec<f32>,
    filled: usize,
    since_hop: usize,
    total: u64,
    scratch: Vec<Complex<f32>>,
    mags: Vec<f32>,
    smooth: [f32; 12],
    key_acc: [f32; 12],
    /// Chord shown, and a challenger that must win twice in a row.
    chord: Option<(i32, ChordKind, f32)>,
    challenger: Option<((i32, ChordKind), u32)>,
    /// Below this level (dBFS) it's treated as silence.
    pub gate_db: f32,
    pub vocab: Vocab,
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Analyzer {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        Analyzer {
            sr: sample_rate.max(1) as f32,
            fft,
            window,
            ring: vec![0.0; FFT_SIZE],
            filled: 0,
            since_hop: 0,
            total: 0,
            scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            mags: vec![0.0; FFT_SIZE / 2],
            smooth: [0.0; 12],
            key_acc: [0.0; 12],
            chord: None,
            challenger: None,
            gate_db: -50.0,
            vocab: Vocab::Extended,
        }
    }

    /// Feed mono samples; returns a result for every hop completed.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Heard> {
        let mut out = Vec::new();
        for &s in samples {
            let i = (self.total % FFT_SIZE as u64) as usize;
            self.ring[i] = s;
            self.total += 1;
            self.filled = (self.filled + 1).min(FFT_SIZE);
            self.since_hop += 1;
            if self.since_hop >= HOP && self.filled >= FFT_SIZE / 2 {
                self.since_hop = 0;
                out.push(self.analyse());
            }
        }
        out
    }

    /// The last `n` samples in order.
    fn recent(&self, n: usize) -> Vec<f32> {
        let n = n.min(FFT_SIZE);
        let end = (self.total % FFT_SIZE as u64) as usize;
        (0..n)
            .map(|k| self.ring[(end + FFT_SIZE - n + k) % FFT_SIZE])
            .collect()
    }

    fn analyse(&mut self) -> Heard {
        let frame = self.recent(FFT_SIZE);
        let time = self.total as f64 / self.sr as f64;
        let rms = (frame[FFT_SIZE - HOP..].iter().map(|x| x * x).sum::<f32>() / HOP as f32).sqrt();
        let level_db = if rms > 1e-9 {
            20.0 * rms.log10()
        } else {
            -100.0
        };
        for (i, c) in self.scratch.iter_mut().enumerate() {
            *c = Complex::new(frame[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.scratch);
        for (m, c) in self.mags.iter_mut().zip(&self.scratch) {
            *m = c.norm();
        }

        let loud = level_db > self.gate_db;
        let raw = if loud { self.chroma() } else { [0.0; 12] };
        // Smooth over ~0.15 s; decay fast when it goes quiet.
        let a = if loud { 0.45 } else { 0.3 };
        for (s, r) in self.smooth.iter_mut().zip(raw) {
            *s = *s * (1.0 - a) + r * a;
        }
        if loud {
            // Key: a long memory (half-life ~20 s).
            let decay = 0.5f32.powf(HOP as f32 / self.sr / 20.0);
            for (k, r) in self.key_acc.iter_mut().zip(raw) {
                *k = *k * decay + r;
            }
        }
        let max = self.smooth.iter().cloned().fold(0.0, f32::max);
        let chroma = if max > 1e-9 {
            self.smooth.map(|x| x / max)
        } else {
            [0.0; 12]
        };

        // Chord with a little stickiness so it doesn't flicker.
        let total: f32 = self.smooth.iter().sum();
        let cand = if total > 1e-6 && loud {
            best_chord(&self.smooth, self.vocab).filter(|c| c.2 > 0.25)
        } else {
            None
        };
        match cand {
            None if !loud => {
                self.chord = None;
                self.challenger = None;
            }
            None => {}
            Some((r, k, s)) => {
                if self.chord.is_some_and(|c| (c.0, c.1) == (r, k)) {
                    self.chord = Some((r, k, s));
                    self.challenger = None;
                } else {
                    let n = match self.challenger {
                        Some((c, n)) if c == (r, k) => n + 1,
                        _ => 1,
                    };
                    self.challenger = Some(((r, k), n));
                    if n >= 2 || self.chord.is_none() && s > 0.5 {
                        self.chord = Some((r, k, s));
                        self.challenger = None;
                    }
                }
            }
        }

        let notes = if loud { self.notes() } else { vec![] };
        let pitch = if loud { self.yin() } else { None };
        let key = estimate_key(&self.key_acc).filter(|k| k.2 > 0.2);
        Heard {
            time,
            level_db,
            chroma,
            chord: self.chord,
            notes,
            pitch,
            key,
        }
    }

    fn bin_hz(&self) -> f32 {
        self.sr / FFT_SIZE as f32
    }

    /// Peaks of the spectrum, folded into 12 pitch classes. Peaks that are
    /// just overtones of a lower note count much less, or every C chord
    /// would look like Cmaj7 (E's third overtone is a B).
    fn chroma(&self) -> [f32; 12] {
        let mut c = [0.0f32; 12];
        let hz = self.bin_hz();
        let lo = (55.0 / hz) as usize;
        let hi = ((2000.0 / hz) as usize).min(self.mags.len() - 2);
        let floor = self.noise_floor(lo, hi);
        let mut peaks: Vec<(f32, f32)> = Vec::new();
        for b in lo.max(1)..hi {
            let m = self.mags[b];
            if m <= self.mags[b - 1] || m < self.mags[b + 1] || m < floor * 3.0 {
                continue;
            }
            // Parabolic interpolation for the true peak frequency.
            let (l, r) = (self.mags[b - 1], self.mags[b + 1]);
            let d = 0.5 * (l - r) / (l - 2.0 * m + r).min(-1e-9);
            peaks.push(((b as f32 + d) * hz, m));
        }
        for &(f, m) in &peaks {
            let overtone = peaks.iter().any(|&(f0, m0)| {
                f0 < f * 0.9
                    && m0 >= m * 1.5
                    && (2..=6).any(|h| ((f / (f0 * h as f32)) - 1.0).abs() < 0.015)
            });
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            let pc = (midi.round() as i32).rem_euclid(12) as usize;
            // Notes far from a semitone count less.
            let off = (midi - midi.round()).abs();
            c[pc] += m.powf(0.75) * (1.0 - off) * if overtone { 0.25 } else { 1.0 };
        }
        c
    }

    fn noise_floor(&self, lo: usize, hi: usize) -> f32 {
        let mut v: Vec<f32> = self.mags[lo.max(1)..hi.max(lo + 2)].to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2].max(1e-6)
    }

    /// Magnitude near a frequency (max of the nearest bins).
    fn mag_at(&self, f: f32) -> f32 {
        let b = (f / self.bin_hz()).round() as usize;
        if b + 1 >= self.mags.len() || b == 0 {
            return 0.0;
        }
        self.mags[b - 1].max(self.mags[b]).max(self.mags[b + 1])
    }

    /// A real spectral peak within half a semitone of `f`, else 0.
    fn peak_near(&self, f: f32) -> f32 {
        let hz = self.bin_hz();
        let b = (f / hz).round() as usize;
        if b < 2 || b + 2 >= self.mags.len() {
            return 0.0;
        }
        let Some(p) = (b - 1..=b + 1)
            .filter(|&i| self.mags[i] >= self.mags[i - 1] && self.mags[i] >= self.mags[i + 1])
            .max_by(|&x, &y| self.mags[x].partial_cmp(&self.mags[y]).unwrap())
        else {
            return 0.0;
        };
        let (l, m, r) = (self.mags[p - 1], self.mags[p], self.mags[p + 1]);
        let d = 0.5 * (l - r) / (l - 2.0 * m + r).min(-1e-9);
        let pf = (p as f32 + d) * hz;
        if (12.0 * (pf / f).log2()).abs() < 0.5 {
            m
        } else {
            0.0
        }
    }

    /// Notes that seem to be sounding: harmonic sums, then drop candidates
    /// that are just overtones of a stronger note.
    fn notes(&self) -> Vec<(i32, f32)> {
        let mut sal: Vec<(i32, f32)> = (36..=96)
            .map(|m| {
                let f0 = 440.0 * 2f32.powf((m as f32 - 69.0) / 12.0);
                let s: f32 = (1..=5)
                    .map(|h| self.mag_at(f0 * h as f32) * 0.75f32.powi(h - 1))
                    .sum();
                // A real note has its fundamental.
                (m, if self.mag_at(f0) > 0.0 { s } else { 0.0 })
            })
            .collect();
        let max = sal.iter().map(|x| x.1).fold(0.0, f32::max);
        if max <= 1e-6 {
            return vec![];
        }
        let lo = (55.0 / self.bin_hz()) as usize;
        let hi = self.mags.len() / 4;
        let floor = self.noise_floor(lo, hi);
        let peak = self.mags[lo..hi].iter().cloned().fold(0.0, f32::max);
        sal.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let mut out: Vec<(i32, f32)> = Vec::new();
        for (m, s) in sal {
            if s < max * 0.3 || out.len() >= 6 {
                break;
            }
            let f0 = 440.0 * 2f32.powf((m as f32 - 69.0) / 12.0);
            let fund = self.peak_near(f0);
            // The fundamental has to really be there.
            if fund < floor * 4.0 || fund < peak * 0.08 {
                continue;
            }
            // Skip neighbours and overtones of notes already found. An
            // overtone of a found note only counts if it's louder than an
            // overtone would be.
            let dup = out.iter().any(|(o, _)| {
                let d = m - o;
                let of = 440.0 * 2f32.powf((*o as f32 - 69.0) / 12.0);
                let ratio = if d == 12 { 0.8 } else { 0.6 };
                d.abs() <= 1 || [12, 19, 24, 28, 31].contains(&d) && fund < self.mag_at(of) * ratio
            });
            // An octave below a found note is often a phantom.
            let sub = out.iter().any(|(o, os)| o - m == 12 && s < os * 1.2);
            if !dup && !sub {
                out.push((m, (s / max).clamp(0.0, 1.0)));
            }
        }
        out
    }

    /// YIN pitch detection on the latest samples.
    fn yin(&self) -> Option<(f32, f32)> {
        let x = self.recent(YIN_WINDOW * 2);
        let min_tau = (self.sr / 1200.0) as usize;
        let max_tau = ((self.sr / 70.0) as usize).min(YIN_WINDOW - 1);
        let mut d = vec![0.0f32; max_tau + 1];
        for (tau, dt) in d.iter_mut().enumerate().skip(1) {
            let mut s = 0.0;
            for j in 0..YIN_WINDOW {
                let diff = x[j] - x[j + tau];
                s += diff * diff;
            }
            *dt = s;
        }
        // Cumulative mean normalised difference.
        let mut cmnd = vec![1.0f32; max_tau + 1];
        let mut running = 0.0;
        for tau in 1..=max_tau {
            running += d[tau];
            cmnd[tau] = if running > 0.0 {
                d[tau] * tau as f32 / running
            } else {
                1.0
            };
        }
        let threshold = 0.15;
        let mut tau = None;
        let mut t = min_tau.max(2);
        while t < max_tau {
            if cmnd[t] < threshold {
                while t + 1 < max_tau && cmnd[t + 1] < cmnd[t] {
                    t += 1;
                }
                tau = Some(t);
                break;
            }
            t += 1;
        }
        let t = tau?;
        let (a, b, c) = (cmnd[t - 1], cmnd[t], cmnd[t + 1]);
        let denom = a - 2.0 * b + c;
        let shift = if denom.abs() > 1e-9 {
            0.5 * (a - c) / denom
        } else {
            0.0
        };
        let f = self.sr / (t as f32 + shift);
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        Some((midi, (1.0 - b).clamp(0.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone with a few overtones, like an instrument.
    fn tone(midi: f32, secs: f32, sr: u32, amp: f32) -> Vec<f32> {
        let f = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
        (0..(secs * sr as f32) as usize)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (1..=4)
                    .map(|h| {
                        (2.0 * std::f32::consts::PI * f * h as f32 * t).sin() * 0.6f32.powi(h - 1)
                    })
                    .sum::<f32>()
                    * amp
            })
            .collect()
    }

    fn mix(parts: &[Vec<f32>]) -> Vec<f32> {
        let n = parts.iter().map(|p| p.len()).min().unwrap();
        (0..n).map(|i| parts.iter().map(|p| p[i]).sum()).collect()
    }

    fn last(a: &mut Analyzer, s: &[f32]) -> Heard {
        a.push(s).pop().unwrap()
    }

    #[test]
    fn hears_a_c_major_chord() {
        let sr = 48_000;
        let mut a = Analyzer::new(sr);
        let c = mix(&[
            tone(48.0, 1.0, sr, 0.1),
            tone(64.0, 1.0, sr, 0.1),
            tone(67.0, 1.0, sr, 0.1),
            tone(60.0, 1.0, sr, 0.1),
        ]);
        let h = last(&mut a, &c);
        assert_eq!(h.chord.map(|c| (c.0, c.1)), Some((0, ChordKind::Major)));
        assert_eq!(h.chord_name().unwrap(), "C");
        assert!(h.level_db > -30.0);
        let notes: Vec<i32> = h.notes.iter().map(|n| n.0).collect();
        for n in [48, 64, 67] {
            assert!(notes.contains(&n), "{n} in {notes:?}");
        }
    }

    #[test]
    fn one_note_is_one_note() {
        for m in [45.0f32, 55.0, 62.0, 72.0] {
            let mut a = Analyzer::new(48_000);
            let h = last(&mut a, &tone(m, 0.6, 48_000, 0.2));
            let notes: Vec<i32> = h.notes.iter().map(|n| n.0).collect();
            assert_eq!(notes, vec![m as i32], "overtones aren't notes");
        }
    }

    #[test]
    fn chord_changes_after_it_settles() {
        let sr = 48_000;
        let mut a = Analyzer::new(sr);
        let am = mix(&[
            tone(57.0, 1.0, sr, 0.1),
            tone(60.0, 1.0, sr, 0.1),
            tone(64.0, 1.0, sr, 0.1),
        ]);
        let g = mix(&[
            tone(55.0, 1.0, sr, 0.1),
            tone(59.0, 1.0, sr, 0.1),
            tone(62.0, 1.0, sr, 0.1),
        ]);
        assert_eq!(last(&mut a, &am).chord_name().as_deref(), Some("Am"));
        assert_eq!(last(&mut a, &g).chord_name().as_deref(), Some("G"));
        // Silence clears it.
        let h = last(&mut a, &vec![0.0; sr as usize]);
        assert_eq!(h.chord, None);
        assert!(h.level_db < -90.0);
    }

    #[test]
    fn a_melody_note_doesnt_make_a_seventh_chord() {
        // C major chord with a melody passing over B (the major 7th).
        let sr = 48_000;
        let mut a = Analyzer::new(sr);
        let c = mix(&[
            tone(48.0, 1.0, sr, 0.1),
            tone(60.0, 1.0, sr, 0.08),
            tone(64.0, 1.0, sr, 0.08),
            tone(67.0, 1.0, sr, 0.08),
            tone(71.0, 1.0, sr, 0.03),
        ]);
        assert_eq!(last(&mut a, &c).chord_name().as_deref(), Some("C"));
    }

    #[test]
    fn follows_a_sung_pitch() {
        let sr = 44_100;
        let mut a = Analyzer::new(sr);
        for m in [57.0f32, 64.0, 69.3, 76.0] {
            let h = last(&mut a, &tone(m, 0.5, sr, 0.2));
            let (p, clarity) = h.pitch.expect("a pitch");
            assert!((p - m).abs() < 0.15, "{p} vs {m}");
            assert!(clarity > 0.8);
        }
        // Noise has no clear pitch.
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..sr / 2)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 16) as f32 / 32768.0 - 1.0
            })
            .map(|x| x * 0.2)
            .collect();
        assert!(last(&mut a, &noise).pitch.is_none());
    }

    #[test]
    fn finds_the_key_of_a_progression() {
        let sr = 48_000;
        let mut a = Analyzer::new(sr);
        // C F G C in C major, a few times.
        let chord = |ns: &[f32]| {
            mix(&ns
                .iter()
                .map(|n| tone(*n, 0.5, sr, 0.08))
                .collect::<Vec<_>>())
        };
        let mut h = Heard::default();
        for _ in 0..3 {
            for c in [
                [60.0, 64.0, 67.0],
                [65.0, 69.0, 72.0],
                [67.0, 71.0, 74.0],
                [60.0, 64.0, 67.0],
            ] {
                h = last(&mut a, &chord(&c));
            }
        }
        let (t, minor, _) = h.key.unwrap();
        assert_eq!((t, minor), (0, false));
    }

    #[test]
    fn quiet_input_is_ignored() {
        let mut a = Analyzer::new(48_000);
        let h = last(&mut a, &tone(60.0, 0.5, 48_000, 0.0005));
        assert!(h.chord.is_none() && h.notes.is_empty() && h.pitch.is_none());
    }
}
