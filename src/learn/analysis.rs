//! The theory overlay: explain whatever is being played, in plain words.
//!
//! Give it the notes held and the key; it names the chord, finds its roman
//! numeral, says what job the chord does in the key (home, away, tension…),
//! why it sounds happy / sad / tense, and how it moved from the last chord.

use super::intervals::interval;
use crate::theory::{Chord, Scale, pc_name};

/// Broad chord family: decides the numeral's case and the basic mood.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Major,
    Minor,
    Dim,
    Aug,
    Sus,
    Power,
}

/// One chord shape the overlay knows.
pub struct Shape {
    /// Pitch classes above the root, sorted, within one octave.
    pub pcs: &'static [i32],
    pub suffix: &'static str,
    pub family: Family,
    /// Extra for the roman numeral (e.g. "7", "maj7").
    pub numeral_ext: &'static str,
    pub feel: &'static str,
    pub why: &'static str,
    pub tension: f32,
}

const MAJOR_WHY: &str = "Major: the 3rd is 4 half steps above the root (a major 3rd). That wider gap is what makes it sound bright and happy.";
const MINOR_WHY: &str = "Minor: the 3rd is only 3 half steps above the root (a minor 3rd). Lowering that one note makes it sound sad or serious.";

pub const SHAPES: &[Shape] = &[
    Shape {
        pcs: &[0, 4, 7],
        suffix: "",
        family: Family::Major,
        numeral_ext: "",
        feel: "Happy, bright",
        why: MAJOR_WHY,
        tension: 0.05,
    },
    Shape {
        pcs: &[0, 3, 7],
        suffix: "m",
        family: Family::Minor,
        numeral_ext: "",
        feel: "Sad, soft",
        why: MINOR_WHY,
        tension: 0.15,
    },
    Shape {
        pcs: &[0, 3, 6],
        suffix: "dim",
        family: Family::Dim,
        numeral_ext: "",
        feel: "Tense, nervous, unstable",
        why: "Diminished: two minor 3rds stacked, so the top note is squeezed down. The root and top are a tritone apart, the most unstable interval.",
        tension: 0.75,
    },
    Shape {
        pcs: &[0, 4, 8],
        suffix: "aug",
        family: Family::Aug,
        numeral_ext: "+",
        feel: "Dreamy, strange, unresolved",
        why: "Augmented: two major 3rds stacked, so the top note is pushed up. It sounds like it's floating and has to go somewhere.",
        tension: 0.65,
    },
    Shape {
        pcs: &[0, 5, 7],
        suffix: "sus4",
        family: Family::Sus,
        numeral_ext: "sus4",
        feel: "Open, floating, waiting",
        why: "Suspended: the 3rd is swapped for the 4th, so it's neither happy nor sad. Your ear waits for the 4th to drop down to the 3rd.",
        tension: 0.35,
    },
    Shape {
        pcs: &[0, 2, 7],
        suffix: "sus2",
        family: Family::Sus,
        numeral_ext: "sus2",
        feel: "Airy, open, modern",
        why: "Suspended 2nd: no 3rd, so no happy/sad colour, just an open, airy sound.",
        tension: 0.25,
    },
    Shape {
        pcs: &[0, 7],
        suffix: "5",
        family: Family::Power,
        numeral_ext: "5",
        feel: "Strong, neutral (power chord)",
        why: "Power chord: just the root and the 5th. No 3rd, so it isn't happy or sad, just strong. Rock guitar lives on these.",
        tension: 0.05,
    },
    Shape {
        pcs: &[0, 4, 7, 10],
        suffix: "7",
        family: Family::Major,
        numeral_ext: "7",
        feel: "Bluesy, restless, wants to move",
        why: "Dominant 7th: a major chord plus a minor 7th. Its 3rd and 7th form a tritone, which badly wants to resolve, usually to the chord a 5th below.",
        tension: 0.6,
    },
    Shape {
        pcs: &[0, 4, 7, 11],
        suffix: "maj7",
        family: Family::Major,
        numeral_ext: "maj7",
        feel: "Soft, dreamy, jazzy",
        why: "Major 7th: a major chord plus the note just below the octave. That gentle rub gives it a warm, dreamy, 'lo-fi' sound.",
        tension: 0.25,
    },
    Shape {
        pcs: &[0, 3, 7, 10],
        suffix: "m7",
        family: Family::Minor,
        numeral_ext: "7",
        feel: "Mellow, smooth, a bit sad",
        why: "Minor 7th: a minor chord plus a minor 7th. Softer and smoother than plain minor; used everywhere in soul, funk and jazz.",
        tension: 0.3,
    },
    Shape {
        pcs: &[0, 3, 6, 10],
        suffix: "m7b5",
        family: Family::Dim,
        numeral_ext: "ø7",
        feel: "Dark, tense, searching",
        why: "Half-diminished: a diminished chord plus a minor 7th. Tense but softer than fully diminished; it often leads to V.",
        tension: 0.7,
    },
    Shape {
        pcs: &[0, 3, 6, 9],
        suffix: "dim7",
        family: Family::Dim,
        numeral_ext: "°7",
        feel: "Very tense, dramatic (silent-movie villain)",
        why: "Diminished 7th: minor 3rds all the way up, splitting the octave into four equal parts. Maximum suspense.",
        tension: 0.9,
    },
    Shape {
        pcs: &[0, 4, 7, 9],
        suffix: "6",
        family: Family::Major,
        numeral_ext: "6",
        feel: "Sweet, old-fashioned, happy",
        why: "Major 6th: a major chord plus the 6th. Sweet and settled, like a 1940s ending.",
        tension: 0.15,
    },
    Shape {
        pcs: &[0, 3, 7, 9],
        suffix: "m6",
        family: Family::Minor,
        numeral_ext: "6",
        feel: "Mysterious, film-noir",
        why: "Minor 6th: a minor chord with a bright 6th on top. Sad but with a twist.",
        tension: 0.35,
    },
    Shape {
        pcs: &[0, 2, 4, 7],
        suffix: "add9",
        family: Family::Major,
        numeral_ext: "add9",
        feel: "Bright, shimmering, open",
        why: "Add9: a major chord plus the 9th (the 2nd, an octave up). Adds sparkle without changing the mood.",
        tension: 0.15,
    },
    Shape {
        pcs: &[0, 2, 3, 7],
        suffix: "madd9",
        family: Family::Minor,
        numeral_ext: "add9",
        feel: "Sad and beautiful, emotional",
        why: "Minor add9: the 9th rubs against the minor 3rd, which makes it ache a little.",
        tension: 0.3,
    },
    Shape {
        pcs: &[0, 5, 7, 10],
        suffix: "7sus4",
        family: Family::Sus,
        numeral_ext: "7sus4",
        feel: "Floating, expectant",
        why: "7sus4: a suspended chord with a 7th. It hangs in the air before a dominant chord or home.",
        tension: 0.45,
    },
    Shape {
        pcs: &[0, 3, 7, 11],
        suffix: "m(maj7)",
        family: Family::Minor,
        numeral_ext: "(maj7)",
        feel: "Spy-movie, mysterious",
        why: "Minor-major 7th: a minor chord with the note right under the octave. Sad and tense at once.",
        tension: 0.6,
    },
    Shape {
        pcs: &[0, 2, 4, 7, 10],
        suffix: "9",
        family: Family::Major,
        numeral_ext: "9",
        feel: "Funky, rich, restless",
        why: "Dominant 9th: a 7th chord with the 9th on top. Funk and blues love it.",
        tension: 0.55,
    },
    Shape {
        pcs: &[0, 2, 4, 7, 11],
        suffix: "maj9",
        family: Family::Major,
        numeral_ext: "maj9",
        feel: "Lush, dreamy",
        why: "Major 9th: maj7 plus the 9th. Very lush; neo-soul and lo-fi sound.",
        tension: 0.25,
    },
    Shape {
        pcs: &[0, 2, 3, 7, 10],
        suffix: "m9",
        family: Family::Minor,
        numeral_ext: "9",
        feel: "Smooth, deep, soulful",
        why: "Minor 9th: m7 plus the 9th. Smooth and deep.",
        tension: 0.3,
    },
];

