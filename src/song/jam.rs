//! Jam mode logic (Part 7), no UI: which of the analyzer's stems hold the
//! parts you play (they're muted so you play them instead), friendly stem
//! names, and the mixer's gains.

use super::Song;
use crate::audio::stems::MAX_STEMS;
use std::collections::{BTreeMap, HashMap};

/// The stems that hold what you play, in the song's stem order.
///
/// A part's own `stem` wins (the melody from the vocal stem mutes the
/// vocals). Parts without one go by role: melody → vocals, chords (and the
/// chord sheet) → piano and guitar if the 6-stem model made them, else
/// "other" (where keys, guitars and pads end up), bass → bass, drums →
/// drums; the easy arrangement (melody + left hand) mutes both.
pub fn my_stems(song: &Song, play_parts: &[usize], play_chords: bool) -> Vec<String> {
    let has = |n: &str| song.stems.iter().any(|s| s.name == n);
    let mut mine: Vec<&str> = Vec::new();
    let harmony = |mine: &mut Vec<&str>| {
        let split: Vec<&str> = ["piano", "guitar"].into_iter().filter(|n| has(n)).collect();
        if split.is_empty() {
            mine.push("other");
        } else {
            mine.extend(split);
        }
    };
    for &i in play_parts {
        let Some(p) = song.parts.get(i) else { continue };
        let own = p.stem.as_deref().filter(|s| has(s));
        match (own, p.role.as_str()) {
            (Some(s), "arrangement") => {
                mine.push(s);
                harmony(&mut mine);
            }
            (Some(s), _) => mine.push(s),
            (None, "melody" | "vocals") => mine.push("vocals"),
            (None, "chords") => harmony(&mut mine),
            (None, "arrangement") => {
                mine.push("vocals");
                harmony(&mut mine);
            }
            (None, "bass") => mine.push("bass"),
            (None, "drums") => mine.push("drums"),
            (None, "piano") => mine.push(if has("piano") { "piano" } else { "other" }),
            (None, "guitar") => mine.push(if has("guitar") { "guitar" } else { "other" }),
            (None, "other") => mine.push("other"),
            _ => {}
        }
    }
    if play_chords {
        harmony(&mut mine);
    }
    song.stems
        .iter()
        .map(|s| s.name.clone())
        .filter(|n| mine.contains(&n.as_str()))
        .collect()
}

/// "Vocals", "Other instruments"…
pub fn stem_label(name: &str) -> String {
    match name {
        "vocals" => "Vocals".into(),
        "drums" => "Drums".into(),
        "bass" => "Bass".into(),
        "other" => "Other instruments".into(),
        "piano" => "Piano".into(),
        "guitar" => "Guitar".into(),
        n => {
            let mut c = n.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        }
    }
}

/// Whether each stem is muted: your stems unless you un-muted them, others
/// only if you muted them (`overrides`: stem → muted).
pub fn muted(song: &Song, mine: &[String], overrides: &HashMap<String, bool>) -> Vec<bool> {
    song.stems
        .iter()
        .map(|s| {
            overrides
                .get(&s.name)
                .copied()
                .unwrap_or_else(|| mine.contains(&s.name))
        })
        .collect()
}

/// Gains for the audio thread: volume per stem name × master, 0 if muted.
/// A solo stem plays alone.
pub fn gains(
    song: &Song,
    muted: &[bool],
    volumes: &BTreeMap<String, f32>,
    master: f32,
    solo: Option<&str>,
) -> [f32; MAX_STEMS] {
    let mut g = [0.0; MAX_STEMS];
    for (i, s) in song.stems.iter().take(MAX_STEMS).enumerate() {
        let on = match solo {
            Some(x) => x == s.name,
            None => !muted.get(i).copied().unwrap_or(false),
        };
        if on {
            g[i] = volumes.get(&s.name).copied().unwrap_or(1.0).clamp(0.0, 1.5)
                * master.clamp(0.0, 1.5);
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Song;

    fn jam_song() -> Song {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jam-song");
        Song::load(&dir).unwrap()
    }

    fn idx(song: &Song, id: &str) -> usize {
        song.parts.iter().position(|p| p.id == id).unwrap()
    }

    #[test]
    fn your_parts_stems_are_muted() {
        let song = jam_song();
        let names: Vec<&str> = song.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["vocals", "drums", "bass", "other"]);
        assert_eq!(my_stems(&song, &[idx(&song, "melody")], false), ["vocals"]);
        assert_eq!(
            my_stems(&song, &[idx(&song, "melody-easy")], false),
            ["vocals"]
        );
        // Chords have no stem: the harmony is in "other" with 4 stems.
        assert_eq!(my_stems(&song, &[idx(&song, "chords")], false), ["other"]);
        assert_eq!(
            my_stems(&song, &[idx(&song, "melody")], true),
            ["vocals", "other"]
        );
        assert_eq!(
            my_stems(&song, &[idx(&song, "bass"), idx(&song, "drums")], false),
            ["drums", "bass"]
        );
        assert!(my_stems(&song, &[], false).is_empty());
    }

    #[test]
    fn six_stems_split_the_harmony() {
        let mut song = jam_song();
        for n in ["piano", "guitar"] {
            song.stems.push(crate::song::Stem {
                name: n.into(),
                file: format!("stems/{n}.flac"),
            });
        }
        assert_eq!(
            my_stems(&song, &[idx(&song, "chords")], false),
            ["piano", "guitar"]
        );
        let m = idx(&song, "melody");
        song.parts[m].stem = None;
        song.parts[m].role = "arrangement".into();
        assert_eq!(
            my_stems(&song, &[idx(&song, "melody")], false),
            ["vocals", "piano", "guitar"]
        );
    }

    #[test]
    fn mixer_gains() {
        let song = jam_song();
        let mine = my_stems(&song, &[idx(&song, "melody")], false);
        let mut over = HashMap::new();
        let m = muted(&song, &mine, &over);
        assert_eq!(m, [true, false, false, false]);
        over.insert("vocals".to_string(), false);
        over.insert("drums".to_string(), true);
        let m = muted(&song, &mine, &over);
        assert_eq!(m, [false, true, false, false]);
        let mut vols = BTreeMap::new();
        vols.insert("bass".to_string(), 0.5);
        let g = gains(&song, &m, &vols, 0.8, None);
        assert_eq!(&g[..4], &[0.8, 0.0, 0.4, 0.8]);
        let g = gains(&song, &m, &vols, 1.0, Some("drums"));
        assert_eq!(&g[..4], &[0.0, 1.0, 0.0, 0.0]);
        assert_eq!(stem_label("other"), "Other instruments");
        assert_eq!(stem_label("strings"), "Strings");
    }
}
