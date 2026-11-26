//! Where Spotify is in the song, between polls, and when to poll.
//!
//! Spotify is asked for its playback state rarely (every ~12 s while the
//! now-playing screen or sync mode is visible, plus once right after a
//! play / pause / seek from here). In between the position is estimated:
//! `progress_ms + time since the answer`, taking half the request's round
//! trip as the moment Spotify measured it. Our own play / pause / seek
//! update the estimate straight away.
//!
//! Pure logic (time is passed in).

use std::time::{Duration, Instant};

/// A playback sample, as good as we know it.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub track_id: Option<String>,
    /// Position at `at`, ms.
    pub progress_ms: f64,
    pub playing: bool,
    pub at: Instant,
    pub duration_ms: f64,
}

impl Estimate {
    /// From an answer that took `sent..received`.
    pub fn from_sample(
        track_id: Option<String>,
        progress_ms: u64,
        playing: bool,
        duration_ms: u64,
        sent: Instant,
        received: Instant,
    ) -> Estimate {
        let rtt = received.saturating_duration_since(sent);
        Estimate {
            track_id,
            progress_ms: progress_ms as f64,
            playing,
            // Spotify read its position somewhere in the middle.
            at: sent + rtt / 2,
            duration_ms: duration_ms as f64,
        }
    }

    /// Estimated position now, ms (never past the end of the track).
    pub fn position_ms(&self, now: Instant) -> f64 {
        let p = if self.playing {
            let dt = match now.checked_duration_since(self.at) {
                Some(d) => d.as_secs_f64(),
                None => -(self.at - now).as_secs_f64(),
            };
            self.progress_ms + dt * 1000.0
        } else {
            self.progress_ms
        };
        if self.duration_ms > 0.0 {
            p.clamp(0.0, self.duration_ms)
        } else {
            p.max(0.0)
        }
    }

    pub fn position_s(&self, now: Instant) -> f64 {
        self.position_ms(now) / 1000.0
    }

    /// When the track should end (if playing).
    pub fn ends_at(&self) -> Option<Instant> {
        if !self.playing || self.duration_ms <= 0.0 {
            return None;
        }
        let left = (self.duration_ms - self.progress_ms).max(0.0);
        Some(self.at + Duration::from_secs_f64(left / 1000.0))
    }

    /// We pressed play / pause here: assume it worked until the next poll.
    pub fn set_playing(&mut self, playing: bool, now: Instant) {
        self.progress_ms = self.position_ms(now);
        self.at = now;
        self.playing = playing;
    }

    /// We sought here.
    pub fn seek(&mut self, ms: f64, now: Instant) {
        self.progress_ms = if self.duration_ms > 0.0 {
            ms.clamp(0.0, self.duration_ms)
        } else {
            ms.max(0.0)
        };
        self.at = now;
    }
}

/// When to ask Spotify for its playback state.
#[derive(Debug)]
pub struct Poller {
    /// Normal gap between polls.
    pub interval: Duration,
    /// Never two polls closer than this (whatever asks for them).
    pub min_gap: Duration,
    last: Option<Instant>,
    /// A one-off poll asked for (after a control, at the track's end).
    soon: Option<Instant>,
    /// Nothing is playing anywhere: poll more slowly.
    pub idle: bool,
}

/// Gap after a play / pause / seek from here before checking it worked.
pub const AFTER_CONTROL: Duration = Duration::from_millis(900);
/// Gap after the expected end of a track (to see what plays next).
pub const AFTER_END: Duration = Duration::from_millis(1500);
/// Nothing playing: poll this much more slowly.
pub const IDLE_FACTOR: u32 = 3;

impl Poller {
    pub fn new(interval_secs: u32) -> Poller {
        Poller {
            interval: Duration::from_secs(interval_secs.clamp(10, 60) as u64),
            min_gap: Duration::from_secs(2),
            last: None,
            soon: None,
            idle: false,
        }
    }

    pub fn set_interval(&mut self, secs: u32) {
        self.interval = Duration::from_secs(secs.clamp(10, 60) as u64);
    }

    fn gap(&self) -> Duration {
        if self.idle {
            self.interval * IDLE_FACTOR
        } else {
            self.interval
        }
    }

    /// When the next poll is due (None = right away).
    pub fn next_at(&self) -> Option<Instant> {
        let last = self.last?;
        let regular = last + self.gap();
        let soon = self.soon.map(|s| s.max(last + self.min_gap));
        Some(match soon {
            Some(s) if s < regular => s,
            _ => regular,
        })
    }

    /// Should a poll go out now? Only when something that shows Spotify's
    /// state is on screen.
    pub fn due(&self, now: Instant, visible: bool) -> bool {
        visible && self.next_at().is_none_or(|t| now >= t)
    }

    /// A poll went out (or was refused by the limiter: wait a full gap
    /// either way, so a refusal never turns into a tight loop).
    pub fn sent(&mut self, now: Instant) {
        self.last = Some(now);
        if self.soon.is_some_and(|s| s <= now + self.min_gap) {
            self.soon = None;
        }
    }