/// A chord found in some notes.
#[derive(Clone, Copy)]
pub struct Found {
    pub root_pc: i32,
    pub bass_pc: i32,
    pub shape: &'static Shape,
}

fn sorted_pcs(notes: &[i32]) -> Vec<i32> {
    let mut pcs: Vec<i32> = notes.iter().map(|n| n.rem_euclid(12)).collect();
    pcs.sort();
    pcs.dedup();
    pcs
}

/// The shape with exactly these pitch classes above `root`, if known.
pub fn shape_for(root_pc: i32, pcs: &[i32]) -> Option<&'static Shape> {
    let mut rel: Vec<i32> = pcs.iter().map(|p| (p - root_pc).rem_euclid(12)).collect();
    rel.sort();
    rel.dedup();
    SHAPES.iter().find(|s| s.pcs == rel.as_slice())
}

/// Find the chord in some notes. Prefers the lowest note as the root, so
/// C6 and Am7 (same notes) are named by what's in the bass.
pub fn identify(notes: &[i32]) -> Option<Found> {
    let pcs = sorted_pcs(notes);
    if pcs.len() < 2 {
        return None;
    }
    let bass_pc = notes.iter().min()?.rem_euclid(12);
    let mut roots = pcs.clone();
    roots.sort_by_key(|r| *r != bass_pc);
    // Two notes only count as a chord when they're a power chord.
    roots.iter().find_map(|&root_pc| {
        shape_for(root_pc, &pcs)
            .filter(|s| pcs.len() > 2 || s.family == Family::Power)
            .map(|shape| Found {
                root_pc,
                bass_pc,
                shape,
            })
    })
}

