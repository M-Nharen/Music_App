//! Time-stretching and pitch-shifting for Jam mode (Part 7), in pure Rust.
//!
//! Two pieces, both allocation-free after `new` so they can run in the
//! audio thread:
//!
//! - [`SincTable`]: windowed-sinc interpolation, used to read a stream at
//!   any fractional step (sample-rate conversion 44.1 → 48 kHz, and the
//!   resampling half of pitch-shifting). The cutoff follows the step, so
//!   reading faster than 1:1 doesn't alias.
//! - [`Pv`]: a phase vocoder with identity phase locking. It works on
//!   *random-access* audio (the stems are in memory): every output frame
//!   is analysed at the source position the song clock asks for, plus one
//!   hop earlier, and the phase advance between those two analyses is what
//!   the synthesis phase moves by. So any speed works (including 0, which
//!   freezes the sound), jumps are just a different position, and there is
//!   no input latency to compensate.
//!
//! Pitch shift by ratio `r` at speed `v` = stretch the source by `r / v`
//! with the vocoder, then read the result `r` times faster.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Taps on each side of the read point.
pub const HALF_TAPS: usize = 16;
const TAPS: usize = HALF_TAPS * 2;
const PHASES: usize = 256;

/// Windowed-sinc interpolation weights for one read step.
pub struct SincTable {
    /// `(PHASES + 1) × TAPS`; row `p` is for a fractional position `p / PHASES`.
    table: Vec<f32>,
    step: f64,
    weights: [f32; TAPS],
}

impl SincTable {
    pub fn new() -> SincTable {
        let mut t = SincTable {
            table: vec![0.0; (PHASES + 1) * TAPS],
            step: f64::NAN,
            weights: [0.0; TAPS],
        };
        t.set_step(1.0);
        t
    }

    pub fn step(&self) -> f64 {
        self.step
    }

    /// Weights for reading `step` input samples per output sample. Only
    /// recomputes when the step changes (no allocation).
    pub fn set_step(&mut self, step: f64) {
        if step == self.step {
            return;
        }
        self.step = step;
        // Cutoff as a fraction of the input's Nyquist frequency.
        let fc = (1.0 / step.max(1e-6)).min(1.0) * 0.95;
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let row = &mut self.table[p * TAPS..(p + 1) * TAPS];
            let mut sum = 0.0;
            for (j, w) in row.iter_mut().enumerate() {
                // Input sample floor(pos) - (HALF_TAPS - 1) + j.
                let t = j as f64 - (HALF_TAPS as f64 - 1.0) - frac;
                let u = t / HALF_TAPS as f64;
                let win = if u.abs() >= 1.0 {
                    0.0
                } else {
                    let a = std::f64::consts::PI * u;
                    0.42 + 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
                };
                let x = std::f64::consts::PI * fc * t;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
                let v = fc * sinc * win;
                *w = v as f32;
                sum += v;
            }
            if sum.abs() > 1e-9 {
                for w in row.iter_mut() {
                    *w = (*w as f64 / sum) as f32;
                }
            }
        }
    }

    /// Weights for the taps around a fractional position `frac` (0..1).
    /// Tap `j` applies to input sample `floor(pos) - (HALF_TAPS - 1) + j`.
    #[inline]
    pub fn weights(&mut self, frac: f64) -> &[f32; TAPS] {
        let f = frac.clamp(0.0, 1.0) * PHASES as f64;
        let p = (f as usize).min(PHASES - 1);
        let a = (f - p as f64) as f32;
        let (r0, r1) = (&self.table[p * TAPS..], &self.table[(p + 1) * TAPS..]);
        for j in 0..TAPS {
            self.weights[j] = r0[j] + (r1[j] - r0[j]) * a;
        }
        &self.weights
    }
}

