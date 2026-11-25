//! Keybinds. A keymap is a plain TOML file mapping key names to actions:
//!
//! ```toml
//! name = "Piano"
//! [bindings]
//! Z = "note 0"        # C of the lower octave
//! S = "note 1"        # C#
//! Space = "sustain"
//! Up = "octave+"
//! ```
//!
//! Key names are egui's (`A`..`Z`, `"1"`..`"0"`, `Comma`, `Period`, `Slash`,
//! `Semicolon`, `Quote`, `OpenBracket`, `CloseBracket`, `Minus`, `Equals`,
//! `Backslash`, `Backtick`, `Space`, `Tab`, `Up`, `Down`, `Left`, `Right`,
//! `PageUp`, `PageDown`, `F1`..`F12`, …).

use egui::Key;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Semitones above the layout's base note (C of the lower octave).
    Note(i32),
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
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("note") {
            return rest.trim().parse().ok().map(Action::Note);
        }
        Some(match s {
            "octave+" => Action::OctaveUp,
            "octave-" => Action::OctaveDown,
            "transpose+" => Action::TransposeUp,
            "transpose-" => Action::TransposeDown,
            "sustain" => Action::Sustain,
            "instrument+" => Action::InstrumentNext,
            "instrument-" => Action::InstrumentPrev,
            "velocity" => Action::Velocity,
            "panic" => Action::Panic,
            _ => return None,
        })
    }

    /// Friendly description for the UI.
    pub fn describe(&self) -> String {
        match self {
            Action::Note(n) => format!("Note +{n}"),
            Action::OctaveUp => "Octave up".into(),
            Action::OctaveDown => "Octave down".into(),
            Action::TransposeUp => "Transpose up".into(),
            Action::TransposeDown => "Transpose down".into(),
            Action::Sustain => "Sustain pedal (hold)".into(),
            Action::InstrumentNext => "Next instrument".into(),
            Action::InstrumentPrev => "Previous instrument".into(),
            Action::Velocity => "Soft / normal / loud".into(),
            Action::Panic => "Stop all sound".into(),
        }
    }

    /// Every non-note action, for the "add binding" menu.
    pub const CONTROLS: [Action; 9] = [
        Action::Sustain,
        Action::OctaveUp,
        Action::OctaveDown,
        Action::TransposeUp,
        Action::TransposeDown,
        Action::InstrumentNext,
        Action::InstrumentPrev,
        Action::Velocity,
        Action::Panic,
    ];
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Note(n) => write!(f, "note {n}"),
            Action::OctaveUp => f.write_str("octave+"),
            Action::OctaveDown => f.write_str("octave-"),
            Action::TransposeUp => f.write_str("transpose+"),
            Action::TransposeDown => f.write_str("transpose-"),
            Action::Sustain => f.write_str("sustain"),
            Action::InstrumentNext => f.write_str("instrument+"),
            Action::InstrumentPrev => f.write_str("instrument-"),
            Action::Velocity => f.write_str("velocity"),
            Action::Panic => f.write_str("panic"),
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

#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    pub name: String,
    pub description: String,
    pub bindings: HashMap<Key, Action>,
}

impl Keymap {
    pub fn get(&self, key: Key) -> Option<Action> {
        self.bindings.get(&key).copied()
    }

    /// Bind `key` to `action`. Returns the action the key had before, if any.
    pub fn bind(&mut self, key: Key, action: Action) -> Option<Action> {
        self.bindings.insert(key, action)
    }

    pub fn unbind(&mut self, key: Key) {
        self.bindings.remove(&key);
    }

    /// Keys bound to this action.
    pub fn keys_for(&self, action: Action) -> Vec<Key> {
        let mut v: Vec<Key> = self
            .bindings
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(k, _)| *k)
            .collect();
        v.sort_by_key(|k| k.name());
        v
    }

    /// Bindings sorted for display: notes low→high, then controls.
    pub fn sorted(&self) -> Vec<(Key, Action)> {
        let mut v: Vec<(Key, Action)> = self.bindings.iter().map(|(k, a)| (*k, *a)).collect();
        v.sort_by_key(|(k, a)| match a {
            Action::Note(n) => (0, *n, k.name()),
            other => (
                1,
                Action::CONTROLS
                    .iter()
                    .position(|c| c == other)
                    .unwrap_or(99) as i32,
                k.name(),
            ),
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
             # Actions: note <semitones>, sustain, octave+, octave-, transpose+, transpose-,\n\
             #          instrument+, instrument-, velocity, panic\n\n{body}"
        )
    }

    /// Parse a keymap. Unknown keys/actions are reported but don't fail the load.
    pub fn from_toml(text: &str) -> Result<(Keymap, Vec<String>), String> {
        let file: KeymapFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        let mut bindings = HashMap::new();
        for (k, a) in file.bindings {
            let Some(key) = Key::from_name(&k) else {
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
            b.insert(k, Action::Note(n));
        }
        b.insert(Space, Action::Sustain);
        b.insert(ArrowUp, Action::OctaveUp);
        b.insert(ArrowDown, Action::OctaveDown);
        b.insert(ArrowRight, Action::TransposeUp);
        b.insert(ArrowLeft, Action::TransposeDown);
        b.insert(PageUp, Action::InstrumentNext);
        b.insert(PageDown, Action::InstrumentPrev);
        b.insert(Tab, Action::Velocity);
        b.insert(Escape, Action::Panic);
        Keymap {
            name: "Piano".into(),
            description: "Two octaves like a piano: Z-row + A-row is the lower octave, \
                          Q-row + number row is the upper one."
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
    fn round_trip() {
        let km = Keymap::piano();
        let text = km.to_toml();
        let (back, warnings) = Keymap::from_toml(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, km);
    }

    #[test]
    fn parse_actions() {
        assert_eq!(Action::parse("note 7"), Some(Action::Note(7)));
        assert_eq!(Action::parse("note -3"), Some(Action::Note(-3)));
        assert_eq!(Action::parse("sustain"), Some(Action::Sustain));
        assert_eq!(Action::parse("bogus"), None);
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
}