/// True for scales whose home chord is minor.
pub fn minor_home(scale: Scale) -> bool {
    matches!(
        scale,
        Scale::Minor | Scale::HarmonicMinor | Scale::Dorian | Scale::MinorPentatonic | Scale::Blues
    )
}

/// Roman numeral for a chord `root_off` semitones above the key's tonic,
/// spelled against the major scale (bIII, bVI, bVII for borrowed chords).
pub fn numeral(root_off: i32, family: Family, ext: &str) -> String {
    const BASE: [(&str, &str); 12] = [
        ("", "I"),
        ("b", "II"),
        ("", "II"),
        ("b", "III"),
        ("", "III"),
        ("", "IV"),
        ("#", "IV"),
        ("", "V"),
        ("b", "VI"),
        ("", "VI"),
        ("b", "VII"),
        ("", "VII"),
    ];
    let (acc, roman) = BASE[root_off.rem_euclid(12) as usize];
    let mut s = match family {
        Family::Minor | Family::Dim => format!("{acc}{}", roman.to_lowercase()),
        _ => format!("{acc}{roman}"),
    };
    match (family, ext) {
        (Family::Dim, "") => s.push('°'),
        (_, "+") => s.push('+'),
        (_, e) => s.push_str(e),
    }
    s
}

/// What job a chord does in the key: (short label, explanation, extra tension).
pub fn role(root_off: i32, family: Family, scale: Scale) -> (&'static str, &'static str, f32) {
    let off = root_off.rem_euclid(12);
    let fam = match family {
        Family::Sus | Family::Power => Family::Major,
        f => f,
    };
    if minor_home(scale) {
        return match (off, fam) {
            (0, Family::Minor) => (
                "Home (tonic)",
                "This is home in a minor key: calm, settled, a bit melancholy. Songs usually end here.",
                -0.1,
            ),
            (0, _) => (
                "Home, made major",
                "The home chord turned major: a surprise ray of light (the 'Picardy third').",
                0.05,
            ),
            (2, Family::Dim) => (
                "Leads to V",
                "ii° is tense and usually walks to V, then home.",
                0.15,
            ),
            (3, Family::Major) => (
                "Relative major",
                "bIII is the major key hiding inside this minor key: the same notes, brighter mood.",
                0.0,
            ),
            (5, Family::Minor) => (
                "Away (subdominant)",
                "iv moves away from home: heavier and sadder, but not tense.",
                0.1,
            ),
            (5, Family::Major) => (
                "Bright IV",
                "A major IV in a minor key: a hopeful lift (Dorian flavour).",
                0.1,
            ),
            (7, Family::Major) => (
                "Tension (dominant)",
                "A major V in a minor key (from harmonic minor): strong pull back to i.",
                0.3,
            ),
            (7, Family::Minor) => (
                "Soft dominant",
                "Minor v: a gentle step away; it doesn't pull home as hard as major V.",
                0.15,
            ),
            (8, Family::Major) => (
                "Epic (bVI)",
                "bVI: warm and cinematic. bVI → bVII → i is the 'epic' build-up.",
                0.1,
            ),
            (10, Family::Major) => (
                "Leads home (bVII)",
                "bVII: a strong, folky step that walks back up to i.",
                0.15,
            ),
            (11, Family::Dim) => (
                "Leading chord",
                "vii°: very unstable, pulls straight up to i.",
                0.3,
            ),
            _ => (
                "Outside colour",
                "This chord isn't part of the key; it adds a surprise colour.",
                0.25,
            ),
        };
    }
    match (off, fam) {
        (0, Family::Major) => (
            "Home (tonic)",
            "I is home: everything feels at rest. Songs usually start and end here.",
            -0.1,
        ),
        (0, Family::Minor) => (
            "Home, made minor",
            "The home chord turned minor (borrowed from the minor key): sudden shadow.",
            0.15,
        ),
        (2, Family::Minor) => (
            "Prepares V",
            "ii is a step away from home. It loves to go to V (ii → V → I is the backbone of jazz).",
            0.05,
        ),
        (2, Family::Major) => (
            "V of V",
            "II major is a 'secondary dominant': it acts like the V of V and pushes hard into V.",
            0.25,
        ),
        (4, Family::Minor) => (
            "Gentle (iii)",
            "iii shares two notes with I, so it feels close to home but a little wistful.",
            0.0,
        ),
        (4, Family::Major) => (
            "V of vi",
            "III major pushes into vi, a dramatic, emotional move.",
            0.25,
        ),
        (5, Family::Major) => (
            "Away (subdominant)",
            "IV moves away from home: open, hopeful, like taking a breath. IV → I sounds like 'Amen'.",
            0.05,
        ),
        (5, Family::Minor) => (
            "Bittersweet (iv)",
            "iv borrowed from the minor key: the same chord turned sad. iv → I is a tear-jerker ending.",
            0.15,
        ),
        (7, Family::Major) => (
            "Tension (dominant)",
            "V is the tension chord: it contains the note right under home (the 'leading note') and wants to resolve to I.",
            0.3,
        ),
        (7, Family::Minor) => (
            "Soft dominant",
            "Minor v: a softer, modal step away (Mixolydian / folk sound).",
            0.1,
        ),
        (9, Family::Minor) => (
            "Relative minor",
            "vi is I's sad twin: it shares two notes with I. Pop songs swing between them.",
            0.0,
        ),
        (9, Family::Major) => (
            "V of ii",
            "VI major pushes toward ii; a bright, old-time sound.",
            0.2,
        ),
        (10, Family::Major) => (
            "Rock colour (bVII)",
            "bVII is borrowed from Mixolydian/minor: a bold, rock-and-roll move. bVII → I is a classic rock ending.",
            0.1,
        ),
        (8, Family::Major) => (
            "Epic (bVI)",
            "bVI is borrowed from the minor key: cinematic and huge. Try bVI → bVII → I.",
            0.15,
        ),
        (3, Family::Major) => (
            "Borrowed (bIII)",
            "bIII is borrowed from the minor key: bold and surprising.",
            0.15,
        ),
        (1, Family::Major) => (
            "Dramatic (bII)",
            "bII (the 'Neapolitan'): dramatic, slips down to I or V.",
            0.35,
        ),
        (11, Family::Dim) => (
            "Leading chord",
            "vii° is very unstable: it's basically V7 without its root, and it pulls up to I.",
            0.3,
        ),
        (6, _) => (
            "Far away",
            "A tritone from home: as far from the key as you can get. Very unstable.",
            0.4,
        ),
        _ => (
            "Outside colour",
            "This chord isn't part of the key; it adds a surprise colour.",
            0.25,
        ),
    }
}

