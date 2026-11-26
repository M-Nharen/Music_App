//! Chord and key guessing from pitch-class weights ("chroma"). Used for MIDI
//! files (which have notes but no chord sheet) and by live listen mode.

use super::{BeatGrid, ChordSpan, Key, Note};
use crate::theory::{ChordKind, chord_symbol, pc_name};

/// "C#" → 1. Accepts sharps and flats.
pub fn pc_from_name(s: &str) -> Option<i32> {
    crate::theory::parse_chord_symbol(s).map(|(pc, _)| pc)
}

/// How a chroma vector compares with a chord template. Chroma is 12
/// non-negative weights, C first.
fn template_score(chroma: &[f32; 12], root: i32, kind: ChordKind) -> f32 {
    let total: f32 = chroma.iter().sum();
    if total <= 1e-9 {
        return 0.0;
    }
    let iv = kind.intervals();
    let mut inside = 0.0;
    for (i, t) in iv.iter().enumerate() {
        let w = chroma[((root + t).rem_euclid(12)) as usize];
        // The root and third say most about a chord.
        inside += w * if i == 0 { 1.15 } else { 1.0 };
    }
    let outside = total
        - iv.iter()
            .map(|t| chroma[((root + t).rem_euclid(12)) as usize])
            .sum::<f32>();
    // Fraction of energy explained, minus a penalty for notes left over,
    // and a small penalty for bigger chords so triads win ties.
    let size_penalty = 0.07 * (iv.len() as f32 - 3.0);
    (inside / total) - 0.6 * (outside / total) - size_penalty
}

/// Which chord types to try.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vocab {
    /// Major and minor only.
    Basic,
    /// Plus 7ths, dim, aug, sus.
    Extended,
}

impl Vocab {
    fn kinds(self) -> &'static [ChordKind] {
        match self {
            Vocab::Basic => &[ChordKind::Major, ChordKind::Minor],
            Vocab::Extended => &ChordKind::ALL,
        }
    }
}

/// The best chord for a chroma vector: (root pc, kind, score 0..1).
pub fn best_chord(chroma: &[f32; 12], vocab: Vocab) -> Option<(i32, ChordKind, f32)> {
    let mut best: Option<(i32, ChordKind, f32)> = None;
    for root in 0..12 {
        for &kind in vocab.kinds() {
            // A 7th chord needs its 7th to really be there: about as loud as
            // the chord's other notes, not just a passing melody note.
            if kind.intervals().len() > 3 {
                let seventh = chroma[((root + kind.intervals()[3]) % 12) as usize];
                let total: f32 = chroma.iter().sum::<f32>().max(1e-9);
                let triad_min = kind.intervals()[..3]
                    .iter()
                    .map(|t| chroma[((root + t) % 12) as usize])
                    .fold(f32::MAX, f32::min);
                if seventh / total < 0.12 || seventh < triad_min * 0.6 {
                    continue;
                }
            }
            let s = template_score(chroma, root, kind);
            if best.is_none_or(|b| s > b.2) {
                best = Some((root, kind, s));
            }
        }
    }
    best.filter(|b| b.2 > 0.0)
        .map(|(r, k, s)| (r, k, s.clamp(0.0, 1.0)))
}

/// Krumhansl–Kessler key profiles.
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

fn correlation(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    if da <= 1e-12 || db <= 1e-12 {
        0.0
    } else {
        num / (da * db).sqrt()
    }
}

/// The key of some pitch-class weights: (tonic pc, minor?, confidence 0..1).
pub fn estimate_key(chroma: &[f32; 12]) -> Option<(i32, bool, f32)> {
    if chroma.iter().sum::<f32>() <= 1e-9 {
        return None;
    }
    let mut scores: Vec<(i32, bool, f32)> = Vec::with_capacity(24);
    for tonic in 0..12 {
        for (minor, profile) in [(false, &MAJOR_PROFILE), (true, &MINOR_PROFILE)] {
            let mut rotated = [0.0; 12];
            for (i, r) in rotated.iter_mut().enumerate() {
                *r = profile[(i + 12 - tonic as usize) % 12];
            }
            scores.push((tonic, minor, correlation(chroma, &rotated)));
        }
    }
    scores.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    let (t, m, s) = scores[0];
    let margin = s - scores[1].2;
    Some((t, m, (s * 0.7 + margin * 3.0).clamp(0.0, 1.0)))
}

pub fn key_from(tonic: i32, minor: bool, confidence: f32) -> Key {
    let name = format!(
        "{} {}",
        pc_name(tonic, false),
        if minor { "minor" } else { "major" }
    );
    Key {
        tonic: pc_name(tonic, false).into(),
        tonic_pc: tonic,
        mode: if minor { "minor" } else { "major" }.into(),
        name,
        confidence,
    }
}

