//! Practice stats and streaks (Part 9).
//!
//! Pure logic plus one small JSON file
//! (`~/.local/share/qwertone/stats.json`). Practice time is *active* time:
//! the gaps between things you do (a note, a chord, a drum hit, a song
//! that's playing while you play along) count as long as they're shorter
//! than [`IDLE_GAP`], so leaving the app open doesn't inflate the numbers.
//!
//! A day counts towards the streak once it has [`STREAK_SECS`] of practice.
//! Today not counting *yet* doesn't break the streak: it runs up to
//! yesterday until midnight.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

/// A gap between two actions longer than this isn't practice (you put the
/// keyboard down, read something, made tea).
pub const IDLE_GAP: f64 = 30.0;
/// Time credited for the first action after a break.
pub const FIRST_TOUCH: f64 = 2.0;
/// A day needs this much practice to count for the streak.
pub const STREAK_SECS: f64 = 60.0;

/// A calendar day as days since 1970-01-01 (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day(pub i32);

impl Day {
    /// Howard Hinnant's `days_from_civil`.
    pub fn from_ymd(y: i32, m: u32, d: u32) -> Day {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = m as i32;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i32 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Day(era * 146_097 + doe - 719_468)
    }

    /// `civil_from_days`.
    pub fn ymd(self) -> (i32, u32, u32) {
        let z = self.0 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = yoe + era * 400 + i32::from(m <= 2);
        (y, m, d)
    }

    /// "2026-11-26".
    pub fn parse(s: &str) -> Option<Day> {
        let mut it = s.trim().splitn(3, '-');
        let y = it.next()?.parse().ok()?;
        let m: u32 = it.next()?.parse().ok()?;
        let d: u32 = it.next()?.parse().ok()?;
        ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| Day::from_ymd(y, m, d))
    }

    pub fn key(self) -> String {
        let (y, m, d) = self.ymd();
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// 0 = Monday … 6 = Sunday. 1970-01-01 was a Thursday.
    pub fn weekday(self) -> u32 {
        (self.0 + 3).rem_euclid(7) as u32
    }

    /// The Monday of this day's week.
    pub fn week_start(self) -> Day {
        Day(self.0 - self.weekday() as i32)
    }

    pub fn plus(self, days: i32) -> Day {
        Day(self.0 + days)
    }

    /// Today in the computer's time zone.
    pub fn today() -> Day {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Day((now + local_offset_secs(now)).div_euclid(86_400) as i32)
    }

    /// "Thu 26 Nov".
    pub fn short_label(self) -> String {
        const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let (_, m, d) = self.ymd();
        format!("{} {d} {}", WD[self.weekday() as usize], MONTHS[m as usize - 1])
    }
}

pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The local UTC offset at `t` (seconds since the epoch), from the C
/// library, which reads `TZ` / `/etc/localtime`.
fn local_offset_secs(t: i64) -> i64 {
    let tt = t as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r only writes into `tm`, which lives on our stack.
    let ok = unsafe { !libc::localtime_r(&tt, &mut tm).is_null() };
    if ok { tm.tm_gmtoff } else { 0 }
}

/// What you were doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// Free play (Play tab, and anything outside Learn / Song).
    Play,
    /// Lessons and ear training.
    Learn,
    /// Song mode (practising, jamming, playing along with Spotify).
    Song,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DayStats {
    /// Seconds of active practice per activity.
    pub play_s: f64,
    pub learn_s: f64,
    pub song_s: f64,
    /// Notes, chords and drum hits you played.
    pub notes: u32,
    /// Lessons finished for the first time.
    pub lessons: u32,
    /// Song run-throughs scored to the end.
    pub songs: u32,
    /// Ear-training questions answered right.
    pub ear: u32,
}

impl DayStats {
    pub fn total_s(&self) -> f64 {
        self.play_s + self.learn_s + self.song_s
    }

    pub fn counts(&self) -> bool {
        self.total_s() >= STREAK_SECS
    }

    fn add_time(&mut self, a: Activity, s: f64) {
        match a {
            Activity::Play => self.play_s += s,
            Activity::Learn => self.learn_s += s,
            Activity::Song => self.song_s += s,
        }
    }
}