/// How the move from the previous chord sounds (cadences and classic moves).
pub fn motion(prev: (i32, Family), cur: (i32, Family), scale: Scale) -> Option<String> {
    let (p, pf) = (prev.0.rem_euclid(12), prev.1);
    let (c, cf) = (cur.0.rem_euclid(12), cur.1);
    if (p, pf) == (c, cf) {
        return None;
    }
    let home = if minor_home(scale) { "i" } else { "I" };
    let s = match (p, pf, c, cf) {
        (7, Family::Major, 0, _) => format!("V → {home}: tension → release. This is a cadence: it sounds like “the end”."),
        (11, Family::Dim, 0, _) => format!("vii° → {home}: the leading chord resolves home."),
        (5, Family::Major, 0, Family::Major) => "IV → I: the soft “Amen” ending (plagal cadence).".into(),
        (5, Family::Minor, 0, Family::Major) => "iv → I: bittersweet, cinematic ending.".into(),
        (7, Family::Major, 9, Family::Minor) => "V → vi: a fake-out (deceptive cadence). Your ear expected home and got its sad twin.".into(),
        (2, Family::Minor, 7, _) | (2, Family::Dim, 7, _) => format!("ii → V: the classic set-up. Now play {home} to finish the ii–V–I."),
        (2, Family::Major, 7, _) => "II → V: V of V leading into V. The pull keeps getting stronger.".into(),
        (10, Family::Major, 0, _) => format!("bVII → {home}: the rock-and-roll ending."),
        (8, Family::Major, 10, Family::Major) => format!("bVI → bVII: the “epic” climb. Land on {home} next!"),
        (0, _, 7, Family::Major) => "Home → V: you've stepped to the tension chord; your ear wants to come back.".into(),
        (0, _, 5, _) => "Home → IV: stepping away, opening up.".into(),
        (0, Family::Major, 9, Family::Minor) => "I → vi: happy to sad, using two shared notes.".into(),
        (9, Family::Minor, 5, Family::Major) => "vi → IV: a lift from sad to hopeful (this is half of the pop “four chord” loop).".into(),
        _ if (p - c).rem_euclid(12) == 7 => "The root fell a 5th: one step around the circle of fifths, the strongest, most natural move there is.".into(),
        _ if (c - p).rem_euclid(12) == 7 => "The root rose a 5th: a step the other way round the circle of fifths. Bright, opening.".into(),
        _ if matches!((c - p).rem_euclid(12), 1 | 2 | 10 | 11) => "Roots a step apart: the chords share no notes, so it's a clear, bold change.".into(),
        _ => return None,
    };
    Some(s)
}

