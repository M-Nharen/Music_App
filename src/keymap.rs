//! Keybinds. A keymap is a plain TOML file mapping key names to actions:
//!
//! ```toml
//! name = "Piano"
//! [bindings]
//! Z = "note 0"        # C of the lower octave
//! S = "note 1"        # C#
//! Space = "sustain"
//! Up = "octave+"
//! Shift = "mod 7th"   # Shift, Ctrl and Alt can be bound too
//! ```
//!
//! Key names are egui's (`A`..`Z`, `"1"`..`"0"`, `Comma`, `Period`, `Slash`,
//! `Semicolon`, `Quote`, `OpenBracket`, `CloseBracket`, `Minus`, `Equals`,
//! `Backslash`, `Backtick`, `Space`, `Tab`, `Up`, `Down`, `Left`, `Right`,
//! `PageUp`, `PageDown`, `F1`..`F12`, …) plus `Shift`, `Ctrl` and `Alt`.

use crate::theory::{ChordMod, ChordRef};
use egui::Key;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// A key the app can bind: a normal key or one of the modifier keys
/// (egui reports those separately, and can't tell left from right).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyId {
    Key(Key),
    Shift,
    Ctrl,
    Alt,
}

impl KeyId {
    pub fn name(&self) -> &'static str {
        match self {
            KeyId::Key(k) => k.name(),
            KeyId::Shift => "Shift",
            KeyId::Ctrl => "Ctrl",
            KeyId::Alt => "Alt",
        }
    }

    pub fn from_name(s: &str) -> Option<KeyId> {
        match s {
            "Shift" => Some(KeyId::Shift),
            "Ctrl" | "Control" => Some(KeyId::Ctrl),
            "Alt" => Some(KeyId::Alt),
            _ => Key::from_name(s).map(KeyId::Key),
        }
    }

    /// Short label for drawing a key cap.
    pub fn label(&self) -> &'static str {
        match self {
            KeyId::Key(k) => key_label(*k),
            other => other.name(),
        }
    }
}

