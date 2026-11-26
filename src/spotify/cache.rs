//! Search results and track info, kept on disk so the same search or track
//! never costs another API call (and still works while Spotify is paused
//! or offline). `~/.cache/qwertone/spotify.json`.

use super::api::Track;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Searches are answered from the cache for this long.
pub const SEARCH_TTL_SECS: i64 = 7 * 24 * 3600;
pub const MAX_SEARCHES: usize = 300;
pub const MAX_TRACKS: usize = 3000;

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Search {
    pub at: i64,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Cache {
    pub searches: BTreeMap<String, Search>,
    /// Track id → track, with when it was last seen (for trimming).
    pub tracks: BTreeMap<String, (i64, Track)>,
    #[serde(skip)]
    pub dirty: bool,
}

pub fn path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("qwertone")
        .join("spotify.json")
}

/// Same search, same key: case, spaces and punctuation don't matter.
pub fn key(q: &str) -> String {
    q.to_lowercase()
        .split(|c: char| c.is_whitespace() || (c.is_ascii_punctuation() && c != ':'))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Cache {
    pub fn load() -> Cache {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(body) = serde_json::to_vec(self) {
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &p).is_ok() {
                self.dirty = false;
            }
        }
    }

    /// Cached results for a search, if fresh (or any age when `stale_ok`,
    /// e.g. while Spotify is paused).
    pub fn search(&self, q: &str, now: i64, stale_ok: bool) -> Option<&Search> {
        self.searches
            .get(&key(q))
            .filter(|s| stale_ok || now - s.at < SEARCH_TTL_SECS)
    }

    pub fn put_search(&mut self, q: &str, tracks: Vec<Track>, now: i64) {
        for t in &tracks {
            self.put_track(t.clone(), now);
        }
        self.searches.insert(key(q), Search { at: now, tracks });
        if self.searches.len() > MAX_SEARCHES {
            let mut by_age: Vec<(i64, String)> =
                self.searches.iter().map(|(k, s)| (s.at, k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(self.searches.len() - MAX_SEARCHES) {
                self.searches.remove(&k);
            }
        }
        self.dirty = true;
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.get(id).map(|(_, t)| t)
    }

    pub fn put_track(&mut self, t: Track, now: i64) {
        let Some(id) = t.id.clone() else { return };
        if self.tracks.get(&id).is_some_and(|(_, old)| *old == t) {
            return;
        }
        self.tracks.insert(id, (now, t));
        if self.tracks.len() > MAX_TRACKS {
            let mut by_age: Vec<(i64, String)> =
                self.tracks.iter().map(|(k, (at, _))| (*at, k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(self.tracks.len() - MAX_TRACKS) {
                self.tracks.remove(&k);
            }
        }
        self.dirty = true;
    }

    /// Search the cached tracks without Spotify (offline): every word of the
    /// query in the title / artists / album.
    pub fn offline_search(&self, q: &str, limit: usize) -> Vec<Track> {
        let words: Vec<String> = key(q)
            .split(' ')
            .filter(|w| !w.contains(':'))
            .map(|w| w.to_string())
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(i64, Track)> = self
            .tracks
            .values()
            .filter(|(_, t)| {
                let hay = key(&format!("{} {} {}", t.name, t.artist_line(), t.album));
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .cloned()
            .collect();
        out.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        out.into_iter().take(limit).map(|(_, t)| t).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: &str, name: &str) -> Track {
        Track {
            id: Some(id.into()),
            uri: format!("spotify:track:{id}"),
            name: name.into(),
            artists: vec!["Anirudh".into()],
            album: "Album".into(),
            duration_ms: 200_000,
            isrc: None,
        }
    }

    #[test]
    fn searches_are_reused_and_expire() {
        let mut c = Cache::default();
        c.put_search("Arabic  Kuthu!", vec![tr("a", "Arabic Kuthu")], 1000);
        assert!(c.search("arabic kuthu", 1000 + 60, false).is_some());
        assert!(c.search("arabic kuthu", 1000 + SEARCH_TTL_SECS + 1, false).is_none());
        assert!(c.search("arabic kuthu", 1000 + SEARCH_TTL_SECS + 1, true).is_some());
        assert_eq!(c.track("a").unwrap().name, "Arabic Kuthu");
        assert!(c.dirty);
        // Offline search over what we've seen.
        assert_eq!(c.offline_search("kuthu anirudh", 10).len(), 1);
        assert!(c.offline_search("vaathi", 10).is_empty());
    }

    #[test]
    fn trims_the_oldest() {
        let mut c = Cache::default();
        for i in 0..(MAX_SEARCHES + 5) {
            c.put_search(&format!("q{i}"), vec![], i as i64);
        }
        assert_eq!(c.searches.len(), MAX_SEARCHES);
        assert!(c.searches.contains_key("q304") && !c.searches.contains_key("q0"));
    }
}
