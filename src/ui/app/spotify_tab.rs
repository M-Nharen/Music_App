//! The Spotify tab (Part 8): log in, search, now playing with synced
//! lyrics, "Learn this", and sync mode (the falling notes and lyrics of the
//! analysed song follow Spotify's playback position).
//!
//! API calls are few on purpose (Development Mode has small, undocumented
//! limits and a quota): the playback state is asked for only while this tab
//! or sync mode is on screen, every `poll_secs` (default 12 s) plus once
//! right after a play / pause / seek from here; in between the position is
//! estimated. Searches and tracks are cached on disk. On a 429 every call
//! waits for Retry-After (longer when it repeats), a "Spotify paused for N
//! min" chip shows in the header, and everything keeps working on the
//! local estimate and the caches meanwhile.

use super::*;
use crate::config::SpotifyLink;
use crate::song::analyze::{self, Job};
use crate::song::library::Entry;
use crate::song::lyrics::{Layer, Layers, Lyrics};
use crate::spotify::api::{Device, Playback, Track};
use crate::spotify::cache::Cache;
use crate::spotify::limit::{Why, fmt_wait};
use crate::spotify::sync::{Estimate, Poller};
use crate::spotify::{self, Client, Ev, Login, Req, auth, matching, store};
use egui::{Sense, vec2};

/// The song that follows Spotify.
#[derive(Clone, Debug)]
pub(super) struct SyncLink {
    pub track_id: String,
    pub track_name: String,
}

/// What "Learn this" found for a track.
#[derive(Clone, Debug)]
enum Learn {
    /// Pick from these analysed songs (score) or local audio files.
    Choose {
        track: Track,
        songs: Vec<(Entry, f64)>,
        files: Vec<(PathBuf, f64)>,
    },
}

pub(super) struct SpotifyUi {
    started: bool,
    client: Option<Client>,
    logged_in: bool,
    stored: Option<store::Where>,
    login: Option<Login>,
    login_error: Option<String>,
    client_id_input: String,
    cache: Cache,
    cache_saved: Instant,
    poller: Poller,
    /// Where Spotify is (estimated between polls).
    est: Option<Estimate>,
    pb: Option<Playback>,
    /// Calls are paused (429 / quota / offline) until then.
    paused: Option<(Instant, Why)>,
    note: Option<(String, Instant)>,
    calls: u64,
    // Search.
    query: String,
    results: Vec<Track>,
    results_for: String,
    results_note: Option<String>,
    searching: Option<String>,
    // Devices (when Spotify says no device is active).
    devices: Option<Vec<Device>>,
    pending_play: Option<Req>,
    // Learn this.
    learn: Option<Learn>,
    pending_link: Option<(Track, PathBuf)>,
    pick: Option<(Track, Receiver<Option<PathBuf>>)>,
    // Lyrics of what plays.
    lyrics: Option<(String, Option<Lyrics>)>,
    lyrics_job: Option<(String, Job)>,
    lyrics_direct: Option<(String, Receiver<Result<Lyrics, String>>)>,
    lyrics_failed: Option<(String, String)>,
    /// Song folder whose lyrics are being re-made with Spotify's metadata.
    relyrics: Option<(PathBuf, Job)>,
    // Sync mode.
    pub sync: Option<SyncLink>,
    last_correction: Instant,
    pub setup_open: bool,
}

impl SpotifyUi {
    pub fn new(cfg: &crate::config::SpotifySettings) -> SpotifyUi {
        SpotifyUi {
            started: false,
            client: None,
            logged_in: false,
            stored: None,
            login: None,
            login_error: None,
            client_id_input: cfg.client_id.clone(),
            cache: Cache::default(),
            cache_saved: Instant::now(),
            poller: Poller::new(cfg.poll_secs),
            est: None,
            pb: None,
            paused: None,
            note: None,
            calls: 0,
            query: String::new(),
            results: Vec::new(),
            results_for: String::new(),
            results_note: None,
            searching: None,
            devices: None,
            pending_play: None,
            learn: None,
            pending_link: None,
            pick: None,
            lyrics: None,
            lyrics_job: None,
            lyrics_direct: None,
            lyrics_failed: None,
            relyrics: None,
            sync: None,
            last_correction: Instant::now(),
            setup_open: false,
        }
    }
}

