//! Auto-accompaniment patterns.
//!
//! A pattern is a small TOML file with one line of text per part, one
//! character per step (a "drum machine grid"):
//!
//! ```toml
//! name = "Pop"
//! description = "Steady 8ths, good for most songs"
//! beats = 4            # beats per bar
//! steps_per_beat = 4   # 4 = 16th notes, 2 = 8th notes, 3 = triplets
//! [tracks]
//! bass  = "x-------x---5---"
//! chord = "x.x.x.x.x.x.x.x."
//! kick  = "x.......x.x....."
//! snare = "....x.......x..."
//! hat   = "x.x.x.x.x.x.x.x."
//! ```
//!
//! Characters: `.` rest, `x` hit, `X` loud hit, `o` soft hit, `-` hold the
//! previous note one more step. In `bass`: `x` root, `5` fifth, `8` root an
//! octave up. In `chord`: `x` the whole chord, `1`–`9` one chord note
//! (1 = lowest; numbers past the chord size continue an octave higher).
//! Any other track name is a drum: kick, snare, clap, rim, hat, openhat,
//! pedalhat, ride, crash, tamb, shaker, cowbell, lowtom, midtom, hitom …
//! A line shorter than the bar repeats.
//!
//! Spaces and `|` are ignored, so `"x... x... | x... x..."` is fine.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BassNote {
    Root,
    Fifth,
    Octave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Bass(BassNote),
    /// Every chord note.
    Chord,
    /// One chord note, 0 = lowest.
    Tone(u8),
    Drum(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub part: Part,
    pub vel: u8,
    /// Length in steps.
    pub len: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub name: String,
    pub description: String,
    pub beats: u32,
    pub steps_per_beat: u32,
    /// One list of hits per step.
    pub steps: Vec<Vec<Hit>>,
    pub has_drums: bool,
}

#[derive(Deserialize)]
struct PatternFile {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "four")]
    beats: u32,
    #[serde(default = "four")]
    steps_per_beat: u32,
    tracks: BTreeMap<String, String>,
}

fn four() -> u32 {
    4
}

const NORMAL: u8 = 96;
const LOUD: u8 = 122;
const SOFT: u8 = 62;

impl Pattern {
    pub fn parse(text: &str) -> Result<Pattern, String> {
        let f: PatternFile = toml::from_str(text).map_err(|e| e.to_string())?;
        if !(1..=12).contains(&f.beats) || !(1..=8).contains(&f.steps_per_beat) {
            return Err("beats must be 1–12 and steps_per_beat 1–8".into());
        }
        let total = (f.beats * f.steps_per_beat) as usize;
        let mut steps: Vec<Vec<Hit>> = vec![Vec::new(); total];
        let mut has_drums = false;
        for (track, line) in &f.tracks {
            let kind = match track.as_str() {
                "bass" => 0,
                "chord" | "chords" => 1,
                other => match crate::drums::parse(other) {
                    Some(n) => {
                        has_drums = true;
                        2 + n as u32
                    }
                    None => return Err(format!("unknown track `{other}`")),
                },
            };
            let chars: Vec<char> = line.chars().filter(|c| !matches!(c, ' ' | '|')).collect();
            if chars.is_empty() {
                continue;
            }
            // First pass: the hits. Ties (`-`) are applied in a second pass.
            for (i, step) in steps.iter_mut().enumerate() {
                let c = chars[i % chars.len()];
                let vel = match c {
                    'X' => LOUD,
                    'o' => SOFT,
                    _ => NORMAL,
                };
                let part = match (kind, c) {
                    (_, '.' | '-') => continue,
                    (0, 'x' | 'X' | 'o') => Part::Bass(BassNote::Root),
                    (0, '5') => Part::Bass(BassNote::Fifth),
                    (0, '8') => Part::Bass(BassNote::Octave),
                    (1, 'x' | 'X' | 'o') => Part::Chord,
                    (1, '1'..='9') => Part::Tone(c as u8 - b'1'),
                    (k, 'x' | 'X' | 'o') if k >= 2 => Part::Drum((k - 2) as u8),
                    _ => return Err(format!("track `{track}`: unexpected `{c}`")),
                };
                step.push(Hit { part, vel, len: 1 });
            }
            // Second pass for ties: walk again and extend.
            let mut last: Option<(usize, usize)> = None;
            for i in 0..total {
                let c = chars[i % chars.len()];
                match c {
                    '.' => last = None,
                    '-' => {
                        if let Some((s, h)) = last {
                            steps[s][h].len += 1;
                        }
                    }
                    _ => {
                        // This track's hit is the last one pushed at step i
                        // that matches the track kind.
                        let idx = steps[i].iter().rposition(|h| match (kind, h.part) {
                            (0, Part::Bass(_)) => true,
                            (1, Part::Chord | Part::Tone(_)) => true,
                            (k, Part::Drum(n)) => k >= 2 && n as u32 == k - 2,
                            _ => false,
                        });
                        last = idx.map(|h| (i, h));
                    }
                }
            }
        }
        Ok(Pattern {
            name: f.name,
            description: f.description,
            beats: f.beats,
            steps_per_beat: f.steps_per_beat,
            steps,
            has_drums,
        })
    }

