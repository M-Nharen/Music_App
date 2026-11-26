//! Keys, scales, chords and voice-leading for the Chords + Melody layout.
//!
//! Everything here is pure (no UI, no audio) so it is easy to test.

use serde::{Deserialize, Serialize};
use std::fmt;

const SHARP_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLAT_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

/// Pitch-class name spelled with sharps or flats.
pub fn pc_name(pc: i32, flats: bool) -> &'static str {
    let i = pc.rem_euclid(12) as usize;
    if flats { FLAT_NAMES[i] } else { SHARP_NAMES[i] }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scale {
    #[default]
    Major,
    Minor,
    Dorian,
    Mixolydian,
    HarmonicMinor,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
}

const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

impl Scale {
    pub const ALL: [Scale; 8] = [
        Scale::Major,
        Scale::Minor,
        Scale::Dorian,
        Scale::Mixolydian,
        Scale::HarmonicMinor,
        Scale::MajorPentatonic,
        Scale::MinorPentatonic,
        Scale::Blues,
    ];

    /// Semitones above the tonic for the notes the melody keys play.
    pub fn steps(self) -> &'static [i32] {
        match self {
            Scale::Major => &MAJOR,
            Scale::Minor => &MINOR,
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Blues => &[0, 3, 5, 6, 7, 10],
        }
    }

    /// The 7-note scale the chord keys are built from. Pentatonic and blues
    /// scales borrow the chords of their parent major / minor scale.
    pub fn chord_steps(self) -> &'static [i32] {
        match self {
            Scale::MajorPentatonic => &MAJOR,
            Scale::MinorPentatonic | Scale::Blues => &MINOR,
            s => s.steps(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Scale::Major => "Major",
            Scale::Minor => "Minor",
            Scale::Dorian => "Dorian",
            Scale::Mixolydian => "Mixolydian",
            Scale::HarmonicMinor => "Harmonic minor",
            Scale::MajorPentatonic => "Major pentatonic",
            Scale::MinorPentatonic => "Minor pentatonic",
            Scale::Blues => "Blues",
        }
    }

    /// One-line, beginner-friendly description of the mood.
    pub fn mood(self) -> &'static str {
        match self {
            Scale::Major => "bright and happy",
            Scale::Minor => "sad or serious",
            Scale::Dorian => "minor but hopeful (jazz, funk)",
            Scale::Mixolydian => "major with a rock/folk edge",
            Scale::HarmonicMinor => "dramatic, classical or Middle-Eastern",
            Scale::MajorPentatonic => "5 notes, impossible to play a wrong note",
            Scale::MinorPentatonic => "5 notes, rock and blues solos",
            Scale::Blues => "minor pentatonic plus the 'blue' note",
        }
    }

    /// Distance from this scale's tonic to the tonic of the major key with
    /// the same notes, used to choose sharps or flats.
    fn to_relative_major(self) -> i32 {
        match self {
            Scale::Major | Scale::MajorPentatonic => 0,
            Scale::Minor | Scale::HarmonicMinor | Scale::MinorPentatonic | Scale::Blues => 3,
            Scale::Dorian => -2,
            Scale::Mixolydian => 5,
        }
    }

    /// Should note names in this key use flats (F, Bb, Eb … majors)?
    pub fn prefers_flats(self, tonic_pc: i32) -> bool {
        matches!(
            (tonic_pc + self.to_relative_major()).rem_euclid(12),
            5 | 10 | 3 | 8 | 1 | 6
        )
    }

    /// MIDI note of scale step `n` above `tonic` (n can be negative or past
    /// the octave).
    pub fn note(self, tonic: i32, n: i32) -> i32 {
        let s = self.steps();
        let len = s.len() as i32;
        tonic + s[n.rem_euclid(len) as usize] + 12 * n.div_euclid(len)
    }

    pub fn contains(self, tonic_pc: i32, note: i32) -> bool {
        self.steps().contains(&(note - tonic_pc).rem_euclid(12))
    }

    /// Nearest note in the scale (ties go up).
    pub fn snap(self, tonic_pc: i32, note: i32) -> i32 {
        for d in [0, 1, -1, 2, -2, 3, -3] {
            if self.contains(tonic_pc, note + d) {
                return note + d;
            }
        }
        note
    }

    pub fn next(self) -> Scale {
        let i = Scale::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Scale::ALL[(i + 1) % Scale::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Major,
    Minor,
    Dim,
    Aug,
}

impl Quality {
    fn from_intervals(third: i32, fifth: i32) -> Quality {
        match (third, fifth) {
            (3, 6) => Quality::Dim,
            (4, 8) => Quality::Aug,
            (3, _) => Quality::Minor,
            _ => Quality::Major,
        }
    }
    fn third_fifth(self) -> (i32, i32) {
        match self {
            Quality::Major => (4, 7),
            Quality::Minor => (3, 7),
            Quality::Dim => (3, 6),
            Quality::Aug => (4, 8),
        }
    }
}

const ROMANS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// What a chord key plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChordRef {
    /// Scale degree 1–7: the chord that naturally belongs to the key.
    Diatonic(u8),
    /// A fixed chord relative to the tonic (borrowed / secondary chords),
    /// e.g. bVII = root 10 semitones up, major.
    Fixed { root: u8, quality: Quality },
    /// An exact chord by name, whatever the key: "G7", "F#m". Used by the
    /// song layouts (Part 5), where each key plays one chord of the song.
    Named { root: u8, kind: ChordKind },
}