impl From<Key> for KeyId {
    fn from(k: Key) -> Self {
        KeyId::Key(k)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Semitones above the layout's base note (C of the lower octave).
    Note(i32),
    /// Scale step above the melody tonic (one octave above the base note).
    /// Always in key, so nothing sounds wrong.
    Melody(i32),
    /// A whole chord from one key.
    Chord(ChordRef),
    /// Hold to change the chord (7th, sus4, add9, major↔minor).
    Mod(ChordMod),
    /// A drum sound (General MIDI drum note).
    Drum(u8),
    OctaveUp,
    OctaveDown,
    TransposeUp,
    TransposeDown,
    /// Hold to sustain, like a piano pedal.
    Sustain,
    InstrumentNext,
    InstrumentPrev,
    /// Cycle soft / normal / loud.
    Velocity,
    /// Stop every sound immediately.
    Panic,
    /// Next / previous chord style (block, strum, arpeggio, patterns).
    StyleNext,
    StylePrev,
    /// Start / stop the accompaniment rhythm.
    Rhythm,
    TempoUp,
    TempoDown,
    /// Next scale (major, minor, …).
    ScaleNext,
    /// Toggle snapping piano keys to the scale.
    ScaleLock,
    /// Switch to the next layout (Piano → Chords → Isomorphic → Drums).
    LayoutNext,
    /// Metronome on / off.
    Metronome,
    /// Tap in time to set the tempo.
    TapTempo,
    /// Start recording (with count-in) / stop and save a MIDI file.
    Record,
}

/// Broad groups, for sorting and the keybind editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionKind {
    Note,
    Melody,
    Chord,
    Mod,
    Drum,
    Control,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        let s = s.trim();
        let arg = |p: &str| {
            s.strip_prefix(p)
                .filter(|r| r.starts_with(' '))
                .map(str::trim)
        };
        if let Some(rest) = arg("note") {
            return rest.parse().ok().map(Action::Note);
        }
        if let Some(rest) = arg("melody") {
            return rest.parse().ok().map(Action::Melody);
        }
        if let Some(rest) = arg("chord") {
            return ChordRef::parse(rest).map(Action::Chord);
        }
        if let Some(rest) = arg("mod") {
            return ChordMod::parse(rest).map(Action::Mod);
        }
        if let Some(rest) = arg("drum") {
            return crate::drums::parse(rest).map(Action::Drum);
        }
        Some(match s {
            "octave+" => Action::OctaveUp,
            "octave-" => Action::OctaveDown,
            "transpose+" | "key+" => Action::TransposeUp,
            "transpose-" | "key-" => Action::TransposeDown,
            "sustain" => Action::Sustain,
            "instrument+" => Action::InstrumentNext,
            "instrument-" => Action::InstrumentPrev,
            "velocity" => Action::Velocity,
            "panic" => Action::Panic,
            "style+" => Action::StyleNext,
            "style-" => Action::StylePrev,
            "rhythm" => Action::Rhythm,
            "tempo+" => Action::TempoUp,
            "tempo-" => Action::TempoDown,
            "scale+" => Action::ScaleNext,
            "scale-lock" => Action::ScaleLock,
            "layout+" => Action::LayoutNext,
            "metronome" => Action::Metronome,
            "tap-tempo" => Action::TapTempo,
            "record" => Action::Record,
            _ => return None,
        })
    }

    pub fn kind(&self) -> ActionKind {
        match self {
            Action::Note(_) => ActionKind::Note,
            Action::Melody(_) => ActionKind::Melody,
            Action::Chord(_) => ActionKind::Chord,
            Action::Mod(_) => ActionKind::Mod,
            Action::Drum(_) => ActionKind::Drum,
            _ => ActionKind::Control,
        }
    }

    /// Friendly description for the UI.
    pub fn describe(&self) -> String {
        match self {
            Action::Note(n) => format!("Note +{n}"),
            Action::Melody(n) => format!("Melody: scale step {n}"),
            Action::Chord(c) => c.describe(),
            Action::Mod(m) => m.describe().into(),
            Action::Drum(d) => format!("Drum: {}", crate::drums::name(*d)),
            Action::OctaveUp => "Octave up".into(),
            Action::OctaveDown => "Octave down".into(),
            Action::TransposeUp => "Key / transpose up".into(),
            Action::TransposeDown => "Key / transpose down".into(),
            Action::Sustain => "Sustain pedal (hold)".into(),
            Action::InstrumentNext => "Next instrument".into(),
            Action::InstrumentPrev => "Previous instrument".into(),
            Action::Velocity => "Soft / normal / loud".into(),
            Action::Panic => "Stop all sound".into(),
            Action::StyleNext => "Next chord style".into(),
            Action::StylePrev => "Previous chord style".into(),
            Action::Rhythm => "Start / stop rhythm".into(),
            Action::TempoUp => "Faster (tempo +5)".into(),
            Action::TempoDown => "Slower (tempo −5)".into(),
            Action::ScaleNext => "Next scale (major, minor…)".into(),
            Action::ScaleLock => "Scale lock on / off".into(),
            Action::LayoutNext => "Next layout".into(),
            Action::Metronome => "Metronome on / off".into(),
            Action::TapTempo => "Tap tempo (tap in time)".into(),
            Action::Record => "Record / stop + save MIDI".into(),
        }
    }

    /// Every control action (not notes/chords/drums), for menus.
    pub const CONTROLS: [Action; 20] = [
        Action::Sustain,
        Action::OctaveUp,
        Action::OctaveDown,
        Action::TransposeUp,
        Action::TransposeDown,
        Action::InstrumentNext,
        Action::InstrumentPrev,
        Action::Velocity,
        Action::Panic,
        Action::Rhythm,
        Action::StyleNext,
        Action::StylePrev,
        Action::TempoUp,
        Action::TempoDown,
        Action::ScaleNext,
        Action::ScaleLock,
        Action::LayoutNext,
        Action::Metronome,
        Action::TapTempo,
        Action::Record,
    ];
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Note(n) => write!(f, "note {n}"),
            Action::Melody(n) => write!(f, "melody {n}"),
            Action::Chord(c) => write!(f, "chord {}", c.code()),
            Action::Mod(m) => write!(f, "mod {}", m.code()),
            Action::Drum(d) => write!(f, "drum {}", crate::drums::code(*d)),
            Action::OctaveUp => f.write_str("octave+"),
            Action::OctaveDown => f.write_str("octave-"),
            Action::TransposeUp => f.write_str("transpose+"),
            Action::TransposeDown => f.write_str("transpose-"),
            Action::Sustain => f.write_str("sustain"),
            Action::InstrumentNext => f.write_str("instrument+"),
            Action::InstrumentPrev => f.write_str("instrument-"),
            Action::Velocity => f.write_str("velocity"),
            Action::Panic => f.write_str("panic"),
            Action::StyleNext => f.write_str("style+"),
            Action::StylePrev => f.write_str("style-"),
            Action::Rhythm => f.write_str("rhythm"),
            Action::TempoUp => f.write_str("tempo+"),
            Action::TempoDown => f.write_str("tempo-"),
            Action::ScaleNext => f.write_str("scale+"),
            Action::ScaleLock => f.write_str("scale-lock"),
            Action::LayoutNext => f.write_str("layout+"),
            Action::Metronome => f.write_str("metronome"),
            Action::TapTempo => f.write_str("tap-tempo"),
            Action::Record => f.write_str("record"),
        }
    }
}

