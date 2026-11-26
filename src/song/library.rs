//! Finding songs: analyzer song folders and MIDI files.

use super::Song;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    /// Analyzer mode ("chords+melody") or "MIDI".
    pub mode: String,
    pub duration: f64,
    pub parts: Vec<String>,
    pub low_confidence: usize,
    pub has_stems: bool,
    pub modified: SystemTime,
    /// Couldn't be read (shown greyed with the reason).
    pub error: Option<String>,
}

fn modified(p: &Path) -> SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Every `song.json` folder in `dir`, newest first.
pub fn scan_songs(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let path = e.path();
        let json = path.join("song.json");
        if !json.is_file() {
            continue;
        }
        let mtime = modified(&json);
        match Song::load_dir(&path) {
            Ok(s) => out.push(Entry {
                title: s.metadata.title.clone(),
                artist: s.metadata.artist.clone(),
                mode: if s.analyzer.mode.is_empty() {
                    s.source.kind.clone()
                } else {
                    s.analyzer.mode.clone()
                },
                duration: s.duration,
                parts: s.parts.iter().map(|p| p.label()).collect(),
                low_confidence: s.parts.iter().map(|p| p.low_confidence_count()).sum(),
                has_stems: !s.stems.is_empty(),
                modified: mtime,
                error: None,
                path,
            }),
            Err(err) => out.push(Entry {
                title: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                artist: None,
                mode: String::new(),
                duration: 0.0,
                parts: vec![],
                low_confidence: 0,
                has_stems: false,
                modified: mtime,
                error: Some(err),
                path,
            }),
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.modified));
    out
}

pub fn is_midi(p: &Path) -> bool {
    p.extension()
        .map(|e| {
            matches!(
                e.to_string_lossy().to_lowercase().as_str(),
                "mid" | "midi" | "kar"
            )
        })
        .unwrap_or(false)
}

pub fn is_audio(p: &Path) -> bool {
    p.extension()
        .map(|e| {
            matches!(
                e.to_string_lossy().to_lowercase().as_str(),
                "mp3" | "flac" | "wav" | "m4a" | "ogg" | "opus" | "aac" | "wma" | "aiff" | "aif"
            )
        })
        .unwrap_or(false)
}

/// MIDI files under `dirs` (a few levels deep, skipping hidden folders and
/// song folders), newest first, at most `limit`.
pub fn scan_midi(dirs: &[PathBuf], depth: usize, limit: usize) -> Vec<Entry> {
    let mut found: Vec<PathBuf> = Vec::new();
    fn walk(d: &Path, depth: usize, found: &mut Vec<PathBuf>, limit: usize) {
        if found.len() >= limit * 4 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth > 0 && !p.join("song.json").exists() {
                    walk(&p, depth - 1, found, limit);
                }
            } else if is_midi(&p) {
                found.push(p);
            }
        }
    }
    for d in dirs {
        walk(d, depth, &mut found, limit);
    }
    found.sort();
    found.dedup();
    let mut out: Vec<Entry> = found
        .into_iter()
        .map(|path| Entry {
            title: path
                .file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            artist: None,
            mode: "MIDI".into(),
            duration: 0.0,
            parts: vec![],
            low_confidence: 0,
            has_stems: false,
            modified: modified(&path),
            error: None,
            path,
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.modified));
    out.truncate(limit);
    out
}

/// Where to look for MIDI files: ~/Music, ~/Downloads, the songs' parent
/// folder (`~/.local/share/qwertone/midi`).
pub fn midi_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    v.extend(dirs::audio_dir());
    v.extend(dirs::download_dir());
    if let Some(parent) = super::songs_dir().parent() {
        v.push(parent.join("midi"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_folders_and_midi() {
        let tmp = std::env::temp_dir().join(format!("qwertone-lib-{}", std::process::id()));
        let song = tmp.join("songs/clip-1234");
        std::fs::create_dir_all(&song).unwrap();
        std::fs::copy(
            crate::song::tests::fixture("clip-midi.song.json"),
            song.join("song.json"),
        )
        .unwrap();
        std::fs::create_dir_all(tmp.join("songs/broken")).unwrap();
        std::fs::write(tmp.join("songs/broken/song.json"), "{").unwrap();
        let mids = tmp.join("music/sub");
        std::fs::create_dir_all(&mids).unwrap();
        std::fs::copy(crate::song::tests::fixture("clip.mid"), mids.join("a.mid")).unwrap();
        std::fs::write(mids.join("notes.txt"), "x").unwrap();

        let songs = scan_songs(&tmp.join("songs"));
        assert_eq!(songs.len(), 2);
        let ok = songs.iter().find(|e| e.error.is_none()).unwrap();
        assert_eq!(ok.mode, "simplified");
        assert_eq!(ok.parts.len(), 5);
        assert!(songs.iter().any(|e| e.error.is_some()));

        let m = scan_midi(&[tmp.join("music")], 3, 50);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].title, "a");
        assert!(scan_midi(&[tmp.join("music")], 0, 50).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(is_audio(Path::new("x.FLAC")) && !is_audio(Path::new("x.mid")));
    }
}