    pub fn steps_per_bar(&self) -> u32 {
        self.beats * self.steps_per_beat
    }
}

/// Built-in patterns, as the same text users can write.
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "pop",
        r#"name = "Pop"
description = "Steady 8th-note chords with bass and drums. Fits most songs."
beats = 4
steps_per_beat = 4
[tracks]
bass  = "x-------x---5---"
chord = "X.x.x.x.X.x.x.x."
kick  = "x.......x.x....."
snare = "....x.......x..."
hat   = "x.x.x.x.x.x.x.x."
"#,
    ),
    (
        "ballad",
        r#"name = "Ballad"
description = "Slow rolling broken chords, no drums. Lovely with piano."
beats = 4
steps_per_beat = 2
[tracks]
bass  = "x-------"
chord = ".1234321"
"#,
    ),
    (
        "country",
        r#"name = "Country"
description = "Boom-chick: bass on 1 and 3, chord on 2 and 4."
beats = 4
steps_per_beat = 2
[tracks]
bass  = "x...5..."
chord = "..x...x."
kick  = "x...x..."
snare = "..o...o."
hat   = "o.o.o.o."
"#,
    ),
    (
        "rock",
        r#"name = "Rock"
description = "Driving 8th-note bass, big chords, rock beat."
beats = 4
steps_per_beat = 2
[tracks]
bass  = "xxxxxxxx"
chord = "X-x-X-x-"
kick  = "x...x.x."
snare = "..x...x."
hat   = "xxxxxxxx"
"#,
    ),
    (
        "waltz",
        r#"name = "Waltz"
description = "3 beats per bar: oom-pah-pah."
beats = 3
steps_per_beat = 2
[tracks]
bass  = "x-...."
chord = "..x.x."
kick  = "x....."
hat   = "..o.o."
"#,
    ),
    (
        "reggae",
        r#"name = "Reggae"
description = "Chords on the off-beat, 'one drop' drums."
beats = 4
steps_per_beat = 4
[tracks]
bass  = "x-....5.x-......"
chord = "..x...x...x...x."
kick  = "........x......."
rim   = "........x......."
hat   = "o.o.o.o.o.o.o.o."
"#,
    ),
    (
        "bossa",
        r#"name = "Bossa nova"
description = "Soft Brazilian groove. Try it with 7th chords (hold Shift)."
beats = 4
steps_per_beat = 4
[tracks]
bass   = "x-----5-x-----5-"
chord  = "x..x..x...x..x.."
rim    = "x..x...x..x..x.."
shaker = "x.o.x.o.x.o.x.o."
"#,
    ),
    (
        "dance",
        r#"name = "Dance"
description = "Four-on-the-floor kick, off-beat bass."
beats = 4
steps_per_beat = 4
[tracks]
bass    = "..x...8...x...8."
chord   = "..x..x....x..x.."
kick    = "x...x...x...x..."
clap    = "....x.......x..."
openhat = "..x...x...x...x."
"#,
    ),
    (
        "alberti",
        r#"name = "Alberti"
description = "Classical low-high-middle-high broken chords (Mozart)."
beats = 4
steps_per_beat = 2
[tracks]
chord = "13231323"
"#,
    ),
];

/// Parse every built-in pattern (they're tested, so this can't fail).
pub fn builtin() -> Vec<Pattern> {
    BUILTIN
        .iter()
        .filter_map(|(_, t)| Pattern::parse(t).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        for (file, text) in BUILTIN {
            let p = Pattern::parse(text).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(p.steps.len() as u32, p.steps_per_bar(), "{file}");
            assert!(p.steps.iter().any(|s| !s.is_empty()), "{file}");
        }
        assert_eq!(builtin().len(), BUILTIN.len());
    }

    #[test]
    fn ties_repeat_and_parts() {
        let p = Pattern::parse(
            "name = \"t\"\nbeats = 2\nsteps_per_beat = 2\n[tracks]\nbass = \"x-5.\"\nchord = \"1.\"\nkick = \"X\"\n",
        )
        .unwrap();
        assert_eq!(p.steps.len(), 4);
        let bass: Vec<&Hit> = p
            .steps
            .iter()
            .flatten()
            .filter(|h| matches!(h.part, Part::Bass(_)))
            .collect();
        assert_eq!(bass.len(), 2);
        assert_eq!(bass[0].len, 2, "x- is two steps long");
        assert_eq!(bass[1].part, Part::Bass(BassNote::Fifth));
        // "1." repeats: tone 0 on steps 0 and 2.
        let tones = p
            .steps
            .iter()
            .flatten()
            .filter(|h| h.part == Part::Tone(0))
            .count();
        assert_eq!(tones, 2);
        // "X" repeats on every step, loud.
        let kicks: Vec<&Hit> = p
            .steps
            .iter()
            .flatten()
            .filter(|h| h.part == Part::Drum(36))
            .collect();
        assert_eq!(kicks.len(), 4);
        assert_eq!(kicks[0].vel, LOUD);
        assert!(p.has_drums);
    }

    #[test]
    fn errors() {
        assert!(Pattern::parse("name=\"x\"\n[tracks]\nbanjo = \"x\"\n").is_err());
        assert!(Pattern::parse("name=\"x\"\n[tracks]\nbass = \"q\"\n").is_err());
    }
}