/// What a single note does in the key, and how settled it feels.
fn degree_text(off: i32, scale: Scale) -> (&'static str, f32) {
    let minor = minor_home(scale);
    match (off, minor) {
        (0, _) => ("the home note: it sounds finished, at rest.", 0.0),
        (7, _) => (
            "the 5th: strong and stable, the backbone of the home chord.",
            0.1,
        ),
        (4, false) => ("the 3rd: it's what makes the key major (happy).", 0.15),
        (3, true) => ("the 3rd: it's what makes the key minor (sad).", 0.15),
        (2, _) => (
            "the 2nd: a passing note that wants to step down to home or up to the 3rd.",
            0.4,
        ),
        (5, _) => ("the 4th: it leans down onto the 3rd.", 0.45),
        (9, false) => (
            "the 6th: sweet; it's the home note of the relative minor.",
            0.3,
        ),
        (8, true) => ("the 6th: dark and heavy; it leans down to the 5th.", 0.4),
        (9, true) => (
            "the raised 6th (Dorian): a hopeful lift inside a minor key.",
            0.35,
        ),
        (11, _) => (
            "the 7th, the 'leading note': it's a half step under home and pulls up to it.",
            0.7,
        ),
        (10, _) => (
            "the flat 7th: bluesy and relaxed, it doesn't pull home as hard.",
            0.35,
        ),
        (6, _) => (
            "the 'blue note' (flat 5th): gritty, slide off it to the 4th or 5th.",
            0.7,
        ),
        (3, false) => ("the flat 3rd: the 'blues' 3rd in a major key.", 0.5),
        (8, false) => ("the flat 6th: borrowed from minor, bittersweet.", 0.5),
        (1, _) => (
            "a half step above home: very tense, it wants to slide back down.",
            0.8,
        ),
        _ => ("a passing note.", 0.4),
    }
}

/// Everything the overlay shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Explanation {
    /// Big text: chord name, note name or "C → E".
    pub title: String,
    /// Roman numeral, if it's a chord.
    pub numeral: Option<String>,
    /// Short job label ("Tension (dominant)", "the 3rd", "Major 3rd").
    pub label: String,
    /// One-line mood ("Happy, bright, settled").
    pub feel: String,
    /// Longer "why" sentences.
    pub why: Vec<String>,
    /// 0 = completely at rest, 1 = very tense.
    pub tension: f32,
    /// Notes outside the key (names).
    pub outside: Vec<String>,
    /// Song examples (intervals).
    pub songs: Vec<String>,
    /// Root offset from the key's tonic and family, for `motion`.
    pub step: Option<(i32, Family)>,
}

