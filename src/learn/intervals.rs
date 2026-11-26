//! The 13 intervals inside an octave, with plain-language moods and famous
//! songs that start with them (a classic way to learn to hear them).

pub struct Interval {
    pub semitones: i32,
    pub name: &'static str,
    pub short: &'static str,
    /// How it feels, in a few words.
    pub mood: &'static str,
    /// Well-known tunes whose first two notes (going up) make this interval.
    pub songs: &'static [&'static str],
    /// 0 = very smooth (consonant) … 1 = very clashy (dissonant).
    pub tension: f32,
}

pub const INTERVALS: [Interval; 13] = [
    Interval {
        semitones: 0,
        name: "Unison",
        short: "P1",
        mood: "the same note twice",
        songs: &["Jingle Bells (“Jin-gle”)"],
        tension: 0.0,
    },
    Interval {
        semitones: 1,
        name: "Minor 2nd (half step)",
        short: "m2",
        mood: "creepy, clashing: the two notes rub",
        songs: &["Jaws theme", "Für Elise (first two notes, going down)"],
        tension: 0.95,
    },
    Interval {
        semitones: 2,
        name: "Major 2nd (whole step)",
        short: "M2",
        mood: "a simple step, like walking up a stair",
        songs: &[
            "Happy Birthday (“Hap-py Birth-”)",
            "Frère Jacques",
            "Silent Night (going down)",
        ],
        tension: 0.55,
    },
    Interval {
        semitones: 3,
        name: "Minor 3rd",
        short: "m3",
        mood: "sad, gentle. The sound of a minor chord",
        songs: &[
            "Greensleeves",
            "Smoke on the Water (first two notes)",
            "Brahms' Lullaby",
        ],
        tension: 0.25,
    },
    Interval {
        semitones: 4,
        name: "Major 3rd",
        short: "M3",
        mood: "happy, bright. The sound of a major chord",
        songs: &[
            "When the Saints Go Marching In",
            "Kumbaya",
            "Beethoven's 5th (going down)",
        ],
        tension: 0.2,
    },
    Interval {
        semitones: 5,
        name: "Perfect 4th",
        short: "P4",
        mood: "open, like a call or a fanfare",
        songs: &["Here Comes the Bride", "Amazing Grace", "Auld Lang Syne"],
        tension: 0.3,
    },
    Interval {
        semitones: 6,
        name: "Tritone",
        short: "TT",
        mood: "spooky, unstable: it splits the octave in half",
        songs: &[
            "The Simpsons theme (“The Simp-”)",
            "Maria (West Side Story)",
            "Black Sabbath",
        ],
        tension: 1.0,
    },
    Interval {
        semitones: 7,
        name: "Perfect 5th",
        short: "P5",
        mood: "strong, heroic, very stable (power chords)",
        songs: &[
            "Star Wars main theme",
            "Twinkle Twinkle Little Star (“Twin-kle twin-kle”)",
        ],
        tension: 0.1,
    },
    Interval {
        semitones: 8,
        name: "Minor 6th",
        short: "m6",
        mood: "bittersweet, yearning",
        songs: &[
            "Love Story theme (“Where do I begin”)",
            "The Entertainer (the high jump)",
        ],
        tension: 0.4,
    },
    Interval {
        semitones: 9,
        name: "Major 6th",
        short: "M6",
        mood: "warm, sweet, a big friendly leap",
        songs: &[
            "My Bonnie Lies Over the Ocean",
            "NBC chimes",
            "Dashing through the snow",
        ],
        tension: 0.3,
    },
    Interval {
        semitones: 10,
        name: "Minor 7th",
        short: "m7",
        mood: "bluesy, wants to move somewhere",
        songs: &["Star Trek (original theme)", "Somewhere (West Side Story)"],
        tension: 0.65,
    },
    Interval {
        semitones: 11,
        name: "Major 7th",
        short: "M7",
        mood: "dreamy but almost an octave: it leans up",
        songs: &[
            "Take On Me (the chorus jump)",
            "Pure Imagination (“Come with me”)",
        ],
        tension: 0.85,
    },
    Interval {
        semitones: 12,
        name: "Octave",
        short: "P8",
        mood: "the same note, higher. Sounds almost like one note",
        songs: &[
            "Somewhere Over the Rainbow (“Some-where”)",
            "Singin' in the Rain",
        ],
        tension: 0.0,
    },
];

/// The interval for a distance in semitones (any size; folded into one
/// octave, but 12 stays an octave).
pub fn interval(semitones: i32) -> &'static Interval {
    let s = semitones.abs();
    let i = if s != 0 && s % 12 == 0 { 12 } else { s % 12 };
    &INTERVALS[i as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        assert_eq!(interval(7).short, "P5");
        assert_eq!(interval(-3).short, "m3");
        assert_eq!(interval(12).short, "P8");
        assert_eq!(interval(16).short, "M3");
        assert_eq!(interval(24).short, "P8");
        for (i, iv) in INTERVALS.iter().enumerate() {
            assert_eq!(iv.semitones, i as i32);
            assert!(!iv.songs.is_empty());
        }
    }
}
