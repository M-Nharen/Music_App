//! Keeping Web API calls few and polite.
//!
//! Development Mode apps have small, undocumented rate limits and a quota
//! (a 429 whose body says `"reason": "QUOTA_EXCEEDED"`). The rules here:
//!
//! - **Our own budget**: at most `WINDOW_MAX` calls per rolling 30 s
//!   (Spotify's own window is 30 s). Background polls stop `RESERVED` short
//!   of that, so a key press never finds the budget used up by polling.
//! - **429**: pause *every* call until `Retry-After` has passed (never
//!   shorter), longer for repeated 429s, and an hour when the quota is used
//!   up and Spotify doesn't say how long. Nothing is retried by itself: the
//!   next poll simply happens after the pause.
//! - **Network / 5xx errors**: a short pause that doubles (15 s … 5 min).
//!
//! While paused, calls fail at once without touching the network and the
//! app runs on its own estimate of the position (offline mode).
//!
//! Pure logic (time is passed in), so it's tested without a network.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Rolling window, like Spotify's.
pub const WINDOW: Duration = Duration::from_secs(30);
/// Our own ceiling per window (far below what gets a 429 in practice).
pub const WINDOW_MAX: usize = 12;
/// Calls in each window that only user actions may use.
pub const RESERVED: usize = 4;
/// First pause after a 429 without `Retry-After`.
pub const DEFAULT_429: Duration = Duration::from_secs(60);
/// Pause when the quota is used up and Spotify doesn't say for how long.
pub const DEFAULT_QUOTA: Duration = Duration::from_secs(60 * 60);
/// Our own back-off never grows past this (a longer `Retry-After` is still
/// respected as it is).
pub const MAX_BACKOFF: Duration = Duration::from_secs(6 * 60 * 60);
/// Network / server errors: first pause, and the most it grows to.
pub const NET_FIRST: Duration = Duration::from_secs(15);
pub const NET_MAX: Duration = Duration::from_secs(5 * 60);

/// Who is asking: a background poll gives way to user actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Urgency {
    Poll,
    User,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    /// Spotify said 429 (rate limit).
    RateLimit,
    /// Spotify said 429 with `QUOTA_EXCEEDED`.
    Quota,
    /// No connection / timeouts / 5xx.
    Offline,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blocked {
    /// Everything waits until then.
    Paused { until: Instant, why: Why },
    /// Our own budget for this 30 s window is used up; try after `at`.
    Budget { at: Instant },
}

#[derive(Debug, Default)]
pub struct Limiter {
    calls: VecDeque<Instant>,
    paused: Option<(Instant, Why)>,
    /// 429s in a row (reset by a good answer).
    strikes: u32,
    /// Network / server failures in a row.
    net_fails: u32,
    /// Calls made in total (for the status line / tests).
    pub total: u64,
}

impl Limiter {
    pub fn new() -> Limiter {
        Limiter::default()
    }

    fn trim(&mut self, now: Instant) {
        while let Some(&t) = self.calls.front() {
            if now.saturating_duration_since(t) >= WINDOW {
                self.calls.pop_front();
            } else {
                break;
            }
        }
    }

    /// The pause in force, if any.
    pub fn paused(&self, now: Instant) -> Option<(Instant, Why)> {
        self.paused.filter(|(until, _)| *until > now)
    }

    /// May a call go out now? `Ok` reserves it (counts it in the window).
    pub fn begin(&mut self, now: Instant, urgency: Urgency) -> Result<(), Blocked> {
        if let Some((until, why)) = self.paused(now) {
            return Err(Blocked::Paused { until, why });
        }
        self.paused = None;
        self.trim(now);
        let cap = match urgency {
            Urgency::User => WINDOW_MAX,
            Urgency::Poll => WINDOW_MAX - RESERVED,
        };
        if self.calls.len() >= cap {
            // Free again when enough old calls leave the window.
            let idx = self.calls.len() + 1 - cap;
            let at = self.calls[idx - 1] + WINDOW;
            return Err(Blocked::Budget { at });
        }
        self.calls.push_back(now);
        self.total += 1;
        Ok(())
    }