impl Default for SincTable {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn unit(c: Complex<f32>) -> Complex<f32> {
    let m = c.norm();
    if m > 1e-20 {
        c / m
    } else {
        Complex::new(1.0, 0.0)
    }
}

/// A stereo phase vocoder. Fill `in_a` (the `n` source samples centred on
/// the analysis position) and `in_b` (centred one `hop` earlier), call
/// [`Pv::synth`], then overlap-add `out` (already windowed and scaled) at
/// hop `hop` around the output position.
pub struct Pv {
    pub n: usize,
    pub hop: usize,
    window: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Spectra: left/right at the position, left/right one hop before.
    al: Vec<Complex<f32>>,
    ar: Vec<Complex<f32>>,
    bl: Vec<Complex<f32>>,
    br: Vec<Complex<f32>>,
    /// Synthesis phase (unit phasor) per bin of the mid signal.
    phase: Vec<Complex<f32>>,
    mag: Vec<f32>,
    peaks: Vec<usize>,
    rots: Vec<Complex<f32>>,
    fresh: bool,
    pub in_a: [Vec<f32>; 2],
    pub in_b: [Vec<f32>; 2],
    pub out: [Vec<f32>; 2],
}

impl Pv {
    /// `n` = FFT size (a power of two); the hop is `n / 4`.
    pub fn new(n: usize) -> Pv {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(n);
        let ifft = planner.plan_fft_inverse(n);
        let scratch_len = fft
            .get_inplace_scratch_len()
            .max(ifft.get_inplace_scratch_len());
        // Periodic Hann: overlap-adds to a constant at n/4 hops.
        let window = (0..n)
            .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos()) as f32)
            .collect();
        let z = Complex::new(0.0, 0.0);
        let bins = n / 2 + 1;
        Pv {
            n,
            hop: n / 4,
            window,
            fft,
            ifft,
            scratch: vec![z; scratch_len],
            al: vec![z; n],
            ar: vec![z; n],
            bl: vec![z; n],
            br: vec![z; n],
            phase: vec![Complex::new(1.0, 0.0); bins],
            mag: vec![0.0; bins],
            peaks: Vec::with_capacity(bins),
            rots: Vec::with_capacity(bins),
            fresh: true,
            in_a: [vec![0.0; n], vec![0.0; n]],
            in_b: [vec![0.0; n], vec![0.0; n]],
            out: [vec![0.0; n], vec![0.0; n]],
        }
    }

    /// Start over: the next frame takes its phases straight from the
    /// analysis (a clean start after a jump).
    pub fn reset(&mut self) {
        self.fresh = true;
    }

    fn forward(&mut self) {
        let w = &self.window;
        for (dst, src) in [
            (&mut self.al, &self.in_a[0]),
            (&mut self.ar, &self.in_a[1]),
            (&mut self.bl, &self.in_b[0]),
            (&mut self.br, &self.in_b[1]),
        ] {
            for i in 0..dst.len() {
                dst[i] = Complex::new(src[i] * w[i], 0.0);
            }
        }
        for buf in [&mut self.al, &mut self.ar, &mut self.bl, &mut self.br] {
            self.fft.process_with_scratch(buf, &mut self.scratch);
        }
    }

    /// Make one output frame from `in_a` / `in_b` into `out`.
    pub fn synth(&mut self) {
        let n = self.n;
        let half = n / 2;
        self.forward();
        // Peaks of the mid (L + R) spectrum: both channels get the same
        // phase rotation, so the stereo image stays put.
        for k in 0..=half {
            self.mag[k] = (self.al[k] + self.ar[k]).norm();
        }
        let floor = self.mag.iter().fold(0.0f32, |a, &b| a.max(b)) * 1e-6 + 1e-12;
        self.peaks.clear();
        for k in 0..=half {
            let m = self.mag[k];
            let left = if k > 0 { self.mag[k - 1] } else { 0.0 };
            let right = if k < half { self.mag[k + 1] } else { 0.0 };
            if m > floor && m >= left && m > right {
                self.peaks.push(k);
            }
        }
        // Rotation per peak: new phase = old phase + the source's own
        // advance over one hop (identity phase locking around the peak).
        self.rots.clear();
        for &p in &self.peaks {
            let a = self.al[p] + self.ar[p];
            let u = if self.fresh {
                unit(a)
            } else {
                let b = self.bl[p] + self.br[p];
                unit(self.phase[p] * unit(a * b.conj()))
            };
            self.rots.push(u * unit(a).conj());
        }
        // Apply each peak's rotation to its region (half-way to the next).
        if self.peaks.is_empty() {
            // Silence (or nothing tonal): keep the analysis phases.
        } else {
            let np = self.peaks.len();
            for i in 0..np {
                let lo = if i == 0 {
                    0
                } else {
                    (self.peaks[i - 1] + self.peaks[i]) / 2 + 1
                };
                let hi = if i + 1 == np {
                    half
                } else {
                    (self.peaks[i] + self.peaks[i + 1]) / 2
                };
                let r = self.rots[i];
                for k in lo..=hi {
                    self.al[k] *= r;
                    self.ar[k] *= r;
                }
            }
        }
        for k in 0..=half {
            self.phase[k] = unit(self.al[k] + self.ar[k]);
        }
        self.fresh = false;
        // Hermitian mirror, inverse FFT, synthesis window, scale.
        for buf in [&mut self.al, &mut self.ar] {
            buf[0].im = 0.0;
            buf[half].im = 0.0;
            for k in 1..half {
                buf[n - k] = buf[k].conj();
            }
            self.ifft.process_with_scratch(buf, &mut self.scratch);
        }
        // Hann² at n/4 hops sums to 1.5.
        let scale = 1.0 / (n as f32 * 1.5);
        for i in 0..n {
            let w = self.window[i] * scale;
            self.out[0][i] = self.al[i].re * w;
            self.out[1][i] = self.ar[i].re * w;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Frequency of a sine by counting upward zero crossings.
    pub fn freq(x: &[f32], rate: f64) -> f64 {
        let mut first = None;
        let mut last = 0usize;
        let mut count = 0;
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                if first.is_none() {
                    first = Some(i);
                } else {
                    count += 1;
                }
                last = i;
            }
        }
        let f = first.unwrap();
        count as f64 * rate / (last - f) as f64
    }

    #[test]
    fn sinc_rows_are_normalised_and_phase_zero_is_identity_at_step_one() {
        let mut t = SincTable::new();
        t.set_step(1.0);
        let w = *t.weights(0.0);
        let s: f32 = w.iter().sum();
        assert!((s - 1.0).abs() < 1e-4);
        // At fc = 0.95 the centre tap dominates.
        assert!(w[HALF_TAPS - 1] > 0.9);
        t.set_step(1.4);
        let s: f32 = t.weights(0.37).iter().sum();
        assert!((s - 1.0).abs() < 1e-4);
    }

    /// Run the vocoder over a sine with the analysis moving `rate` source
    /// samples per output sample; returns the output.
    fn run(x: &[f32], rate: f64, frames: usize) -> Vec<f32> {
        let mut pv = Pv::new(2048);
        let (n, hop) = (pv.n, pv.hop);
        let len = frames * hop + n;
        let mut out = vec![0.0f32; len];
        for k in 0..frames {
            let centre = (n as f64 + k as f64 * hop as f64 * rate).round() as i64;
            for i in 0..n {
                let ia = (centre - n as i64 / 2 + i as i64) as usize;
                let ib = ia - hop;
                for c in 0..2 {
                    pv.in_a[c][i] = x.get(ia).copied().unwrap_or(0.0);
                    pv.in_b[c][i] = x.get(ib).copied().unwrap_or(0.0);
                }
            }
            pv.synth();
            for i in 0..n {
                out[k * hop + i] += pv.out[0][i];
            }
        }
        out
    }

    #[test]
    fn stretching_keeps_pitch_and_level() {
        let rate = 44100.0;
        let x: Vec<f32> = (0..200_000)
            .map(|i| (i as f64 * 440.0 * std::f64::consts::TAU / rate).sin() as f32 * 0.5)
            .collect();
        for speed in [0.5, 1.0, 1.5, 0.0] {
            let y = run(&x, speed, 120);
            let mid = &y[4096..y.len() - 4096];
            let f = freq(mid, rate);
            assert!((f - 440.0).abs() < 2.0, "speed {speed}: {f} Hz");
            let peak = mid.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!((peak - 0.5).abs() < 0.05, "speed {speed}: level {peak}");
        }
    }
}
