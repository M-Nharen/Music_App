//! Jam mode (Part 7): the analyzer's stems (separated vocals, drums, bass,
//! …) played as the backing track, in the audio thread.
//!
//! - [`StemSet`] is the decoded audio (16-bit stereo in memory, decoded on
//!   a background thread). The UI keeps an `Arc` to it and only drops one
//!   after the audio thread has let go (`Arc::strong_count == 1`), so the
//!   audio thread never frees it.
//! - [`StemPlayer`] follows the song clock of [`super::songplay::SongPlayer`]:
//!   at 100 % speed and no pitch change it just reads the stems (plus
//!   sample-rate conversion); otherwise a phase vocoder time-stretches
//!   (pitch kept) and pitch-shifts them. Wait mode freezes the sound and
//!   lets it fade, pausing fades it out quickly, and seeks / loop jumps
//!   cross-fade over 10 ms.
//!
//! Each "lane" turns the source into a stream at `pitch × source rate`,
//! read by a windowed-sinc resampler at `pitch × source rate / output
//! rate` stream samples per output sample. Two lanes per bus let a jump or
//! a mode change cross-fade; a second bus keeps the drums out of a pitch
//! shift. Everything is allocated in `new`; `render` never allocates.

use super::stretch::{HALF_TAPS, Pv, SincTable};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

pub const MAX_STEMS: usize = 8;
/// FFT size of the vocoder (46 ms at 44.1 kHz).
const PV_N: usize = 2048;
const RING: usize = 8192;
const RMASK: usize = RING - 1;
const XFADE_S: f64 = 0.010;
/// Output samples rendered per piece (bounds how far lanes read ahead).
const CHUNK: usize = 128;
const MIX_CHUNK: usize = 512;

/// One decoded stem: interleaved stereo, 16 bits.
pub struct StemAudio {
    pub name: String,
    pub frames: usize,
    data: Vec<i16>,
}

/// All of a song's stems, at one sample rate.
pub struct StemSet {
    pub rate: u32,
    pub stems: Vec<StemAudio>,
    /// Index of the drum stem (kept out of pitch shifts).
    pub drums: Option<usize>,
}