/// On-disk form.
#[derive(Serialize, Deserialize, Default)]
struct KeymapFile {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    bindings: BTreeMap<String, String>,
}

/// Default metronome / tempo / record keys (F2–F3 belong to Learn).
const RECORD_KEYS: [(Key, Action); 5] = [
    (Key::F5, Action::Metronome),
    (Key::F6, Action::TapTempo),
    (Key::F7, Action::TempoDown),
    (Key::F8, Action::TempoUp),
    (Key::F9, Action::Record),
];

/// The layouts that ship with the app: (file name, display name).
pub const BUILTIN_LAYOUTS: [(&str, &str); 4] = [
    ("piano", "Piano"),
    ("chords", "Chords + Melody"),
    ("isomorphic", "Isomorphic"),
    ("drums", "Drums"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    pub name: String,
    pub description: String,
    pub bindings: HashMap<KeyId, Action>,
}

impl Keymap {
    pub fn get(&self, key: KeyId) -> Option<Action> {
        self.bindings.get(&key).copied()
    }

    /// Bind `key` to `action`. Returns the action the key had before, if any.
    pub fn bind(&mut self, key: KeyId, action: Action) -> Option<Action> {
        self.bindings.insert(key, action)
    }

    pub fn unbind(&mut self, key: KeyId) {
        self.bindings.remove(&key);
    }

    /// Keys bound to this action.
    pub fn keys_for(&self, action: Action) -> Vec<KeyId> {
        let mut v: Vec<KeyId> = self
            .bindings
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(k, _)| *k)
            .collect();
        v.sort_by_key(|k| k.name());
        v
    }

    pub fn has_kind(&self, kind: ActionKind) -> bool {
        self.bindings.values().any(|a| a.kind() == kind)
    }

    /// Bindings sorted for display: notes, melody, chords, modifiers,
    /// drums, then controls.
    pub fn sorted(&self) -> Vec<(KeyId, Action)> {
        let mut v: Vec<(KeyId, Action)> = self.bindings.iter().map(|(k, a)| (*k, *a)).collect();
        v.sort_by_key(|(k, a)| {
            let order = match a {
                Action::Note(n) | Action::Melody(n) => *n,
                Action::Chord(c) => match c {
                    ChordRef::Diatonic(d) => *d as i32,
                    ChordRef::Fixed { root, .. } => 100 + *root as i32,
                    ChordRef::Named { root, kind } => 200 + *root as i32 * 16 + *kind as i32,
                },
                Action::Mod(m) => ChordMod::ALL.iter().position(|x| x == m).unwrap_or(0) as i32,
                Action::Drum(d) => *d as i32,
                other => Action::CONTROLS
                    .iter()
                    .position(|c| c == other)
                    .unwrap_or(99) as i32,
            };
            (a.kind(), order, k.name())
        });
        v
    }

    pub fn to_toml(&self) -> String {
        let file = KeymapFile {
            name: self.name.clone(),
            description: self.description.clone(),
            bindings: self
                .bindings
                .iter()
                .map(|(k, a)| (k.name().to_string(), a.to_string()))
                .collect(),
        };
        let body = toml::to_string_pretty(&file).unwrap_or_default();
        format!(
            "# Qwertone keymap. Edit freely: the app reloads this file when it changes.\n\
             # Keys: egui key names (A, Num1 or \"1\", Comma, Space, F1 …) plus Shift, Ctrl, Alt.\n\
             # Actions:\n\
             #   note <semitones>      a fixed note (0 = lowest C of the layout)\n\
             #   melody <step>         a note of the current scale (0 = the key's home note)\n\
             #   chord <1-7>           the chord on that scale step (I ii iii IV V vi vii°)\n\
             #   chord <numeral>       a borrowed chord: bVII, bVI, bIII, iv, II, III, v …\n\
             #   mod 7th|sus4|add9|swap   hold to change the chord\n\
             #   drum <name or number> kick, snare, clap, rim, hat, openhat, crash, ride, …\n\
             #   sustain, octave+, octave-, transpose+, transpose-, instrument+, instrument-,\n\
             #   velocity, panic, rhythm, style+, style-, tempo+, tempo-, scale+, scale-lock, layout+\n\n{body}"
        )
    }

    /// Parse a keymap. Unknown keys/actions are reported but don't fail the load.
    pub fn from_toml(text: &str) -> Result<(Keymap, Vec<String>), String> {
        let file: KeymapFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        let mut bindings = HashMap::new();
        for (k, a) in file.bindings {
            let Some(key) = KeyId::from_name(&k) else {
                warnings.push(format!("unknown key name `{k}`"));
                continue;
            };
            let Some(action) = Action::parse(&a) else {
                warnings.push(format!("unknown action `{a}` for key {k}"));
                continue;
            };
            bindings.insert(key, action);
        }
        Ok((
            Keymap {
                name: file.name,
                description: file.description,
                bindings,
            },
            warnings,
        ))
    }

    /// One of the built-in layouts by file name.
    pub fn builtin(file: &str) -> Option<Keymap> {
        Some(match file {
            "piano" => Keymap::piano(),
            "chords" => Keymap::chords(),
            "isomorphic" => Keymap::isomorphic(),
            "drums" => Keymap::drums(),
            _ => return None,
        })
    }

    /// Older saved layouts have no "next layout" key: give them F1 if free.
    /// Returns true if something changed.
    pub fn ensure_layout_key(&mut self) -> bool {
        let f1 = KeyId::Key(Key::F1);
        if self.keys_for(Action::LayoutNext).is_empty() && self.get(f1).is_none() {
            self.bind(f1, Action::LayoutNext);
            return true;
        }
        false
    }

    /// Part 4a keys for older saved layouts, where free.
    pub fn ensure_record_keys(&mut self) -> bool {
        let mut changed = false;
        for (k, a) in RECORD_KEYS {
            let id = KeyId::Key(k);
            if self.keys_for(a).is_empty() && self.get(id).is_none() {
                self.bind(id, a);
                changed = true;
            }
        }
        changed
    }

    fn with_controls(mut b: HashMap<KeyId, Action>) -> HashMap<KeyId, Action> {
        use Key::*;
        for (k, a) in [
            (Space, Action::Sustain),
            (ArrowUp, Action::OctaveUp),
            (ArrowDown, Action::OctaveDown),
            (ArrowRight, Action::TransposeUp),
            (ArrowLeft, Action::TransposeDown),
            (PageUp, Action::InstrumentNext),
            (PageDown, Action::InstrumentPrev),
            (Tab, Action::Velocity),
            (Escape, Action::Panic),
            (F1, Action::LayoutNext),
        ]
        .into_iter()
        .chain(RECORD_KEYS)
        {
            b.entry(KeyId::Key(k)).or_insert(a);
        }
        b
    }

    /// The classic tracker / DAW layout: two rows per octave, like a piano.
    pub fn piano() -> Keymap {
        use Key::*;
        let mut b = HashMap::new();
        // Lower octave: Z-row white keys, A-row black keys.
        let lower = [
            (Z, 0),
            (S, 1),
            (X, 2),
            (D, 3),
            (C, 4),
            (V, 5),
            (G, 6),
            (B, 7),
            (H, 8),
            (N, 9),
            (J, 10),
            (M, 11),
            (Comma, 12),
            (L, 13),
            (Period, 14),
            (Semicolon, 15),
            (Slash, 16),
        ];
        // Upper octave: Q-row white keys, number-row black keys.
        let upper = [
            (Q, 12),
            (Num2, 13),
            (W, 14),
            (Num3, 15),
            (E, 16),
            (R, 17),
            (Num5, 18),
            (T, 19),
            (Num6, 20),
            (Y, 21),
            (Num7, 22),
            (U, 23),
            (I, 24),
            (Num9, 25),
            (O, 26),
            (Num0, 27),
            (P, 28),
            (OpenBracket, 29),
            (Equals, 30),
            (CloseBracket, 31),
        ];
        for (k, n) in lower.into_iter().chain(upper) {
            b.insert(KeyId::Key(k), Action::Note(n));
        }
        Keymap {
            name: "Piano".into(),
            description: "Two octaves like a piano: Z-row + A-row is the lower octave, \
                          Q-row + number row is the upper one."
                .into(),
            bindings: Self::with_controls(b),
        }
    }

    /// Left hand plays whole chords, right hand plays a melody that is
    /// always in key.
    pub fn chords() -> Keymap {
        use Key::*;
        let mut b = HashMap::new();
        // Bottom row: the 7 chords of the key, I ii iii IV V vi vii°. In C
        // their roots are C D E F G A B, the same notes as the Piano layout.
        for (i, k) in [Z, X, C, V, B, N, M].into_iter().enumerate() {
            b.insert(
                KeyId::Key(k),
                Action::Chord(ChordRef::Diatonic(i as u8 + 1)),
            );
        }
        // Row above: a "different colour" of the chord below it.
        for (k, numeral) in [
            (A, "i"),
            (S, "II"),
            (D, "bIII"),
            (F, "iv"),
            (G, "III"),
            (H, "bVI"),
            (J, "bVII"),
        ] {
            if let Some(c) = ChordRef::parse(numeral) {
                b.insert(KeyId::Key(k), Action::Chord(c));
            }
        }
        // Hold to change the chord.
        b.insert(KeyId::Shift, Action::Mod(ChordMod::Seventh));
        b.insert(KeyId::Alt, Action::Mod(ChordMod::Sus4));
        b.insert(KeyId::Ctrl, Action::Mod(ChordMod::Swap));
        for (k, m) in [
            (Num1, ChordMod::Seventh),
            (Num2, ChordMod::Sus4),
            (Num3, ChordMod::Add9),
            (Num4, ChordMod::Swap),
        ] {
            b.insert(KeyId::Key(k), Action::Mod(m));
        }
        // Top letter row: melody, locked to the scale.
        for (i, k) in [Q, W, E, R, T, Y, U, I, O, P, OpenBracket, CloseBracket]
            .into_iter()
            .enumerate()
        {
            b.insert(KeyId::Key(k), Action::Melody(i as i32));
        }
        for (k, a) in [
            (Enter, Action::Rhythm),
            (Backspace, Action::StyleNext),
            (Minus, Action::TempoDown),
            (Equals, Action::TempoUp),
            (Backslash, Action::ScaleNext),
        ] {
            b.insert(KeyId::Key(k), a);
        }
        Keymap {
            name: "Chords + Melody".into(),
            description: "Bottom row: the 7 chords of the key. Row above: borrowed chords. \
                          Hold Shift (7th), Alt (sus4), Ctrl (major/minor) or 1–4 to change a chord. \
                          Q-row: melody that is always in key."
                .into(),
            bindings: Self::with_controls(b),
        }
    }

    /// Wicki-Hayden style isomorphic layout: one key right = 2 semitones,
    /// up-left = 5, up-right = 7. Every chord and scale has the same shape
    /// in every key.
    pub fn isomorphic() -> Keymap {
        use Key::*;
        let rows: [(&[Key], i32); 4] = [
            (&[Z, X, C, V, B, N, M, Comma, Period, Slash], 0),
            (&[A, S, D, F, G, H, J, K, L, Semicolon, Quote], 5),
            (
                &[Q, W, E, R, T, Y, U, I, O, P, OpenBracket, CloseBracket],
                10,
            ),
            (
                &[
                    Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0, Minus, Equals,
                ],
                15,
            ),
        ];
        let mut b = HashMap::new();
        for (keys, start) in rows {
            for (i, k) in keys.iter().enumerate() {
                b.insert(KeyId::Key(*k), Action::Note(start + 2 * i as i32));
            }
        }
        Keymap {
            name: "Isomorphic".into(),
            description: "Each key right is a whole step, up-left is a 4th, up-right a 5th. \
                          A major chord is always the same triangle: root, two keys right, \
                          and up-right."
                .into(),
            bindings: Self::with_controls(b),
        }
    }

    pub fn drums() -> Keymap {
        use Key::*;
        let mut b = HashMap::new();
        let rows: [&[(Key, &str)]; 4] = [
            &[
                (Z, "kick"),
                (X, "snare"),
                (C, "rim"),
                (V, "clap"),
                (B, "kick"),
                (N, "snare"),
                (M, "clap"),
                (Comma, "tamb"),
                (Period, "shaker"),
                (Slash, "cowbell"),
            ],
            &[
                (A, "hat"),
                (S, "pedalhat"),
                (D, "openhat"),
                (F, "ride"),
                (G, "bell"),
                (H, "hat"),
                (J, "openhat"),
                (K, "conga"),
                (L, "conga2"),
                (Semicolon, "bongo"),
                (Quote, "bongo2"),
            ],
            &[
                (Q, "floortom"),
                (W, "lowtom"),
                (E, "midtom"),
                (R, "hitom"),
                (T, "crash"),
                (Y, "crash2"),
                (U, "hitom"),
                (I, "midtom"),
                (O, "lowtom"),
                (P, "floortom"),
                (OpenBracket, "claves"),
                (CloseBracket, "block"),
            ],
            &[
                (Num1, "crash"),
                (Num2, "splash"),
                (Num3, "china"),
                (Num8, "china"),
                (Num9, "splash"),
                (Num0, "crash2"),
            ],
        ];
        for row in rows {
            for (k, name) in row {
                if let Some(n) = crate::drums::parse(name) {
                    b.insert(KeyId::Key(*k), Action::Drum(n));
                }
            }
        }
        b.insert(KeyId::Key(Space), Action::Drum(36));
        for (k, a) in [
            (Enter, Action::Rhythm),
            (Backspace, Action::StyleNext),
            (Minus, Action::TempoDown),
            (Equals, Action::TempoUp),
        ] {
            b.insert(KeyId::Key(k), a);
        }
        let mut b = Self::with_controls(b);
        // Space is a kick here, not the pedal.
        b.insert(KeyId::Key(Space), Action::Drum(36));
        Keymap {
            name: "Drums".into(),
            description: "Bottom row: kick, snare, clap. Home row: hi-hats and cymbals. \
                          Q-row: toms. Space is a kick too. Enter plays the current \
                          pattern's beat so you can drum along."
                .into(),
            bindings: b,
        }
    }
}

