//! The auto song layout: a keymap made for one song, with the fewest keys.
//!
//! - Each different chord of the song gets one key, bottom row first
//!   (Z X C V …, then A S D …), sorted by their place in the key so the home
//!   chord is on Z, like the Chords + Melody layout.
//! - The notes of the melody (or whatever part you practise) get one key
//!   each, low to high, on the Q row and then the number row.
//! - Without chords, the part's notes use all four letter rows.
//! - Too many chords? 7ths become plain chords first, then the rarest
//!   chords share the key of the closest one.

use super::chords::span_kind;
use super::{ChordSpan, Key as SongKey};
use crate::keymap::{Action, KeyId, Keymap};
use crate::theory::{ChordKind, ChordRef, chord_symbol};
use egui::Key;
use std::collections::HashMap;

const CHORD_ROWS: [&[Key]; 2] = [
    &[
        Key::Z,
        Key::X,
        Key::C,
        Key::V,
        Key::B,
        Key::N,
        Key::M,
        Key::Comma,
        Key::Period,
        Key::Slash,
    ],
    &[
        Key::A,
        Key::S,
        Key::D,
        Key::F,
        Key::G,
        Key::H,
        Key::J,
        Key::K,
        Key::L,
        Key::Semicolon,
        Key::Quote,
    ],
];

const NOTE_ROWS: [&[Key]; 2] = [
    &[
        Key::Q,
        Key::W,
        Key::E,
        Key::R,
        Key::T,
        Key::Y,
        Key::U,
        Key::I,
        Key::O,
        Key::P,
        Key::OpenBracket,
        Key::CloseBracket,
    ],
    &[
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
        Key::Num0,
        Key::Minus,
        Key::Equals,
    ],
];

/// The result: a keymap plus what went where.
#[derive(Clone, Debug)]
pub struct SongLayout {
    pub keymap: Keymap,
    pub chords: Vec<(KeyId, i32, ChordKind)>,
    pub notes: Vec<(KeyId, i32)>,
    pub drums: Vec<(KeyId, u8)>,
    /// Chords that had to share a key (symbol → symbol of the key).
    pub merged: Vec<(String, String)>,
    /// Note pitches that didn't get a key.
    pub left_out: Vec<i32>,
}

impl SongLayout {
    /// One line for the UI: "6 chords on Z–N, 9 notes on Q–O".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.chords.is_empty() {
            parts.push(format!(
                "{} chord{} on {}",
                self.chords.len(),
                if self.chords.len() == 1 { "" } else { "s" },
                span(self.chords.iter().map(|c| c.0))
            ));
        }
        if !self.notes.is_empty() {
            parts.push(format!(
                "{} note{} on {}",
                self.notes.len(),
                if self.notes.len() == 1 { "" } else { "s" },
                span(self.notes.iter().map(|c| c.0))
            ));
        }
        if !self.drums.is_empty() {
            parts.push(format!("{} drums", self.drums.len()));
        }
        let mut s = parts.join(", ");
        if !self.merged.is_empty() {
            s += &format!(" · {} rare chords share a key", self.merged.len());
        }
        if !self.left_out.is_empty() {
            s += &format!(" · {} notes didn't fit", self.left_out.len());
        }
        s
    }
}

fn span(mut keys: impl Iterator<Item = KeyId>) -> String {
    let first = keys.next();
    let last = keys.last();
    match (first, last) {
        (Some(a), Some(b)) => format!("{}–{}", a.label(), b.label()),
        (Some(a), None) => a.label().to_string(),
        _ => String::new(),
    }
}

/// Distinct chords of a chord sheet, with how long each sounds in total.
pub fn distinct_chords(chords: &[ChordSpan]) -> Vec<((i32, ChordKind), f64)> {
    let mut v: Vec<((i32, ChordKind), f64)> = Vec::new();
    for c in chords {
        let Some(k) = span_kind(c) else { continue };
        let d = (c.end - c.start).max(0.0);
        match v.iter_mut().find(|x| x.0 == k) {
            Some(x) => x.1 += d,
            None => v.push((k, d)),
        }
    }
    v
}

/// How different two chords sound: notes not shared.
fn chord_distance(a: (i32, ChordKind), b: (i32, ChordKind)) -> usize {
    let pa: Vec<i32> = a.1.intervals().iter().map(|t| (a.0 + t) % 12).collect();
    let pb: Vec<i32> = b.1.intervals().iter().map(|t| (b.0 + t) % 12).collect();
    let shared = pa.iter().filter(|p| pb.contains(p)).count();
    pa.len() + pb.len() - 2 * shared + if a.0 == b.0 { 0 } else { 1 }
}

