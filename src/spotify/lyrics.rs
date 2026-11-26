//! Lyrics for whatever Spotify plays, looked up by Spotify's title / artist
//! / album / duration. Spotify's own lyrics can't be used (no API, and
//! they're licensed to Spotify), so this asks LRCLIB like the analyzer does.
//!
//! - **With the analyzer** (preferred): a tiny stand-in song folder per
//!   track in `~/.cache/qwertone/spotify/lyrics/<track id>/` (a `song.json`
//!   with just the metadata and length, no audio, no parts) and
//!   `qwertone-analyze lyrics <folder>`: LRCLIB, then romanisation
//!   (Tanglish / Tenglish / Hinglish …) and the English translation, exactly
//!   as for analysed songs.
//! - **Without it**: LRCLIB straight from here; the lines are shown in
//!   their own script (no romanisation).
//!
//! Either way the result is a `lyrics.json` in that folder, so a track is
//! looked up once. "Not found" answers are asked again after a week.

use super::api::Track;
use crate::song::lyrics::{Lyrics, Source, Sources, parse_lrc};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const LRCLIB: &str = "https://lrclib.net/api";
/// Re-ask after this long when nothing was found.
pub const MISS_TTL_SECS: u64 = 7 * 24 * 3600;

pub fn root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("qwertone")
        .join("spotify")
        .join("lyrics")
}

/// Folder for a track (ids are base62; anything else is dropped).
pub fn dir_for(track: &Track) -> Option<PathBuf> {
    let id: String = track
        .id
        .as_deref()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    (!id.is_empty()).then(|| root().join(id))
}

/// The stand-in `song.json` the analyzer's lyrics step reads.
pub fn stub_song(track: &Track) -> Value {
    let id = track.id.clone().unwrap_or_default();
    json!({
        "format": "qwertone-song",
        "format_version": 1,
        "source": {
            "kind": "spotify",
            "file": format!("{} - {}", track.artist_line(), track.name),
            "sha256": format!("spotify-{id}"),
            "spotify_uri": track.uri,
        },
        "metadata": {
            "title": track.name,
            "artist": track.artists.first().cloned(),
            "artists": track.artists,
            "album": track.album,
        },
        "duration": track.duration_s(),
        "beats": [],
        "parts": [],
        "chords": [],
    })
}

/// Make the stand-in folder (idempotent).
pub fn prepare(track: &Track) -> Result<PathBuf, String> {
    let dir = dir_for(track).ok_or("this track has no Spotify id (a local file?)")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("song.json");
    let body = serde_json::to_string_pretty(&stub_song(track)).map_err(|e| e.to_string())?;
    if std::fs::read_to_string(&p).ok().as_deref() != Some(body.as_str()) {
        std::fs::write(&p, body).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Already looked up (and not a stale miss)?
pub fn cached(dir: &Path) -> Option<Lyrics> {
    let doc = Lyrics::load(dir).ok()??;
    if doc.status == "not_found" {
        let age = std::fs::metadata(dir.join(crate::song::lyrics::FILE))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .unwrap_or_default();
        if age > Duration::from_secs(MISS_TTL_SECS) {
            return None;
        }
    }
    Some(doc)
}

/// The main language of some text, from its script (for lyrics fetched
/// without the analyzer). Latin text → None (Tanglish or English: can't tell).
pub fn script_language(text: &str) -> Option<&'static str> {
    let mut counts: [(u32, &str); 9] = [
        (0, "ta"),
        (0, "te"),
        (0, "hi"),
        (0, "kn"),
        (0, "ml"),
        (0, "bn"),
        (0, "pa"),
        (0, "gu"),
        (0, "or"),
    ];
    for c in text.chars() {
        let i = match c as u32 {
            0x0B80..=0x0BFF => 0,
            0x0C00..=0x0C7F => 1,
            0x0900..=0x097F => 2,
            0x0C80..=0x0CFF => 3,
            0x0D00..=0x0D7F => 4,
            0x0980..=0x09FF => 5,
            0x0A00..=0x0A7F => 6,
            0x0A80..=0x0AFF => 7,
            0x0B00..=0x0B7F => 8,
            _ => continue,
        };
        counts[i].0 += 1;
    }
    counts
        .iter()
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, l)| *l)
}