/// One song you've practised (Song mode run-throughs scored to the end).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SongRecord {
    pub title: String,
    pub plays: u32,
    /// Best accuracy at any speed, in %.
    pub best: u32,
    /// Accuracy of the last run, in %.
    pub last: u32,
    /// "YYYY-MM-DD".
    pub last_day: String,
}

/// Everything in `stats.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub version: u32,
    /// "YYYY-MM-DD" → that day.
    pub days: BTreeMap<String, DayStats>,
    /// Song id (`Song::id`) → how it's going.
    pub songs: BTreeMap<String, SongRecord>,
}

/// Summary numbers for the Progress tab.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Streak {
    /// Days in a row up to today (or up to yesterday, if today hasn't
    /// counted yet).
    pub current: u32,
    pub best: u32,
    /// Today already counts.
    pub today_done: bool,
}

impl Stats {
    pub fn day(&self, d: Day) -> DayStats {
        self.days.get(&d.key()).cloned().unwrap_or_default()
    }

    fn day_mut(&mut self, d: Day) -> &mut DayStats {
        self.days.entry(d.key()).or_default()
    }

    fn counted_days(&self) -> Vec<Day> {
        let mut v: Vec<Day> = self
            .days
            .iter()
            .filter(|(_, s)| s.counts())
            .filter_map(|(k, _)| Day::parse(k))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn streak(&self, today: Day) -> Streak {
        let days = self.counted_days();
        let has = |d: Day| days.binary_search(&d).is_ok();
        let today_done = has(today);
        let mut current = 0;
        let mut d = if today_done { today } else { today.plus(-1) };
        while has(d) {
            current += 1;
            d = d.plus(-1);
        }
        let mut best = 0;
        let mut run = 0;
        let mut prev: Option<Day> = None;
        for &d in &days {
            run = if prev.is_some_and(|p| p.plus(1) == d) {
                run + 1
            } else {
                1
            };
            best = best.max(run);
            prev = Some(d);
        }
        Streak {
            current,
            best: best.max(current),
            today_done,
        }
    }

    /// Seconds practised from `from` to `to` (inclusive).
    pub fn total_between(&self, from: Day, to: Day) -> f64 {
        self.days
            .range(from.key()..=to.key())
            .map(|(_, s)| s.total_s())
            .sum()
    }

    pub fn totals(&self) -> DayStats {
        let mut t = DayStats::default();
        for s in self.days.values() {
            t.play_s += s.play_s;
            t.learn_s += s.learn_s;
            t.song_s += s.song_s;
            t.notes += s.notes;
            t.lessons += s.lessons;
            t.songs += s.songs;
            t.ear += s.ear;
        }
        t
    }

    pub fn days_practised(&self) -> usize {
        self.days.values().filter(|s| s.counts()).count()
    }
}

/// Counts practice as it happens and saves now and then.
pub struct Tracker {
    pub stats: Stats,
    path: Option<PathBuf>,
    last: Option<Instant>,
    dirty: bool,
    saved_at: Instant,
    /// Today, fixed by the caller in tests.
    today_override: Option<Day>,
}

impl Tracker {
    /// Loads `stats.json` (a damaged file is kept as `stats.json.bad` and
    /// counting starts over).
    pub fn load() -> Tracker {
        let path = stats_path();
        let stats = match path.as_ref().map(std::fs::read_to_string) {
            Some(Ok(text)) => serde_json::from_str(&text).unwrap_or_else(|_| {
                if let Some(p) = &path {
                    let _ = std::fs::rename(p, p.with_extension("json.bad"));
                }
                Stats::default()
            }),
            _ => Stats::default(),
        };
        Tracker::new(stats, path)
    }

    pub fn new(stats: Stats, path: Option<PathBuf>) -> Tracker {
        Tracker {
            stats,
            path,
            last: None,
            dirty: false,
            saved_at: Instant::now(),
            today_override: None,
        }
    }

    pub fn today(&self) -> Day {
        self.today_override.unwrap_or_else(Day::today)
    }

    #[cfg(test)]
    pub fn set_today(&mut self, d: Day) {
        self.today_override = Some(d);
    }