/// A chord as (root pitch class, kind).
type ChordId = (i32, ChordKind);

/// Map chords to at most `slots` keys. Returns the kept chords (in key
/// order) and a map from every chord to the one whose key plays it.
pub fn reduce_chords(
    mut chords: Vec<(ChordId, f64)>,
    slots: usize,
    tonic_pc: i32,
) -> (Vec<ChordId>, HashMap<ChordId, ChordId>) {
    let mut map: HashMap<(i32, ChordKind), (i32, ChordKind)> =
        chords.iter().map(|(c, _)| (*c, *c)).collect();
    if chords.len() > slots {
        // Plain chords instead of 7ths / sus.
        let mut merged: Vec<((i32, ChordKind), f64)> = Vec::new();
        for ((r, k), d) in chords {
            let t = (r, k.triad());
            for v in map.values_mut() {
                if *v == (r, k) {
                    *v = t;
                }
            }
            match merged.iter_mut().find(|x| x.0 == t) {
                Some(x) => x.1 += d,
                None => merged.push((t, d)),
            }
        }
        chords = merged;
    }
    if chords.len() > slots {
        // Keep the most used; the rest share the closest kept chord's key.
        chords.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let (keep, drop) = chords.split_at(slots);
        let keep: Vec<((i32, ChordKind), f64)> = keep.to_vec();
        for (c, _) in drop {
            let target = keep
                .iter()
                .min_by_key(|(k, _)| chord_distance(*c, *k))
                .unwrap()
                .0;
            for v in map.values_mut() {
                if *v == *c {
                    *v = target;
                }
            }
        }
        chords = keep;
    }
    // Key order: by distance from the home note, so I is on Z.
    let mut order: Vec<(i32, ChordKind)> = chords.into_iter().map(|(c, _)| c).collect();
    order.sort_by_key(|(r, k)| ((r - tonic_pc).rem_euclid(12), *k));
    (order, map)
}

