//! Logging in with the Authorization Code flow + PKCE (no client secret).
//!
//! 1. Make a random `code_verifier` and its SHA-256 `code_challenge`.
//! 2. Open `accounts.spotify.com/authorize` in the browser.
//! 3. Spotify sends the browser back to `http://127.0.0.1:<port>/callback`
//!    (a loopback IP literal: Spotify doesn't allow `localhost`), where a
//!    tiny one-shot HTTP server here picks up `code` and checks `state`.
//! 4. Swap the code (+ verifier) for an access token and a refresh token.
//!
//! The HTTP parts are pure functions over strings so they're tested.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const AUTHORIZE: &str = "https://accounts.spotify.com/authorize";
pub const TOKEN: &str = "https://accounts.spotify.com/api/token";
/// Read playback (now playing, sync) and control it. Search needs no scope.
pub const SCOPES: &str =
    "user-read-playback-state user-modify-playback-state user-read-currently-playing";
/// How long the login page waits for you.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scope: String,
    /// The app (Client ID) these belong to.
    #[serde(default)]
    pub client_id: String,
}

// Never print tokens (logs, panics).
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Tokens {
    /// Refresh a minute early.
    pub fn expired(&self, now_unix: i64) -> bool {
        now_unix >= self.expires_at - 60
    }

    /// From the token endpoint's JSON. A refresh answer may leave out the
    /// refresh token (then the old one stays valid) or send a new one.
    pub fn from_response(
        body: &str,
        old_refresh: Option<&str>,
        client_id: &str,
        now_unix: i64,
    ) -> Result<Tokens, String> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("bad token answer: {e}"))?;
        let access = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .ok_or("no access_token in the answer")?;
        let refresh = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .or(old_refresh)
            .ok_or("no refresh_token in the answer")?;
        let expires_in = v.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(3600);
        Ok(Tokens {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_at: now_unix + expires_in,
            scope: v
                .get("scope")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            client_id: client_id.to_string(),
        })
    }
}

/// Random URL-safe text from `n` random bytes.
pub fn random_text(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("the OS random number generator failed");
    URL_SAFE_NO_PAD.encode(&buf)
}

/// `code_verifier`: 43–128 characters of [A-Za-z0-9-._~].
pub fn new_verifier() -> String {
    random_text(64) // 86 characters
}

pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// Percent-encode a query / form value (RFC 3986 unreserved kept).
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn authorize_url(client_id: &str, port: u16, challenge: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE}?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("scope", SCOPES),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge),
            ("redirect_uri", &redirect_uri(port)),
            ("state", state),
        ])
    )
}

pub fn code_request(client_id: &str, port: u16, code: &str, verifier: &str) -> String {
    form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect_uri(port)),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ])
}

pub fn refresh_request(client_id: &str, refresh_token: &str) -> String {
    form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])
}

#[derive(Debug, PartialEq)]
pub enum Callback {
    Code(String),
    /// Spotify said no (`error=access_denied` when you press Cancel).
    Denied(String),
    /// Some other request (favicon…): ignore.
    Other,
}

/// Read the request line `GET /callback?code=…&state=… HTTP/1.1`.
pub fn parse_callback(request_line: &str, want_state: &str) -> Result<Callback, String> {
    let mut parts = request_line.split_whitespace();
    let (Some(_method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(Callback::Other);
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        return Ok(Callback::Other);
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "code" => code = Some(dec(v)),
            "state" => state = Some(dec(v)),
            "error" => error = Some(dec(v)),
            _ => {}
        }
    }
    if state.as_deref() != Some(want_state) {
        return Err("the login answer didn't match (state mismatch); try again".into());
    }
    if let Some(e) = error {
        return Ok(Callback::Denied(e));
    }
    code.map(Callback::Code)
        .ok_or_else(|| "Spotify sent no code back".to_string())
}

const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>Qwertone</title>\
<body style=\"font-family:sans-serif;background:#1c1b1f;color:#e6e1e5;text-align:center;padding-top:15vh\">\
<h1>&#9835; Logged in to Spotify</h1><p>You can close this tab and go back to Qwertone.</p>";
const PAGE_NO: &str = "<!doctype html><meta charset=utf-8><title>Qwertone</title>\
<body style=\"font-family:sans-serif;background:#1c1b1f;color:#e6e1e5;text-align:center;padding-top:15vh\">\
<h1>Not logged in</h1><p>You can close this tab. Qwertone says why.</p>";

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// Bind the callback port first (so a clash is reported before the browser
/// opens).
pub fn bind(port: u16) -> Result<TcpListener, String> {
    let l = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!(
            "Couldn't listen on 127.0.0.1:{port} for the login ({e}). Is another Qwertone logging in? \
             You can pick another port in the Spotify settings (and add it to your app's redirect URIs)."
        )
    })?;
    l.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(l)
}