/// LRCLIB's best record for a track: `/get` (exact title / artist / album
/// / duration), else `/search`, picking the closest length within 3 s.
/// Ok(None) = nothing there.
pub fn lrclib_lookup(agent: &ureq::Agent, track: &Track) -> Result<Option<Value>, String> {
    let enc = super::auth::enc;
    let artist = track.artists.first().cloned().unwrap_or_default();
    let secs = track.duration_s().round() as u64;
    let url = format!(
        "{LRCLIB}/get?track_name={}&artist_name={}&album_name={}&duration={secs}",
        enc(&track.name),
        enc(&artist),
        enc(&track.album)
    );
    let get = |url: &str| -> Result<(u16, String), String> {
        let resp = agent
            .get(url)
            .header("User-Agent", "Qwertone (https://github.com/M-Nharen/Music_App)")
            .call()
            .map_err(|e| format!("LRCLIB: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .read_to_string()
            .map_err(|e| format!("LRCLIB: {e}"))?;
        Ok((status, body))
    };
    let (st, body) = get(&url)?;
    if st == 200
        && let Ok(v) = serde_json::from_str::<Value>(&body)
    {
        return Ok(Some(v));
    }
    if st != 404 {
        return Err(format!("LRCLIB answered {st}"));
    }
    let title = super::matching::norm_title(&track.name);
    let url = format!(
        "{LRCLIB}/search?track_name={}&artist_name={}",
        enc(&title),
        enc(&artist)
    );
    let (st, body) = get(&url)?;
    if st != 200 {
        return Err(format!("LRCLIB answered {st}"));
    }
    let list: Vec<Value> = serde_json::from_str(&body).unwrap_or_default();
    Ok(pick_record(list, track.duration_s()))
}

/// From search results: synced first, then the closest length (≤ 3 s off).
pub fn pick_record(list: Vec<Value>, secs: f64) -> Option<Value> {
    let mut best: Option<(bool, f64, Value)> = None;
    for r in list {
        let d = r
            .get("duration")
            .and_then(|x| x.as_f64())
            .map(|x| (x - secs).abs())
            .unwrap_or(99.0);
        if d > 3.0 {
            continue;
        }
        let synced = r
            .get("syncedLyrics")
            .and_then(|x| x.as_str())
            .is_some_and(|s| !s.trim().is_empty());
        let better = match &best {
            None => true,
            Some((bs, bd, _)) => (synced && !bs) || (synced == *bs && d < *bd),
        };
        if better {
            best = Some((synced, d, r));
        }
    }
    best.map(|b| b.2)
}

/// A `lyrics.json` from an LRCLIB record (or a miss), without the analyzer.
pub fn lyrics_from_record(rec: Option<&Value>, track: &Track) -> Lyrics {
    let text = rec
        .and_then(|r| {
            r.get("syncedLyrics")
                .and_then(|x| x.as_str())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| r.get("plainLyrics").and_then(|x| x.as_str()))
        })
        .unwrap_or("");
    let lang = script_language(text);
    let mut doc = parse_lrc(text, lang.unwrap_or("und"));
    for l in &mut doc.lines {
        // Latin lines: Tanglish or English, can't tell; leave it open.
        l.language = script_language(&l.text).map(|s| s.to_string());
    }
    doc.language = lang.map(|s| s.to_string());
    doc.languages = lang.map(|s| vec![s.to_string()]).unwrap_or_default();
    let instrumental = rec
        .and_then(|r| r.get("instrumental"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if doc.lines.is_empty() {
        doc.status = if instrumental { "instrumental" } else { "not_found" }.into();
    }
    if let Some(r) = rec {
        let mut extra = serde_json::Map::new();
        for k in ["id", "trackName", "artistName", "albumName", "duration"] {
            if let Some(v) = r.get(k) {
                extra.insert(k.into(), v.clone());
            }
        }
        let src = Source {
            kind: "lrclib".into(),
            name: "LRCLIB".into(),
            url: r
                .get("id")
                .map(|id| format!("https://lrclib.net/api/get/{id}")),
            file: None,
            fetched: Some(crate::song::lyrics::now_iso()),
            extra,
        };
        doc.sources = Sources {
            words: Some(src.clone()),
            timing: doc.synced().then_some(src),
            ..Default::default()
        };
    } else {
        doc.sources = Sources::default();
    }
    doc.extra.insert(
        "song".into(),
        json!({"title": track.name, "artist": track.artists.first(), "album": track.album,
               "duration": track.duration_s(), "spotify_uri": track.uri}),
    );
    doc.format = "qwertone-lyrics".into();
    doc.format_version = 1;
    doc
}

pub fn write(dir: &Path, doc: &Lyrics) -> Result<(), String> {
    let body = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    let p = dir.join(crate::song::lyrics::FILE);
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        Track {
            id: Some("abc123".into()),
            uri: "spotify:track:abc123".into(),
            name: "Kannaana Kanney".into(),
            artists: vec!["D. Imman".into(), "Sid Sriram".into()],
            album: "Viswasam".into(),
            duration_ms: 263_400,
            isrc: None,
        }
    }

    #[test]
    fn stub_has_what_the_analyzer_needs() {
        let v = stub_song(&track());
        assert_eq!(v["format"], "qwertone-song");
        assert_eq!(v["metadata"]["title"], "Kannaana Kanney");
        assert_eq!(v["metadata"]["artist"], "D. Imman");
        assert_eq!(v["duration"], 263.4);
        assert_eq!(v["source"]["sha256"], "spotify-abc123");
        assert!(dir_for(&track()).unwrap().ends_with("abc123"));
        let mut t = track();
        t.id = None;
        assert!(dir_for(&t).is_none());
    }

    #[test]
    fn records() {
        let list: Vec<Value> = serde_json::from_str(
            r#"[{"id":1,"duration":300,"syncedLyrics":"[00:01.00]x"},
                {"id":2,"duration":264,"plainLyrics":"x"},
                {"id":3,"duration":262,"syncedLyrics":"[00:01.00]y"}]"#,
        )
        .unwrap();
        assert_eq!(pick_record(list, 263.4).unwrap()["id"], 3);
        assert!(pick_record(vec![], 10.0).is_none());

        let rec: Value = serde_json::from_str(
            r#"{"id": 77, "trackName": "Kannaana Kanney", "duration": 263,
                "syncedLyrics": "[00:12.00]கண்ணான கண்ணே\n[00:18.50]நீ கலங்காதடி\n[00:25.00]oh oh"}"#,
        )
        .unwrap();
        let d = lyrics_from_record(Some(&rec), &track());
        assert_eq!(d.status, "synced");
        assert_eq!(d.language.as_deref(), Some("ta"));
        assert_eq!(d.lines.len(), 3);
        assert_eq!(d.lines[1].start, Some(18.5));
        assert_eq!(d.lines[2].language, None);
        assert_eq!(d.sources.words.as_ref().unwrap().kind, "lrclib");
        assert!(d.sources.timing.is_some());
        // Round trip through lyrics.json.
        let back = Lyrics::from_json(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back.lines.len(), 3);

        let miss = lyrics_from_record(None, &track());
        assert_eq!(miss.status, "not_found");
        let inst: Value = serde_json::from_str(r#"{"id": 5, "instrumental": true}"#).unwrap();
        assert_eq!(lyrics_from_record(Some(&inst), &track()).status, "instrumental");
    }

    #[test]
    fn scripts() {
        assert_eq!(script_language("నువ్వు"), Some("te"));
        assert_eq!(script_language("तुम ही हो"), Some("hi"));
        assert_eq!(script_language("nee enakku"), None);
    }
}