    /// A good answer (any 2xx, or a 4xx that isn't about limits).
    pub fn ok(&mut self) {
        self.strikes = 0;
        self.net_fails = 0;
    }

    /// A 429. Returns the time until which everything pauses.
    pub fn too_many(&mut self, now: Instant, retry_after: Option<Duration>, quota: bool) -> Instant {
        self.strikes += 1;
        let base = if quota { DEFAULT_QUOTA } else { DEFAULT_429 };
        // Our own back-off: base × 2^(strikes−1), capped.
        let ours = base
            .checked_mul(1u32 << (self.strikes - 1).min(16))
            .unwrap_or(MAX_BACKOFF)
            .min(MAX_BACKOFF);
        let wait = match retry_after {
            // Never shorter than Spotify asks; on repeats, our back-off can
            // make it longer.
            Some(ra) if self.strikes == 1 => ra,
            Some(ra) => ra.max(ours),
            None => ours,
        };
        // A second of slack so we don't land exactly on the edge.
        let until = now + wait + Duration::from_secs(1);
        let why = if quota { Why::Quota } else { Why::RateLimit };
        self.paused = Some((until, why));
        until
    }

    /// No connection, a timeout or a 5xx.
    pub fn failed(&mut self, now: Instant) -> Instant {
        self.net_fails += 1;
        let wait = NET_FIRST
            .checked_mul(1u32 << (self.net_fails - 1).min(8))
            .unwrap_or(NET_MAX)
            .min(NET_MAX);
        let until = now + wait;
        // A rate-limit pause that lasts longer stays.
        match self.paused {
            Some((u, w)) if u > until && w != Why::Offline => {}
            _ => self.paused = Some((until, Why::Offline)),
        }
        until
    }

    /// Forget an offline pause (e.g. the user pressed "Try again"). A
    /// rate-limit pause can't be skipped.
    pub fn retry_now(&mut self, now: Instant) -> bool {
        match self.paused(now) {
            Some((_, Why::Offline)) => {
                self.paused = None;
                true
            }
            Some(_) => false,
            None => true,
        }
    }
}

/// `Retry-After` is seconds (Spotify) or, by the HTTP spec, a date; dates
/// are rare here, so anything unreadable counts as "not given".
pub fn parse_retry_after(v: Option<&str>) -> Option<Duration> {
    let v = v?.trim();
    let secs: f64 = v.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs.min(7.0 * 86400.0)))
}

/// Does a 429 body say the quota is used up?
pub fn is_quota(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/reason")
                .and_then(|r| r.as_str())
                .map(|r| r.eq_ignore_ascii_case("QUOTA_EXCEEDED"))
        })
        .unwrap_or_else(|| body.contains("QUOTA_EXCEEDED"))
}