/// Chord types a song's chord sheet uses (the analyzer's `quality` values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChordKind {
    Major,
    Minor,
    Dom7,
    Maj7,
    Min7,
    Dim,
    Aug,
    Sus2,
    Sus4,
}

impl ChordKind {
    pub const ALL: [ChordKind; 9] = [
        ChordKind::Major,
        ChordKind::Minor,
        ChordKind::Dom7,
        ChordKind::Maj7,
        ChordKind::Min7,
        ChordKind::Dim,
        ChordKind::Aug,
        ChordKind::Sus2,
        ChordKind::Sus4,
    ];

    /// Semitones above the root.
    pub fn intervals(self) -> &'static [i32] {
        match self {
            ChordKind::Major => &[0, 4, 7],
            ChordKind::Minor => &[0, 3, 7],
            ChordKind::Dom7 => &[0, 4, 7, 10],
            ChordKind::Maj7 => &[0, 4, 7, 11],
            ChordKind::Min7 => &[0, 3, 7, 10],
            ChordKind::Dim => &[0, 3, 6],
            ChordKind::Aug => &[0, 4, 8],
            ChordKind::Sus2 => &[0, 2, 7],
            ChordKind::Sus4 => &[0, 5, 7],
        }
    }

    /// Name suffix: "", "m", "7", "maj7", "m7", "dim", "aug", "sus2", "sus4".
    pub fn suffix(self) -> &'static str {
        match self {
            ChordKind::Major => "",
            ChordKind::Minor => "m",
            ChordKind::Dom7 => "7",
            ChordKind::Maj7 => "maj7",
            ChordKind::Min7 => "m7",
            ChordKind::Dim => "dim",
            ChordKind::Aug => "aug",
            ChordKind::Sus2 => "sus2",
            ChordKind::Sus4 => "sus4",
        }
    }

    /// From a name suffix or the analyzer's quality ("maj", "min", "7"…).
    pub fn parse(s: &str) -> Option<ChordKind> {
        Some(match s.trim() {
            "" | "maj" | "M" => ChordKind::Major,
            "m" | "min" | "-" => ChordKind::Minor,
            "7" | "dom7" => ChordKind::Dom7,
            "maj7" | "M7" | "Δ" | "Δ7" => ChordKind::Maj7,
            "m7" | "min7" | "-7" => ChordKind::Min7,
            "dim" | "°" | "o" => ChordKind::Dim,
            "aug" | "+" => ChordKind::Aug,
            "sus2" => ChordKind::Sus2,
            "sus4" | "sus" => ChordKind::Sus4,
            _ => return None,
        })
    }

    /// The plain three-note chord this one is built on (G7 → G).
    pub fn triad(self) -> ChordKind {
        match self {
            ChordKind::Dom7 | ChordKind::Maj7 => ChordKind::Major,
            ChordKind::Min7 => ChordKind::Minor,
            k => k,
        }
    }

    /// Minor-sounding (minor third, no major third).
    pub fn is_minor(self) -> bool {
        matches!(self, ChordKind::Minor | ChordKind::Min7 | ChordKind::Dim)
    }

    /// The kind with exactly these intervals above the root, if any.
    pub fn from_intervals(iv: &[i32]) -> Option<ChordKind> {
        let mut v: Vec<i32> = iv.iter().map(|i| i.rem_euclid(12)).collect();
        v.sort();
        v.dedup();
        ChordKind::ALL
            .into_iter()
            .find(|k| k.intervals() == v.as_slice())
    }
}