    /// Poll again no earlier than `at` (keeps an earlier request).
    pub fn wait_until(&mut self, at: Instant) {
        let last = self.last.unwrap_or(at);
        // Push the regular poll out too.
        if at > last + self.gap() {
            self.last = Some(at - self.gap());
        }
        if self.soon.is_some_and(|s| s < at) {
            self.soon = Some(at);
        }
    }

    /// Check shortly after we pressed play / pause / seek.
    pub fn after_control(&mut self, now: Instant) {
        self.ask(now + AFTER_CONTROL);
    }

    /// Check just after the current track should have ended.
    pub fn at_track_end(&mut self, end: Instant) {
        self.ask(end + AFTER_END);
    }

    fn ask(&mut self, at: Instant) {
        self.soon = Some(match self.soon {
            Some(s) if s < at => s,
            _ => at,
        });
    }

    /// Became visible again after a while: a poll is due if the last one is
    /// older than a gap (that's just `due`), nothing else to do.
    pub fn last(&self) -> Option<Instant> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn estimate_moves_while_playing() {
        let t0 = Instant::now();
        let e = Estimate::from_sample(Some("a".into()), 10_000, true, 200_000, t0, t0 + ms(200));
        // Measured mid-request.
        assert_eq!(e.at, t0 + ms(100));
        assert!((e.position_ms(t0 + ms(100)) - 10_000.0).abs() < 1e-6);
        assert!((e.position_ms(t0 + ms(5100)) - 15_000.0).abs() < 1e-6);
        // Never past the end.
        assert_eq!(e.position_ms(t0 + Duration::from_secs(1000)), 200_000.0);
        assert_eq!(e.ends_at(), Some(t0 + ms(100) + ms(190_000)));
        // Paused: stays put.
        let p = Estimate::from_sample(None, 5_000, false, 0, t0, t0);
        assert_eq!(p.position_ms(t0 + Duration::from_secs(9)), 5_000.0);
        assert_eq!(p.ends_at(), None);
    }

    #[test]
    fn local_controls_update_the_estimate() {
        let t0 = Instant::now();
        let mut e = Estimate::from_sample(None, 0, true, 100_000, t0, t0);
        e.set_playing(false, t0 + ms(3000));
        assert!((e.position_ms(t0 + ms(9000)) - 3000.0).abs() < 1e-6);
        e.seek(50_000.0, t0 + ms(9000));
        e.set_playing(true, t0 + ms(9000));
        assert!((e.position_ms(t0 + ms(10_000)) - 51_000.0).abs() < 1e-6);
        e.seek(1e9, t0 + ms(10_000));
        assert_eq!(e.progress_ms, 100_000.0);
    }

    #[test]
    fn polls_only_when_visible_and_not_too_often() {
        let t0 = Instant::now();
        let mut p = Poller::new(12);
        assert!(!p.due(t0, false), "nothing visible: no poll");
        assert!(p.due(t0, true), "first poll right away");
        p.sent(t0);
        assert!(!p.due(t0 + ms(11_000), true));
        assert!(p.due(t0 + ms(12_000), true));
        assert!(!p.due(t0 + ms(12_000), false));
        // Count the polls over a minute of a visible screen, checking every
        // frame: about 60 / 12.
        let mut p = Poller::new(12);
        let mut n = 0;
        for f in 0..(60 * 60) {
            let now = t0 + ms(f * 16);
            if p.due(now, true) {
                p.sent(now);
                n += 1;
            }
        }
        assert!((4..=6).contains(&n), "{n} polls");
    }

    #[test]
    fn controls_and_track_end_poll_sooner_but_not_in_a_burst() {
        let t0 = Instant::now();
        let mut p = Poller::new(12);
        p.sent(t0);
        p.after_control(t0 + ms(1000));
        assert!(!p.due(t0 + ms(1500), true));
        // Due 0.9 s after the control, but never within min_gap of the last.
        assert!(!p.due(t0 + ms(1900), true));
        assert!(p.due(t0 + ms(2000), true));
        p.sent(t0 + ms(2000));
        // That one-off is used up; the next is a regular gap later.
        assert!(!p.due(t0 + ms(5000), true));
        // Pressing play/pause many times quickly: at most one poll per min_gap.
        let mut p = Poller::new(12);
        p.sent(t0);
        let mut n = 0;
        for f in 0..300 {
            let now = t0 + ms(f * 10);
            p.after_control(now);
            if p.due(now, true) {
                p.sent(now);
                n += 1;
            }
        }
        assert!(n <= 2, "{n} polls in 3 s");
        // Track end.
        let mut p = Poller::new(12);
        p.sent(t0);
        p.at_track_end(t0 + ms(4000));
        assert!(!p.due(t0 + ms(5000), true));
        assert!(p.due(t0 + ms(5600), true));
    }

    #[test]
    fn waiting_out_a_pause_and_idle() {
        let t0 = Instant::now();
        let mut p = Poller::new(12);
        p.sent(t0);
        p.after_control(t0);
        p.wait_until(t0 + Duration::from_secs(300));
        assert!(!p.due(t0 + Duration::from_secs(299), true));
        assert!(p.due(t0 + Duration::from_secs(300), true));
        let mut p = Poller::new(12);
        p.idle = true;
        p.sent(t0);
        assert!(!p.due(t0 + Duration::from_secs(30), true));
        assert!(p.due(t0 + Duration::from_secs(36), true));
    }
}
