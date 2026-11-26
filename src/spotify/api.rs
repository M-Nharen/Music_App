//! The few Web API objects Qwertone uses, read from JSON (pure, tested).
//! Only endpoints still open to Development Mode apps are used:
//! `GET /search` (limit ≤ 10), `GET /me/player`, `PUT /me/player/play`,
//! `PUT /me/player/pause`, `PUT /me/player/seek`, `GET /me/player/devices`,
//! `PUT /me/player` (transfer) and `GET /tracks/{id}`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API: &str = "https://api.spotify.com/v1";
/// Development Mode caps `limit` at 10.
pub const SEARCH_LIMIT: u32 = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Track {
    /// None for local files in your Spotify library.
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub isrc: Option<String>,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }

    pub fn duration_s(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }

    pub fn from_json(v: &Value) -> Option<Track> {
        if v.is_null() {
            return None;
        }
        let name = v.get("name")?.as_str()?.to_string();
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
        Some(Track {
            id: s("id"),
            uri: s("uri").unwrap_or_default(),
            name,
            artists: v
                .get("artists")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                        .map(|n| n.to_string())
                        .collect()
                })
                .unwrap_or_default(),
            album: v
                .pointer("/album/name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            duration_ms: v.get("duration_ms").and_then(|x| x.as_u64()).unwrap_or(0),
            isrc: v
                .pointer("/external_ids/isrc")
                .and_then(|x| x.as_str())
                .map(|x| x.to_string()),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    /// "Computer", "Smartphone", "Speaker", …
    pub kind: String,
    pub active: bool,
    pub restricted: bool,
}

impl Device {
    pub fn from_json(v: &Value) -> Option<Device> {
        Some(Device {
            id: v.get("id").and_then(|x| x.as_str()).map(|x| x.to_string()),
            name: v.get("name")?.as_str()?.to_string(),
            kind: v
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            active: v.get("is_active").and_then(|x| x.as_bool()).unwrap_or(false),
            restricted: v
                .get("is_restricted")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        })
    }

    /// Spotify on a computer (probably this one, through the same output
    /// as Qwertone, so the Bluetooth delay applies to it too).
    pub fn is_computer(&self) -> bool {
        self.kind.eq_ignore_ascii_case("computer")
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Playback {
    pub playing: bool,
    pub progress_ms: u64,
    /// None for ads / podcasts / nothing loaded.
    pub track: Option<Track>,
    pub device: Option<Device>,
    /// "track", "episode", "ad", "unknown".
    pub kind: String,
}

impl Playback {
    pub fn from_json(v: &Value) -> Option<Playback> {
        if !v.is_object() {
            return None;
        }
        Some(Playback {
            playing: v.get("is_playing").and_then(|x| x.as_bool()).unwrap_or(false),
            progress_ms: v.get("progress_ms").and_then(|x| x.as_u64()).unwrap_or(0),
            track: v.get("item").and_then(Track::from_json),
            device: v.get("device").and_then(Device::from_json),
            kind: v
                .get("currently_playing_type")
                .and_then(|x| x.as_str())
                .unwrap_or("track")
                .to_string(),
        })
    }
}

pub fn parse_search(v: &Value) -> Vec<Track> {
    v.pointer("/tracks/items")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(Track::from_json).collect())
        .unwrap_or_default()
}

pub fn parse_devices(v: &Value) -> Vec<Device> {
    v.get("devices")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(Device::from_json).collect())
        .unwrap_or_default()
}

/// `{"error": {"status": 404, "message": "…", "reason": "NO_ACTIVE_DEVICE"}}`
pub fn error_of(body: &str) -> (Option<String>, Option<String>) {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let e = v.get("error");
    // The token endpoint uses {"error": "invalid_grant", "error_description": …}.
    if let Some(s) = e.and_then(|x| x.as_str()) {
        return (
            v.get("error_description")
                .and_then(|x| x.as_str())
                .map(|d| format!("{s}: {d}"))
                .or(Some(s.to_string())),
            Some(s.to_string()),
        );
    }
    (
        e.and_then(|x| x.get("message"))
            .and_then(|x| x.as_str())
            .map(|x| x.to_string()),
        e.and_then(|x| x.get("reason"))
            .and_then(|x| x.as_str())
            .map(|x| x.to_string()),
    )
}

/// Spotify's search syntax: `track:… artist:…` narrows it down.
pub fn search_query(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: &str = r#"{
      "device": {"id": "dev1", "is_active": true, "is_restricted": false,
                 "name": "arch-laptop", "type": "Computer", "volume_percent": 70},
      "shuffle_state": false, "repeat_state": "off", "timestamp": 1764160000000,
      "progress_ms": 61234, "is_playing": true, "currently_playing_type": "track",
      "item": {"id": "4uLU6hMCjMI75M1A2tKUQC", "uri": "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
               "name": "Kannaana Kanney", "duration_ms": 263000,
               "artists": [{"name": "D. Imman"}, {"name": "Sid Sriram"}],
               "album": {"name": "Viswasam"}, "external_ids": {"isrc": "INS181900123"}}
    }"#;

    #[test]
    fn player_state() {
        let v: Value = serde_json::from_str(PLAYER).unwrap();
        let p = Playback::from_json(&v).unwrap();
        assert!(p.playing);
        assert_eq!(p.progress_ms, 61234);
        let t = p.track.unwrap();
        assert_eq!(t.name, "Kannaana Kanney");
        assert_eq!(t.artist_line(), "D. Imman, Sid Sriram");
        assert_eq!(t.album, "Viswasam");
        assert_eq!(t.duration_ms, 263000);
        assert_eq!(t.isrc.as_deref(), Some("INS181900123"));
        let d = p.device.unwrap();
        assert!(d.is_computer() && d.active);
        // An ad: no item.
        let ad: Value = serde_json::from_str(
            r#"{"is_playing": true, "progress_ms": 3000, "item": null,
                "currently_playing_type": "ad", "device": {"name": "phone", "type": "Smartphone"}}"#,
        )
        .unwrap();
        let p = Playback::from_json(&ad).unwrap();
        assert!(p.track.is_none() && p.kind == "ad");
    }

    #[test]
    fn search_and_devices() {
        let v: Value = serde_json::from_str(
            r#"{"tracks": {"items": [
                {"id": "a", "uri": "spotify:track:a", "name": "One", "duration_ms": 1000,
                 "artists": [{"name": "X"}], "album": {"name": "Al"}},
                {"id": null, "uri": "spotify:local:::Song:200", "name": "Local", "duration_ms": 200000,
                 "artists": [], "album": {"name": ""}, "is_local": true}
            ]}}"#,
        )
        .unwrap();
        let r = parse_search(&v);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].id, None);
        let d: Value = serde_json::from_str(
            r#"{"devices": [{"id": "d", "name": "Web Player", "type": "Computer", "is_active": false}]}"#,
        )
        .unwrap();
        assert_eq!(parse_devices(&d)[0].name, "Web Player");
        assert!(parse_search(&Value::Null).is_empty());
    }

    #[test]
    fn errors() {
        assert_eq!(
            error_of(r#"{"error": {"status": 404, "message": "No active device found", "reason": "NO_ACTIVE_DEVICE"}}"#),
            (
                Some("No active device found".into()),
                Some("NO_ACTIVE_DEVICE".into())
            )
        );
        assert_eq!(
            error_of(r#"{"error": "invalid_grant", "error_description": "Refresh token revoked"}"#).1,
            Some("invalid_grant".into())
        );
        assert_eq!(error_of("<html>"), (None, None));
    }
}
