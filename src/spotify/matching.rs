//! Finding the practice version of a Spotify track: an analysed song with
//! the same title / artist / length, or a local audio file to analyse.
//! Pure string logic, tested.

use std::path::{Path, PathBuf};

/// Lower case, no brackets ("(From \"Vikram\")", "[Remastered]"), no
/// "- Remastered 2011" tails, no "feat. …", only letters and digits.
pub fn norm_title(s: &str) -> String {
    let mut t = s.to_lowercase();
    // Drop bracketed parts.
    let mut out = String::new();
    let mut depth = 0i32;
    for c in t.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    t = out;
    // " - Remastered", " - From …", " - Live", " - Radio Edit" …
    if let Some(i) = t.find(" - ") {
        t.truncate(i);
    }
    for tag in [" feat.", " feat ", " ft.", " ft "] {
        if let Some(i) = t.find(tag) {
            t.truncate(i);
        }
    }
    words(&t).join(" ")
}

pub fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// 0–1: how much of the shorter word set is in the longer one.
pub fn overlap(a: &str, b: &str) -> f64 {
    let (wa, wb) = (words(a), words(b));
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let (short, long) = if wa.len() <= wb.len() {
        (&wa, &wb)
    } else {
        (&wb, &wa)
    };
    short.iter().filter(|w| long.contains(w)).count() as f64 / short.len() as f64
}

/// How sure we are that a song (title, artist, seconds) is this Spotify
/// track: 0–1. Title must match; artist and length make it surer, a very
/// different length makes it unlikely (another version).
pub fn score(
    sp_title: &str,
    sp_artists: &[String],
    sp_secs: f64,
    title: &str,
    artist: Option<&str>,
    secs: f64,
) -> f64 {
    let (a, b) = (norm_title(sp_title), norm_title(title));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let t = if a == b { 1.0 } else { overlap(&a, &b) };
    if t < 0.75 {
        return 0.0;
    }
    let mut s = 0.6 * t;
    match artist {
        Some(ar) if !ar.trim().is_empty() => {
            let best = sp_artists
                .iter()
                .map(|x| overlap(x, ar))
                .fold(0.0f64, f64::max);
            s += 0.25 * best;
        }
        // Unknown artist: neutral.
        _ => s += 0.12,
    }
    if secs > 0.0 && sp_secs > 0.0 {
        let d = (secs - sp_secs).abs();
        s += if d <= 3.0 {
            0.15
        } else if d <= 10.0 {
            0.07
        } else if d > 30.0 {
            -0.3
        } else {
            0.0
        };
    } else {
        s += 0.05;
    }
    s.clamp(0.0, 1.0)
}

/// Good enough to offer without asking.
pub const SURE: f64 = 0.8;
/// Worth offering as "maybe this one?".
pub const MAYBE: f64 = 0.55;

/// A file name like "Anirudh - Arabic Kuthu (Official).mp3" → title words.
pub fn file_title(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().replace(['_', '.'], " "))
        .unwrap_or_default()
}

/// Score a local audio file by its name only (tags would need decoding).
pub fn file_score(sp_title: &str, sp_artists: &[String], p: &Path) -> f64 {
    let name = file_title(p);
    let t = norm_title(sp_title);
    if t.is_empty() {
        return 0.0;
    }
    // Every word of the title in the file name.
    let tw = words(&t);
    let nw = words(&name);
    let hit = tw.iter().filter(|w| nw.contains(w)).count() as f64 / tw.len() as f64;
    if hit < 0.99 {
        return 0.0;
    }
    let artist = sp_artists
        .iter()
        .map(|a| overlap(a, &name))
        .fold(0.0f64, f64::max);
    // Fewer extra words = surer.
    let extra = nw.len().saturating_sub(tw.len()) as f64;
    (0.7 + 0.25 * artist - 0.02 * extra).clamp(0.0, 1.0)
}

/// Audio files under `dirs` (a few levels deep, skipping hidden folders).
pub fn audio_files(dirs: &[PathBuf], depth: usize, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(d: &Path, depth: usize, out: &mut Vec<PathBuf>, limit: usize) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            if out.len() >= limit {
                return;
            }
            let p = e.path();
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth > 0 && !p.join("song.json").exists() {
                    walk(&p, depth - 1, out, limit);
                }
            } else if crate::song::library::is_audio(&p) {
                out.push(p);
            }
        }
    }
    for d in dirs {
        walk(d, depth, &mut out, limit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn titles() {
        assert_eq!(
            norm_title("Arabic Kuthu - Halamithi Habibo (From \"Beast\")"),
            "arabic kuthu"
        );
        assert_eq!(norm_title("Kaavaalaa [Remastered]"), "kaavaalaa");
        assert_eq!(norm_title("Naatu Naatu (feat. X)"), "naatu naatu");
        assert_eq!(norm_title("Song feat. Someone"), "song");
        assert_eq!(norm_title("Rowdy Baby"), "rowdy baby");
    }

    #[test]
    fn scoring() {
        let sp = v(&["Anirudh Ravichander", "Jonita Gandhi"]);
        let s = score(
            "Arabic Kuthu - Halamithi Habibo",
            &sp,
            279.0,
            "Arabic Kuthu",
            Some("Anirudh Ravichander"),
            280.5,
        );
        assert!(s >= SURE, "{s}");
        // No artist tag, same length: still a match.
        let s = score("Arabic Kuthu", &sp, 279.0, "arabic kuthu", None, 279.0);
        assert!(s >= SURE, "{s}");
        // Same title, other song (very different length, other artist).
        let s = score(
            "Arabic Kuthu",
            &sp,
            279.0,
            "Arabic Kuthu",
            Some("Cover Band"),
            150.0,
        );
        assert!(s < MAYBE, "{s}");
        // Different title.
        assert_eq!(
            score("Vaathi Coming", &sp, 200.0, "Arabic Kuthu", None, 200.0),
            0.0
        );
    }

    #[test]
    fn file_names() {
        let sp = v(&["Anirudh Ravichander"]);
        let a = file_score(
            "Arabic Kuthu",
            &sp,
            Path::new("/m/Anirudh Ravichander - Arabic Kuthu.mp3"),
        );
        let b = file_score("Arabic Kuthu", &sp, Path::new("/m/arabic_kuthu.flac"));
        let c = file_score("Arabic Kuthu", &sp, Path::new("/m/Vaathi Coming.mp3"));
        assert!(a > b && b > MAYBE && c == 0.0, "{a} {b} {c}");
    }
}
