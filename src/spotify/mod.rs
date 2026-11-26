//! Spotify (Part 8): log in with PKCE, search, control playback, now
//! playing, and the clock for sync mode.
//!
//! All Web API calls go through one worker thread, one at a time, through
//! the [`limit::Limiter`] (our own budget, 429 / quota pauses with
//! Retry-After, back-off on network errors). Requests waiting in the queue
//! are merged (several polls → one, several searches / seeks → the last).
//! The UI never blocks: it sends [`Req`]s and reads [`Ev`]s each frame.
//!
//! Spotify can't be the audio for practice (against its terms): it only
//! tells us what plays and where, so the analysed version of the song can
//! follow along (sync mode).

pub mod api;
pub mod auth;
pub mod cache;
pub mod limit;
pub mod lyrics;
pub mod matching;
pub mod store;
pub mod sync;

use api::{Device, Playback, Track};
use auth::Tokens;
use crossbeam_channel::{Receiver, Sender, unbounded};
use limit::{Blocked, Limiter, Urgency, Why};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What the UI asks for.
#[derive(Clone, Debug)]
pub enum Req {
    /// Use these credentials (after login, or loaded at start).
    SetLogin(Box<Tokens>, String),
    Logout,
    Search(String),
    /// Ask for the playback state (`GET /me/player`).
    Poll,
    /// Play a track (`uri`) from `position_ms`, or resume (`uri` None).
    Play {
        uri: Option<String>,
        position_ms: Option<u64>,
        device: Option<String>,
    },
    Pause,
    Seek(u64),
    Devices,
    /// Move playback to a device and play there.
    Transfer(String),
    /// "Try again" after an offline pause.
    RetryNow,
}

/// What comes back.
#[derive(Clone, Debug)]
pub enum Ev {
    /// The access token was refreshed and saved (or couldn't be saved).
    Saved(Result<store::Where, String>),
    /// The login doesn't work any more (revoked, wrong app): log in again.
    LoggedOut(String),
    Search {
        q: String,
        tracks: Vec<Track>,
    },
    /// `None` = nothing playing (204).
    Playback {
        pb: Option<Playback>,
        sent: Instant,
        received: Instant,
    },
    Devices(Vec<Device>),
    /// A control went through.
    Done(&'static str),
    /// Spotify said there's no active device (open Spotify somewhere, or
    /// pick a device).
    NoDevice,
    /// Calls are paused (429 / quota / offline) until then.
    Paused {
        until: Instant,
        why: Why,
    },
    /// Our own budget: this one wasn't sent; try after `at`.
    Busy {
        what: &'static str,
        at: Instant,
    },
    Error(String),
    /// Calls made so far (shown in the settings).
    Calls(u64),
}

/// The worker's handle.
pub struct Client {
    tx: Sender<Req>,
    pub rx: Receiver<Ev>,
}

impl Client {
    pub fn start(store_pref: String, repaint: impl Fn() + Send + 'static) -> Client {
        // QWERTONE_SPOTIFY_API: a fake Web API for testing the UI.
        let base = std::env::var("QWERTONE_SPOTIFY_API").unwrap_or_else(|_| api::API.to_string());
        Client::start_with(base, store_pref, repaint)
    }

    /// With another API address (tests use a fake server).
    pub fn start_with(
        api_base: String,
        store_pref: String,
        repaint: impl Fn() + Send + 'static,
    ) -> Client {
        let (tx, wrx) = unbounded::<Req>();
        let (wtx, rx) = unbounded::<Ev>();
        std::thread::Builder::new()
            .name("qwertone-spotify".into())
            .spawn(move || {
                let mut w = Worker::new(store_pref, wtx, Box::new(repaint));
                w.api_base = api_base;
                w.run(wrx)
            })
            .expect("spawn the Spotify thread");
        Client { tx, rx }
    }