const ROMANS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// Roman numeral of a chord in a key, e.g. "V7", "bVII", "ii".
pub fn roman(root_pc: i32, kind: ChordKind, key: &Key) -> (String, bool) {
    let off = (root_pc - key.tonic_pc).rem_euclid(12);
    let scale: [i32; 7] = if key.minor() {
        [0, 2, 3, 5, 7, 8, 10]
    } else {
        [0, 2, 4, 5, 7, 9, 11]
    };
    let (deg, acc) = match scale.iter().position(|s| *s == off) {
        Some(d) => (d, ""),
        None => match scale.iter().position(|s| *s == (off + 1) % 12) {
            Some(d) => (d, "b"),
            None => (
                scale
                    .iter()
                    .position(|s| *s == (off + 11) % 12)
                    .unwrap_or(0),
                "#",
            ),
        },
    };
    let base = ROMANS[deg];
    let mut s = if kind.is_minor() {
        format!("{acc}{}", base.to_lowercase())
    } else {
        format!("{acc}{base}")
    };
    s += match kind {
        ChordKind::Dim => "°",
        ChordKind::Aug => "+",
        ChordKind::Dom7 | ChordKind::Min7 => "7",
        ChordKind::Maj7 => "maj7",
        ChordKind::Sus2 => "sus2",
        ChordKind::Sus4 => "sus4",
        _ => "",
    };
    // Diatonic: every chord tone is in the key's scale.
    let diatonic = acc.is_empty()
        && kind
            .intervals()
            .iter()
            .all(|t| scale.contains(&((off + t).rem_euclid(12))));
    (s, diatonic)
}

/// Pitch-class weights of the notes sounding between `t0` and `t1`, weighted
/// by overlap and a bit more for the lowest note (the bass names the chord).
pub fn chroma_of_notes<'a>(notes: impl Iterator<Item = &'a Note>, t0: f64, t1: f64) -> [f32; 12] {
    let mut c = [0.0f32; 12];
    let mut lowest: Option<(u8, f32)> = None;
    for n in notes {
        let ov = (n.end.min(t1) - n.start.max(t0)) as f32;
        if ov <= 0.0 {
            continue;
        }
        let w = ov * (n.velocity.max(1) as f32 / 100.0);
        c[(n.pitch % 12) as usize] += w;
        if lowest.is_none_or(|(p, _)| n.pitch < p) {
            lowest = Some((n.pitch, w));
        }
    }
    if let Some((p, w)) = lowest {
        c[(p % 12) as usize] += w * 0.5;
    }
    c
}

/// A chord sheet for notes that came without one (MIDI files): one guess
/// every `beats_per_chord` beats, joined when the same chord repeats.
pub fn estimate_chords(
    notes: &[&Note],
    grid: &BeatGrid,
    duration: f64,
    key: &Key,
    beats_per_chord: f64,
    low_threshold: f32,
) -> Vec<ChordSpan> {
    let mut out: Vec<ChordSpan> = Vec::new();
    if notes.is_empty() || duration <= 0.0 {
        return out;
    }
    let first = grid.beat_at(0.0).floor().min(0.0);
    let last = grid.beat_at(duration).ceil();
    let mut b = first;
    while b < last {
        let b1 = (b + beats_per_chord).min(last);
        let (t0, t1) = (grid.time_at(b).max(0.0), grid.time_at(b1).min(duration));
        if t1 <= t0 {
            b = b1;
            continue;
        }
        let chroma = chroma_of_notes(notes.iter().copied(), t0, t1);
        let span = match best_chord(&chroma, Vocab::Extended) {
            Some((root, kind, score)) => {
                let (r, diatonic) = roman(root, kind, key);
                let conf = (0.35 + score * 0.75).clamp(0.0, 1.0);
                ChordSpan {
                    start: t0,
                    end: t1,
                    start_beat: grid.beat_at(t0),
                    end_beat: grid.beat_at(t1),
                    symbol: chord_symbol(root, kind),
                    root: Some(pc_name(root, false).into()),
                    quality: Some(kind_quality(kind).into()),
                    bass: None,
                    pitch_classes: kind.intervals().iter().map(|t| (root + t) % 12).collect(),
                    roman: Some(r),
                    confidence: conf,
                    low_confidence: conf < low_threshold || !diatonic && score < 0.5,
                }
            }
            None => ChordSpan {
                start: t0,
                end: t1,
                start_beat: grid.beat_at(t0),
                end_beat: grid.beat_at(t1),
                symbol: "N".into(),
                root: None,
                quality: None,
                bass: None,
                pitch_classes: vec![],
                roman: None,
                confidence: 1.0,
                low_confidence: false,
            },
        };
        match out.last_mut() {
            Some(prev) if prev.symbol == span.symbol => {
                prev.end = span.end;
                prev.end_beat = span.end_beat;
                prev.confidence = prev.confidence.min(span.confidence);
                prev.low_confidence |= span.low_confidence;
            }
            _ => out.push(span),
        }
        b = b1;
    }
    out
}