/// "G7" for a root pitch class and kind (sharps, like the analyzer).
pub fn chord_symbol(root_pc: i32, kind: ChordKind) -> String {
    format!("{}{}", pc_name(root_pc, false), kind.suffix())
}

/// Parse "G7", "F#m", "Bbmaj7" (the bass of "C/E" is ignored).
pub fn parse_chord_symbol(s: &str) -> Option<(i32, ChordKind)> {
    let s = s.split('/').next()?.trim();
    let mut chars = s.chars();
    let letter = chars.next()?;
    let base: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (acc, rest) = if let Some(r) = rest.strip_prefix('#').or_else(|| rest.strip_prefix('♯')) {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b').or_else(|| rest.strip_prefix('♭')) {
        (-1, r)
    } else {
        (0, rest)
    };
    Some(((base + acc).rem_euclid(12), ChordKind::parse(rest)?))
}

impl ChordRef {
    /// Parse `1`..`7` (diatonic) or a roman numeral such as `bVII`, `iv`,
    /// `II`, `vii°` / `viio`, `III+`.
    pub fn parse(s: &str) -> Option<ChordRef> {
        let s = s.trim();
        if let Ok(n) = s.parse::<u8>() {
            return (1..=7).contains(&n).then_some(ChordRef::Diatonic(n));
        }
        // Chord names start with a capital letter A–G ("G7", "Bb", "F#m");
        // roman numerals never do.
        if s.starts_with(|c: char| ('A'..='G').contains(&c)) {
            let (root, kind) = parse_chord_symbol(s)?;
            return Some(ChordRef::Named {
                root: root as u8,
                kind,
            });
        }
        let mut rest = s;
        let mut acc = 0;
        loop {
            if let Some(r) = rest.strip_prefix('b').or_else(|| rest.strip_prefix('♭')) {
                acc -= 1;
                rest = r;
            } else if let Some(r) = rest.strip_prefix('#').or_else(|| rest.strip_prefix('♯')) {
                acc += 1;
                rest = r;
            } else {
                break;
            }
        }
        let len = rest
            .chars()
            .take_while(|c| matches!(c, 'I' | 'V' | 'i' | 'v'))
            .count();
        let (numeral, suffix) = rest.split_at(len);
        let upper = numeral.to_ascii_uppercase();
        let deg = ROMANS.iter().position(|r| *r == upper)?;
        let is_upper = numeral.chars().all(|c| c.is_ascii_uppercase());
        let is_lower = numeral.chars().all(|c| c.is_ascii_lowercase());
        let quality = match suffix.trim() {
            "°" | "o" | "dim" => Quality::Dim,
            "+" | "aug" => Quality::Aug,
            "" if is_upper => Quality::Major,
            "" if is_lower => Quality::Minor,
            _ => return None,
        };
        let root = (MAJOR[deg] + acc).rem_euclid(12) as u8;
        Some(ChordRef::Fixed { root, quality })
    }

    /// Common chords offered in the keybind editor.
    pub fn presets() -> Vec<ChordRef> {
        let mut v: Vec<ChordRef> = (1..=7).map(ChordRef::Diatonic).collect();
        for s in [
            "i", "II", "bII", "bIII", "III", "iv", "v", "bVI", "VI", "bVII", "VII",
        ] {
            v.extend(ChordRef::parse(s));
        }
        v
    }

    /// Text for the keymap file ("1".."7" or a roman numeral).
    pub fn code(&self) -> String {
        match *self {
            ChordRef::Diatonic(d) => d.to_string(),
            ChordRef::Fixed { root, quality } => fixed_numeral(root as i32, quality),
            ChordRef::Named { root, kind } => chord_symbol(root as i32, kind),
        }
    }