/// Short label for drawing a key cap.
pub fn key_label(key: Key) -> &'static str {
    match key {
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Backslash => "\\",
        Key::Backtick => "`",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        k => k.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_builtins() {
        for (file, _) in BUILTIN_LAYOUTS {
            let km = Keymap::builtin(file).unwrap();
            let text = km.to_toml();
            let (back, warnings) = Keymap::from_toml(&text).unwrap();
            assert!(warnings.is_empty(), "{file}: {warnings:?}");
            assert_eq!(back, km, "{file}");
        }
    }

    #[test]
    fn parse_actions() {
        assert_eq!(Action::parse("note 7"), Some(Action::Note(7)));
        assert_eq!(Action::parse("note -3"), Some(Action::Note(-3)));
        assert_eq!(Action::parse("sustain"), Some(Action::Sustain));
        assert_eq!(Action::parse("melody 3"), Some(Action::Melody(3)));
        assert_eq!(
            Action::parse("chord 5"),
            Some(Action::Chord(ChordRef::Diatonic(5)))
        );
        assert!(matches!(
            Action::parse("chord bVII"),
            Some(Action::Chord(_))
        ));
        assert_eq!(
            Action::parse("mod 7th"),
            Some(Action::Mod(ChordMod::Seventh))
        );
        assert_eq!(Action::parse("drum kick"), Some(Action::Drum(36)));
        assert_eq!(Action::parse("drum 38"), Some(Action::Drum(38)));
        assert_eq!(Action::parse("notes 3"), None);
        assert_eq!(Action::parse("bogus"), None);
        for a in Action::CONTROLS {
            assert_eq!(Action::parse(&a.to_string()), Some(a));
        }
    }

    #[test]
    fn modifier_keys_bind() {
        let (km, w) = Keymap::from_toml(
            "name = \"x\"\n[bindings]\nShift = \"mod 7th\"\nCtrl = \"sustain\"\n",
        )
        .unwrap();
        assert!(w.is_empty());
        assert_eq!(km.get(KeyId::Shift), Some(Action::Mod(ChordMod::Seventh)));
        assert_eq!(km.get(KeyId::Ctrl), Some(Action::Sustain));
    }

    #[test]
    fn bad_entries_are_warnings() {
        let (km, w) = Keymap::from_toml(
            "name = \"x\"\n[bindings]\nQ = \"note 0\"\nNotAKey = \"note 1\"\nW = \"fly\"\n",
        )
        .unwrap();
        assert_eq!(km.bindings.len(), 1);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn layout_key_migration() {
        let mut km = Keymap::piano();
        km.unbind(KeyId::Key(Key::F1));
        assert!(km.ensure_layout_key());
        assert!(!km.ensure_layout_key());
        for (k, _) in RECORD_KEYS {
            km.unbind(KeyId::Key(k));
        }
        assert!(km.ensure_record_keys());
        assert!(!km.ensure_record_keys());
        assert_eq!(km.get(KeyId::Key(Key::F9)), Some(Action::Record));
    }
}