    pub fn send(&self, r: Req) {
        let _ = self.tx.send(r);
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent("Qwertone")
        .build()
        .into()
}

/// A raw answer.
struct Answer {
    status: u16,
    retry_after: Option<String>,
    body: String,
}

enum Fail {
    /// Network / timeout.
    Net(String),
}

fn call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<(&str, String)>,
) -> Result<Answer, Fail> {
    let auth = bearer.map(|t| format!("Bearer {t}"));
    let resp = match method {
        "GET" => {
            let mut r = agent.get(url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.call()
        }
        "PUT" | "POST" => {
            let mut r = if method == "PUT" {
                agent.put(url)
            } else {
                agent.post(url)
            };
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            match body {
                Some((ct, b)) => r.content_type(ct).send(b),
                None => r.send_empty(),
            }
        }
        _ => unreachable!(),
    };
    let resp = resp.map_err(|e| Fail::Net(e.to_string()))?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = resp.into_body().read_to_string().unwrap_or_default();
    Ok(Answer {
        status,
        retry_after,
        body,
    })
}

struct Worker {
    agent: ureq::Agent,
    api_base: String,
    limiter: Limiter,
    tokens: Option<Tokens>,
    store_pref: String,
    tx: Sender<Ev>,
    repaint: Box<dyn Fn() + Send>,
}

/// What a call ended in, for the caller.
enum Outcome {
    Ok(Answer),
    /// Already reported to the UI (pause, budget, logout, error).
    Stopped,
}

impl Worker {
    fn new(store_pref: String, tx: Sender<Ev>, repaint: Box<dyn Fn() + Send>) -> Worker {
        Worker {
            agent: agent(),
            api_base: api::API.to_string(),
            limiter: Limiter::new(),
            tokens: None,
            store_pref,
            tx,
            repaint,
        }
    }

    fn emit(&self, e: Ev) {
        let _ = self.tx.send(e);
        (self.repaint)();
    }

    fn run(mut self, rx: Receiver<Req>) {
        while let Ok(first) = rx.recv() {
            // Merge what queued up meanwhile.
            let mut batch = vec![first];
            while let Ok(r) = rx.try_recv() {
                batch.push(r);
            }
            for r in merge(batch) {
                self.handle(r);
            }
            self.emit(Ev::Calls(self.limiter.total));
        }
    }

    fn handle(&mut self, r: Req) {
        match r {
            Req::SetLogin(t, pref) => {
                self.tokens = Some(*t);
                self.store_pref = pref;
            }
            Req::Logout => {
                self.tokens = None;
                store::clear();
            }
            Req::RetryNow => {
                if !self.limiter.retry_now(Instant::now())
                    && let Some((until, why)) = self.limiter.paused(Instant::now())
                {
                    self.emit(Ev::Paused { until, why });
                }
            }
            Req::Poll => {
                let sent = Instant::now();
                if let Outcome::Ok(a) = self.api("GET", "/me/player", None, Urgency::Poll, "poll") {
                    let received = Instant::now();
                    let pb = if a.status == 204 || a.body.trim().is_empty() {
                        None
                    } else {
                        serde_json::from_str(&a.body)
                            .ok()
                            .and_then(|v| Playback::from_json(&v))
                    };
                    self.emit(Ev::Playback { pb, sent, received });
                }
            }
            Req::Search(q) => {
                let url = format!(
                    "/search?type=track&limit={}&q={}",
                    api::SEARCH_LIMIT,
                    auth::enc(&api::search_query(&q))
                );
                if let Outcome::Ok(a) = self.api("GET", &url, None, Urgency::User, "search") {
                    let v = serde_json::from_str(&a.body).unwrap_or_default();
                    self.emit(Ev::Search {
                        q,
                        tracks: api::parse_search(&v),
                    });
                }
            }
            Req::Play {
                uri,
                position_ms,
                device,
            } => {
                let mut body = serde_json::Map::new();
                if let Some(u) = &uri {
                    body.insert("uris".into(), serde_json::json!([u]));
                }
                if let Some(p) = position_ms {
                    body.insert("position_ms".into(), p.into());
                }
                let path = match &device {
                    Some(d) => format!("/me/player/play?device_id={}", auth::enc(d)),
                    None => "/me/player/play".into(),
                };
                let b = (!body.is_empty()).then(|| {
                    (
                        "application/json",
                        serde_json::Value::Object(body).to_string(),
                    )
                });
                self.control("PUT", &path, b, "play");
            }
            Req::Pause => self.control("PUT", "/me/player/pause", None, "pause"),
            Req::Seek(ms) => self.control(
                "PUT",
                &format!("/me/player/seek?position_ms={ms}"),
                None,
                "seek",
            ),
            Req::Transfer(id) => self.control(
                "PUT",
                "/me/player",
                Some((
                    "application/json",
                    serde_json::json!({"device_ids": [id], "play": true}).to_string(),
                )),
                "transfer",
            ),
            Req::Devices => {
                if let Outcome::Ok(a) =
                    self.api("GET", "/me/player/devices", None, Urgency::User, "devices")
                {
                    let v = serde_json::from_str(&a.body).unwrap_or_default();
                    self.emit(Ev::Devices(api::parse_devices(&v)));
                }
            }
        }
    }

    fn control(
        &mut self,
        method: &str,
        path: &str,
        body: Option<(&str, String)>,
        what: &'static str,
    ) {
        if let Outcome::Ok(_) = self.api(method, path, body, Urgency::User, what) {
            self.emit(Ev::Done(what));
        }
    }

    /// Make sure the access token is fresh (refreshing costs one call to
    /// the accounts service, not the Web API).
    fn fresh_token(&mut self) -> Option<String> {
        let t = self.tokens.as_ref()?;
        if !t.expired(auth::unix_now()) {
            return Some(t.access_token.clone());
        }
        self.refresh()
            .then(|| self.tokens.as_ref().unwrap().access_token.clone())
    }

    fn refresh(&mut self) -> bool {
        let Some(t) = self.tokens.clone() else {
            return false;
        };
        let now = Instant::now();
        if let Err(b) = self.limiter.begin(now, Urgency::User) {
            self.report_blocked(b, "login refresh");
            return false;
        }
        let body = auth::refresh_request(&t.client_id, &t.refresh_token);
        match call(
            &self.agent,
            "POST",
            auth::TOKEN,
            None,
            Some(("application/x-www-form-urlencoded", body)),
        ) {
            Err(Fail::Net(e)) => {
                let until = self.limiter.failed(now);
                self.emit(Ev::Paused {
                    until,
                    why: Why::Offline,
                });
                log(&format!("token refresh failed: {e}"));
                false
            }
            Ok(a) if a.status == 200 => {
                self.limiter.ok();
                match Tokens::from_response(
                    &a.body,
                    Some(&t.refresh_token),
                    &t.client_id,
                    auth::unix_now(),
                ) {
                    Ok(nt) => {
                        let saved = store::save(&self.store_pref, &nt);
                        self.tokens = Some(nt);
                        self.emit(Ev::Saved(saved));
                        true
                    }
                    Err(e) => {
                        self.emit(Ev::Error(format!("Spotify login refresh: {e}")));
                        false
                    }
                }
            }
            Ok(a) if a.status == 429 => {
                self.too_many(&a);
                false
            }
            Ok(a) if a.status >= 500 => {
                let until = self.limiter.failed(now);
                self.emit(Ev::Paused {
                    until,
                    why: Why::Offline,
                });
                false
            }
            Ok(a) => {
                // 400 invalid_grant: revoked / expired / another app.
                let (msg, _) = api::error_of(&a.body);
                self.tokens = None;
                store::clear();
                self.emit(Ev::LoggedOut(format!(
                    "Spotify ended the login ({}). Log in again.",
                    msg.unwrap_or_else(|| format!("HTTP {}", a.status))
                )));
                false
            }
        }
    }

    fn too_many(&mut self, a: &Answer) {
        let quota = limit::is_quota(&a.body);
        let ra = limit::parse_retry_after(a.retry_after.as_deref());
        let until = self.limiter.too_many(Instant::now(), ra, quota);
        log(&format!(
            "429 ({}), Retry-After {:?}: pausing {}",
            if quota { "quota" } else { "rate limit" },
            a.retry_after,
            limit::fmt_wait(until - Instant::now())
        ));
        self.emit(Ev::Paused {
            until,
            why: if quota { Why::Quota } else { Why::RateLimit },
        });
    }

    fn report_blocked(&self, b: Blocked, what: &'static str) {
        match b {
            Blocked::Paused { until, why } => self.emit(Ev::Paused { until, why }),
            Blocked::Budget { at } => self.emit(Ev::Busy { what, at }),
        }
    }

    /// One Web API call with every rule applied. A 401 refreshes the token
    /// and tries once more; nothing else is retried here.
    fn api(
        &mut self,
        method: &str,
        path: &str,
        body: Option<(&str, String)>,
        urgency: Urgency,
        what: &'static str,
    ) -> Outcome {
        if self.tokens.is_none() {
            self.emit(Ev::LoggedOut("Not logged in to Spotify.".into()));
            return Outcome::Stopped;
        }
        for attempt in 0..2 {
            let Some(token) = self.fresh_token() else {
                return Outcome::Stopped;
            };
            let now = Instant::now();
            if let Err(b) = self.limiter.begin(now, urgency) {
                self.report_blocked(b, what);
                return Outcome::Stopped;
            }
            let url = format!("{}{}", self.api_base, path);
            let a = match call(&self.agent, method, &url, Some(&token), body.clone()) {
                Ok(a) => a,
                Err(Fail::Net(e)) => {
                    let until = self.limiter.failed(now);
                    log(&format!("{what}: {e}"));
                    self.emit(Ev::Paused {
                        until,
                        why: Why::Offline,
                    });
                    return Outcome::Stopped;
                }
            };
            match a.status {
                200..=299 => {
                    self.limiter.ok();
                    return Outcome::Ok(a);
                }
                401 if attempt == 0 => {
                    // Expired early / revoked: one refresh, one more try.
                    if let Some(t) = &mut self.tokens {
                        t.expires_at = 0;
                    }
                    continue;
                }
                429 => {
                    self.too_many(&a);
                    return Outcome::Stopped;
                }
                500..=599 => {
                    let until = self.limiter.failed(now);
                    self.emit(Ev::Paused {
                        until,
                        why: Why::Offline,
                    });
                    return Outcome::Stopped;
                }
                s => {
                    self.limiter.ok();
                    let (msg, reason) = api::error_of(&a.body);
                    if s == 404 && reason.as_deref() == Some("NO_ACTIVE_DEVICE")
                        || s == 404 && what != "track" && what != "search" && what != "poll"
                    {
                        self.emit(Ev::NoDevice);
                    } else if s == 403 && what != "poll" {
                        self.emit(Ev::Error(format!(
                            "Spotify refused to {what}: {} (Premium is needed for playback control; \
                             in Development Mode your account must be added under User Management)",
                            msg.unwrap_or_else(|| "forbidden".into())
                        )));
                    } else if s == 401 {
                        self.tokens = None;
                        store::clear();
                        self.emit(Ev::LoggedOut(
                            "Spotify doesn't accept the login any more. Log in again.".into(),
                        ));
                    } else {
                        self.emit(Ev::Error(format!(
                            "Spotify {what}: {}",
                            msg.unwrap_or_else(|| format!("HTTP {s}"))
                        )));
                    }
                    return Outcome::Stopped;
                }
            }
        }
        Outcome::Stopped
    }
}

/// Merge a batch of queued requests: polls collapse to one (after any
/// controls, so it sees their effect), searches and seeks keep the last.
fn merge(batch: Vec<Req>) -> Vec<Req> {
    let mut out: Vec<Req> = Vec::new();
    let mut poll = false;
    let last_search = batch.iter().rposition(|r| matches!(r, Req::Search(_)));
    let last_seek = batch.iter().rposition(|r| matches!(r, Req::Seek(_)));
    for (i, r) in batch.into_iter().enumerate() {
        match r {
            Req::Poll => poll = true,
            Req::Search(_) if Some(i) != last_search => {}
            Req::Seek(_) if Some(i) != last_seek => {}
            r => out.push(r),
        }
    }
    if poll {
        out.push(Req::Poll);
    }
    out
}

fn log(msg: &str) {
    eprintln!("qwertone spotify: {msg}");
}

/// A login in progress: its own thread waits for the browser.
pub struct Login {
    pub url: String,
    pub rx: Receiver<Result<Tokens, String>>,
    cancel: Arc<AtomicBool>,
}

impl Login {
    /// Open the browser and wait (in the background) for Spotify to send
    /// it back, then swap the code for tokens.
    pub fn start(
        client_id: &str,
        port: u16,
        repaint: impl Fn() + Send + 'static,
    ) -> Result<Login, String> {
        let client_id = client_id.trim().to_string();
        if client_id.len() < 16 || !client_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(
                "That doesn't look like a Client ID (32 letters and digits from your app's page)."
                    .into(),
            );
        }
        let listener = auth::bind(port)?;
        let verifier = auth::new_verifier();
        let state = auth::random_text(16);
        let url = auth::authorize_url(&client_id, port, &auth::challenge(&verifier), &state);
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let _ = auth::open_browser(&url);
        std::thread::Builder::new()
            .name("qwertone-spotify-login".into())
            .spawn(move || {
                let res = auth::wait_for_code(listener, &state, c2, auth::LOGIN_TIMEOUT).and_then(
                    |code| {
                        let a = call(
                            &agent(),
                            "POST",
                            auth::TOKEN,
                            None,
                            Some((
                                "application/x-www-form-urlencoded",
                                auth::code_request(&client_id, port, &code, &verifier),
                            )),
                        )
                        .map_err(|Fail::Net(e)| format!("Couldn't reach Spotify: {e}"))?;
                        if a.status != 200 {
                            let (msg, _) = api::error_of(&a.body);
                            return Err(format!(
                                "Spotify didn't accept the login: {}. Check the Client ID and that \
                             {} is in the app's Redirect URIs.",
                                msg.unwrap_or_else(|| format!("HTTP {}", a.status)),
                                auth::redirect_uri(port)
                            ));
                        }
                        Tokens::from_response(&a.body, None, &client_id, auth::unix_now())
                    },
                );
                let _ = tx.send(res);
                repaint();
            })
            .map_err(|e| e.to_string())?;
        Ok(Login { url, rx, cancel })
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// A fake Web API: answers by path from a script the test changes, and
    /// logs every request it gets.
    struct Fake {
        port: u16,
        log: Arc<Mutex<Vec<String>>>,
        answer: Arc<Mutex<(u16, Vec<(String, String)>, String)>>,
    }

    fn fake() -> Fake {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(Mutex::new((200u16, vec![], String::new())));
        let (lg, an) = (log.clone(), answer.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut first = String::new();
                r.read_line(&mut first).unwrap();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                r.read_exact(&mut body).unwrap();
                lg.lock().unwrap().push(first.trim().to_string());
                let (st, hs, b) = an.lock().unwrap().clone();
                let mut resp = format!(
                    "HTTP/1.1 {st} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    b.len()
                );
                for (k, v) in hs {
                    resp += &format!("{k}: {v}\r\n");
                }
                resp += "\r\n";
                resp += &b;
                let _ = s.write_all(resp.as_bytes());
            }
        });
        Fake { port, log, answer }
    }