/// "3 min", "1 h 20 min", "40 s".
pub fn fmt_wait(d: Duration) -> String {
    let s = d.as_secs_f64().ceil() as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s.div_ceil(60))
    } else {
        let m = s.div_ceil(60);
        if m % 60 == 0 {
            format!("{} h", m / 60)
        } else {
            format!("{} h {} min", m / 60, m % 60)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn budget_leaves_room_for_the_user() {
        let t0 = Instant::now();
        let mut l = Limiter::new();
        for i in 0..(WINDOW_MAX - RESERVED) {
            assert!(l.begin(t0 + s(i as u64), Urgency::Poll).is_ok());
        }
        // Polls are now refused, user actions still go.
        let t = t0 + s(10);
        match l.begin(t, Urgency::Poll) {
            Err(Blocked::Budget { at }) => assert!(at > t && at <= t0 + WINDOW + s(1)),
            other => panic!("{other:?}"),
        }
        for _ in 0..RESERVED {
            assert!(l.begin(t, Urgency::User).is_ok());
        }
        assert!(matches!(l.begin(t, Urgency::User), Err(Blocked::Budget { .. })));
        // Once the window has moved on, calls go again.
        assert!(l.begin(t0 + WINDOW + s(11), Urgency::Poll).is_ok());
    }

    #[test]
    fn retry_after_is_respected_and_nothing_goes_out_meanwhile() {
        let t0 = Instant::now();
        let mut l = Limiter::new();
        l.begin(t0, Urgency::Poll).unwrap();
        let until = l.too_many(t0, Some(s(120)), false);
        assert!(until >= t0 + s(120));
        for k in [1, 60, 119] {
            let e = l.begin(t0 + s(k), Urgency::User).unwrap_err();
            assert_eq!(
                e,
                Blocked::Paused {
                    until,
                    why: Why::RateLimit
                }
            );
        }
        assert_eq!(l.total, 1, "no call went out while paused");
        assert!(l.begin(until, Urgency::Poll).is_ok());
    }

    #[test]
    fn repeated_429s_back_off_more() {
        let t0 = Instant::now();
        let mut l = Limiter::new();
        let a = l.too_many(t0, None, false) - t0;
        let b = l.too_many(t0, None, false) - t0;
        let c = l.too_many(t0, Some(s(5)), false) - t0;
        assert!(a >= DEFAULT_429 && b >= 2 * DEFAULT_429 && c >= 4 * DEFAULT_429);
        // A very long Retry-After is kept as it is.
        let mut l = Limiter::new();
        let d = l.too_many(t0, Some(s(20 * 3600)), true) - t0;
        assert!(d >= s(20 * 3600));
        // A good answer resets the back-off.
        let mut l = Limiter::new();
        l.too_many(t0, None, false);
        l.too_many(t0, None, false);
        l.ok();
        assert!(l.too_many(t0, None, false) - t0 < 2 * DEFAULT_429);
    }

    #[test]
    fn quota_without_retry_after_waits_an_hour() {
        let t0 = Instant::now();
        let mut l = Limiter::new();
        let u = l.too_many(t0, None, true);
        assert!(u - t0 >= DEFAULT_QUOTA);
        assert_eq!(l.paused(t0).unwrap().1, Why::Quota);
        // "Try again" can't skip a quota pause.
        assert!(!l.retry_now(t0 + s(5)));
    }

    #[test]
    fn network_errors_double_up_to_a_cap() {
        let t0 = Instant::now();
        let mut l = Limiter::new();
        let waits: Vec<Duration> = (0..12).map(|_| l.failed(t0) - t0).collect();
        assert_eq!(waits[0], NET_FIRST);
        assert_eq!(waits[1], NET_FIRST * 2);
        assert!(waits.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(*waits.last().unwrap(), NET_MAX);
        // An offline pause can be skipped by the user.
        assert!(l.retry_now(t0 + s(1)));
        assert!(l.begin(t0 + s(1), Urgency::User).is_ok());
        // A network error doesn't shorten a rate-limit pause.
        let mut l = Limiter::new();
        let u = l.too_many(t0, Some(s(600)), false);
        l.failed(t0);
        assert_eq!(l.paused(t0).unwrap(), (u, Why::RateLimit));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_retry_after(Some("30")), Some(s(30)));
        assert_eq!(parse_retry_after(Some(" 2.5 ")), Some(Duration::from_millis(2500)));
        assert_eq!(parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")), None);
        assert_eq!(parse_retry_after(None), None);
        assert!(is_quota(
            r#"{"error":{"status":429,"message":"Too many requests","reason":"QUOTA_EXCEEDED"}}"#
        ));
        assert!(!is_quota(r#"{"error":{"status":429,"message":"Too many requests"}}"#));
        assert!(!is_quota(""));
        assert_eq!(fmt_wait(s(40)), "40 s");
        assert_eq!(fmt_wait(s(61)), "2 min");
        assert_eq!(fmt_wait(s(3600)), "1 h");
        assert_eq!(fmt_wait(s(4800)), "1 h 20 min");
    }
}