    /// Something happened at `now`: credit the time since the last thing
    /// (if it wasn't a break) to `a`.
    fn ping(&mut self, a: Activity, now: Instant) {
        let gap = self
            .last
            .map(|l| now.saturating_duration_since(l).as_secs_f64());
        let credit = match gap {
            Some(g) if g <= IDLE_GAP => g,
            _ => FIRST_TOUCH,
        };
        // Skip tiny gaps (fast trills, frame-rate callers): the time isn't
        // lost, the next ping credits it.
        if gap.is_some_and(|g| g < 0.25) {
            return;
        }
        let today = self.today();
        self.stats.day_mut(today).add_time(a, credit);
        self.last = Some(now);
        self.dirty = true;
    }

    /// A note / chord / drum hit you played.
    pub fn note(&mut self, a: Activity, now: Instant) {
        let today = self.today();
        self.stats.day_mut(today).notes += 1;
        self.dirty = true;
        self.ping(a, now);
    }

    pub fn lesson_done(&mut self) {
        let today = self.today();
        self.stats.day_mut(today).lessons += 1;
        self.dirty = true;
    }

    /// A song run scored to the end: `accuracy` in %.
    pub fn song_done(&mut self, id: &str, title: &str, accuracy: u32) {
        let today = self.today();
        self.stats.day_mut(today).songs += 1;
        let r = self.stats.songs.entry(id.to_string()).or_default();
        r.title = title.to_string();
        r.plays += 1;
        r.best = r.best.max(accuracy);
        r.last = accuracy;
        r.last_day = today.key();
        self.dirty = true;
    }

    pub fn ear_right(&mut self) {
        let today = self.today();
        self.stats.day_mut(today).ear += 1;
        self.dirty = true;
    }

    pub fn streak(&self) -> Streak {
        self.stats.streak(self.today())
    }

    pub fn today_stats(&self) -> DayStats {
        self.stats.day(self.today())
    }

    /// Save if something changed and the last save was a while ago (or
    /// `force`, on exit).
    pub fn maybe_save(&mut self, force: bool) -> std::io::Result<()> {
        if !self.dirty || (!force && self.saved_at.elapsed().as_secs() < 30) {
            return Ok(());
        }
        self.dirty = false;
        self.saved_at = Instant::now();
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.stats)?)?;
        std::fs::rename(tmp, path)
    }

    /// Forget everything (the Progress tab's reset, two clicks).
    pub fn reset(&mut self) {
        // `last` too, so the next note starts a fresh stretch.
        self.stats = Stats::default();
        self.last = None;
        self.dirty = true;
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }
}

pub fn stats_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("qwertone").join("stats.json"))
}

