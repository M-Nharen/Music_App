//! Tap-along latency test: the engine plays clicks, you tap Space in time
//! with what you *hear*. The average gap between when a click was produced
//! and when you tapped is your output delay (Bluetooth adds most of it).

use std::time::Instant;

pub const TEST_BPM: f32 = 80.0;
/// Taps ignored at the start while you find the beat.
const WARMUP_TAPS: usize = 3;
pub const TARGET_TAPS: usize = 16;

#[derive(Default)]
pub struct TapTest {
    pub running: bool,
    clicks: Vec<Instant>,
    taps: Vec<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapResult {
    pub latency_ms: f32,
    /// Median distance of taps from the result; lower = more trustworthy.
    pub spread_ms: f32,
    pub taps_used: usize,
}

impl TapTest {
    pub fn start(&mut self) {
        self.running = true;
        self.clicks.clear();
        self.taps.clear();
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn click(&mut self, at: Instant) {
        if self.running {
            self.clicks.push(at);
        }
    }

    pub fn tap(&mut self, at: Instant) {
        if self.running && !self.clicks.is_empty() {
            self.taps.push(at);
        }
    }

    pub fn taps(&self) -> usize {
        self.taps.len()
    }

    pub fn done(&self) -> bool {
        self.taps.len() >= TARGET_TAPS + WARMUP_TAPS
    }

    pub fn result(&self) -> Option<TapResult> {
        let first = *self.clicks.first()?;
        let interval_ms = 60_000.0 / TEST_BPM;
        let offsets: Vec<f32> = self
            .taps
            .iter()
            .skip(WARMUP_TAPS)
            .filter(|t| **t >= first)
            .map(|t| t.duration_since(first).as_secs_f32() * 1000.0)
            .map(|ms| wrap_offset(ms, interval_ms))
            .collect();
        compute(&offsets)
    }
}

/// Position of a tap within the beat, allowing slightly-early taps to count
/// as negative instead of wrapping to almost a full beat late.
fn wrap_offset(ms_since_first_click: f32, interval_ms: f32) -> f32 {
    let o = ms_since_first_click.rem_euclid(interval_ms);
    if o > interval_ms * 0.85 {
        o - interval_ms
    } else {
        o
    }
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn compute(offsets: &[f32]) -> Option<TapResult> {
    if offsets.len() < 4 {
        return None;
    }
    let mut v = offsets.to_vec();
    let m = median(&mut v);
    let mut dev: Vec<f32> = offsets.iter().map(|o| (o - m).abs()).collect();
    let spread = median(&mut dev);
    Some(TapResult {
        latency_ms: m.max(0.0),
        spread_ms: spread,
        taps_used: offsets.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn finds_bluetooth_delay() {
        let mut t = TapTest::default();
        t.start();
        let base = Instant::now();
        let beat = Duration::from_millis(750);
        for i in 0..25u32 {
            t.click(base + beat * i);
        }
        // Taps ~200 ms after each click, with some human jitter.
        let jitter = [-15i64, 10, -5, 20, 0, -10, 5, 15, -20, 0];
        for i in 0..22u32 {
            let j = jitter[i as usize % jitter.len()];
            let at = base + beat * i + Duration::from_millis((200 + j) as u64);
            t.tap(at);
        }
        let r = t.result().unwrap();
        assert!((r.latency_ms - 200.0).abs() < 10.0, "{r:?}");
        assert!(r.spread_ms < 20.0);
    }

    #[test]
    fn early_taps_dont_wrap() {
        assert!((wrap_offset(750.0 * 3.0 - 20.0, 750.0) + 20.0).abs() < 1e-3);
        assert!((wrap_offset(750.0 * 2.0 + 300.0, 750.0) - 300.0).abs() < 1e-3);
    }
}