fn to16(s: i32, bits: u32) -> i16 {
    let v = if bits > 16 {
        s >> (bits - 16)
    } else {
        s << (16 - bits)
    };
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Decode a FLAC file to interleaved 16-bit stereo. `progress(done, total)`.
fn decode_flac(path: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(u32, Vec<i16>), String> {
    let mut reader =
        claxon::FlacReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let info = reader.streaminfo();
    let ch = info.channels.max(1);
    let bits = info.bits_per_sample;
    let total = info.samples.unwrap_or(0);
    let mut data: Vec<i16> = Vec::with_capacity(total as usize * 2);
    let mut blocks = reader.blocks();
    let mut buf = Vec::new();
    let mut done = 0u64;
    loop {
        match blocks.read_next_or_eof(buf) {
            Ok(Some(block)) => {
                for i in 0..block.duration() {
                    let l = block.sample(0, i);
                    let r = if ch > 1 { block.sample(1, i) } else { l };
                    data.push(to16(l, bits));
                    data.push(to16(r, bits));
                }
                done += block.duration() as u64;
                if done % (1 << 16) < block.duration() as u64 {
                    progress(done, total.max(done));
                }
                buf = block.into_buffer();
            }
            Ok(None) => break,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok((info.sample_rate, data))
}

impl StemSet {
    /// Load `(name, file relative to dir)` pairs. `progress` goes 0 → 1000.
    pub fn load(
        dir: &Path,
        files: &[(String, String)],
        progress: &AtomicU32,
    ) -> Result<StemSet, String> {
        let files = &files[..files.len().min(MAX_STEMS)];
        if files.is_empty() {
            return Err("this song has no stems".into());
        }
        let mut stems = Vec::new();
        let mut rate = 0;
        let n = files.len() as u64;
        for (i, (name, file)) in files.iter().enumerate() {
            let path = dir.join(file);
            let i = i as u64;
            let (r, data) = decode_flac(&path, &mut |d, t| {
                let p = (i * 1000 + d * 1000 / t.max(1)) / n;
                progress.store(p as u32, Ordering::Relaxed);
            })?;
            if rate == 0 {
                rate = r;
            } else if r != rate {
                return Err(format!("{file} is at {r} Hz, the other stems at {rate} Hz"));
            }
            stems.push(StemAudio {
                name: name.clone(),
                frames: data.len() / 2,
                data,
            });
            progress.store(((i + 1) * 1000 / n) as u32, Ordering::Relaxed);
        }
        Ok(StemSet::new(rate, stems))
    }

    fn new(rate: u32, stems: Vec<StemAudio>) -> StemSet {
        let drums = stems.iter().position(|s| s.name == "drums");
        StemSet { rate, stems, drums }
    }

    /// From float samples (interleaved stereo), for tests.
    #[cfg(test)]
    pub fn from_f32(rate: u32, stems: Vec<(&str, Vec<f32>)>) -> StemSet {
        let stems = stems
            .into_iter()
            .map(|(name, x)| StemAudio {
                name: name.to_string(),
                frames: x.len() / 2,
                data: x
                    .iter()
                    .map(|v| (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16)
                    .collect(),
            })
            .collect();
        StemSet::new(rate, stems)
    }

    #[cfg(test)]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.stems.iter().position(|s| s.name == name)
    }

    #[cfg(test)]
    pub fn duration(&self) -> f64 {
        self.stems.iter().map(|s| s.frames).max().unwrap_or(0) as f64 / self.rate.max(1) as f64
    }

    /// Memory used by the samples.
    pub fn bytes(&self) -> usize {
        self.stems.iter().map(|s| s.data.len() * 2).sum()
    }

    /// Mix the stems in `mask` from source frame `start` into `l` / `r`
    /// (overwriting), with each stem's gain going linearly from `g0` to
    /// `g1` over the buffer. Outside the audio = silence.
    fn mix(
        &self,
        g0: &[f32; MAX_STEMS],
        g1: &[f32; MAX_STEMS],
        mask: u32,
        start: i64,
        l: &mut [f32],
        r: &mut [f32],
    ) {
        l.fill(0.0);
        r.fill(0.0);
        let len = l.len();
        if len == 0 {
            return;
        }
        let inv = 1.0 / 32768.0;
        for (k, s) in self.stems.iter().enumerate() {
            if mask & (1 << k) == 0 || (g0[k] <= 0.0 && g1[k] <= 0.0) {
                continue;
            }
            // Overlap of [start, start + len) with [0, frames).
            let lo = (-start).clamp(0, len as i64) as usize;
            let hi = (s.frames as i64 - start).clamp(0, len as i64) as usize;
            if lo >= hi {
                continue;
            }
            let dg = (g1[k] - g0[k]) / len as f32;
            for i in lo..hi {
                let g = (g0[k] + dg * i as f32) * inv;
                let j = (start + i as i64) as usize * 2;
                l[i] += s.data[j] as f32 * g;
                r[i] += s.data[j + 1] as f32 * g;
            }
        }
    }
}

/// What the song clock says, for one piece of audio.
#[derive(Clone, Copy, Debug)]
pub struct SongCtx {
    /// Song time (s) of the first sample of this piece.
    pub pos: f64,
    pub speed: f64,
    pub playing: bool,
    /// Playing and not held by wait mode.
    pub moving: bool,
    /// Wait mode's stop point.
    pub wait: Option<f64>,
    /// Bumped on every seek and loop jump.
    pub jumps: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Idle,
    /// Read the stems as they are (+ sample-rate conversion).
    Direct,
    /// Phase vocoder: time-stretch / pitch-shift / freeze.
    Pv,
}

/// Where the source position comes from.
#[derive(Clone, Copy, PartialEq)]
enum Drive {
    /// Follow the song clock.
    Clock,
    /// Keep going the way it was (a lane fading out after a jump).
    Coast,
}

/// The clock at the start of a render call, seen from one lane.
#[derive(Clone, Copy)]
struct Predict {
    rp0: f64,
    pos: f64,
    speed: f64,
    moving: bool,
    wait: Option<f64>,
    fs: f64,
    fo: f64,
}

struct Lane {
    mode: Mode,
    l: Vec<f32>,
    r: Vec<f32>,
    /// Stream samples before this are complete.
    final_end: i64,
    /// Stream samples from here on haven't been cleared for overlap-add.
    zero_end: i64,
    /// Read position in the stream.
    rp: f64,
    rp0: f64,
    /// Direct: stream sample 0 is source frame `src0`.
    src0: i64,
    /// Vocoder: stream time of the next frame's centre.
    next_t: i64,
    drive: Drive,
    last_x: f64,
    last_t: f64,
    /// Source frames per stream sample (for coasting).
    rate: f64,
    fade: f32,
    fade_step: f32,
    mask: u32,
    ratio: f64,
    g_last: [f32; MAX_STEMS],
    tmp: [Vec<f32>; 2],
    pv: Pv,
    sinc: SincTable,
}

impl Lane {
    fn new() -> Lane {
        Lane {
            mode: Mode::Idle,
            l: vec![0.0; RING],
            r: vec![0.0; RING],
            final_end: 0,
            zero_end: 0,
            rp: 0.0,
            rp0: 0.0,
            src0: 0,
            next_t: 0,
            drive: Drive::Clock,
            last_x: 0.0,
            last_t: 0.0,
            rate: 0.0,
            fade: 0.0,
            fade_step: 0.0,
            mask: 0,
            ratio: 1.0,
            g_last: [0.0; MAX_STEMS],
            tmp: [vec![0.0; MIX_CHUNK], vec![0.0; MIX_CHUNK]],
            pv: Pv::new(PV_N),
            sinc: SincTable::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        mode: Mode,
        x: f64,
        mask: u32,
        ratio: f64,
        step: f64,
        gains: &[f32; MAX_STEMS],
        fade: f32,
        fade_step: f32,
    ) {
        self.mode = mode;
        self.mask = mask;
        self.ratio = ratio;
        self.sinc.set_step(step);
        self.drive = Drive::Clock;
        self.fade = fade;
        self.fade_step = fade_step;
        self.g_last = *gains;
        match mode {
            Mode::Direct => {
                self.src0 = x.floor() as i64;
                self.rp = x - x.floor();
                self.final_end = -(HALF_TAPS as i64);
            }
            Mode::Pv => {
                let n = self.pv.n as i64;
                self.pv.reset();
                self.rp = 0.0;
                self.next_t = -n / 2 - 32;
                self.final_end = self.next_t - n / 2;
                self.zero_end = self.final_end;
                self.last_x = x;
                self.last_t = 0.0;
            }
            Mode::Idle => {}
        }
        self.rp0 = self.rp;
    }

    /// Fade out, carrying on the way it was going.
    fn fade_out(&mut self, step: f32) {
        self.drive = Drive::Coast;
        self.fade_step = -step;
    }

    /// Source position (frames) for stream time `t`.
    fn source_at(&self, t: f64, p: &Predict) -> f64 {
        match self.drive {
            Drive::Coast => self.last_x + (t - self.last_t) * self.rate,
            Drive::Clock => {
                let out_dt = (t - p.rp0) / self.sinc.step();
                let mut x = p.fs
                    * (p.pos
                        + if p.moving {
                            out_dt / p.fo * p.speed
                        } else {
                            0.0
                        });
                if let Some(w) = p.wait {
                    // Wait mode: freeze on the last frame before the note.
                    x = x.min(w * p.fs - (self.pv.n / 2) as f64);
                }
                x
            }
        }
    }

    /// Make the stream complete up to `need_end` (exclusive).
    fn fill(&mut self, need_end: i64, set: &StemSet, gains: &[f32; MAX_STEMS], p: &Predict) {
        match self.mode {
            Mode::Idle => {}
            Mode::Direct => {
                while self.final_end < need_end {
                    let m = ((need_end - self.final_end) as usize).min(MIX_CHUNK);
                    let [tl, tr] = &mut self.tmp;
                    set.mix(
                        &self.g_last,
                        gains,
                        self.mask,
                        self.src0 + self.final_end,
                        &mut tl[..m],
                        &mut tr[..m],
                    );
                    self.g_last = *gains;
                    for i in 0..m {
                        let idx = (self.final_end + i as i64) as u64 as usize & RMASK;
                        self.l[idx] = self.tmp[0][i];
                        self.r[idx] = self.tmp[1][i];
                    }
                    self.final_end += m as i64;
                }
            }
            Mode::Pv => {
                let n = self.pv.n as i64;
                let hop = self.pv.hop as i64;
                if self.drive == Drive::Clock {
                    self.rate = if p.moving { p.speed / self.ratio } else { 0.0 };
                }
                while self.final_end < need_end {
                    let t = self.next_t;
                    let x = self.source_at(t as f64, p);
                    let xi = x.round() as i64;
                    {
                        let [a0, a1] = &mut self.pv.in_a;
                        set.mix(gains, gains, self.mask, xi - n / 2, a0, a1);
                        let [b0, b1] = &mut self.pv.in_b;
                        set.mix(gains, gains, self.mask, xi - hop - n / 2, b0, b1);
                    }
                    self.pv.synth();
                    // Clear the part of the ring this frame reaches first.
                    let lo = self.zero_end.max(t - n / 2);
                    for q in lo..t + n / 2 {
                        let idx = q as u64 as usize & RMASK;
                        self.l[idx] = 0.0;
                        self.r[idx] = 0.0;
                    }
                    self.zero_end = self.zero_end.max(t + n / 2);
                    for i in 0..n as usize {
                        let idx = (t - n / 2 + i as i64) as u64 as usize & RMASK;
                        self.l[idx] += self.pv.out[0][i];
                        self.r[idx] += self.pv.out[1][i];
                    }
                    self.final_end = t + hop - n / 2;
                    self.last_x = x;
                    self.last_t = t as f64;
                    self.next_t += hop;
                }
            }
        }
    }

    /// Stream end needed to read `m` more output samples.
    fn need(&self, m: usize) -> i64 {
        (self.rp + (m.max(1) - 1) as f64 * self.sinc.step()).floor() as i64 + HALF_TAPS as i64 + 1
    }

    /// One output sample (before the cross-fade gain).
    #[inline]
    fn read(&mut self) -> (f32, f32) {
        let base = self.rp.floor();
        let start = base as i64 - (HALF_TAPS as i64 - 1);
        let w = self.sinc.weights(self.rp - base);
        let (mut a, mut b) = (0.0f32, 0.0f32);
        for (j, wj) in w.iter().enumerate() {
            let idx = (start + j as i64) as u64 as usize & RMASK;
            a += wj * self.l[idx];
            b += wj * self.r[idx];
        }
        self.rp += self.sinc.step();
        (a, b)
    }

    /// Where this lane is in the source right now (frames).
    fn source_now(&self, p: &Predict) -> f64 {
        match self.mode {
            Mode::Direct => self.src0 as f64 + self.rp,
            _ => self.source_at(self.rp, p),
        }
    }
}

struct Bus {
    lanes: [Lane; 2],
    cur: usize,
}

/// Plays a [`StemSet`] along with the song clock. Lives in the audio thread.
pub struct StemPlayer {
    set: Option<Arc<StemSet>>,
    on: bool,
    /// Gain per stem, as asked and as it is now (ramped).
    target: [f32; MAX_STEMS],
    gains: [f32; MAX_STEMS],
    semitones: i32,
    out_rate: f64,
    env: f32,
    buses: [Bus; 2],
    seen_jumps: u32,
    was_moving: bool,
}

impl StemPlayer {
    pub fn new(out_rate: u32) -> StemPlayer {
        let bus = || Bus {
            lanes: [Lane::new(), Lane::new()],
            cur: 0,
        };
        StemPlayer {
            set: None,
            on: false,
            target: [1.0; MAX_STEMS],
            gains: [1.0; MAX_STEMS],
            semitones: 0,
            out_rate: out_rate.max(1) as f64,
            env: 0.0,
            buses: [bus(), bus()],
            seen_jumps: 0,
            was_moving: false,
        }
    }

    fn stop_all(&mut self) {
        for b in &mut self.buses {
            for l in &mut b.lanes {
                l.mode = Mode::Idle;
            }
        }
        self.env = 0.0;
    }

    /// New stems (the old `Arc` is dropped here; the UI still holds it).
    pub fn set_stems(&mut self, set: Option<Arc<StemSet>>) {
        self.set = set;
        self.stop_all();
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    pub fn set_gains(&mut self, g: [f32; MAX_STEMS]) {
        self.target = g.map(|x| x.clamp(0.0, 2.0));
    }

    pub fn set_semitones(&mut self, s: i32) {
        self.semitones = s.clamp(-12, 12);
    }

    #[cfg(test)]
    fn modes(&self) -> Vec<Mode> {
        self.buses
            .iter()
            .flat_map(|b| b.lanes.iter().map(|l| l.mode))
            .collect()
    }

    /// Add the stems for `l.len()` samples starting at the clock `ctx`.
    pub fn render(&mut self, l: &mut [f32], r: &mut [f32], ctx: SongCtx) {
        let Some(set) = self.set.clone() else {
            return;
        };
        let frames = l.len().min(r.len());
        if frames == 0 {
            return;
        }
        let fs = set.rate.max(1) as f64;
        let fo = self.out_rate;
        // Volume / mute changes ramp over 20 ms.
        let max_step = (frames as f64 / (0.02 * fo)) as f32;
        for k in 0..MAX_STEMS {
            let d = (self.target[k] - self.gains[k]).clamp(-max_step, max_step);
            self.gains[k] += d;
        }
        // Loudness envelope: fade in on play, freeze-and-fade in wait
        // mode, quick fade on pause.
        let (target, tau) = if !self.on || !ctx.playing {
            (0.0f32, 0.008)
        } else if !ctx.moving {
            (0.0, 0.35)
        } else {
            (1.0, 0.005)
        };
        let jumped = ctx.jumps != self.seen_jumps;
        self.seen_jumps = ctx.jumps;
        if target == 0.0 && self.env < 1e-3 {
            self.stop_all();
            self.was_moving = false;
            return;
        }
        let resumed = ctx.moving && !self.was_moving && self.env < 0.3;
        self.was_moving = ctx.moving;

        // Bus 0: everything (or all but the drums when pitch-shifting);
        // bus 1: the drums at their own pitch.
        let all = (1u32 << set.stems.len().min(MAX_STEMS)) - 1;
        let ratio = 2f64.powf(self.semitones as f64 / 12.0);
        let split = self.semitones != 0 && set.drums.is_some();
        let drum_bit = set.drums.map(|d| 1u32 << d).unwrap_or(0);
        let want = [
            (if split { all & !drum_bit } else { all }, ratio),
            (if split { drum_bit } else { 0 }, 1.0),
        ];
        let xfade = (1.0 / (XFADE_S * fo)) as f32;
        let quiet = self.env < 1e-3;
        for (b, &(mask, ratio)) in want.iter().enumerate() {
            let bus = &mut self.buses[b];
            let cur = &bus.lanes[bus.cur];
            let p = Predict {
                rp0: cur.rp,
                pos: ctx.pos,
                speed: ctx.speed,
                moving: ctx.moving,
                wait: ctx.wait,
                fs,
                fo,
            };
            let unit_speed = (ctx.speed - 1.0).abs() < 1e-9;
            let desired =
                if ctx.moving && unit_speed && (ratio - 1.0).abs() < 1e-9 && ctx.wait.is_none() {
                    Mode::Direct
                } else {
                    Mode::Pv
                };
            let restart = if cur.mode == Mode::Idle {
                mask != 0 && ctx.playing
            } else {
                jumped
                    || resumed
                    || cur.mask != mask
                    || (cur.ratio - ratio).abs() > 1e-9
                    || (ctx.playing && cur.mode != desired)
                    // Drifted (shouldn't happen; a safety net).
                    || (cur.mode == Mode::Direct
                        && ctx.moving
                        && (cur.source_now(&p) - ctx.pos * fs).abs() > 0.002 * fs)
            };
            if !restart {
                continue;
            }
            let old = bus.cur;
            let old_on = bus.lanes[old].mode != Mode::Idle && !quiet;
            if old_on {
                bus.lanes[old].fade_out(xfade);
            } else {
                bus.lanes[old].mode = Mode::Idle;
            }
            if mask == 0 {
                continue;
            }
            let new = 1 - old;
            let step = if desired == Mode::Direct {
                fs / fo
            } else {
                ratio * fs / fo
            };
            let (fade, fstep) = if old_on { (0.0, xfade) } else { (1.0, 0.0) };
            bus.lanes[new].start(
                desired,
                ctx.pos * fs,
                mask,
                ratio,
                step,
                &self.gains,
                fade,
                fstep,
            );
            bus.cur = new;
        }

        // Render.
        let k_env = 1.0 - (-1.0 / (tau * fo)).exp() as f32;
        for bus in &mut self.buses {
            for lane in &mut bus.lanes {
                lane.rp0 = lane.rp;
            }
        }
        let mut done = 0;
        while done < frames {
            let m = (frames - done).min(CHUNK);
            for bus in &mut self.buses {
                for lane in &mut bus.lanes {
                    if lane.mode == Mode::Idle {
                        continue;
                    }
                    let p = Predict {
                        rp0: lane.rp0,
                        pos: ctx.pos,
                        speed: ctx.speed,
                        moving: ctx.moving,
                        wait: ctx.wait,
                        fs,
                        fo,
                    };
                    lane.fill(lane.need(m), &set, &self.gains, &p);
                }
            }
            for i in done..done + m {
                let (mut a, mut b) = (0.0f32, 0.0f32);
                for bus in &mut self.buses {
                    for lane in &mut bus.lanes {
                        if lane.mode == Mode::Idle {
                            continue;
                        }
                        let (x, y) = lane.read();
                        let f = lane.fade;
                        let g = f * f * (3.0 - 2.0 * f);
                        a += x * g;
                        b += y * g;
                        if lane.fade_step != 0.0 {
                            lane.fade = (lane.fade + lane.fade_step).clamp(0.0, 1.0);
                            if lane.fade >= 1.0 {
                                lane.fade_step = 0.0;
                            } else if lane.fade <= 0.0 {
                                lane.mode = Mode::Idle;
                            }
                        }
                    }
                }
                self.env += (target - self.env) * k_env;
                l[i] += a * self.env;
                r[i] += b * self.env;
            }
            done += m;
        }
        if target == 0.0 && self.env < 1e-3 {
            self.stop_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::stretch::tests as st;

    const FO: u32 = 48_000;
    const FS: u32 = 44_100;

    fn sine(f: f64, secs: f64, amp: f32) -> Vec<f32> {
        let n = (secs * FS as f64) as usize;
        let mut v = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = (i as f64 * f * std::f64::consts::TAU / FS as f64).sin() as f32 * amp;
            v.push(s);
            v.push(s);
        }
        v
    }

    /// Silence with a 20 ms 1 kHz burst starting at `at` seconds.
    fn burst(at: f64, secs: f64) -> Vec<f32> {
        let n = (secs * FS as f64) as usize;
        let s0 = (at * FS as f64) as usize;
        let mut v = vec![0.0; n * 2];
        for i in s0..s0 + FS as usize / 50 {
            let s =
                (((i - s0) as f64) * 1000.0 * std::f64::consts::TAU / FS as f64).sin() as f32 * 0.8;
            v[2 * i] = s;
            v[2 * i + 1] = s;
        }
        v
    }

    /// A tiny song clock: drives the player like SongPlayer does.
    struct Clock {
        pos: f64,
        speed: f64,
        playing: bool,
        wait: Option<f64>,
        jumps: u32,
    }

    impl Clock {
        fn new(speed: f64) -> Clock {
            Clock {
                pos: 0.0,
                speed,
                playing: true,
                wait: None,
                jumps: 0,
            }
        }
        fn moving(&self) -> bool {
            self.playing && !self.wait.is_some_and(|w| self.pos >= w - 1e-9)
        }
        fn run(&mut self, p: &mut StemPlayer, secs: f64) -> Vec<f32> {
            let frames = (secs * FO as f64) as usize;
            let mut out = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            let mut i = 0;
            while i < frames {
                let m = (frames - i).min(128);
                let ctx = SongCtx {
                    pos: self.pos,
                    speed: self.speed,
                    playing: self.playing,
                    moving: self.moving(),
                    wait: self.wait,
                    jumps: self.jumps,
                };
                p.render(&mut out[i..i + m], &mut right[i..i + m], ctx);
                if ctx.moving {
                    self.pos += m as f64 / FO as f64 * self.speed;
                    if let Some(w) = self.wait {
                        self.pos = self.pos.min(w);
                    }
                }
                i += m;
            }
            out
        }
    }

    fn player(stems: Vec<(&str, Vec<f32>)>) -> StemPlayer {
        let mut p = StemPlayer::new(FO);
        p.set_stems(Some(Arc::new(StemSet::from_f32(FS, stems))));
        p.set_on(true);
        p
    }

    fn onset(y: &[f32]) -> f64 {
        let peak = y.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        let i = y.iter().position(|v| v.abs() > peak * 0.5).unwrap();
        i as f64 / FO as f64
    }

    fn rms(y: &[f32]) -> f32 {
        (y.iter().map(|v| v * v).sum::<f32>() / y.len().max(1) as f32).sqrt()
    }

    #[test]
    fn sounds_line_up_with_the_song_clock_at_any_speed_and_pitch() {
        for (speed, semis, tol) in [
            (1.0, 0, 0.001),
            (0.5, 0, 0.012),
            (0.75, 3, 0.012),
            (1.0, -5, 0.012),
        ] {
            let mut p = player(vec![("other", burst(1.0, 3.0))]);
            p.set_semitones(semis);
            let mut c = Clock::new(speed);
            let y = c.run(&mut p, 1.0 / speed + 0.5);
            let t = onset(&y);
            assert!(
                (t - 1.0 / speed).abs() < tol,
                "speed {speed} pitch {semis}: burst at {t:.4} s, wanted {:.4}",
                1.0 / speed
            );
        }
    }

    #[test]
    fn keeps_pitch_when_slowed_and_shifts_it_when_asked() {
        for (speed, semis) in [
            (1.0, 0),
            (0.5, 0),
            (1.25, 0),
            (1.0, 12),
            (0.75, -5),
            (1.0, 2),
        ] {
            let mut p = player(vec![("other", sine(440.0, 6.0, 0.5))]);
            p.set_semitones(semis);
            let y = Clock::new(speed).run(&mut p, 2.0);
            let mid = &y[FO as usize / 2..];
            let want = 440.0 * 2f64.powf(semis as f64 / 12.0);
            let f = st::freq(mid, FO as f64);
            assert!(
                (f - want).abs() < want * 0.005,
                "speed {speed} pitch {semis}: {f} Hz, wanted {want}"
            );
            let level = rms(mid);
            assert!(
                (level - 0.354).abs() < 0.04,
                "speed {speed} pitch {semis}: rms {level}"
            );
        }
    }

    #[test]
    fn the_drums_are_not_pitch_shifted() {
        let mut p = player(vec![("drums", sine(200.0, 4.0, 0.5))]);
        p.set_semitones(4);
        let y = Clock::new(1.0).run(&mut p, 1.0);
        let f = st::freq(&y[FO as usize / 4..], FO as f64);
        assert!((f - 200.0).abs() < 1.0, "{f}");
    }

    #[test]
    fn gains_mute_stems() {
        let mut p = player(vec![
            ("vocals", sine(440.0, 4.0, 0.5)),
            ("bass", sine(110.0, 4.0, 0.5)),
        ]);
        let mut g = [1.0; MAX_STEMS];
        g[0] = 0.0;
        p.set_gains(g);
        p.gains = p.target;
        let y = Clock::new(1.0).run(&mut p, 1.0);
        let f = st::freq(&y[FO as usize / 4..], FO as f64);
        assert!((f - 110.0).abs() < 1.0, "only the bass is left: {f}");
        // Muting while playing ramps (no click).
        p.set_gains([0.0; MAX_STEMS]);
        let mut c = Clock::new(1.0);
        c.pos = 1.0;
        let y = c.run(&mut p, 0.2);
        let jump = y
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(jump < 0.02, "{jump}");
        assert!(rms(&y[FO as usize / 10..]) < 1e-3);
    }

    #[test]
    fn loop_jumps_and_mode_changes_cross_fade() {
        for speed in [1.0, 0.6] {
            let mut p = player(vec![("other", sine(440.0, 6.0, 0.5))]);
            let mut c = Clock::new(speed);
            let mut y = c.run(&mut p, 1.0);
            // Loop back.
            c.pos = 0.5;
            c.jumps += 1;
            y.extend(c.run(&mut p, 0.5));
            // Speed change (Direct ↔ vocoder).
            c.speed = if speed == 1.0 { 0.8 } else { 1.0 };
            y.extend(c.run(&mut p, 0.5));
            let start = FO as usize / 10;
            let jump = y[start..]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max);
            // A 440 Hz sine at 0.5 moves at most 0.029 per sample.
            assert!(jump < 0.06, "speed {speed}: step {jump}");
            assert!(rms(&y[start..]) > 0.3);
        }
    }

    #[test]
    fn wait_mode_freezes_then_fades_and_pause_is_quick() {
        let mut p = player(vec![("other", sine(440.0, 6.0, 0.5))]);
        let mut c = Clock::new(1.0);
        c.wait = Some(1.0);
        let y = c.run(&mut p, 2.5);
        let s = FO as usize;
        assert!(
            rms(&y[s / 2..s * 9 / 10]) > 0.3,
            "plays up to the wait point"
        );
        assert!(
            rms(&y[s + s / 50..s + s / 10]) > 0.15,
            "the sound hangs on a moment"
        );
        assert!(rms(&y[s * 2..]) < 0.02, "then fades");
        // Released: plays again.
        c.wait = None;
        let y = c.run(&mut p, 0.5);
        assert!(rms(&y[FO as usize / 10..]) > 0.3);
        // Pause: quiet within 50 ms.
        c.playing = false;
        let y = c.run(&mut p, 0.2);
        assert!(rms(&y[FO as usize / 20..]) < 1e-3);
        assert!(
            p.modes().iter().all(|m| *m == Mode::Idle),
            "nothing runs while paused"
        );
    }

    #[test]
    fn before_and_after_the_audio_is_silence() {
        let mut p = player(vec![("other", sine(440.0, 1.0, 0.5))]);
        let mut c = Clock::new(0.7);
        c.pos = -1.0;
        let y = c.run(&mut p, 0.5);
        assert!(rms(&y) < 1e-4);
        c.pos = 1.2;
        c.jumps += 1;
        let y = c.run(&mut p, 0.5);
        assert!(rms(&y[FO as usize / 10..]) < 1e-4);
    }

    #[test]
    fn rendering_never_allocates() {
        let mut p = player(vec![
            ("other", sine(440.0, 6.0, 0.5)),
            ("drums", sine(100.0, 6.0, 0.5)),
        ]);
        let mut c = Clock::new(1.0);
        c.run(&mut p, 0.1);
        let before = crate::testalloc::count();
        let mut out = [0.0f32; 128];
        let mut right = [0.0f32; 128];
        for step in 0..600 {
            match step {
                100 => c.speed = 0.5,
                200 => p.set_semitones(3),
                250 => {
                    c.pos = 0.5;
                    c.jumps += 1
                }
                300 => c.wait = Some(c.pos + 0.1),
                400 => c.wait = None,
                450 => p.set_gains([0.5; MAX_STEMS]),
                500 => c.playing = false,
                550 => c.playing = true,
                _ => {}
            }
            let ctx = SongCtx {
                pos: c.pos,
                speed: c.speed,
                playing: c.playing,
                moving: c.moving(),
                wait: c.wait,
                jumps: c.jumps,
            };
            p.render(&mut out, &mut right, ctx);
            if ctx.moving {
                c.pos += 128.0 / FO as f64 * c.speed;
                if let Some(w) = c.wait {
                    c.pos = c.pos.min(w);
                }
            }
        }
        assert_eq!(crate::testalloc::count(), before);
    }

    /// `cargo test --release -- --ignored --nocapture cost`: how much of
    /// one CPU core the stems take (6 stems, slowed and pitch-shifted).
    #[test]
    #[ignore]
    fn cost() {
        let stems: Vec<(&str, Vec<f32>)> = ["vocals", "drums", "bass", "other", "piano", "guitar"]
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, sine(110.0 * (i + 1) as f64, 30.0, 0.1)))
            .collect();
        for (speed, semis) in [(1.0, 0), (0.75, 0), (0.75, 3)] {
            let mut p = player(stems.clone());
            p.set_semitones(semis);
            let mut c = Clock::new(speed);
            let t = std::time::Instant::now();
            c.run(&mut p, 10.0);
            let el = t.elapsed().as_secs_f64();
            eprintln!(
                "speed {speed} pitch {semis:+}: {:.2} % of a core",
                el / 10.0 * 100.0
            );
        }
    }

    #[test]
    fn loads_flac_stems() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jam-song");
        let progress = AtomicU32::new(0);
        let files: Vec<(String, String)> = ["vocals", "drums", "bass", "other"]
            .iter()
            .map(|n| (n.to_string(), format!("stems/{n}.flac")))
            .collect();
        let set = StemSet::load(&dir, &files, &progress).unwrap();
        assert_eq!(progress.load(Ordering::Relaxed), 1000);
        assert_eq!(set.rate, 44100);
        assert_eq!(set.stems.len(), 4);
        assert_eq!(set.drums, Some(1));
        assert!((set.duration() - 4.0).abs() < 0.01, "{}", set.duration());
        assert_eq!(set.index_of("bass"), Some(2));
        // Not silent, and stereo.
        let v = &set.stems[0];
        assert!(v.data.iter().any(|s| s.unsigned_abs() > 1000));
        let missing = StemSet::load(&dir, &[("x".into(), "stems/nope.flac".into())], &progress);
        assert!(missing.is_err());
    }
}