/// Build the layout. `chords`: the song's chord sheet (empty = no chord
/// keys). `pitches`: the notes to put on keys. `drums`: drum notes.
/// `base_note`: what `note 0` plays right now (so the keys play the
/// song's exact pitches).
pub fn build(
    title: &str,
    chords: &[ChordSpan],
    key: Option<&SongKey>,
    pitches: &[i32],
    drums: &[u8],
    base_note: i32,
) -> SongLayout {
    let tonic = key.map(|k| k.tonic_pc).unwrap_or(0);
    let mut b: HashMap<KeyId, Action> = HashMap::new();
    let mut out_chords = Vec::new();
    let mut merged = Vec::new();
    let distinct = distinct_chords(chords);
    let has_chords = !distinct.is_empty();
    if has_chords {
        let slots = CHORD_ROWS.iter().map(|r| r.len()).sum();
        let (kept, map) = reduce_chords(distinct.clone(), slots, tonic);
        let keys = CHORD_ROWS.iter().flat_map(|r| r.iter());
        for ((root, kind), k) in kept.iter().zip(keys) {
            let id = KeyId::Key(*k);
            b.insert(
                id,
                Action::Chord(ChordRef::Named {
                    root: *root as u8,
                    kind: *kind,
                }),
            );
            out_chords.push((id, *root, *kind));
        }
        for ((c, _), _) in distinct.iter().zip(0..) {
            let to = map[c];
            if to != *c {
                merged.push((chord_symbol(c.0, c.1), chord_symbol(to.0, to.1)));
            }
        }
    }

    // Notes: distinct pitches, low to high.
    let mut ps: Vec<i32> = pitches.to_vec();
    ps.sort();
    ps.dedup();
    let note_keys: Vec<Key> = if has_chords || !drums.is_empty() {
        NOTE_ROWS.iter().flat_map(|r| r.iter().copied()).collect()
    } else {
        CHORD_ROWS
            .iter()
            .chain(NOTE_ROWS.iter())
            .flat_map(|r| r.iter().copied())
            .collect()
    };
    let mut out_notes = Vec::new();
    let mut left_out = Vec::new();
    // Too many notes: keep the ones in the middle of the range.
    let skip = ps.len().saturating_sub(note_keys.len()) / 2;
    for (i, p) in ps.iter().enumerate() {
        if i < skip || i - skip >= note_keys.len() {
            left_out.push(*p);
            continue;
        }
        let id = KeyId::Key(note_keys[i - skip]);
        b.insert(id, Action::Note(p - base_note));
        out_notes.push((id, *p));
    }

    // Drums on the A row (or Z row when there are no chords).
    let mut out_drums = Vec::new();
    let mut ds: Vec<u8> = drums.to_vec();
    ds.sort_by_key(|d| {
        crate::drums::DRUMS
            .iter()
            .position(|x| x.note == *d)
            .unwrap_or(99)
    });
    ds.dedup();
    let drum_keys: Vec<Key> = if has_chords {
        Vec::new()
    } else {
        CHORD_ROWS.iter().flat_map(|r| r.iter().copied()).collect()
    };
    for (d, k) in ds.iter().zip(drum_keys) {
        let id = KeyId::Key(k);
        b.insert(id, Action::Drum(*d));
        out_drums.push((id, *d));
    }

    for (k, a) in [
        (Key::Space, Action::Sustain),
        (Key::Escape, Action::Panic),
        (Key::F1, Action::LayoutNext),
    ] {
        b.entry(KeyId::Key(k)).or_insert(a);
    }
    let mut desc = format!("Made for “{title}”.");
    if !out_chords.is_empty() {
        desc += " Bottom rows: one key per chord of the song.";
    }
    if !out_notes.is_empty() {
        desc += " Q row (and number row): the song's notes, low to high.";
    }
    SongLayout {
        keymap: Keymap {
            name: format!("Song: {title}"),
            description: desc,
            bindings: b,
        },
        chords: out_chords,
        notes: out_notes,
        drums: out_drums,
        merged,
        left_out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::tests::clip_song;

    #[test]
    fn clip_layout_uses_one_key_per_chord() {
        let s = clip_song();
        let mel: Vec<i32> = s
            .part("melody")
            .unwrap()
            .notes
            .iter()
            .map(|n| n.pitch as i32)
            .collect();
        let l = build(s.title(), &s.chords, s.key.as_ref(), &mel, &[], 48);
        // G major song: G on Z, then Am/Bm/C/D/Em… in key order.
        let distinct = distinct_chords(&s.chords).len();
        assert_eq!(l.chords.len(), distinct);
        assert_eq!(l.chords[0].0, KeyId::Key(Key::Z));
        assert_eq!((l.chords[0].1, l.chords[0].2), (7, ChordKind::Major));
        assert_eq!(
            l.keymap.get(KeyId::Key(Key::Z)),
            Some(Action::Chord(ChordRef::Named {
                root: 7,
                kind: ChordKind::Major
            }))
        );
        // Melody notes low to high on the Q row, each playing its pitch.
        let mut ps = mel.clone();
        ps.sort();
        ps.dedup();
        assert_eq!(l.notes.len(), ps.len());
        assert_eq!(l.notes[0], (KeyId::Key(Key::Q), ps[0]));
        assert_eq!(
            l.keymap.get(KeyId::Key(Key::Q)),
            Some(Action::Note(ps[0] - 48))
        );
        assert!(l.merged.is_empty() && l.left_out.is_empty());
        assert!(l.summary().contains("chords on Z"));
        // It survives a round trip through the keymap file format.
        let text = l.keymap.to_toml();
        let (back, warnings) = Keymap::from_toml(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.bindings, l.keymap.bindings);
    }

    #[test]
    fn too_many_chords_get_merged() {
        // 24 chords: every major and minor.
        let mut chords = Vec::new();
        for r in 0..12 {
            for k in [ChordKind::Major, ChordKind::Minor] {
                chords.push(((r, k), 1.0 + r as f64 * 0.01));
            }
        }
        chords.push(((0, ChordKind::Dom7), 0.5));
        let (kept, map) = reduce_chords(chords, 21, 0);
        assert_eq!(kept.len(), 21);
        // C7 became C first.
        assert_eq!(map[&(0, ChordKind::Dom7)].1, ChordKind::Major);
        // Every chord maps to a kept one.
        assert!(map.values().all(|v| kept.contains(v)));
        // Home chord first.
        assert_eq!(kept[0].0, 0);
    }

    #[test]
    fn without_chords_notes_use_all_rows() {
        let ps: Vec<i32> = (40..90).collect();
        let l = build("x", &[], None, &ps, &[], 48);
        assert_eq!(l.notes.len(), 45);
        assert_eq!(l.left_out.len(), 5);
        assert_eq!(l.notes[0].0, KeyId::Key(Key::Z));
    }

    #[test]
    fn drums_get_pads() {
        let l = build("x", &[], None, &[], &[36, 38, 42, 38], 48);
        assert_eq!(l.drums.len(), 3);
        assert_eq!(l.drums[0], (KeyId::Key(Key::Z), 36));
    }
}