    fn wait_for(c: &Client, pred: impl Fn(&Ev) -> bool) -> Ev {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(e) = c.rx.recv_timeout(Duration::from_millis(100))
                && pred(&e)
            {
                return e;
            }
        }
        panic!("no such event");
    }

    fn logged_in(f: &Fake) -> Client {
        let c = Client::start_with(
            format!("http://127.0.0.1:{}/v1", f.port),
            "file".into(),
            || {},
        );
        c.send(Req::SetLogin(
            Box::new(Tokens {
                access_token: "AT".into(),
                refresh_token: "RT".into(),
                expires_at: auth::unix_now() + 3600,
                scope: String::new(),
                client_id: "cid".into(),
            }),
            "file".into(),
        ));
        c
    }

    #[test]
    fn a_429_pauses_everything_until_retry_after() {
        let f = fake();
        let c = logged_in(&f);
        *f.answer.lock().unwrap() = (
            200,
            vec![],
            r#"{"is_playing": true, "progress_ms": 1000, "item": {"id": "a", "uri": "u", "name": "N",
                "duration_ms": 5000, "artists": [], "album": {"name": ""}}}"#
                .into(),
        );
        c.send(Req::Poll);
        match wait_for(&c, |e| matches!(e, Ev::Playback { .. })) {
            Ev::Playback { pb: Some(pb), .. } => assert_eq!(pb.progress_ms, 1000),
            e => panic!("{e:?}"),
        }
        // Now Spotify says: quota used up, come back in 120 s.
        *f.answer.lock().unwrap() = (
            429,
            vec![("Retry-After".into(), "120".into())],
            r#"{"error": {"status": 429, "message": "Too many requests", "reason": "QUOTA_EXCEEDED"}}"#.into(),
        );
        let before = Instant::now();
        c.send(Req::Poll);
        match wait_for(&c, |e| matches!(e, Ev::Paused { .. })) {
            Ev::Paused { until, why } => {
                assert_eq!(why, limit::Why::Quota);
                assert!(until >= before + Duration::from_secs(120));
            }
            e => panic!("{e:?}"),
        }
        let n = f.log.lock().unwrap().len();
        assert_eq!(n, 2);
        // Everything after that is refused here, without a request.
        for r in [
            Req::Poll,
            Req::Search("x".into()),
            Req::Pause,
            Req::Seek(5),
            Req::Poll,
        ] {
            c.send(r);
            assert!(matches!(
                wait_for(&c, |e| matches!(e, Ev::Paused { .. })),
                Ev::Paused { .. }
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(f.log.lock().unwrap().len(), n, "no request while paused");
    }

    #[test]
    fn controls_and_search_hit_the_right_endpoints() {
        let f = fake();
        let c = logged_in(&f);
        *f.answer.lock().unwrap() = (204, vec![], String::new());
        c.send(Req::Pause);
        wait_for(&c, |e| matches!(e, Ev::Done("pause")));
        c.send(Req::Seek(61_000));
        wait_for(&c, |e| matches!(e, Ev::Done("seek")));
        *f.answer.lock().unwrap() = (200, vec![], r#"{"tracks": {"items": []}}"#.into());
        c.send(Req::Search("arabic kuthu".into()));
        wait_for(&c, |e| matches!(e, Ev::Search { .. }));
        // Nothing playing anywhere.
        *f.answer.lock().unwrap() = (
            404,
            vec![],
            r#"{"error": {"status": 404, "message": "No active device found", "reason": "NO_ACTIVE_DEVICE"}}"#.into(),
        );
        c.send(Req::Play {
            uri: Some("spotify:track:a".into()),
            position_ms: None,
            device: None,
        });
        wait_for(&c, |e| matches!(e, Ev::NoDevice));
        let log = f.log.lock().unwrap().clone();
        assert_eq!(log[0], "PUT /v1/me/player/pause HTTP/1.1");
        assert_eq!(log[1], "PUT /v1/me/player/seek?position_ms=61000 HTTP/1.1");
        assert_eq!(
            log[2],
            "GET /v1/search?type=track&limit=10&q=arabic%20kuthu HTTP/1.1"
        );
        assert_eq!(log[3], "PUT /v1/me/player/play HTTP/1.1");
    }

    #[test]
    fn queued_requests_are_merged() {
        let out = merge(vec![
            Req::Poll,
            Req::Search("a".into()),
            Req::Seek(1000),
            Req::Poll,
            Req::Pause,
            Req::Search("ab".into()),
            Req::Seek(2000),
            Req::Poll,
        ]);
        let names: Vec<String> = out.iter().map(|r| format!("{r:?}")).collect();
        assert_eq!(
            names,
            vec![
                "Pause".to_string(),
                "Search(\"ab\")".into(),
                "Seek(2000)".into(),
                "Poll".into()
            ]
        );
    }
}