/// Wait for the browser to come back with the code.
pub fn wait_for_code(
    listener: TcpListener,
    state: &str,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Login cancelled.".into());
        }
        if Instant::now() > deadline {
            return Err("Login timed out (nothing came back from the browser).".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                match parse_callback(&line, state) {
                    Ok(Callback::Other) => respond(&mut stream, "404 Not Found", ""),
                    Ok(Callback::Code(c)) => {
                        respond(&mut stream, "200 OK", PAGE_OK);
                        return Ok(c);
                    }
                    Ok(Callback::Denied(e)) => {
                        respond(&mut stream, "200 OK", PAGE_NO);
                        return Err(if e == "access_denied" {
                            "You cancelled the login on Spotify's page.".into()
                        } else {
                            format!("Spotify refused the login: {e}")
                        });
                    }
                    Err(e) => {
                        respond(&mut stream, "400 Bad Request", PAGE_NO);
                        return Err(e);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("login callback failed: {e}")),
        }
    }
}

/// Open a URL in the default browser.
pub fn open_browser(url: &str) -> Result<(), String> {
    for cmd in ["xdg-open", "gio", "firefox", "chromium"] {
        let mut c = std::process::Command::new(cmd);
        if cmd == "gio" {
            c.arg("open");
        }
        if c.arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("Couldn't open a browser (xdg-open). Copy the link from the Spotify tab instead.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(challenge(v), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = new_verifier();
        assert!((43..=128).contains(&v.len()));
        assert!(
            v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        );
        assert_ne!(new_verifier(), v);
    }

    #[test]
    fn urls_and_forms() {
        let u = authorize_url("abc123", 8898, "CH", "ST");
        assert!(u.starts_with(
            "https://accounts.spotify.com/authorize?response_type=code&client_id=abc123"
        ));
        assert!(u.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fcallback"));
        assert!(u.contains("scope=user-read-playback-state%20user-modify-playback-state"));
        assert!(u.contains("code_challenge_method=S256&code_challenge=CH"));
        assert_eq!(dec(&enc("a b/ç?&=")), "a b/ç?&=");
        assert_eq!(dec("a%2"), "a%2");
        assert_eq!(dec("50%"), "50%");
        assert!(refresh_request("id", "r t").contains("refresh_token=r%20t"));
    }

    #[test]
    fn callback_parsing() {
        let ok = parse_callback("GET /callback?code=AQB%2Bx&state=s1 HTTP/1.1\r\n", "s1");
        assert_eq!(ok, Ok(Callback::Code("AQB+x".into())));
        assert!(parse_callback("GET /callback?code=x&state=evil HTTP/1.1", "s1").is_err());
        assert_eq!(
            parse_callback("GET /callback?error=access_denied&state=s1 HTTP/1.1", "s1"),
            Ok(Callback::Denied("access_denied".into()))
        );
        assert_eq!(
            parse_callback("GET /favicon.ico HTTP/1.1", "s1"),
            Ok(Callback::Other)
        );
    }

    #[test]
    fn token_answers() {
        let t = Tokens::from_response(
            r#"{"access_token":"ACCESSXYZ","token_type":"Bearer","expires_in":3600,"refresh_token":"REFRESHXYZ","scope":"x"}"#,
            None,
            "cid",
            1000,
        )
        .unwrap();
        assert_eq!(
            (t.access_token.as_str(), t.refresh_token.as_str()),
            ("ACCESSXYZ", "REFRESHXYZ")
        );
        assert_eq!(t.expires_at, 4600);
        assert!(!t.expired(4000) && t.expired(4541));
        // A refresh without a new refresh token keeps the old one.
        let r = Tokens::from_response(
            r#"{"access_token":"B","expires_in":3600}"#,
            Some("R"),
            "cid",
            0,
        )
        .unwrap();
        assert_eq!(r.refresh_token, "R");
        assert!(Tokens::from_response("{}", None, "cid", 0).is_err());
        // Debug never shows the secrets.
        let shown = format!("{t:?}");
        assert!(!shown.contains("ACCESSXYZ") && !shown.contains("REFRESHXYZ"));
    }

    #[test]
    fn one_shot_server() {
        let l = bind(0).unwrap();
        let port = l.local_addr().unwrap().port();
        let cancel = Arc::new(AtomicBool::new(false));
        let h = std::thread::spawn(move || wait_for_code(l, "st", cancel, Duration::from_secs(5)));
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "GET /callback?code=abc&state=st HTTP/1.1\r\nHost: x\r\n\r\n"
        )
        .unwrap();
        let mut reply = String::new();
        let _ = std::io::Read::read_to_string(&mut s, &mut reply);
        assert!(reply.starts_with("HTTP/1.1 200"));
        assert_eq!(h.join().unwrap(), Ok("abc".into()));
    }
}