/// What's sounding.
pub struct Heard<'a> {
    /// Single notes held (melody / piano keys).
    pub melody: &'a [i32],
    /// The chord from a chord key, if one is sounding.
    pub chord: Option<&'a Chord>,
    pub tonic_pc: i32,
    pub scale: Scale,
}

pub fn explain(h: &Heard) -> Option<Explanation> {
    let tonic = h.tonic_pc.rem_euclid(12);
    let flats = h.scale.prefers_flats(tonic);
    let key = format!(
        "{} {}",
        pc_name(tonic, flats),
        h.scale.name().to_lowercase()
    );
    let sp = |n: i32| spell(n, tonic, h.scale);
    // Melody notes, outside the key.
    let outside_of = |notes: &[i32]| -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for n in notes {
            if !h.scale.chord_steps().contains(&(n - tonic).rem_euclid(12)) {
                let s = sp(*n).to_string();
                if !v.contains(&s) {
                    v.push(s);
                }
            }
        }
        v
    };

    // ---- A chord key is sounding.
    if let Some(ch) = h.chord {
        let pcs: Vec<i32> = ch
            .tones
            .iter()
            .map(|t| (ch.root_pc + t).rem_euclid(12))
            .collect();
        let shape = shape_for(ch.root_pc, &pcs);
        let family = shape.map(|s| s.family).unwrap_or(Family::Major);
        let off = (ch.root_pc - tonic).rem_euclid(12);
        let (label, role_text, extra) = role(off, family, h.scale);
        let mut why = vec![role_text.to_string()];
        if let Some(s) = shape {
            why.push(s.why.to_string());
        }
        let mut all: Vec<i32> = pcs.clone();
        all.extend(h.melody.iter().copied());
        for m in h.melody {
            why.push(melody_over_chord(
                *m, ch.root_pc, &pcs, &ch.name, h.scale, tonic,
            ));
        }
        let outside = outside_of(&all);
        let tension =
            (shape.map(|s| s.tension).unwrap_or(0.4) + extra + 0.1 * outside.len() as f32)
                .clamp(0.0, 1.0);
        return Some(Explanation {
            title: ch.name.clone(),
            numeral: Some(format!("{} in {key}", ch.numeral)),
            label: label.into(),
            feel: shape.map(|s| s.feel).unwrap_or("A colourful chord").into(),
            why,
            tension,
            outside,
            songs: Vec::new(),
            step: Some((off, family)),
        });
    }

    let mut notes: Vec<i32> = h.melody.to_vec();
    notes.sort();
    notes.dedup();
    let outside = outside_of(&notes);
    match notes.len() {
        0 => None,
        1 => {
            let n = notes[0];
            let off = (n - tonic).rem_euclid(12);
            let (text, tension) = degree_text(off, h.scale);
            let in_key = outside.is_empty();
            let steps = h.scale.chord_steps();
            let label = match steps.iter().position(|s| *s == off) {
                Some(i) if in_key => format!("note {} of {key}", i + 1),
                _ => format!("outside {key}"),
            };
            let mut why = vec![format!("{} is {text}", sp(n))];
            if !in_key {
                why.push(
                    "Notes outside the key are called 'chromatic'. They add spice; \
                     slide to a neighbouring key note to make them sound intentional."
                        .into(),
                );
            }
            why.push(format!(
                "It's {} (octave number {}). The same letter higher or lower is the same note in another octave.",
                crate::music::note_name(n),
                n.div_euclid(12) - 1
            ));
            Some(Explanation {
                title: sp(n).to_string(),
                numeral: None,
                label,
                feel: if in_key {
                    "Part of the key".into()
                } else {
                    "Outside the key: spicy".into()
                },
                why,
                tension: if in_key { tension } else { tension.max(0.75) },
                outside,
                songs: Vec::new(),
                step: None,
            })
        }
        2 if identify(&notes).is_none() => {
            let (lo, hi) = (notes[0], notes[1]);
            let iv = interval(hi - lo);
            let mut why = vec![format!(
                "{} → {}: {} half steps apart. {}.",
                sp(lo),
                sp(hi),
                hi - lo,
                capitalise(iv.mood)
            )];
            if matches!(iv.semitones, 3 | 4) {
                why.push(
                    "Add the note a 5th above the lower one and you have a full chord.".into(),
                );
            }
            if hi - lo > 12 {
                why.push(format!(
                    "(Bigger than an octave: it's a {} plus an octave.)",
                    iv.name
                ));
            }
            Some(Explanation {
                title: format!("{} + {}", sp(lo), sp(hi)),
                numeral: None,
                label: iv.name.to_string(),
                feel: capitalise(iv.mood),
                why,
                tension: (iv.tension + 0.1 * outside.len() as f32).clamp(0.0, 1.0),
                outside,
                songs: iv.songs.iter().map(|s| s.to_string()).collect(),
                step: None,
            })
        }
        _ => match identify(&notes) {
            Some(f) => {
                let off = (f.root_pc - tonic).rem_euclid(12);
                let (label, role_text, extra) = role(off, f.shape.family, h.scale);
                let name = format!("{}{}", sp(f.root_pc), f.shape.suffix);
                let mut why = vec![role_text.to_string(), f.shape.why.to_string()];
                let title = if f.bass_pc != f.root_pc {
                    why.push(format!(
                        "{} is in the bass, so this is an inversion: the same chord, stacked differently. It sounds lighter than root position.",
                        sp(f.bass_pc)
                    ));
                    format!("{name}/{}", sp(f.bass_pc))
                } else {
                    name
                };
                let tension =
                    (f.shape.tension + extra + 0.1 * outside.len() as f32).clamp(0.0, 1.0);
                Some(Explanation {
                    title,
                    numeral: Some(format!(
                        "{} in {key}",
                        numeral(off, f.shape.family, f.shape.numeral_ext)
                    )),
                    label: label.into(),
                    feel: f.shape.feel.into(),
                    why,
                    tension,
                    outside,
                    songs: Vec::new(),
                    step: Some((off, f.shape.family)),
                })
            }
            None => {
                let names: Vec<&str> = notes.iter().map(|n| sp(*n)).collect();
                let close = notes.windows(2).any(|w| w[1] - w[0] <= 2);
                Some(Explanation {
                    title: names.join(" "),
                    numeral: None,
                    label: "Not a standard chord".into(),
                    feel: if close {
                        "Crunchy (a 'cluster')".into()
                    } else {
                        "Mixed colours".into()
                    },
                    why: vec![if close {
                        "Some of these notes are right next to each other, so they rub. Spread them out, or drop one, to hear a chord.".into()
                    } else {
                        "These notes don't stack into a common chord shape. Try root, 3rd and 5th: every other white key.".into()
                    }],
                    tension: if close { 0.8 } else { 0.5 },
                    outside,
                    songs: Vec::new(),
                    step: None,
                })
            }
        },
    }
}