fn fmt_ms(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn track_id(t: &Track) -> String {
    t.id.clone().unwrap_or_else(|| t.uri.clone())
}

impl QwertoneApp {
    // ------------------------------------------------------------ state

    /// Following Spotify with the open song.
    pub(super) fn syncing(&self) -> bool {
        self.spotify.sync.is_some() && self.song_ui.song.is_some()
    }

    fn sp_paused(&self) -> Option<(Instant, Why)> {
        self.spotify
            .paused
            .filter(|(until, _)| *until > Instant::now())
    }

    /// The header chip: "Spotify paused for 4 min".
    pub(super) fn spotify_header_chip(&self) -> Option<(String, bool)> {
        let (until, why) = self.sp_paused()?;
        let left = fmt_wait(until.saturating_duration_since(Instant::now()));
        Some(match why {
            Why::Quota => (format!("Spotify paused for {left} (quota)"), true),
            Why::RateLimit => (format!("Spotify paused for {left}"), true),
            Why::Offline => (format!("Spotify offline · retry in {left}"), false),
        })
    }

    fn sp_note(&mut self, msg: impl Into<String>) {
        self.spotify.note = Some((msg.into(), Instant::now()));
    }

    fn sp_send(&mut self, r: Req) {
        if let Some(c) = &self.spotify.client {
            c.send(r);
        }
    }

    /// Load the saved login (once, when first needed).
    fn spotify_start(&mut self, ctx: &egui::Context) {
        if self.spotify.started {
            return;
        }
        self.spotify.started = true;
        self.spotify.cache = Cache::load();
        let c = ctx.clone();
        let client = Client::start(self.cfg.spotify.token_store.clone(), move || {
            c.request_repaint()
        });
        self.spotify.client = Some(client);
        if self.cfg.spotify.client_id.is_empty() {
            return;
        }
        if let Some((t, w)) = store::load(&self.cfg.spotify.token_store)
            && (t.client_id.is_empty() || t.client_id == self.cfg.spotify.client_id)
        {
            self.spotify.stored = Some(w);
            self.spotify.logged_in = true;
            let pref = self.cfg.spotify.token_store.clone();
            self.sp_send(Req::SetLogin(Box::new(t), pref));
        }
    }

    fn spotify_visible(&self, ctx: &egui::Context) -> bool {
        if ctx.input(|i| i.viewport().minimized.unwrap_or(false)) {
            return false;
        }
        let on_tab = self.tab == Tab::Spotify;
        let sync = self.tab == Tab::Song
            && self.syncing()
            && matches!(
                self.song_ui.view,
                song::SongView::Practice | song::SongView::Lyrics
            );
        self.spotify.logged_in && (on_tab || sync)
    }

    // ------------------------------------------------------------ polling

    /// Every frame. Returns true while something moves on screen.
    pub(super) fn spotify_poll(&mut self, ctx: &egui::Context) -> bool {
        if self.tab == Tab::Spotify || self.spotify.sync.is_some() {
            self.spotify_start(ctx);
        }
        if !self.spotify.started {
            return false;
        }
        let now = Instant::now();
        // Login finished?
        if let Some(l) = &self.spotify.login
            && let Ok(res) = l.rx.try_recv()
        {
            self.spotify.login = None;
            match res {
                Ok(t) => {
                    let pref = self.cfg.spotify.token_store.clone();
                    match store::save(&pref, &t) {
                        Ok(w) => self.spotify.stored = Some(w),
                        Err(e) => {
                            self.spotify.stored = None;
                            self.sp_note(format!("Logged in, but the login couldn't be saved ({e}); you'll log in again next time."));
                        }
                    }
                    self.spotify.logged_in = true;
                    self.spotify.login_error = None;
                    self.sp_send(Req::SetLogin(Box::new(t), pref));
                    self.notify("Logged in to Spotify.");
                    self.spotify.poller = Poller::new(self.cfg.spotify.poll_secs);
                }
                Err(e) => self.spotify.login_error = Some(e),
            }
        }
        // Answers from the worker.
        let evs: Vec<Ev> = match &self.spotify.client {
            Some(c) => c.rx.try_iter().collect(),
            None => Vec::new(),
        };
        for e in evs {
            self.spotify_event(e);
        }
        // Poll the playback state, only while it's on screen.
        let visible = self.spotify_visible(ctx);
        self.spotify.poller.set_interval(self.cfg.spotify.poll_secs);
        if self.spotify.poller.due(now, visible) {
            if let Some((until, _)) = self.sp_paused() {
                // Paused: don't even ask; the next poll is after the pause.
                self.spotify.poller.wait_until(until);
            } else {
                self.sp_send(Req::Poll);
                self.spotify.poller.sent(now);
            }
        }
        self.spotify_lyrics_poll();
        // Picked a file for "Learn this".
        if let Some((track, rx)) = &self.spotify.pick
            && let Ok(res) = rx.try_recv()
        {
            let track = track.clone();
            self.spotify.pick = None;
            if let Some(p) = res {
                self.spotify_use_file(&track, &p);
            }
        }
        if self.spotify.cache.dirty && self.spotify.cache_saved.elapsed() > Duration::from_secs(5) {
            self.spotify.cache.save();
            self.spotify.cache_saved = Instant::now();
        }
        let playing = self.spotify.est.as_ref().is_some_and(|e| e.playing);
        visible && playing || self.spotify.login.is_some() || self.spotify.lyrics_job.is_some()
    }

    fn spotify_event(&mut self, e: Ev) {
        let now = Instant::now();
        match e {
            Ev::Saved(Ok(w)) => self.spotify.stored = Some(w),
            Ev::Saved(Err(e)) => {
                self.sp_note(format!("Couldn't save the refreshed login: {e}"));
            }
            Ev::LoggedOut(msg) => {
                self.spotify.logged_in = false;
                self.spotify.stored = None;
                self.spotify.login_error = Some(msg);
            }
            Ev::Search { q, tracks } => {
                let unix = auth::unix_now();
                self.spotify.cache.put_search(&q, tracks.clone(), unix);
                if self.spotify.searching.as_deref() == Some(q.as_str()) {
                    self.spotify.searching = None;
                    self.spotify.results = tracks;
                    self.spotify.results_for = q;
                    self.spotify.results_note = None;
                }
            }
            Ev::Playback { pb, sent, received } => {
                self.spotify.paused = self.sp_paused();
                let old_id = self.spotify.est.as_ref().and_then(|e| e.track_id.clone());
                match &pb {
                    Some(p) => {
                        let tr = p.track.clone();
                        let est = Estimate::from_sample(
                            tr.as_ref().map(track_id),
                            p.progress_ms,
                            p.playing,
                            tr.as_ref().map(|t| t.duration_ms).unwrap_or(0),
                            sent,
                            received,
                        );
                        if let Some(end) = est.ends_at() {
                            self.spotify.poller.at_track_end(end);
                        }
                        self.spotify.poller.idle = false;
                        self.spotify.est = Some(est);
                        if let Some(t) = tr {
                            self.spotify.cache.put_track(t, auth::unix_now());
                        }
                    }
                    None => {
                        self.spotify.est = None;
                        self.spotify.poller.idle = true;
                    }
                }
                self.spotify.pb = pb;
                let new_id = self.spotify.est.as_ref().and_then(|e| e.track_id.clone());
                if new_id != old_id {
                    self.spotify_track_changed();
                }
            }
            Ev::Devices(d) => self.spotify.devices = Some(d),
            Ev::Done(what) => {
                if what == "transfer" {
                    self.sp_note("Playback moved.");
                }
                // Check it worked, shortly (one poll, not a loop).
                self.spotify.poller.after_control(now);
            }
            Ev::NoDevice => {
                self.sp_note("Spotify isn't open on any device. Open Spotify (app or web player), or pick a device:");
                self.sp_send(Req::Devices);
            }
            Ev::Paused { until, why } => {
                self.spotify.paused = Some((until, why));
                self.spotify.poller.wait_until(until);
                self.spotify.searching = None;
                if why != Why::Offline {
                    let left = fmt_wait(until.saturating_duration_since(now));
                    self.notify(format!(
                        "Spotify asked us to slow down: paused for {left}. Everything else keeps working."
                    ));
                }
            }
            Ev::Busy { what, at } => {
                if what == "poll" {
                    self.spotify.poller.wait_until(at);
                } else {
                    self.spotify.searching = None;
                    let left = fmt_wait(at.saturating_duration_since(now));
                    self.sp_note(format!(
                        "Too many Spotify requests in a short time: {what} skipped, try again in {left}."
                    ));
                }
            }
            Ev::Error(e) => {
                self.spotify.searching = None;
                self.sp_note(e);
            }
            Ev::Calls(n) => self.spotify.calls = n,
        }
    }

    /// The track changed: lyrics, and stop following if it's another song.
    fn spotify_track_changed(&mut self) {
        let track = self.spotify.pb.as_ref().and_then(|p| p.track.clone());
        if let Some(t) = &track {
            self.spotify_lyrics_for(t.clone());
        } else {
            self.spotify.lyrics = None;
        }
        if let Some(s) = &self.spotify.sync {
            let now_id = track.as_ref().map(track_id);
            if now_id.as_deref() != Some(s.track_id.as_str()) {
                // Spotify moved on: if the new track is linked, follow it.
                let next = track
                    .as_ref()
                    .and_then(|t| t.id.clone())
                    .and_then(|id| self.cfg.spotify.links.get(&id).cloned().map(|l| (id, l)));
                match (next, track) {
                    (Some((_, link)), Some(t)) if link.song.exists() && self.tab == Tab::Song => {
                        self.spotify_start_sync(&t, &link);
                    }
                    (_, t) => {
                        if self.song_ui.song.is_some() && self.song_clock_now().1 {
                            self.song_play_pause_local();
                        }
                        let name = t.map(|t| t.name).unwrap_or_else(|| "nothing".into());
                        self.notify(format!(
                            "Spotify is playing {name} now, not this song: the notes wait."
                        ));
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ controls

    fn spotify_ready(&mut self, what: &str) -> bool {
        if !self.spotify.logged_in {
            self.notify("Log in to Spotify first (Spotify tab).");
            return false;
        }
        if let Some((until, why)) = self.sp_paused() {
            let left = fmt_wait(until.saturating_duration_since(Instant::now()));
            self.notify(match why {
                Why::Offline => format!("Can't reach Spotify (retry in {left}). Use the Spotify app to {what}."),
                _ => format!("Spotify is paused for {left} (too many requests). Use the Spotify app to {what} meanwhile."),
            });
            return false;
        }
        true
    }

    /// Play / pause Spotify.
    pub(super) fn spotify_toggle(&mut self) {
        let playing = self.spotify.est.as_ref().is_some_and(|e| e.playing);
        if !self.spotify_ready(if playing { "pause" } else { "play" }) {
            return;
        }
        let now = Instant::now();
        if playing {
            self.sp_send(Req::Pause);
        } else {
            self.sp_send(Req::Play {
                uri: None,
                position_ms: None,
                device: None,
            });
            self.spotify.pending_play = Some(Req::Play {
                uri: None,
                position_ms: None,
                device: None,
            });
        }
        if let Some(e) = &mut self.spotify.est {
            e.set_playing(!playing, now);
        }
    }

    pub(super) fn spotify_seek_ms(&mut self, ms: f64) {
        if !self.spotify_ready("seek") {
            return;
        }
        let ms = ms.max(0.0);
        self.sp_send(Req::Seek(ms as u64));
        if let Some(e) = &mut self.spotify.est {
            e.seek(ms, Instant::now());
        }
    }

    /// Seek Spotify so the song (sync mode) lands at song time `t`.
    pub(super) fn spotify_seek_song(&mut self, t: f64) {
        let off = self.spotify_offset_s();
        self.spotify_seek_ms((t - off) * 1000.0);
    }

    fn spotify_play_track(&mut self, t: &Track, device: Option<String>) {
        if !self.spotify_ready("play it") {
            return;
        }
        let r = Req::Play {
            uri: Some(t.uri.clone()),
            position_ms: None,
            device,
        };
        self.spotify.pending_play = Some(r.clone());
        self.sp_send(r);
        // Assume it worked until the check right after.
        let now = Instant::now();
        self.spotify.est = Some(Estimate::from_sample(
            Some(track_id(t)),
            0,
            true,
            t.duration_ms,
            now,
            now,
        ));
        let mut pb = self.spotify.pb.clone().unwrap_or_default();
        pb.track = Some(t.clone());
        pb.playing = true;
        pb.progress_ms = 0;
        self.spotify.pb = Some(pb);
        self.spotify_track_changed();
    }

    fn spotify_search(&mut self) {
        let q = self.spotify.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        let unix = auth::unix_now();
        let paused = self.sp_paused().is_some();
        if let Some(hit) = self.spotify.cache.search(&q, unix, paused) {
            self.spotify.results = hit.tracks.clone();
            self.spotify.results_for = q;
            self.spotify.results_note = paused.then(|| "Saved results (Spotify is paused).".into());
            return;
        }
        if paused || !self.spotify.logged_in {
            self.spotify.results = self.spotify.cache.offline_search(&q, 20);
            self.spotify.results_for = q;
            self.spotify.results_note = Some(if paused {
                "Spotify is paused: these are songs seen before.".into()
            } else {
                "Not logged in: these are songs seen before.".into()
            });
            return;
        }
        self.spotify.searching = Some(q.clone());
        self.sp_send(Req::Search(q));
    }

    // ------------------------------------------------------------ learn this

    /// The link for a track, if its song still exists.
    fn spotify_link(&mut self, t: &Track) -> Option<SpotifyLink> {
        let id = t.id.as_ref()?;
        let l = self.cfg.spotify.links.get(id)?.clone();
        if l.song.exists() {
            Some(l)
        } else {
            self.cfg.spotify.links.remove(id);
            self.mark_dirty();
            None
        }
    }

    fn spotify_make_link(&mut self, t: &Track, song: &std::path::Path) -> Option<SpotifyLink> {
        let id = t.id.clone()?;
        let link = SpotifyLink {
            song: song.to_path_buf(),
            offset_ms: self
                .cfg
                .spotify
                .links
                .get(&id)
                .map(|l| l.offset_ms)
                .unwrap_or(0.0),
            title: format!("{} — {}", t.name, t.artist_line()),
        };
        self.cfg.spotify.links.insert(id, link.clone());
        self.mark_dirty();
        Some(link)
    }

    /// "Learn this": open the practice version, or say how to make one.
    fn spotify_learn(&mut self, t: &Track) {
        if let Some(link) = self.spotify_link(t) {
            self.spotify_open_practice(t, &link);
            return;
        }
        let lib = self.song_library_entries();
        let mut songs: Vec<(Entry, f64)> = lib
            .into_iter()
            .filter(|e| e.error.is_none())
            .map(|e| {
                let s = matching::score(
                    &t.name,
                    &t.artists,
                    t.duration_s(),
                    &e.title,
                    e.artist.as_deref(),
                    e.duration,
                );
                (e, s)
            })
            .filter(|(_, s)| *s >= matching::MAYBE)
            .collect();
        songs.sort_by(|a, b| b.1.total_cmp(&a.1));
        if let Some((e, s)) = songs.first()
            && *s >= matching::SURE
            && songs.get(1).is_none_or(|x| x.1 < matching::SURE)
        {
            let path = e.path.clone();
            if let Some(link) = self.spotify_make_link(t, &path) {
                self.notify(format!("Found it in your songs: {}", e.title));
                self.spotify_open_practice(t, &link);
                return;
            }
        }
        let mut dirs: Vec<PathBuf> = [dirs::audio_dir(), dirs::download_dir()]
            .into_iter()
            .flatten()
            .collect();
        dirs.dedup();
        let mut files: Vec<(PathBuf, f64)> = matching::audio_files(&dirs, 4, 5000)
            .into_iter()
            .map(|p| {
                let s = matching::file_score(&t.name, &t.artists, &p);
                (p, s)
            })
            .filter(|(_, s)| *s >= matching::MAYBE)
            .collect();
        files.sort_by(|a, b| b.1.total_cmp(&a.1));
        files.truncate(6);
        songs.truncate(4);
        self.spotify.learn = Some(Learn::Choose {
            track: t.clone(),
            songs,
            files,
        });
    }

    fn spotify_open_practice(&mut self, t: &Track, link: &SpotifyLink) {
        let playing_this = self
            .spotify
            .est
            .as_ref()
            .and_then(|e| e.track_id.clone())
            .is_some_and(|id| id == track_id(t));
        self.spotify.learn = None;
        if playing_this {
            self.spotify_start_sync(t, link);
        } else {
            self.open_song(&link.song);
        }
        self.tab = Tab::Song;
        self.spotify_maybe_relyrics(t, &link.song);
    }

    /// Open the linked song and follow Spotify with it.
    fn spotify_start_sync(&mut self, t: &Track, link: &SpotifyLink) {
        let open_now = self.song_open_path();
        let same = open_now.as_ref().is_some_and(|p| {
            p == &link.song
                || p.parent() == Some(link.song.as_path())
                || link.song.parent() == Some(p.as_path())
        });
        if !same {
            self.open_song(&link.song);
        }
        if self.song_ui.song.is_none() {
            return;
        }
        self.spotify.sync = Some(SyncLink {
            track_id: track_id(t),
            track_name: t.name.clone(),
        });
        // Wait mode etc. depend on it: new targets.
        self.song_rebuild(false);
        self.song_ui.view = song::SongView::Practice;
        self.spotify.last_correction = Instant::now() - Duration::from_secs(5);
        if let Some(d) = self.song_ui.song.as_ref().map(|s| s.duration)
            && (d - t.duration_s()).abs() > 3.0
        {
            self.notify(format!(
                "Following Spotify. Your file is {:.0} s {} than Spotify's version: if the notes drift, use the offset.",
                (d - t.duration_s()).abs(),
                if d > t.duration_s() { "longer" } else { "shorter" }
            ));
        } else {
            self.notify("Following Spotify: the notes and lyrics move with Spotify.");
        }
        // Ask where Spotify is right away (if a poll is due anyway).
        self.spotify.poller.after_control(Instant::now());
    }

    /// Stop following (button).
    pub(super) fn spotify_stop_sync(&mut self) {
        if self.spotify.sync.take().is_some() {
            if self.song_clock_now().1 {
                self.song_play_pause_local();
            }
            self.song_rebuild(false);
            self.notify("Stopped following Spotify: practise at your own speed.");
        }
    }

    /// Opening another song by hand: stop silently.
    pub(super) fn spotify_stop_sync_quiet(&mut self) {
        self.spotify.sync = None;
    }

    /// Analysis finished: link it to the track it was started for.
    pub(super) fn spotify_on_analyzed(&mut self, file: &std::path::Path, dir: &std::path::Path) {
        let Some((t, f)) = self.spotify.pending_link.clone() else {
            return;
        };
        if f != file {
            return;
        }
        self.spotify.pending_link = None;
        if let Some(link) = self.spotify_make_link(&t, dir) {
            self.tab = Tab::Song;
            let playing_this = self
                .spotify
                .est
                .as_ref()
                .and_then(|e| e.track_id.clone())
                .is_some_and(|id| id == track_id(&t));
            if playing_this {
                self.spotify_start_sync(&t, &link);
            }
            self.spotify_maybe_relyrics(&t, dir);
        }
    }

    /// A file was picked / clicked for "Learn this".
    fn spotify_use_file(&mut self, t: &Track, p: &std::path::Path) {
        if crate::song::library::is_midi(p)
            || p.join("song.json").exists()
            || p.ends_with("song.json")
        {
            let target = if p.ends_with("song.json") {
                p.parent().unwrap_or(p).to_path_buf()
            } else {
                p.to_path_buf()
            };
            if let Some(link) = self.spotify_make_link(t, &target) {
                self.spotify_open_practice(t, &link);
            }
            return;
        }
        match self.song_start_analysis(p) {
            Ok(()) => {
                self.spotify.pending_link = Some((t.clone(), p.to_path_buf()));
                self.notify(
                    "Analysing… it opens by itself when done (the Song tab shows the details).",
                );
            }
            Err(e) => self.notify(e),
        }
    }

    fn spotify_pick_file(&mut self, t: &Track) {
        let (tx, rx) = unbounded();
        let start = dirs::audio_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        std::thread::spawn(move || {
            let try_run = |cmd: &str, args: &[&str]| -> Option<Option<PathBuf>> {
                let out = std::process::Command::new(cmd).args(args).output().ok()?;
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                Some((out.status.success() && !s.is_empty()).then(|| PathBuf::from(s)))
            };
            let start_s = format!("{}/", start.display());
            let res = try_run(
                "zenity",
                &[
                    "--file-selection",
                    "--title=Your copy of this song (audio or MIDI)",
                    "--file-filter=Audio / MIDI | *.mp3 *.flac *.wav *.m4a *.ogg *.opus *.aac *.mid *.midi",
                    "--file-filter=All files | *",
                    &format!("--filename={start_s}"),
                ],
            )
            .or_else(|| try_run("kdialog", &["--getopenfilename", &start_s]))
            .or_else(|| try_run("yad", &["--file", "--title=Your copy of this song"]));
            let _ = tx.send(res.flatten());
        });
        self.spotify.pick = Some((t.clone(), rx));
    }

    /// An analysed song without lyrics: look them up with Spotify's exact
    /// title / artist / album (better than file tags).
    fn spotify_maybe_relyrics(&mut self, t: &Track, dir: &std::path::Path) {
        if !dir.join("song.json").exists() || self.spotify.relyrics.is_some() {
            return;
        }
        let status = Lyrics::load(dir).ok().flatten().map(|d| d.status);
        if !matches!(status.as_deref(), None | Some("not_found")) {
            return;
        }
        let Some(program) = analyze::find_analyzer() else {
            return;
        };
        let artist = t.artists.first().cloned().unwrap_or_default();
        let args = [
            "--title",
            t.name.as_str(),
            "--artist",
            artist.as_str(),
            "--album",
            t.album.as_str(),
            "--force",
        ];
        if let Ok(j) = Job::lyrics(&program, dir, &args) {
            self.spotify.relyrics = Some((dir.to_path_buf(), j));
        }
    }

    // ------------------------------------------------------------ lyrics

    /// Lyrics for what Spotify plays: the linked song's own, else a lookup
    /// by Spotify's metadata (cached per track).
    fn spotify_lyrics_for(&mut self, t: Track) {
        let id = track_id(&t);
        if self.spotify.lyrics.as_ref().is_some_and(|(i, _)| *i == id) {
            return;
        }
        if let Some(link) = self.spotify_link(&t)
            && let Ok(Some(doc)) = Lyrics::load(&link.song)
            && !doc.lines.is_empty()
        {
            self.spotify.lyrics = Some((id, Some(doc)));
            return;
        }
        let Some(dir) = spotify::lyrics::dir_for(&t) else {
            self.spotify.lyrics = Some((id, None));
            return;
        };
        if let Some(doc) = spotify::lyrics::cached(&dir) {
            self.spotify.lyrics = Some((id, Some(doc)));
            return;
        }
        self.spotify.lyrics = Some((id.clone(), None));
        self.spotify_lyrics_lookup(t);
    }

    fn spotify_lyrics_lookup(&mut self, t: Track) {
        let id = track_id(&t);
        self.spotify.lyrics_failed = None;
        // One lookup at a time: a newer track wins.
        if let Some((_, j)) = &mut self.spotify.lyrics_job {
            j.cancel();
        }
        self.spotify.lyrics_job = None;
        self.spotify.lyrics_direct = None;
        let dir = match spotify::lyrics::prepare(&t) {
            Ok(d) => d,
            Err(e) => {
                self.spotify.lyrics_failed = Some((id, e));
                return;
            }
        };
        if let Some(program) = analyze::find_analyzer() {
            match Job::lyrics(&program, &dir, &[]) {
                Ok(j) => {
                    self.spotify.lyrics_job = Some((id, j));
                    return;
                }
                Err(e) => self.spotify.lyrics_failed = Some((id.clone(), e)),
            }
        }
        // No analyzer: LRCLIB from here.
        let (tx, rx) = unbounded();
        std::thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(12)))
                .build()
                .into();
            let res = spotify::lyrics::lrclib_lookup(&agent, &t).and_then(|rec| {
                let doc = spotify::lyrics::lyrics_from_record(rec.as_ref(), &t);
                spotify::lyrics::write(&dir, &doc)?;
                Ok(doc)
            });
            let _ = tx.send(res);
        });
        self.spotify.lyrics_direct = Some((id, rx));
    }

    fn spotify_lyrics_poll(&mut self) {
        if let Some((id, job)) = &mut self.spotify.lyrics_job {
            job.poll();
            if let Some(res) = job.result.clone() {
                let id = id.clone();
                self.spotify.lyrics_job = None;
                match res {
                    Ok(dir) => {
                        let doc = Lyrics::load(&dir).ok().flatten();
                        if self.spotify.lyrics.as_ref().is_some_and(|(i, _)| *i == id) {
                            self.spotify.lyrics = Some((id, doc));
                        }
                    }
                    Err(e) => self.spotify.lyrics_failed = Some((id, e)),
                }
            }
        }
        if let Some((id, rx)) = &self.spotify.lyrics_direct
            && let Ok(res) = rx.try_recv()
        {
            let id = id.clone();
            self.spotify.lyrics_direct = None;
            match res {
                Ok(doc) => {
                    if self.spotify.lyrics.as_ref().is_some_and(|(i, _)| *i == id) {
                        self.spotify.lyrics = Some((id, Some(doc)));
                    }
                }
                Err(e) => self.spotify.lyrics_failed = Some((id, e)),
            }
        }
        if let Some((dir, job)) = &mut self.spotify.relyrics {
            job.poll();
            if job.result.is_some() {
                let dir = dir.clone();
                self.spotify.relyrics = None;
                if self.song_open_path().is_some_and(|p| p == dir) {
                    self.song_lyrics_reload();
                }
                // The now-playing lyrics may come from this song too.
                self.spotify.lyrics = None;
                if let Some(t) = self.spotify.pb.as_ref().and_then(|p| p.track.clone()) {
                    self.spotify_lyrics_for(t);
                }
            }
        }
    }

    // ------------------------------------------------------------ sync mode

    /// Offset between Spotify's time and the song's (+ = notes earlier), s.
    fn spotify_offset_s(&self) -> f64 {
        let link = self
            .spotify
            .sync
            .as_ref()
            .and_then(|s| self.cfg.spotify.links.get(&s.track_id))
            .map(|l| l.offset_ms)
            .unwrap_or(0.0);
        (self.cfg.spotify.offset_ms + link) as f64 / 1000.0
    }

    /// Every frame while following: keep the song's clock on Spotify's.
    pub(super) fn spotify_sync_tick(&mut self) {
        let Some(sync) = self.spotify.sync.clone() else {
            return;
        };
        let Some(est) = self.spotify.est.clone() else {
            return;
        };
        if est.track_id.as_deref() != Some(sync.track_id.as_str()) {
            return;
        }
        let now = Instant::now();
        // The audio thread reports back a frame or two later: don't act
        // again on a state it hasn't seen yet.
        if self.spotify.last_correction.elapsed() < Duration::from_millis(300) {
            return;
        }
        let target = est.position_s(now) + self.spotify_offset_s();
        let dur = self
            .song_ui
            .song
            .as_ref()
            .map(|s| s.duration)
            .unwrap_or(0.0);
        let (ours, playing) = self.song_clock_now();
        let off = ours - target;
        if est.playing && !playing {
            if target >= dur - 0.05 {
                // Spotify's version runs longer than the file: nothing left here.
                return;
            }
            self.song_seek_local(target);
            self.song_clear_ended();
            self.song_play_pause_local();
            self.spotify.last_correction = now;
        } else if !est.playing && playing {
            self.song_play_pause_local();
            self.song_seek_local(target);
            self.spotify.last_correction = now;
        } else if !est.playing && off.abs() > 0.05 {
            // Paused, but Spotify was moved: follow it.
            self.song_seek_local(target);
            self.spotify.last_correction = now;
        } else if est.playing
            && off.abs() > 0.12
            && self.spotify.last_correction.elapsed() > Duration::from_millis(500)
        {
            // Drifted (a new sample, or someone sought on another device).
            self.song_seek_local(target);
            self.spotify.last_correction = now;
        }
    }

    /// Sync mode's bar on the practice screen (instead of the transport).
    pub(super) fn spotify_sync_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(sync) = self.spotify.sync.clone() else {
            return;
        };
        let now = Instant::now();
        let est = self.spotify.est.clone();
        let this = est
            .as_ref()
            .is_some_and(|e| e.track_id.as_deref() == Some(sync.track_id.as_str()));
        let playing = this && est.as_ref().is_some_and(|e| e.playing);
        ui.horizontal_wrapped(|ui| {
            chip(
                ui,
                p.primary_container,
                p.on_primary_container,
                "♫ Following Spotify",
            );
            if let Some(d) = self.spotify.pb.as_ref().and_then(|x| x.device.clone()) {
                chip(
                    ui,
                    p.surface_container_high,
                    p.on_surface,
                    format!("on {}", d.name),
                );
            }
            if ui
                .small_button("−10 s")
                .on_hover_text("Spotify back 10 s")
                .clicked()
                && let Some(e) = &est
            {
                self.spotify_seek_ms(e.position_ms(now) - 10_000.0);
            }
            let (label, fill, fg) = if playing {
                ("⏸  Pause", p.primary, p.on_primary)
            } else {
                ("▶  Play", p.secondary_container, p.on_secondary_container)
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).color(fg).strong())
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(255)),
                )
                .on_hover_text("Plays / pauses Spotify (F5)")
                .clicked()
            {
                self.spotify_toggle();
            }
            if ui
                .small_button("+10 s")
                .on_hover_text("Spotify ahead 10 s")
                .clicked()
                && let Some(e) = &est
            {
                self.spotify_seek_ms(e.position_ms(now) + 10_000.0);
            }
            if let Some(e) = &est {
                ui.label(
                    RichText::new(format!(
                        "{} / {}",
                        fmt_ms(e.position_ms(now)),
                        fmt_ms(e.duration_ms)
                    ))
                    .monospace(),
                );
            }
            ui.separator();
            // Offset for this song (+ = notes earlier).
            let mut o = self
                .cfg
                .spotify
                .links
                .get(&sync.track_id)
                .map(|l| l.offset_ms)
                .unwrap_or(0.0);
            let mut changed = false;
            ui.label("Notes");
            if ui
                .small_button("− later")
                .on_hover_text("The notes come before the music: move them later (50 ms)")
                .clicked()
            {
                o -= 50.0;
                changed = true;
            }
            changed |= ui
                .add(
                    egui::DragValue::new(&mut o)
                        .range(-10_000.0..=10_000.0)
                        .speed(5.0)
                        .suffix(" ms"),
                )
                .on_hover_text(
                    "How far your analysed file is ahead of Spotify's version (+ = notes earlier). \
                     Different releases often start a little apart. Saved for this song.",
                )
                .changed();
            if ui
                .small_button("+ earlier")
                .on_hover_text("The notes come after the music: move them earlier (50 ms)")
                .clicked()
            {
                o += 50.0;
                changed = true;
            }
            if changed && let Some(l) = self.cfg.spotify.links.get_mut(&sync.track_id) {
                l.offset_ms = o.clamp(-10_000.0, 10_000.0);
                self.mark_dirty();
            }
            ui.separator();
            if ui.button("Stop following").clicked() {
                self.spotify_stop_sync();
            }
        });
        let mut notes: Vec<(String, Color32)> = Vec::new();
        if !this {
            let name = self
                .spotify
                .pb
                .as_ref()
                .and_then(|x| x.track.as_ref())
                .map(|t| t.name.clone());
            notes.push((
                match name {
                    Some(n) => format!("Spotify is playing “{n}”, not “{}”. Play this song on Spotify (or press Play here) and the notes follow.", sync.track_name),
                    None => format!("Nothing is playing on Spotify. Press Play to start “{}”.", sync.track_name),
                },
                p.tertiary,
            ));
        }
        if let Some((until, why)) = self.sp_paused() {
            let left = fmt_wait(until.saturating_duration_since(now));
            notes.push((
                match why {
                    Why::Offline => format!("Can't reach Spotify (retry in {left}): the notes keep going from the last known position."),
                    _ => format!("Spotify paused for {left}: the notes keep going from the last known position. Control Spotify in its own app meanwhile."),
                },
                p.tertiary,
            ));
        }
        if self.cfg.song.line_up && self.output_delay_ms() > 1.0 {
            notes.push((
                format!(
                    "The notes run {:.0} ms ahead of what you hear (Spotify plays through the same delay), so press on the line.",
                    self.output_delay_ms()
                ),
                p.outline,
            ));
        }
        notes.push((
            "Speed, loops and wait mode are off while following Spotify (it doesn't wait). Stop following to practise slowly."
                .into(),
            p.outline,
        ));
        for (n, c) in notes {
            ui.label(RichText::new(n).small().color(c));
        }
        // Play button on Spotify when nothing is playing: play this track.
        if !this
            && let Some(t) = self.spotify.cache.track(&sync.track_id).cloned()
            && ui
                .small_button(format!("▶ Play “{}” on Spotify", t.name))
                .clicked()
        {
            self.spotify_play_track(&t, None);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }

    // ------------------------------------------------------------ the tab

    pub(super) fn spotify_tab(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.spotify_start(&ctx);
        let p = self.palette;
        egui::ScrollArea::vertical()
            .id_salt("spotify-tab")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some((msg, at)) = self.spotify.note.clone() {
                    if at.elapsed() < Duration::from_secs(20) {
                        ui.label(RichText::new(format!("ⓘ {msg}")).color(p.tertiary));
                        if let Some(devs) = self.spotify.devices.clone() {
                            self.spotify_device_picker(ui, &devs);
                        }
                    } else {
                        self.spotify.note = None;
                    }
                }
                if !self.spotify.logged_in {
                    self.spotify_login_card(ui);
                    return;
                }
                ui.columns(2, |cols| {
                    self.spotify_now_playing(&mut cols[0]);
                    cols[0].add_space(8.0);
                    self.spotify_lyrics_card(&mut cols[0]);
                    self.spotify_search_card(&mut cols[1]);
                    if self.spotify.learn.is_some() {
                        cols[1].add_space(8.0);
                        self.spotify_learn_card(&mut cols[1]);
                    }
                    cols[1].add_space(8.0);
                    self.spotify_settings(&mut cols[1]);
                });
            });
        if self.spotify.est.as_ref().is_some_and(|e| e.playing) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn spotify_device_picker(&mut self, ui: &mut egui::Ui, devs: &[Device]) {
        if devs.is_empty() {
            ui.label("No Spotify devices found. Open the Spotify app (or open.spotify.com) and try again.");
            if ui.button("Look again").clicked() {
                self.sp_send(Req::Devices);
            }
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for d in devs {
                let Some(id) = d.id.clone() else { continue };
                if ui
                    .button(format!("{} ({})", d.name, d.kind))
                    .on_hover_text("Play there")
                    .clicked()
                {
                    match self.spotify.pending_play.take() {
                        Some(Req::Play {
                            uri, position_ms, ..
                        }) => self.sp_send(Req::Play {
                            uri,
                            position_ms,
                            device: Some(id.clone()),
                        }),
                        _ => self.sp_send(Req::Transfer(id.clone())),
                    }
                    self.spotify.devices = None;
                    self.spotify.note = None;
                }
            }
        });
    }

    fn spotify_login_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        card(ui, &p, "Connect Spotify", |ui| {
            ui.label(
                "Qwertone can show what Spotify plays, search and control it, find the practice \
                 version of a song, and move the falling notes and lyrics along with Spotify. \
                 It needs your own (free) Spotify developer app, once:",
            );
            ui.add_space(4.0);
            let uri = auth::redirect_uri(self.cfg.spotify.redirect_port);
            for (i, step) in [
                "Open developer.spotify.com/dashboard, log in with your Premium account, and press “Create app”.".to_string(),
                "Any name and description. Redirect URI, exactly:".to_string(),
                "Tick “Web API”, agree, Save. Open the app's Settings and copy the Client ID (no secret needed).".to_string(),
                "Paste it below and press Log in. Your browser asks you to allow it once.".to_string(),
            ]
            .iter()
            .enumerate()
            {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{}.", i + 1)).strong());
                    ui.label(step);
                    if i == 1 {
                        ui.code(&uri);
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(uri.clone());
                        }
                    }
                });
            }
            ui.label(
                RichText::new(
                    "Full guide with pictures-in-words: docs/SPOTIFY_SETUP.md in the app's folder.",
                )
                .small()
                .color(p.outline),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Client ID");
                ui.add(
                    egui::TextEdit::singleline(&mut self.spotify.client_id_input)
                        .hint_text("32 letters and digits")
                        .desired_width(300.0),
                );
            });
            if let Some(l) = &self.spotify.login {
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label("Waiting for you to allow it in the browser…");
                    if ui
                        .small_button("Copy the link")
                        .on_hover_text("If no browser opened")
                        .clicked()
                    {
                        ui.ctx().copy_text(l.url.clone());
                    }
                });
                if ui.button("Cancel").clicked() {
                    self.spotify.login = None;
                }
            } else if ui
                .add(
                    egui::Button::new(RichText::new("Log in with Spotify").strong())
                        .fill(p.primary_container),
                )
                .clicked()
            {
                let id = self.spotify.client_id_input.trim().to_string();
                self.cfg.spotify.client_id = id.clone();
                self.mark_dirty();
                let ctx = ui.ctx().clone();
                match Login::start(&id, self.cfg.spotify.redirect_port, move || {
                    ctx.request_repaint()
                }) {
                    Ok(l) => {
                        self.spotify.login = Some(l);
                        self.spotify.login_error = None;
                    }
                    Err(e) => self.spotify.login_error = Some(e),
                }
            }
            if let Some(e) = &self.spotify.login_error {
                ui.label(RichText::new(format!("⚠ {e}")).color(p.error));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Your login is kept in the desktop keyring (secret-tool), or in a private file only you can \
                     read if there's no keyring. Spotify's audio is never used for practice: songs are analysed \
                     from your own files.",
                )
                .small()
                .color(p.outline),
            );
        });
    }

    fn spotify_now_playing(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let now = Instant::now();
        let pb = self.spotify.pb.clone();
        let est = self.spotify.est.clone();
        card(ui, &p, "Now playing", |ui| {
            let Some(pb) = pb else {
                ui.label(if self.spotify.poller.last().is_none() {
                    "Asking Spotify…"
                } else {
                    "Nothing is playing. Start something in Spotify, or search on the right."
                });
                return;
            };
            let Some(t) = pb.track.clone() else {
                ui.label(match pb.kind.as_str() {
                    "ad" => "An ad is playing.",
                    "episode" => "A podcast is playing (no practice version for those).",
                    _ => "Nothing to show.",
                });
                return;
            };
            ui.label(RichText::new(&t.name).size(22.0).strong().color(p.primary));
            ui.label(RichText::new(t.artist_line()).size(16.0));
            ui.label(RichText::new(&t.album).color(p.on_surface_variant));
            if let Some(d) = &pb.device {
                ui.label(
                    RichText::new(format!("on {} ({})", d.name, d.kind))
                        .small()
                        .color(p.outline),
                );
            }
            ui.add_space(6.0);
            if let Some(e) = &est {
                // Progress bar: click to seek.
                let w = ui.available_width();
                let (r, resp) = ui.allocate_exact_size(vec2(w, 10.0), Sense::click());
                let frac = if e.duration_ms > 0.0 {
                    (e.position_ms(now) / e.duration_ms) as f32
                } else {
                    0.0
                };
                ui.painter()
                    .rect_filled(r, egui::CornerRadius::same(5), p.surface_container_high);
                let mut done = r;
                done.set_width(r.width() * frac.clamp(0.0, 1.0));
                ui.painter()
                    .rect_filled(done, egui::CornerRadius::same(5), p.primary);
                if resp.clicked()
                    && let Some(pos) = resp.interact_pointer_pos()
                {
                    let f = ((pos.x - r.left()) / r.width()).clamp(0.0, 1.0) as f64;
                    self.spotify_seek_ms(f * e.duration_ms);
                }
                let resp = resp.on_hover_text("Click to jump");
                let _ = resp;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(fmt_ms(e.position_ms(now))).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(fmt_ms(e.duration_ms)).monospace());
                    });
                });
            }
            ui.horizontal(|ui| {
                let playing = est.as_ref().is_some_and(|e| e.playing);
                if ui.button("−10 s").clicked()
                    && let Some(e) = &est
                {
                    self.spotify_seek_ms(e.position_ms(now) - 10_000.0);
                }
                let (label, fill, fg) = if playing {
                    ("⏸  Pause", p.primary, p.on_primary)
                } else {
                    ("▶  Play", p.secondary_container, p.on_secondary_container)
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).color(fg).strong())
                            .fill(fill)
                            .corner_radius(egui::CornerRadius::same(255)),
                    )
                    .clicked()
                {
                    self.spotify_toggle();
                }
                if ui.button("+10 s").clicked()
                    && let Some(e) = &est
                {
                    self.spotify_seek_ms(e.position_ms(now) + 10_000.0);
                }
            });
            ui.add_space(6.0);
            let link =
                t.id.as_ref()
                    .and_then(|id| self.cfg.spotify.links.get(id).cloned());
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("🎹  Learn this").strong())
                            .fill(p.primary_container),
                    )
                    .on_hover_text(if link.is_some() {
                        "Open the practice version and follow Spotify"
                    } else {
                        "Find or make the practice version (from your own copy of the song)"
                    })
                    .clicked()
                {
                    self.spotify_learn(&t);
                }
                if let Some(l) = &link {
                    chip(
                        ui,
                        p.surface_container_high,
                        p.on_surface,
                        "practice version ready",
                    );
                    if ui
                        .small_button("Practise at my own speed")
                        .on_hover_text(
                            "Open it without following Spotify (slow down, loop, wait mode)",
                        )
                        .clicked()
                    {
                        self.spotify_stop_sync_quiet();
                        self.open_song(&l.song.clone());
                        self.tab = Tab::Song;
                    }
                }
            });
            if let Some(age) = self.spotify.poller.last().map(|l| l.elapsed()) {
                ui.label(
                    RichText::new(format!(
                        "Checked with Spotify {} ago; the position in between is estimated.",
                        fmt_wait(age)
                    ))
                    .small()
                    .color(p.outline),
                );
            }
        });
    }

    fn spotify_lyrics_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let now = Instant::now();
        let track = self.spotify.pb.as_ref().and_then(|x| x.track.clone());
        let Some(t) = track else { return };
        let id = track_id(&t);
        let doc = self
            .spotify
            .lyrics
            .as_ref()
            .filter(|(i, _)| *i == id)
            .and_then(|(_, d)| d.clone());
        let looking = self
            .spotify
            .lyrics_job
            .as_ref()
            .is_some_and(|(i, _)| *i == id)
            || self
                .spotify
                .lyrics_direct
                .as_ref()
                .is_some_and(|(i, _)| *i == id);
        let failed = self
            .spotify
            .lyrics_failed
            .as_ref()
            .filter(|(i, _)| *i == id)
            .map(|x| x.1.clone());
        card(ui, &p, "Lyrics", |ui| {
            let Some(doc) = doc else {
                if looking {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            "Looking for lyrics (LRCLIB, by Spotify's title / artist / length)…",
                        );
                    });
                } else if let Some(e) = failed {
                    ui.label(RichText::new(format!("Couldn't get lyrics: {e}")).color(p.tertiary));
                    if ui.small_button("Try again").clicked() {
                        self.spotify_lyrics_lookup(t.clone());
                    }
                } else {
                    ui.label(RichText::new("No lyrics yet.").color(p.outline));
                }
                return;
            };
            match doc.status.as_str() {
                "not_found" => {
                    ui.label("No lyrics found for this song.");
                    return;
                }
                "instrumental" => {
                    ui.label("Instrumental (no lyrics).");
                    return;
                }
                _ => {}
            }
            // Layer toggles.
            let have = doc.available();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Show").color(p.on_surface_variant));
                let l = &mut self.cfg.lyrics;
                let mut changed = false;
                if have.native {
                    changed |= ui
                        .selectable_label(l.native, "Original script")
                        .clicked()
                        .then(|| l.native = !l.native)
                        .is_some();
                }
                if have.romanized {
                    changed |= ui
                        .selectable_label(l.romanized, "English letters")
                        .clicked()
                        .then(|| l.romanized = !l.romanized)
                        .is_some();
                }
                if have.english {
                    changed |= ui
                        .selectable_label(l.english, "English (machine translation)")
                        .clicked()
                        .then(|| l.english = !l.english)
                        .is_some();
                }
                if changed {
                    self.mark_dirty();
                }
            });
            if !have.romanized
                && doc.language.as_deref().is_some_and(|l| l != "en")
                && analyze::find_analyzer().is_none()
            {
                ui.label(
                    RichText::new("Install the analyzer (analyzer/setup.sh) to get these lyrics in English letters too.")
                        .small()
                        .color(p.outline),
                );
            }
            let want = Layers {
                native: self.cfg.lyrics.native,
                romanized: self.cfg.lyrics.romanized,
                english: self.cfg.lyrics.english,
            };
            let lt = self.spotify_lyrics_time(&t, now);
            if doc.synced() {
                let pos = doc.position(lt);
                let cur = pos.line.unwrap_or(0);
                let from = cur.saturating_sub(2);
                let to = (cur + 5).min(doc.lines.len());
                let size = 15.0 * self.cfg.lyrics.size.clamp(0.6, 2.5);
                for i in from..to {
                    let line = &doc.lines[i];
                    let is_cur = Some(i) == pos.line && pos.active;
                    let texts = doc.texts(line, want);
                    for (k, (layer, s)) in texts.iter().enumerate() {
                        let mut rt = RichText::new(*s).size(if is_cur && k == 0 {
                            size * 1.35
                        } else {
                            size
                        });
                        rt = if is_cur {
                            rt.strong().color(if *layer == Layer::English {
                                p.on_surface_variant
                            } else {
                                p.primary
                            })
                        } else if Some(i) < pos.line {
                            rt.color(p.outline)
                        } else {
                            rt.color(p.on_surface_variant)
                        };
                        let r = ui.add(egui::Label::new(rt).sense(Sense::click()));
                        if r.clicked()
                            && let Some(st) = line.start
                        {
                            // Jump Spotify to this line.
                            let back = lt
                                - self
                                    .spotify
                                    .est
                                    .as_ref()
                                    .map(|e| e.position_s(now))
                                    .unwrap_or(0.0);
                            self.spotify_seek_ms((st - back) * 1000.0);
                        }
                    }
                    ui.add_space(4.0);
                }
            } else {
                ui.label(
                    RichText::new("These lyrics have no timing (shown as a list).")
                        .small()
                        .color(p.outline),
                );
                for line in &doc.lines {
                    for (_, s) in doc.texts(line, want) {
                        ui.label(s);
                    }
                }
            }
            ui.add_space(4.0);
            // Source + nudge.
            let src = doc
                .sources
                .words
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "?".into());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("Lyrics: {src}"))
                        .small()
                        .color(p.outline),
                );
                let mut o = self.cfg.lyrics.offset_ms;
                ui.label(RichText::new("· timing").small().color(p.outline));
                if ui
                    .add(
                        egui::DragValue::new(&mut o)
                            .range(-3000.0..=3000.0)
                            .speed(5.0)
                            .suffix(" ms"),
                    )
                    .on_hover_text("+ = lyrics earlier, − = later")
                    .changed()
                {
                    self.cfg.lyrics.offset_ms = o;
                    self.mark_dirty();
                }
            });
        });
    }

    /// Song time for Spotify's lyrics now: Spotify's position, minus the
    /// output delay when Spotify plays on a computer (probably this one,
    /// through the same earbuds), plus the nudges.
    fn spotify_lyrics_time(&self, t: &Track, now: Instant) -> f64 {
        let Some(e) = &self.spotify.est else {
            return 0.0;
        };
        let on_computer = self
            .spotify
            .pb
            .as_ref()
            .and_then(|x| x.device.as_ref())
            .is_none_or(|d| d.is_computer());
        let delay = if on_computer {
            self.output_delay_ms()
        } else {
            0.0
        };
        let link =
            t.id.as_ref()
                .and_then(|id| self.cfg.spotify.links.get(id))
                .map(|l| l.offset_ms)
                .unwrap_or(0.0);
        e.position_s(now)
            + (self.cfg.spotify.offset_ms + link + self.cfg.lyrics.offset_ms - delay) as f64
                / 1000.0
    }

    fn spotify_search_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        card(ui, &p, "Search", |ui| {
            let mut go = false;
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.spotify.query)
                        .hint_text("song, artist… (Enter)")
                        .desired_width(ui.available_width() - 80.0),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                if ui.button("Search").clicked() {
                    go = true;
                }
            });
            if go {
                self.spotify_search();
            }
            if self.spotify.searching.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Searching…");
                });
            }
            if let Some(n) = &self.spotify.results_note {
                ui.label(RichText::new(n).small().color(p.tertiary));
            }
            let results = self.spotify.results.clone();
            if results.is_empty()
                && !self.spotify.results_for.is_empty()
                && self.spotify.searching.is_none()
            {
                ui.label(RichText::new("Nothing found.").color(p.outline));
            }
            for t in results {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&t.name).strong());
                        ui.label(
                            RichText::new(format!(
                                "{} · {} · {}",
                                t.artist_line(),
                                t.album,
                                fmt_ms(t.duration_ms as f64)
                            ))
                            .small()
                            .color(p.on_surface_variant),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Learn this").clicked() {
                            self.spotify_learn(&t);
                        }
                        if ui
                            .small_button("▶ Play")
                            .on_hover_text("Play it on Spotify")
                            .clicked()
                        {
                            self.spotify_play_track(&t, None);
                        }
                        if t.id
                            .as_ref()
                            .is_some_and(|id| self.cfg.spotify.links.contains_key(id))
                        {
                            ui.label(RichText::new("🎹").color(p.primary))
                                .on_hover_text("Has a practice version");
                        }
                    });
                });
            }
        });
    }

    fn spotify_learn_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(Learn::Choose {
            track,
            songs,
            files,
        }) = self.spotify.learn.clone()
        else {
            return;
        };
        let (job, job_err) = self.song_job_status();
        card(ui, &p, &format!("Learn “{}”", track.name), |ui| {
            if let Some((file, frac, msg)) = &job {
                ui.label(
                    RichText::new(format!(
                        "Analysing {}",
                        file.file_name().unwrap_or_default().to_string_lossy()
                    ))
                    .strong(),
                );
                ui.add(
                    egui::ProgressBar::new(*frac)
                        .show_percentage()
                        .animate(true),
                );
                ui.label(RichText::new(msg).small().color(p.on_surface_variant));
                ui.label(
                    RichText::new("It opens by itself when done (a few minutes the first time).")
                        .small()
                        .color(p.outline),
                );
                if ui.small_button("Cancel").clicked() {
                    self.song_cancel_analysis();
                    self.spotify.pending_link = None;
                }
                ui.ctx().request_repaint_after(Duration::from_millis(250));
                return;
            }
            if let Some(e) = job_err
                && self.spotify.pending_link.is_some()
            {
                ui.label(RichText::new(format!("⚠ The analysis failed: {e}")).color(p.error));
            }
            ui.label(
                "There's no practice version yet. Spotify's audio can't be used for that, so it's made \
                 from your own copy of the song (mp3, flac…) or a MIDI file.",
            );
            if !songs.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Is it one of your analysed songs?").strong());
                for (e, s) in &songs {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{}{} · {}",
                            e.title,
                            e.artist
                                .as_ref()
                                .map(|a| format!(" — {a}"))
                                .unwrap_or_default(),
                            fmt_ms(e.duration * 1000.0)
                        ));
                        ui.label(
                            RichText::new(format!("{:.0}% sure", s * 100.0))
                                .small()
                                .color(p.outline),
                        );
                        if ui.small_button("Yes, use it").clicked() {
                            let path = e.path.clone();
                            if let Some(link) = self.spotify_make_link(&track, &path) {
                                self.spotify_open_practice(&track, &link);
                            }
                        }
                    });
                }
            }
            if !files.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Files on your computer that look like it:").strong());
                for (f, _) in &files {
                    ui.horizontal(|ui| {
                        ui.label(f.file_name().unwrap_or_default().to_string_lossy());
                        if ui
                            .small_button("Analyze this")
                            .on_hover_text(f.display().to_string())
                            .clicked()
                        {
                            self.spotify_use_file(&track, f);
                        }
                    });
                }
            } else {
                ui.label(
                    RichText::new("No file with that name in your Music or Downloads folders.")
                        .small()
                        .color(p.outline),
                );
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.spotify.pick.is_none(),
                        egui::Button::new("Choose a file…"),
                    )
                    .clicked()
                {
                    self.spotify_pick_file(&track);
                }
                if ui.button("Close").clicked() {
                    self.spotify.learn = None;
                }
            });
        });
    }

    fn spotify_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::CollapsingHeader::new("Spotify settings")
            .id_salt("sp-settings")
            .default_open(self.spotify.setup_open)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Logged in");
                    if let Some(w) = self.spotify.stored {
                        ui.label(RichText::new(format!("(kept {})", w.describe())).small().color(p.outline));
                    }
                    if ui.button("Log out").clicked() {
                        self.sp_send(Req::Logout);
                        self.spotify_stop_sync_quiet();
                        self.spotify.logged_in = false;
                        self.spotify.pb = None;
                        self.spotify.est = None;
                        self.spotify.stored = None;
                    }
                });
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Keep the login in");
                    let s = &mut self.cfg.spotify.token_store;
                    for (v, l) in [("auto", "keyring if possible"), ("keyring", "keyring only"), ("file", "private file")] {
                        changed |= ui.selectable_value(s, v.to_string(), l).changed();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Check what's playing every");
                    changed |= ui
                        .add(egui::Slider::new(&mut self.cfg.spotify.poll_secs, 10..=60).suffix(" s"))
                        .on_hover_text("Only while this tab or sync mode is on screen. In between the position is estimated. Fewer checks = less risk of hitting Spotify's limits.")
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Sync offset (all songs)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.cfg.spotify.offset_ms).range(-3000.0..=3000.0).speed(5.0).suffix(" ms"))
                        .on_hover_text("+ = notes and lyrics earlier. For Spotify's own buffering; each song also has its own offset.")
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Redirect URI");
                    ui.code(auth::redirect_uri(self.cfg.spotify.redirect_port));
                });
                if changed {
                    self.mark_dirty();
                }
                ui.label(
                    RichText::new(format!(
                        "{} Spotify calls since Qwertone started{}.",
                        self.spotify.calls,
                        match self.sp_paused() {
                            Some((u, w)) => format!(
                                " · paused for {} ({})",
                                fmt_wait(u.saturating_duration_since(Instant::now())),
                                match w {
                                    Why::Quota => "quota used up",
                                    Why::RateLimit => "rate limit",
                                    Why::Offline => "offline",
                                }
                            ),
                            None => String::new(),
                        }
                    ))
                    .small()
                    .color(p.outline),
                );
                if self.sp_paused().is_some_and(|(_, w)| w == Why::Offline) && ui.small_button("Try again now").clicked() {
                    self.spotify.paused = None;
                    self.sp_send(Req::RetryNow);
                    self.spotify.poller.after_control(Instant::now());
                }
                let links: Vec<(String, SpotifyLink)> =
                    self.cfg.spotify.links.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                if !links.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Songs with a practice version").strong());
                    let mut remove = None;
                    for (id, l) in links {
                        ui.horizontal(|ui| {
                            ui.label(&l.title);
                            if l.offset_ms != 0.0 {
                                ui.label(RichText::new(format!("{:+.0} ms", l.offset_ms)).small().color(p.outline));
                            }
                            if ui.small_button("Unlink").on_hover_text(l.song.display().to_string()).clicked() {
                                remove = Some(id.clone());
                            }
                        });
                    }
                    if let Some(id) = remove {
                        self.cfg.spotify.links.remove(&id);
                        self.mark_dirty();
                    }
                }
            });
    }
}