/// The analyzer's `quality` string for a kind.
pub fn kind_quality(k: ChordKind) -> &'static str {
    match k {
        ChordKind::Major => "maj",
        ChordKind::Minor => "min",
        ChordKind::Dom7 => "7",
        ChordKind::Maj7 => "maj7",
        ChordKind::Min7 => "m7",
        ChordKind::Dim => "dim",
        ChordKind::Aug => "aug",
        ChordKind::Sus2 => "sus2",
        ChordKind::Sus4 => "sus4",
    }
}

/// The kind of a chord sheet entry.
pub fn span_kind(c: &ChordSpan) -> Option<(i32, ChordKind)> {
    if c.is_none() {
        return None;
    }
    if let (Some(root), Some(q)) = (c.root_pc(), c.quality.as_deref())
        && let Some(k) = ChordKind::parse(q)
    {
        return Some((root, k));
    }
    if let Some(x) = crate::theory::parse_chord_symbol(&c.symbol) {
        return Some(x);
    }
    // Last resort: from the pitch classes, root first.
    let root = *c.pitch_classes.first()?;
    let iv: Vec<i32> = c.pitch_classes.iter().map(|p| p - root).collect();
    ChordKind::from_intervals(&iv).map(|k| (root, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chroma(pcs: &[(i32, f32)]) -> [f32; 12] {
        let mut c = [0.0; 12];
        for (p, w) in pcs {
            c[*p as usize] += w;
        }
        c
    }

    #[test]
    fn finds_triads_and_sevenths() {
        let c = chroma(&[(0, 1.0), (4, 1.0), (7, 1.0)]);
        assert_eq!(
            best_chord(&c, Vocab::Extended).map(|x| (x.0, x.1)),
            Some((0, ChordKind::Major))
        );
        let am = chroma(&[(9, 1.2), (0, 1.0), (4, 1.0)]);
        assert_eq!(
            best_chord(&am, Vocab::Extended).map(|x| (x.0, x.1)),
            Some((9, ChordKind::Minor))
        );
        let g7 = chroma(&[(7, 1.0), (11, 1.0), (2, 1.0), (5, 0.9)]);
        assert_eq!(
            best_chord(&g7, Vocab::Extended).map(|x| (x.0, x.1)),
            Some((7, ChordKind::Dom7))
        );
        assert_eq!(
            best_chord(&g7, Vocab::Basic).map(|x| (x.0, x.1)),
            Some((7, ChordKind::Major))
        );
        // A little noise doesn't change the answer.
        let noisy = chroma(&[(2, 1.0), (6, 0.9), (9, 1.0), (1, 0.1), (8, 0.08)]);
        assert_eq!(
            best_chord(&noisy, Vocab::Extended).map(|x| (x.0, x.1)),
            Some((2, ChordKind::Major))
        );
        assert_eq!(best_chord(&[0.0; 12], Vocab::Basic), None);
    }

    #[test]
    fn keys() {
        // C major scale, tonic weighted.
        let c = chroma(&[
            (0, 3.0),
            (2, 1.0),
            (4, 2.0),
            (5, 1.0),
            (7, 2.5),
            (9, 1.0),
            (11, 1.0),
        ]);
        let (t, minor, _) = estimate_key(&c).unwrap();
        assert_eq!((t, minor), (0, false));
        let am = chroma(&[
            (9, 3.0),
            (11, 1.0),
            (0, 2.0),
            (2, 1.0),
            (4, 2.5),
            (5, 1.0),
            (7, 0.8),
        ]);
        let (t, minor, _) = estimate_key(&am).unwrap();
        assert_eq!((t, minor), (9, true));
    }

    #[test]
    fn roman_numerals() {
        let g = key_from(7, false, 1.0);
        assert_eq!(roman(2, ChordKind::Dom7, &g), ("V7".into(), true));
        assert_eq!(roman(4, ChordKind::Minor, &g), ("vi".into(), true));
        assert_eq!(roman(5, ChordKind::Major, &g).0, "bVII");
        let am = key_from(9, true, 1.0);
        assert_eq!(roman(0, ChordKind::Major, &am), ("III".into(), true));
        assert_eq!(roman(4, ChordKind::Major, &am), ("V".into(), false));
    }

    #[test]
    fn spans_from_analyzer_fields() {
        let s = super::super::tests::clip_song();
        let kinds: Vec<_> = s.chords.iter().filter_map(span_kind).collect();
        assert!(kinds.contains(&(7, ChordKind::Major)));
        assert!(kinds.contains(&(4, ChordKind::Minor)));
        assert_eq!(pc_from_name("F#"), Some(6));
    }
}