fn melody_over_chord(
    m: i32,
    root_pc: i32,
    pcs: &[i32],
    chord_name: &str,
    scale: Scale,
    tonic: i32,
) -> String {
    let rel = (m - root_pc).rem_euclid(12);
    let name = spell(m, tonic, scale);
    if pcs.contains(&m.rem_euclid(12)) {
        let part = match rel {
            0 => "root",
            3 | 4 => "3rd",
            5 => "4th",
            2 => "9th",
            6..=8 => "5th",
            9 => "6th",
            _ => "7th",
        };
        format!("Melody {name} is the {part} of {chord_name}: it fits, so it sounds settled.")
    } else if scale.contains(tonic, m) {
        format!(
            "Melody {name} isn't in {chord_name} but is in the key: a passing note. Step to a chord note to relax it."
        )
    } else {
        format!("Melody {name} is outside the key: it clashes on purpose (or by accident!).")
    }
}

/// Spell a note in a key: key notes the key's way, borrowed notes the usual
/// way (b3, b6, b7, b2 as flats; the 4th raised in major, lowered 5th in minor).
pub fn spell(n: i32, tonic: i32, scale: Scale) -> &'static str {
    let off = (n - tonic).rem_euclid(12);
    let key_flats = scale.prefers_flats(tonic.rem_euclid(12));
    let flats = if scale.chord_steps().contains(&off) {
        key_flats
    } else {
        match off {
            1 | 3 | 8 | 10 => true,
            6 => minor_home(scale),
            _ => key_flats,
        }
    };
    pc_name(n, flats)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theory::{ChordMod, ChordRef, build_chord};

    fn heard(notes: &[i32]) -> Option<Explanation> {
        explain(&Heard {
            melody: notes,
            chord: None,
            tonic_pc: 0,
            scale: Scale::Major,
        })
    }

    #[test]
    fn identifies_chords() {
        let f = identify(&[60, 64, 67]).unwrap();
        assert_eq!((f.root_pc, f.shape.suffix), (0, ""));
        let f = identify(&[57, 60, 64]).unwrap();
        assert_eq!((f.root_pc, f.shape.suffix), (9, "m"));
        // C6 vs Am7: the bass decides.
        assert_eq!(identify(&[60, 64, 67, 69]).unwrap().shape.suffix, "6");
        assert_eq!(identify(&[57, 60, 64, 67]).unwrap().shape.suffix, "m7");
        // Inversion: E in the bass.
        let f = identify(&[64, 67, 72]).unwrap();
        assert_eq!((f.root_pc, f.bass_pc), (0, 4));
        assert!(identify(&[60, 64]).is_none());
        assert_eq!(identify(&[60, 67]).unwrap().shape.suffix, "5");
    }

    #[test]
    fn numerals() {
        assert_eq!(numeral(7, Family::Major, "7"), "V7");
        assert_eq!(numeral(9, Family::Minor, ""), "vi");
        assert_eq!(numeral(11, Family::Dim, ""), "vii°");
        assert_eq!(numeral(11, Family::Dim, "ø7"), "viiø7");
        assert_eq!(numeral(10, Family::Major, ""), "bVII");
        assert_eq!(numeral(3, Family::Major, ""), "bIII");
    }

    #[test]
    fn explains_free_play() {
        let e = heard(&[55, 59, 62, 65]).unwrap();
        assert_eq!(e.title, "G7");
        assert_eq!(e.numeral.as_deref(), Some("V7 in C major"));
        assert!(e.label.contains("dominant"), "{}", e.label);
        assert!(e.tension > 0.7);

        let e = heard(&[60, 64, 67]).unwrap();
        assert_eq!(e.numeral.as_deref(), Some("I in C major"));
        assert!(e.tension < 0.1);
        assert!(e.feel.contains("Happy"));

        let e = heard(&[57, 60, 64]).unwrap();
        assert!(e.feel.contains("Sad"));

        let e = heard(&[60, 64]).unwrap();
        assert_eq!(e.label, "Major 3rd");
        assert!(!e.songs.is_empty());

        let e = heard(&[61]).unwrap();
        assert_eq!(e.outside, vec!["Db"]);
        let e = heard(&[71]).unwrap();
        assert!(e.why[0].contains("leading note"));
        assert!(heard(&[]).is_none());

        // In A minor, Am is home.
        let e = explain(&Heard {
            melody: &[57, 60, 64],
            chord: None,
            tonic_pc: 9,
            scale: Scale::Minor,
        })
        .unwrap();
        assert_eq!(e.label, "Home (tonic)");
        assert_eq!(e.numeral.as_deref(), Some("i in A minor"));
    }

    #[test]
    fn explains_chord_keys() {
        let g7 = build_chord(
            ChordRef::Diatonic(5),
            ChordMod::Seventh.bit(),
            0,
            Scale::Major,
        );
        let e = explain(&Heard {
            melody: &[72],
            chord: Some(&g7),
            tonic_pc: 0,
            scale: Scale::Major,
        })
        .unwrap();
        assert_eq!(e.title, "G7");
        assert_eq!(e.numeral.as_deref(), Some("V7 in C major"));
        assert!(e.why.iter().any(|w| w.contains("passing")), "{:?}", e.why);
        assert_eq!(e.step, Some((7, Family::Major)));
        let bb = build_chord(ChordRef::parse("bVII").unwrap(), 0, 0, Scale::Major);
        let e = explain(&Heard {
            melody: &[],
            chord: Some(&bb),
            tonic_pc: 0,
            scale: Scale::Major,
        })
        .unwrap();
        assert_eq!(e.outside, vec!["Bb"]);
    }

    #[test]
    fn cadences() {
        let s = Scale::Major;
        assert!(
            motion((7, Family::Major), (0, Family::Major), s)
                .unwrap()
                .contains("cadence")
        );
        assert!(
            motion((7, Family::Major), (9, Family::Minor), s)
                .unwrap()
                .contains("deceptive")
        );
        assert!(
            motion((5, Family::Major), (0, Family::Major), s)
                .unwrap()
                .contains("Amen")
        );
        assert!(motion((0, Family::Major), (0, Family::Major), s).is_none());
        // A → D: a 5th down, generic circle move.
        assert!(
            motion((9, Family::Major), (2, Family::Major), s)
                .unwrap()
                .contains("circle")
        );
    }
}
