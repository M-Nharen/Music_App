//! Small music-theory helpers shared by the whole app.

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Name of a MIDI note, e.g. 60 -> "C4".
pub fn note_name(midi: i32) -> String {
    let name = NOTE_NAMES[midi.rem_euclid(12) as usize];
    let octave = midi.div_euclid(12) - 1;
    format!("{name}{octave}")
}

/// Name without the octave, e.g. 61 -> "C#".
pub fn pitch_class_name(midi: i32) -> &'static str {
    NOTE_NAMES[midi.rem_euclid(12) as usize]
}

/// True for the "black keys" of a piano.
pub fn is_black(midi: i32) -> bool {
    matches!(midi.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

/// Frequency in Hz for a MIDI note (A4 = 440 Hz).
pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

/// One playable sound. `program` is the General MIDI program number used
/// with a SoundFont; `bt_friendly` marks soft-attack sounds that hide
/// Bluetooth delay well.
#[derive(Clone, Copy, Debug)]
pub struct Instrument {
    pub name: &'static str,
    pub program: u8,
    pub bt_friendly: bool,
}

pub const INSTRUMENTS: &[Instrument] = &[
    Instrument {
        name: "Grand Piano",
        program: 0,
        bt_friendly: false,
    },
    Instrument {
        name: "Bright Piano",
        program: 1,
        bt_friendly: false,
    },
    Instrument {
        name: "Electric Piano",
        program: 4,
        bt_friendly: false,
    },
    Instrument {
        name: "Music Box",
        program: 10,
        bt_friendly: false,
    },
    Instrument {
        name: "Vibraphone",
        program: 11,
        bt_friendly: false,
    },
    Instrument {
        name: "Organ",
        program: 19,
        bt_friendly: true,
    },
    Instrument {
        name: "Nylon Guitar",
        program: 24,
        bt_friendly: false,
    },
    Instrument {
        name: "Strings",
        program: 48,
        bt_friendly: true,
    },
    Instrument {
        name: "Slow Strings",
        program: 49,
        bt_friendly: true,
    },
    Instrument {
        name: "Choir",
        program: 52,
        bt_friendly: true,
    },
    Instrument {
        name: "Flute",
        program: 73,
        bt_friendly: true,
    },
    Instrument {
        name: "Square Lead",
        program: 80,
        bt_friendly: false,
    },
    Instrument {
        name: "Saw Lead",
        program: 81,
        bt_friendly: false,
    },
    Instrument {
        name: "Warm Pad",
        program: 89,
        bt_friendly: true,
    },
    Instrument {
        name: "Polysynth Pad",
        program: 90,
        bt_friendly: true,
    },
    Instrument {
        name: "Halo Pad",
        program: 94,
        bt_friendly: true,
    },
];

/// Name the chord formed by these MIDI notes, e.g. [60, 64, 67] -> "C".
/// Tries every note as the root; returns None if it isn't a known shape.
pub fn chord_name(notes: &[i32]) -> Option<String> {
    let mut pcs: Vec<i32> = notes.iter().map(|n| n.rem_euclid(12)).collect();
    pcs.sort();
    pcs.dedup();
    if pcs.len() < 2 {
        return None;
    }
    let bass = notes.iter().min().map(|n| n.rem_euclid(12));
    // (intervals above root, suffix)
    const SHAPES: &[(&[i32], &str)] = &[
        (&[0, 4, 7], ""),
        (&[0, 3, 7], "m"),
        (&[0, 3, 6], "dim"),
        (&[0, 4, 8], "aug"),
        (&[0, 2, 7], "sus2"),
        (&[0, 5, 7], "sus4"),
        (&[0, 4, 7, 10], "7"),
        (&[0, 4, 7, 11], "maj7"),
        (&[0, 3, 7, 10], "m7"),
        (&[0, 3, 6, 10], "m7b5"),
        (&[0, 3, 6, 9], "dim7"),
        (&[0, 4, 7, 9], "6"),
        (&[0, 3, 7, 9], "m6"),
        (&[0, 4, 7, 2], "add9"),
        (&[0, 7], "5"),
        (&[0, 4], " (major 3rd)"),
        (&[0, 3], " (minor 3rd)"),
    ];
    // Prefer the bass note as root so inversions are named naturally.
    let mut roots: Vec<i32> = pcs.clone();
    if let Some(b) = bass {
        roots.sort_by_key(|r| *r != b);
    }
    for (shape, suffix) in SHAPES {
        if shape.len() != pcs.len() {
            continue;
        }
        for &root in &roots {
            let mut ivs: Vec<i32> = pcs.iter().map(|p| (p - root).rem_euclid(12)).collect();
            ivs.sort();
            let mut want = shape.to_vec();
            want.sort();
            if ivs == want {
                let mut name = format!("{}{}", NOTE_NAMES[root as usize], suffix);
                if let Some(b) = bass
                    && b != root
                    && pcs.len() >= 3
                {
                    name = format!("{name}/{}", NOTE_NAMES[b as usize]);
                }
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        assert_eq!(chord_name(&[60, 64, 67]).as_deref(), Some("C"));
        assert_eq!(chord_name(&[57, 60, 64]).as_deref(), Some("Am"));
        assert_eq!(chord_name(&[64, 67, 72]).as_deref(), Some("C/E"));
        assert_eq!(chord_name(&[55, 59, 62, 65]).as_deref(), Some("G7"));
        assert_eq!(chord_name(&[60]), None);
    }

    #[test]
    fn names() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(21), "A0");
        assert!(is_black(61));
        assert!(!is_black(64));
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-3);
    }
}