    /// Plain-language description for the keybind editor.
    pub fn describe(&self) -> String {
        match *self {
            ChordRef::Diatonic(d) => format!(
                "Chord on scale step {d} ({})",
                ["I", "ii", "iii", "IV", "V", "vi", "vii°"][(d as usize - 1).min(6)]
            ),
            ChordRef::Fixed { .. } => format!("Borrowed chord {}", self.code()),
            ChordRef::Named { .. } => format!("Chord {}", self.code()),
        }
    }
}

impl fmt::Display for ChordRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.code())
    }
}

fn fixed_numeral(root: i32, q: Quality) -> String {
    let root = root.rem_euclid(12);
    let (deg, acc) = match MAJOR.iter().position(|s| *s == root) {
        Some(d) => (d, ""),
        None => (MAJOR.iter().position(|s| *s == root + 1).unwrap_or(0), "b"),
    };
    let base = ROMANS[deg];
    match q {
        Quality::Major => format!("{acc}{base}"),
        Quality::Minor => format!("{acc}{}", base.to_lowercase()),
        Quality::Dim => format!("{acc}{}°", base.to_lowercase()),
        Quality::Aug => format!("{acc}{base}+"),
    }
}

/// Hold-to-change chord modifiers. Several can be held at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChordMod {
    Seventh,
    Sus4,
    Add9,
    /// Major ↔ minor.
    Swap,
}

impl ChordMod {
    pub const ALL: [ChordMod; 4] = [
        ChordMod::Seventh,
        ChordMod::Sus4,
        ChordMod::Add9,
        ChordMod::Swap,
    ];

    pub fn bit(self) -> u8 {
        match self {
            ChordMod::Seventh => 1,
            ChordMod::Sus4 => 2,
            ChordMod::Add9 => 4,
            ChordMod::Swap => 8,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            ChordMod::Seventh => "7th",
            ChordMod::Sus4 => "sus4",
            ChordMod::Add9 => "add9",
            ChordMod::Swap => "swap",
        }
    }
    pub fn parse(s: &str) -> Option<ChordMod> {
        Some(match s.trim() {
            "7th" | "7" | "seventh" => ChordMod::Seventh,
            "sus4" | "sus" => ChordMod::Sus4,
            "add9" | "9" => ChordMod::Add9,
            "swap" | "minor" | "major" => ChordMod::Swap,
            _ => return None,
        })
    }
    pub fn describe(self) -> &'static str {
        match self {
            ChordMod::Seventh => "Add the 7th (hold)",
            ChordMod::Sus4 => "Suspended 4th (hold)",
            ChordMod::Add9 => "Add the 9th (hold)",
            ChordMod::Swap => "Swap major ↔ minor (hold)",
        }
    }
    /// Tiny label for key caps.
    pub fn short(self) -> &'static str {
        match self {
            ChordMod::Seventh => "+7",
            ChordMod::Sus4 => "sus4",
            ChordMod::Add9 => "+9",
            ChordMod::Swap => "maj/min",
        }
    }
}

/// A fully worked-out chord, ready to name and voice.
#[derive(Clone, Debug, PartialEq)]
pub struct Chord {
    pub root_pc: i32,
    /// Semitones above the root, root first (0, 3rd, 5th, 7th?, 9th?).
    pub tones: Vec<i32>,
    pub name: String,
    pub numeral: String,
}