/// "1 h 05 min", "12 min", "40 s".
pub fn fmt_duration(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn civil_dates_round_trip() {
        assert_eq!(Day::from_ymd(1970, 1, 1), Day(0));
        assert_eq!(Day::from_ymd(2000, 3, 1).ymd(), (2000, 3, 1));
        assert_eq!(Day::from_ymd(2024, 2, 29).plus(1).ymd(), (2024, 3, 1));
        assert_eq!(Day::from_ymd(2026, 12, 31).plus(1).key(), "2027-01-01");
        for n in -1000..40_000 {
            let (y, m, d) = Day(n).ymd();
            assert_eq!(Day::from_ymd(y, m, d), Day(n));
            assert_eq!(Day::parse(&Day(n).key()), Some(Day(n)));
        }
        assert_eq!(Day::parse("2026-13-01"), None);
        assert_eq!(Day::parse("hello"), None);
    }

    #[test]
    fn weekdays() {
        assert_eq!(Day(0).weekday(), 3); // Thursday
        let d = Day::from_ymd(2026, 11, 26); // a Thursday
        assert_eq!(d.weekday(), 3);
        assert_eq!(d.week_start().key(), "2026-11-23");
        assert_eq!(d.short_label(), "Thu 26 Nov");
    }

    fn day_with(secs: f64) -> DayStats {
        DayStats {
            play_s: secs,
            ..Default::default()
        }
    }

    #[test]
    fn streaks() {
        let today = Day::from_ymd(2026, 11, 26);
        let mut s = Stats::default();
        assert_eq!(s.streak(today), Streak::default());
        // Three days up to yesterday; today not yet.
        for i in 1..=3 {
            s.days.insert(today.plus(-i).key(), day_with(120.0));
        }
        let st = s.streak(today);
        assert_eq!((st.current, st.best, st.today_done), (3, 3, false));
        // Today with too little practice doesn't count yet, nor break it.
        s.days.insert(today.key(), day_with(30.0));
        assert_eq!(s.streak(today).current, 3);
        s.days.insert(today.key(), day_with(61.0));
        let st = s.streak(today);
        assert_eq!((st.current, st.today_done), (4, true));
        // A gap two days ago in the past: an older, longer run is the best.
        for i in 10..=16 {
            s.days.insert(today.plus(-i).key(), day_with(300.0));
        }
        assert_eq!(s.streak(today).best, 7);
        // Missing yesterday and today: the streak is over.
        let st = s.streak(today.plus(2));
        assert_eq!(st.current, 0);
        assert_eq!(st.best, 7);
    }

    #[test]
    fn active_time_ignores_breaks() {
        let mut t = Tracker::new(Stats::default(), None);
        let today = Day::from_ymd(2026, 11, 26);
        t.set_today(today);
        let t0 = Instant::now();
        t.note(Activity::Play, t0); // first touch: 2 s
        t.note(Activity::Play, t0 + Duration::from_secs(10)); // +10
        t.note(Activity::Play, t0 + Duration::from_secs(20)); // +10
        // A five-minute break: only a first touch again.
        t.note(Activity::Learn, t0 + Duration::from_secs(320));
        t.note(Activity::Learn, t0 + Duration::from_secs(325)); // +5
        let d = t.today_stats();
        assert_eq!(d.notes, 5);
        assert!((d.play_s - 22.0).abs() < 1e-9, "{}", d.play_s);
        assert!((d.learn_s - 7.0).abs() < 1e-9, "{}", d.learn_s);
        // Frame-rate pings don't add up to more than real time.
        let mut at = t0 + Duration::from_secs(400);
        t.ping(Activity::Song, at);
        for _ in 0..600 {
            at += Duration::from_millis(16);
            t.ping(Activity::Song, at);
        }
        let s = t.today_stats().song_s;
        assert!((s - (2.0 + 9.6)).abs() < 0.3, "{s}");
    }

    #[test]
    fn counts_lessons_songs_and_ear() {
        let mut t = Tracker::new(Stats::default(), None);
        t.set_today(Day::from_ymd(2026, 1, 1));
        t.lesson_done();
        t.song_done("abc:1", "Song", 70);
        t.song_done("abc:1", "Song", 55);
        t.ear_right();
        t.ear_right();
        let d = t.today_stats();
        assert_eq!((d.lessons, d.songs, d.ear), (1, 2, 2));
        let r = &t.stats.songs["abc:1"];
        assert_eq!((r.plays, r.best, r.last), (2, 70, 55));
        assert_eq!(r.last_day, "2026-01-01");
        assert_eq!(t.stats.totals().ear, 2);
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("qwertone-stats-{}", std::process::id()));
        let path = dir.join("stats.json");
        let mut t = Tracker::new(Stats::default(), Some(path.clone()));
        t.set_today(Day::from_ymd(2026, 11, 26));
        t.note(Activity::Song, Instant::now());
        t.maybe_save(true).unwrap();
        let back: Stats = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, t.stats);
        assert_eq!(back.days["2026-11-26"].notes, 1);
        // Nothing new: no write.
        std::fs::remove_file(&path).unwrap();
        t.maybe_save(true).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn totals_between_days() {
        let mut s = Stats::default();
        let d = Day::from_ymd(2026, 11, 23);
        for i in 0..7 {
            s.days.insert(d.plus(i).key(), day_with(60.0));
        }
        assert_eq!(s.total_between(d, d.plus(2)), 180.0);
        assert_eq!(s.days_practised(), 7);
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(40.0), "40 s");
        assert_eq!(fmt_duration(12.0 * 60.0 + 5.0), "12 min");
        assert_eq!(fmt_duration(3900.0), "1 h 05 min");
    }
}