/// Work out the chord a key plays in this key/scale with these modifiers.
pub fn build_chord(cref: ChordRef, mods: u8, tonic_pc: i32, scale: Scale) -> Chord {
    let has = |m: ChordMod| mods & m.bit() != 0;
    let tonic_pc = tonic_pc.rem_euclid(12);
    if let ChordRef::Named { root, kind } = cref {
        return build_named(root as i32, kind, mods, tonic_pc);
    }
    let (root_off, mut q, mut seventh, flats, base_numeral) = match cref {
        ChordRef::Named { .. } => unreachable!(),
        ChordRef::Diatonic(d) => {
            let s = scale.chord_steps();
            let d = (d.clamp(1, 7) - 1) as usize;
            let at = |k: usize| s[(d + k) % 7] + 12 * ((d + k) / 7) as i32;
            let root = at(0);
            let q = Quality::from_intervals(at(2) - root, at(4) - root);
            let numeral = ROMANS[d].to_string();
            (
                root,
                q,
                at(6) - root,
                scale.prefers_flats(tonic_pc),
                (numeral, String::new()),
            )
        }
        ChordRef::Fixed { root, quality } => {
            let r = root as i32;
            let code = fixed_numeral(r, quality);
            let acc = if code.starts_with('b') { "b" } else { "" };
            let deg = match MAJOR.iter().position(|s| *s == r) {
                Some(d) => d,
                None => MAJOR.iter().position(|s| *s == r + 1).unwrap_or(0),
            };
            (
                r,
                quality,
                10,
                !acc.is_empty() || scale.prefers_flats(tonic_pc),
                (ROMANS[deg].to_string(), acc.to_string()),
            )
        }
    };
    if has(ChordMod::Swap) {
        q = match q {
            Quality::Major | Quality::Aug => Quality::Minor,
            Quality::Minor | Quality::Dim => Quality::Major,
        };
        // A swapped chord is "outside" the key: use the 7th that sounds
        // natural for its new quality (II7, VI7 = secondary dominants).
        seventh = 10;
    }
    let sus = has(ChordMod::Sus4);
    let (mut third, mut fifth) = q.third_fifth();
    if sus {
        third = 5;
        fifth = 7;
    }
    if q == Quality::Dim && seventh == 11 {
        seventh = 10;
    }
    let root_pc = (tonic_pc + root_off).rem_euclid(12);

    let mut tones = vec![0, third, fifth];
    let sev = has(ChordMod::Seventh).then_some(seventh);
    if let Some(s) = sev {
        tones.push(s);
    }
    let add9 = has(ChordMod::Add9);
    if add9 {
        tones.push(14);
    }

    // ---- name, e.g. "G7", "Dm", "Csus4", "Fmaj9"
    let ext = if add9 { "9" } else { "7" };
    let suffix = match (sus, q, sev) {
        (true, _, None) => {
            if add9 {
                "sus4add9".to_string()
            } else {
                "sus4".to_string()
            }
        }
        (true, _, Some(11)) => format!("maj{ext}sus4"),
        (true, _, Some(_)) => format!("{ext}sus4"),
        (false, q, None) => {
            let base = match q {
                Quality::Major => "",
                Quality::Minor => "m",
                Quality::Dim => "dim",
                Quality::Aug => "aug",
            };
            match (add9, q) {
                (false, _) => base.to_string(),
                (true, Quality::Major) => "add9".into(),
                (true, Quality::Minor) => "madd9".into(),
                (true, _) => format!("{base}(add9)"),
            }
        }
        (false, Quality::Major, Some(11)) => format!("maj{ext}"),
        (false, Quality::Major, Some(_)) => ext.to_string(),
        (false, Quality::Minor, Some(11)) => format!("m(maj{ext})"),
        (false, Quality::Minor, Some(_)) => format!("m{ext}"),
        (false, Quality::Dim, Some(9)) => "dim7".into(),
        (false, Quality::Dim, Some(_)) => format!("m{ext}b5"),
        (false, Quality::Aug, Some(11)) => format!("maj{ext}#5"),
        (false, Quality::Aug, Some(_)) => format!("aug{ext}"),
    };
    let name = format!("{}{suffix}", pc_name(root_pc, flats));

    // ---- roman numeral, e.g. "V7", "ii", "bVII", "viiø7"
    let (roman, acc) = base_numeral;
    let mut numeral = match (sus, q) {
        (true, _) | (false, Quality::Major) => format!("{acc}{roman}"),
        (false, Quality::Minor) => format!("{acc}{}", roman.to_lowercase()),
        (false, Quality::Dim) => format!("{acc}{}°", roman.to_lowercase()),
        (false, Quality::Aug) => format!("{acc}{roman}+"),
    };
    if let Some(s) = sev {
        if q == Quality::Dim && s == 10 && !sus {
            numeral = numeral.replace('°', "ø");
        }
        numeral += match (q, s) {
            (_, 11) => "maj7",
            (Quality::Dim, 9) => "7",
            _ => "7",
        };
        if add9 {
            numeral = numeral.replacen('7', "9", 1);
        }
    } else if add9 {
        numeral += "add9";
    }
    if sus {
        numeral += "sus4";
    }

    Chord {
        root_pc,
        tones,
        name,
        numeral,
    }
}

/// A named chord (song layouts) with the hold-to-change modifiers applied.
fn build_named(root_pc: i32, kind: ChordKind, mods: u8, tonic_pc: i32) -> Chord {
    let has = |m: ChordMod| mods & m.bit() != 0;
    let root_pc = root_pc.rem_euclid(12);
    let mut tones: Vec<i32> = kind.intervals().to_vec();
    if has(ChordMod::Swap) {
        for t in tones.iter_mut() {
            *t = match *t {
                4 => 3,
                3 => 4,
                x => x,
            };
        }
    }
    if has(ChordMod::Sus4) {
        for t in tones.iter_mut() {
            if matches!(*t, 2..=4) {
                *t = 5;
            }
        }
    }
    if has(ChordMod::Seventh) && !tones.iter().any(|t| matches!(*t, 10 | 11)) {
        tones.push(10);
    }
    if has(ChordMod::Add9) {
        tones.push(14);
    }
    tones.sort();
    tones.dedup();
    let core: Vec<i32> = tones.iter().copied().filter(|t| *t < 12).collect();
    let name = match ChordKind::from_intervals(&core) {
        Some(k) if !tones.contains(&14) => chord_symbol(root_pc, k),
        Some(k) => format!("{}add9", chord_symbol(root_pc, k)),
        None => {
            let mut n = chord_symbol(root_pc, kind);
            if has(ChordMod::Seventh) {
                n += "+7";
            }
            if has(ChordMod::Add9) {
                n += "add9";
            }
            n
        }
    };
    let q = if core.contains(&3) && !core.contains(&4) {
        if core.contains(&6) && !core.contains(&7) {
            Quality::Dim
        } else {
            Quality::Minor
        }
    } else if core.contains(&8) && !core.contains(&7) {
        Quality::Aug
    } else {
        Quality::Major
    };
    let mut numeral = fixed_numeral((root_pc - tonic_pc).rem_euclid(12), q);
    if core.contains(&11) {
        numeral += "maj7";
    } else if core.contains(&10) {
        numeral += "7";
    }
    if core.contains(&5) && !core.contains(&4) && !core.contains(&3) {
        numeral += "sus4";
    } else if core.contains(&2) && !core.contains(&4) && !core.contains(&3) {
        numeral += "sus2";
    }
    Chord {
        root_pc,
        tones,
        name,
        numeral,
    }
}

/// Pick actual MIDI notes for a chord.
///
/// With `smooth` on, tries every inversion near `center` and keeps the one
/// whose notes move least from `prev` (that's what pianists do: C → F keeps
/// the C and moves the other fingers a step). Without it, plays root
/// position just below `center`.
pub fn voice_chord(chord: &Chord, prev: &[i32], center: i32, smooth: bool) -> Vec<i32> {
    let mut pcs: Vec<i32> = Vec::new();
    for t in &chord.tones {
        let pc = (chord.root_pc + t).rem_euclid(12);
        if !pcs.contains(&pc) {
            pcs.push(pc);
        }
    }
    let stack = |order: &[i32], start: i32| -> Vec<i32> {
        let mut out = vec![start];
        let mut cur = start;
        for pc in &order[1..] {
            let up = (pc - cur).rem_euclid(12);
            cur += if up == 0 { 12 } else { up };
            out.push(cur);
        }
        out
    };
    let nearest = |pc: i32, target: i32| -> i32 {
        let base = target - (target - pc).rem_euclid(12);
        if target - base > 6 { base + 12 } else { base }
    };
    if !smooth || prev.is_empty() {
        let start = nearest(pcs[0], center - 4);
        return stack(&pcs, start);
    }
    let mut best: Option<(f32, Vec<i32>)> = None;
    for rot in 0..pcs.len() {
        let mut order = pcs.clone();
        order.rotate_left(rot);
        for start in (center - 10)..=(center + 4) {
            if (start - order[0]).rem_euclid(12) != 0 {
                continue;
            }
            let cand = stack(&order, start);
            let moves: i32 = cand
                .iter()
                .map(|n| prev.iter().map(|p| (n - p).abs()).min().unwrap_or(0))
                .sum::<i32>()
                + prev
                    .iter()
                    .map(|p| cand.iter().map(|n| (n - p).abs()).min().unwrap_or(0))
                    .sum::<i32>();
            let mean = cand.iter().sum::<i32>() as f32 / cand.len() as f32;
            let spread = (cand[cand.len() - 1] - cand[0]) as f32;
            let cost = moves as f32 + 0.6 * (mean - center as f32).abs() + 0.15 * spread;
            if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                best = Some((cost, cand));
            }
        }
    }
    best.map(|(_, v)| v)
        .unwrap_or_else(|| stack(&pcs, nearest(pcs[0], center - 4)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(c: &str, mods: &[ChordMod], tonic: i32, scale: Scale) -> Chord {
        let bits = mods.iter().fold(0, |a, m| a | m.bit());
        build_chord(ChordRef::parse(c).unwrap(), bits, tonic, scale)
    }

    #[test]
    fn diatonic_chords_in_c_major() {
        let names: Vec<String> = (1..=7)
            .map(|d| build_chord(ChordRef::Diatonic(d), 0, 0, Scale::Major).name)
            .collect();
        assert_eq!(names, ["C", "Dm", "Em", "F", "G", "Am", "Bdim"]);
        let numerals: Vec<String> = (1..=7)
            .map(|d| build_chord(ChordRef::Diatonic(d), 0, 0, Scale::Major).numeral)
            .collect();
        assert_eq!(numerals, ["I", "ii", "iii", "IV", "V", "vi", "vii°"]);
    }

    #[test]
    fn minor_key_and_flats() {
        let names: Vec<String> = (1..=7)
            .map(|d| build_chord(ChordRef::Diatonic(d), 0, 9, Scale::Minor).name)
            .collect();
        assert_eq!(names, ["Am", "Bdim", "C", "Dm", "Em", "F", "G"]);
        // F major uses flats: IV is Bb.
        assert_eq!(
            build_chord(ChordRef::Diatonic(4), 0, 5, Scale::Major).name,
            "Bb"
        );
        // Borrowed chords in C.
        assert_eq!(chord("bVII", &[], 0, Scale::Major).name, "Bb");
        assert_eq!(chord("bVI", &[], 0, Scale::Major).name, "Ab");
        assert_eq!(chord("iv", &[], 0, Scale::Major).name, "Fm");
        assert_eq!(chord("II", &[], 0, Scale::Major).name, "D");
    }

    #[test]
    fn modifiers() {
        use ChordMod::*;
        let g7 = build_chord(ChordRef::Diatonic(5), Seventh.bit(), 0, Scale::Major);
        assert_eq!(g7.name, "G7");
        assert_eq!(g7.numeral, "V7");
        assert_eq!(g7.tones, vec![0, 4, 7, 10]);
        assert_eq!(
            build_chord(ChordRef::Diatonic(1), Seventh.bit(), 0, Scale::Major).name,
            "Cmaj7"
        );
        assert_eq!(
            build_chord(ChordRef::Diatonic(2), Seventh.bit(), 0, Scale::Major).name,
            "Dm7"
        );
        assert_eq!(
            build_chord(ChordRef::Diatonic(7), Seventh.bit(), 0, Scale::Major).name,
            "Bm7b5"
        );
        assert_eq!(
            build_chord(ChordRef::Diatonic(1), Sus4.bit(), 0, Scale::Major).name,
            "Csus4"
        );
        assert_eq!(
            build_chord(ChordRef::Diatonic(1), Add9.bit(), 0, Scale::Major).name,
            "Cadd9"
        );
        assert_eq!(
            build_chord(ChordRef::Diatonic(6), Swap.bit(), 0, Scale::Major).name,
            "A"
        );
        assert_eq!(
            build_chord(
                ChordRef::Diatonic(6),
                Swap.bit() | Seventh.bit(),
                0,
                Scale::Major
            )
            .name,
            "A7"
        );
        assert_eq!(
            build_chord(
                ChordRef::Diatonic(5),
                Seventh.bit() | Add9.bit(),
                0,
                Scale::Major
            )
            .name,
            "G9"
        );
        assert_eq!(
            build_chord(ChordRef::Diatonic(1), Swap.bit(), 0, Scale::Major).numeral,
            "i"
        );
    }

    #[test]
    fn parse_numerals() {
        assert_eq!(ChordRef::parse("5"), Some(ChordRef::Diatonic(5)));
        assert_eq!(ChordRef::parse("8"), None);
        assert_eq!(
            ChordRef::parse("bVII"),
            Some(ChordRef::Fixed {
                root: 10,
                quality: Quality::Major
            })
        );
        assert_eq!(
            ChordRef::parse("vii°"),
            Some(ChordRef::Fixed {
                root: 11,
                quality: Quality::Dim
            })
        );
        assert_eq!(ChordRef::parse("Vx"), None);
        for p in ChordRef::presets() {
            assert_eq!(ChordRef::parse(&p.code()), Some(p), "{}", p.code());
        }
    }

    #[test]
    fn scales_and_snap() {
        assert_eq!(Scale::Major.note(60, 0), 60);
        assert_eq!(Scale::Major.note(60, 7), 72);
        assert_eq!(Scale::Major.note(60, -1), 59);
        assert_eq!(Scale::MajorPentatonic.note(60, 5), 72);
        assert_eq!(Scale::Major.snap(0, 61), 62);
        assert_eq!(Scale::Major.snap(0, 64), 64);
        assert_eq!(Scale::Minor.snap(9, 61), 62); // A minor has no C#
    }

    #[test]
    fn voice_leading_moves_little() {
        let c = build_chord(ChordRef::Diatonic(1), 0, 0, Scale::Major);
        let f = build_chord(ChordRef::Diatonic(4), 0, 0, Scale::Major);
        let g = build_chord(ChordRef::Diatonic(5), 0, 0, Scale::Major);
        let v1 = voice_chord(&c, &[], 55, true);
        assert_eq!(v1, vec![48, 52, 55]);
        let v2 = voice_chord(&f, &v1, 55, true);
        // C stays, E→F, G→A: F/C (48 53 57).
        assert_eq!(v2, vec![48, 53, 57]);
        let v3 = voice_chord(&g, &v2, 55, true);
        let total: i32 = v3.iter().zip(&v2).map(|(a, b)| (a - b).abs()).sum();
        assert!(total <= 6, "{v2:?} -> {v3:?}");
        // Without smoothing, root position.
        assert_eq!(voice_chord(&f, &v1, 55, false), vec![53, 57, 60]);
        // Long progressions don't drift away.
        let mut prev = v1;
        for d in [6, 2, 5, 1, 4, 5, 1, 6, 4, 5, 1, 2, 7, 3, 6] {
            let ch = build_chord(ChordRef::Diatonic(d), 0, 0, Scale::Major);
            prev = voice_chord(&ch, &prev, 55, true);
            assert!(prev.iter().all(|n| (40..=72).contains(n)), "{prev:?}");
        }
    }

    #[test]
    fn named_chords_parse_and_build() {
        use ChordMod::*;
        let g7 = ChordRef::parse("G7").unwrap();
        assert_eq!(
            g7,
            ChordRef::Named {
                root: 7,
                kind: ChordKind::Dom7
            }
        );
        assert_eq!(g7.code(), "G7");
        assert_eq!(ChordRef::parse("Bbm"), ChordRef::parse("A#m"));
        // Roman numerals still work, and "bVII" isn't "B".
        assert!(matches!(
            ChordRef::parse("bVII"),
            Some(ChordRef::Fixed { .. })
        ));
        let c = build_chord(g7, 0, 0, Scale::Major);
        assert_eq!((c.name.as_str(), c.numeral.as_str()), ("G7", "V7"));
        assert_eq!(c.tones, vec![0, 4, 7, 10]);
        // Same chord whatever key the app is in; only the numeral changes.
        let c = build_chord(g7, 0, 7, Scale::Major);
        assert_eq!((c.name.as_str(), c.numeral.as_str()), ("G7", "I7"));
        let am = ChordRef::parse("Am").unwrap();
        assert_eq!(build_chord(am, 0, 0, Scale::Major).numeral, "vi");
        assert_eq!(build_chord(am, Swap.bit(), 0, Scale::Major).name, "A");
        assert_eq!(build_chord(am, Seventh.bit(), 0, Scale::Major).name, "Am7");
        assert_eq!(
            build_chord(ChordRef::parse("D").unwrap(), Sus4.bit(), 0, Scale::Major).name,
            "Dsus4"
        );
        assert_eq!(parse_chord_symbol("C/E"), Some((0, ChordKind::Major)));
        assert_eq!(parse_chord_symbol("N"), None);
        assert_eq!(
            ChordKind::from_intervals(&[0, 7, 3]),
            Some(ChordKind::Minor)
        );
    }
}
