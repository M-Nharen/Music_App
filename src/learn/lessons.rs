//! The lessons: short, hands-on, one idea each.
//!
//! Notes are written as semitones above "lesson C" (the C at the bottom of
//! the Piano layout: C3 at octave 0). The Learn tab finds the keys that play
//! them in whatever layout and keybinds you use, and lights them up.
//!
//! Text can mention keys with placeholders, filled in from your keybinds:
//! `{n:4}` the key for note 4 (E), `{c:V}` the key for chord V,
//! `{a:sustain}` the key(s) for an action (any keymap action text).

/// Something to play.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// One note (semitones above lesson C).
    Note(i32),
    /// Several notes held at the same time (any octave counts).
    Together(&'static [i32]),
    /// A chord, as a roman numeral in C major ("V", "vi", "bVII"). Played
    /// with its chord key if the layout has one, otherwise as its notes.
    Chord(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Just text; press Next.
    Read(&'static str),
    /// Text with an example to hear. `sound` is a list of moments; each
    /// moment is the notes that start together.
    Listen {
        text: &'static str,
        sound: &'static [&'static [i32]],
        gap_ms: u32,
    },
    /// Play the targets in order. `done` is shown when you've got it.
    Play {
        text: &'static str,
        targets: &'static [Target],
        done: &'static str,
    },
    /// Hear an interval from `root`, then play it (name, mood and famous
    /// songs come from `intervals.rs`).
    Interval { root: i32, semitones: i32 },
}

/// Which layout a lesson is easiest in (the tab offers to switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Needs every note: Piano (or Isomorphic).
    Notes,
    /// One-key chords are nicest; Piano works too (play the notes).
    Chords,
}

impl Layout {
    pub fn file(self) -> &'static str {
        match self {
            Layout::Notes => "piano",
            Layout::Chords => "chords",
        }
    }
}

pub struct Lesson {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub layout: Layout,
    /// Show the circle of fifths next to the lesson.
    pub circle: bool,
    pub steps: &'static [Step],
}

pub struct Chapter {
    pub title: &'static str,
    pub lessons: &'static [Lesson],
}

use Step::*;
use Target::*;

pub const CHAPTERS: &[Chapter] = &[
    Chapter {
        title: "1 · Notes & octaves",
        lessons: &[
            Lesson {
                id: "alphabet",
                title: "The musical alphabet",
                summary: "12 notes, white keys and black keys",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "Music only has 12 different notes. They repeat over and over, higher and lower.\n\nThe white keys are named with letters: C D E F G A B, then C again. On your keyboard, the Piano layout's bottom row is the white keys.",
                    ),
                    Play {
                        text: "This is C, the note everything starts from. Press the lit key ({n:0}).",
                        targets: &[Note(0)],
                        done: "That's C. Easy to find on a piano: it's just left of the two black keys.",
                    },
                    Play {
                        text: "Now walk up the white keys: C D E F G A B C. Follow the lit keys.",
                        targets: &[
                            Note(0),
                            Note(2),
                            Note(4),
                            Note(5),
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                        ],
                        done: "You just played the C major scale: the 'do re mi' everyone knows.",
                    },
                    Read(
                        "The black keys sit between some white keys. They're named after their neighbours: the one just above C is C♯ (C sharp), which is also D♭ (D flat). Same key, two names.\n\nOn the Piano layout, black keys are the row above the white keys (A-row and number row).",
                    ),
                    Play {
                        text: "Play C, then C♯ right above it.",
                        targets: &[Note(0), Note(1)],
                        done: "C to C♯ is the smallest step in music: a half step.",
                    },
                    Play {
                        text: "Play all five black keys: C♯ D♯ F♯ G♯ A♯.",
                        targets: &[Note(1), Note(3), Note(6), Note(8), Note(10)],
                        done: "The black keys alone make a pentatonic scale. Notice it's impossible to sound bad on them!",
                    },
                ],
            },
            Lesson {
                id: "octaves",
                title: "Octaves",
                summary: "Same note, higher or lower",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "When you go up 12 notes (12 half steps) you land on the same letter again, just higher. That jump is an octave.\n\nNotes an octave apart sound so similar that we give them the same name. The number after a note (C3, C4) says which octave it's in.",
                    ),
                    Listen {
                        text: "Hear C, then the C an octave higher, then both together. Together they almost blend into one note.",
                        sound: &[&[0], &[12], &[0, 12]],
                        gap_ms: 650,
                    },
                    Play {
                        text: "Play low C, then high C.",
                        targets: &[Note(0), Note(12)],
                        done: "That's an octave: {n:0} to {n:12}.",
                    },
                    Play {
                        text: "Now hold both together.",
                        targets: &[Together(&[0, 12])],
                        done: "They melt into one bigger-sounding note.",
                    },
                    Play {
                        text: "Find E in both octaves.",
                        targets: &[Note(4), Note(16)],
                        done: "Same letter, same feeling, different height.",
                    },
                    Read(
                        "Want the whole keyboard higher or lower? {a:octave+} / {a:octave-} move everything by an octave.",
                    ),
                ],
            },
        ],
    },
    Chapter {
        title: "2 · Intervals",
        lessons: &[
            Lesson {
                id: "steps",
                title: "Half steps and whole steps",
                summary: "The two building blocks",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "The distance between two notes is called an interval. Every melody is just intervals.\n\nThe smallest one is the half step: the very next key, white or black. Two half steps make a whole step.",
                    ),
                    Interval {
                        root: 0,
                        semitones: 1,
                    },
                    Interval {
                        root: 0,
                        semitones: 2,
                    },
                    Play {
                        text: "E to F has no black key between them, so it's only a half step. Play E then F.",
                        targets: &[Note(4), Note(5)],
                        done: "Same for B to C. Those two spots are why the piano's black keys come in groups of 2 and 3.",
                    },
                ],
            },
            Lesson {
                id: "thirds",
                title: "3rds: happy and sad",
                summary: "The interval that decides the mood",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "A 3rd skips one white key: C to E, D to F. There are two sizes, and the difference between them is the difference between happy and sad music.",
                    ),
                    Interval {
                        root: 0,
                        semitones: 4,
                    },
                    Interval {
                        root: 0,
                        semitones: 3,
                    },
                    Read(
                        "Major 3rd = 4 half steps = happy. Minor 3rd = 3 half steps = sad. One half step changes everything. You'll see this again when we build chords.",
                    ),
                ],
            },
            Lesson {
                id: "fifths",
                title: "4ths, 5ths and the tritone",
                summary: "Strong, open, and spooky",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Interval {
                        root: 0,
                        semitones: 7,
                    },
                    Interval {
                        root: 0,
                        semitones: 5,
                    },
                    Read(
                        "A 5th up and a 4th up land on the same letter from opposite sides: C up to G is a 5th, G up to C is a 4th. Together they make an octave.",
                    ),
                    Interval {
                        root: 0,
                        semitones: 6,
                    },
                    Read(
                        "The tritone is exactly half an octave. Medieval musicians called it 'the devil in music'. Today it's the secret ingredient of tension chords.",
                    ),
                ],
            },
            Lesson {
                id: "leaps",
                title: "Big leaps",
                summary: "6ths, 7ths and the octave",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Interval {
                        root: 0,
                        semitones: 9,
                    },
                    Interval {
                        root: 0,
                        semitones: 8,
                    },
                    Interval {
                        root: 0,
                        semitones: 10,
                    },
                    Interval {
                        root: 0,
                        semitones: 11,
                    },
                    Interval {
                        root: 0,
                        semitones: 12,
                    },
                    Read(
                        "That's all 12. Tip: when you hear a melody, hum its first two notes and ask 'which song starts like this?'. The Ear training tab has an interval game to practise.",
                    ),
                ],
            },
        ],
    },
    Chapter {
        title: "3 · Scales",
        lessons: &[
            Lesson {
                id: "major-scale",
                title: "The major scale",
                summary: "Whole, whole, half, whole, whole, whole, half",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "A scale is a set of notes that sound good together. A key (like 'C major') means: these 7 notes, with C as home.\n\nThe major scale is built from this pattern of steps:\nwhole · whole · half · whole · whole · whole · half",
                    ),
                    Listen {
                        text: "C major: all the white keys from C to C.",
                        sound: &[&[0], &[2], &[4], &[5], &[7], &[9], &[11], &[12]],
                        gap_ms: 330,
                    },
                    Play {
                        text: "Play it.",
                        targets: &[
                            Note(0),
                            Note(2),
                            Note(4),
                            Note(5),
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                        ],
                        done: "Notice how the last two notes (B → C) pull home. That half step is called the leading note.",
                    },
                    Read(
                        "Start the same pattern on G and something changes: to keep the steps right, F has to become F♯. That's why G major has one sharp. Every major key is the same pattern from a different starting note.",
                    ),
                    Play {
                        text: "G major: G A B C D E F♯ G.",
                        targets: &[
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                            Note(14),
                            Note(16),
                            Note(18),
                            Note(19),
                        ],
                        done: "Same sound as C major, just higher, thanks to that one black key.",
                    },
                ],
            },
            Lesson {
                id: "minor-scale",
                title: "The minor scale",
                summary: "The sad version",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "Play the white keys from A to A instead of C to C and you get A minor. Same notes as C major, different home, completely different mood.",
                    ),
                    Listen {
                        text: "A minor.",
                        sound: &[&[9], &[11], &[12], &[14], &[16], &[17], &[19], &[21]],
                        gap_ms: 330,
                    },
                    Play {
                        text: "Play A minor.",
                        targets: &[
                            Note(9),
                            Note(11),
                            Note(12),
                            Note(14),
                            Note(16),
                            Note(17),
                            Note(19),
                            Note(21),
                        ],
                        done: "A minor and C major share all their notes. They're called relative keys.",
                    },
                    Read(
                        "To make C itself minor, lower three notes by a half step: the 3rd (E → E♭), the 6th (A → A♭) and the 7th (B → B♭). The lowered 3rd is the big one: it's the minor 3rd you met in the intervals chapter.",
                    ),
                    Listen {
                        text: "C major, then C minor. Listen for the 3rd note.",
                        sound: &[
                            &[0],
                            &[2],
                            &[4],
                            &[5],
                            &[7],
                            &[],
                            &[0],
                            &[2],
                            &[3],
                            &[5],
                            &[7],
                        ],
                        gap_ms: 330,
                    },
                    Play {
                        text: "C minor: C D E♭ F G A♭ B♭ C.",
                        targets: &[
                            Note(0),
                            Note(2),
                            Note(3),
                            Note(5),
                            Note(7),
                            Note(8),
                            Note(10),
                            Note(12),
                        ],
                        done: "Darker, right? That's the minor scale.",
                    },
                ],
            },
            Lesson {
                id: "pentatonic",
                title: "Pentatonic: no wrong notes",
                summary: "5 notes for instant solos",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "Drop the two 'leaning' notes (F and B) from C major and you get the major pentatonic: C D E G A. With no half steps left, nothing clashes. Folk tunes, Chinese and Scottish music, and many pop hooks use it.",
                    ),
                    Play {
                        text: "Play C major pentatonic.",
                        targets: &[Note(0), Note(2), Note(4), Note(7), Note(9), Note(12)],
                        done: "Now noodle around on just these keys: whatever you play sounds like a tune.",
                    },
                    Read(
                        "The same 5 notes starting from A (A C D E G) is the minor pentatonic: the scale of rock and blues guitar solos. In the Chords + Melody layout, choose 'Minor pentatonic' as the scale and the melody keys play only these notes.",
                    ),
                ],
            },
        ],
    },
    Chapter {
        title: "4 · Building chords",
        lessons: &[
            Lesson {
                id: "triads",
                title: "Your first chord",
                summary: "Root, 3rd, 5th",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "A chord is 3 or more notes played together. The basic chord, a triad, is a stack of 3rds: pick a note (the root), skip a key, take the next, skip, take the next.\n\nC + E + G = C major.",
                    ),
                    Listen {
                        text: "C major, one note at a time, then together.",
                        sound: &[&[0], &[4], &[7], &[0, 4, 7]],
                        gap_ms: 500,
                    },
                    Play {
                        text: "Hold C, E and G together.",
                        targets: &[Together(&[0, 4, 7])],
                        done: "Your first chord! Watch the panel on the right: it names it and explains it.",
                    },
                    Play {
                        text: "Same shape from F: F A C.",
                        targets: &[Together(&[5, 9, 12])],
                        done: "F major.",
                    },
                    Play {
                        text: "And from G: G B D.",
                        targets: &[Together(&[7, 11, 14])],
                        done: "G major. C, F and G are the three most important chords in C major.",
                    },
                ],
            },
            Lesson {
                id: "major-minor",
                title: "Major and minor chords",
                summary: "Move one note, change the mood",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Listen {
                        text: "C major, then C minor. Only the middle note moves.",
                        sound: &[&[0, 4, 7], &[0, 3, 7]],
                        gap_ms: 1100,
                    },
                    Play {
                        text: "Play C minor: C E♭ G.",
                        targets: &[Together(&[0, 3, 7])],
                        done: "The 3rd dropped a half step: major 3rd → minor 3rd, happy → sad.",
                    },
                    Play {
                        text: "A minor: A C E (all white keys).",
                        targets: &[Together(&[9, 12, 16])],
                        done: "A minor, the 'sad twin' of C major.",
                    },
                    Play {
                        text: "D minor: D F A.",
                        targets: &[Together(&[2, 5, 9])],
                        done: "Stacking 3rds on white keys gives you major chords on C, F, G and minor chords on D, E, A.",
                    },
                ],
            },
            Lesson {
                id: "colours",
                title: "Diminished, augmented, sus",
                summary: "Tense, dreamy and floating chords",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Play {
                        text: "B diminished: B D F. Two minor 3rds.",
                        targets: &[Together(&[11, 14, 17])],
                        done: "Tense and unstable: it wants to move to C.",
                    },
                    Play {
                        text: "C augmented: C E G♯. Two major 3rds.",
                        targets: &[Together(&[0, 4, 8])],
                        done: "Dreamy and strange: it's floating.",
                    },
                    Play {
                        text: "C sus4: C F G. The 3rd is replaced by the 4th.",
                        targets: &[Together(&[0, 5, 7])],
                        done: "Neither happy nor sad. Now drop F to E…",
                    },
                    Play {
                        text: "…and resolve: C E G.",
                        targets: &[Together(&[0, 4, 7])],
                        done: "Suspension → release. You'll hear that in a thousand songs.",
                    },
                ],
            },
            Lesson {
                id: "key-chords",
                title: "The 7 chords of a key",
                summary: "I ii iii IV V vi vii°",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "Build a triad on each note of the C major scale, using only white keys, and you get the 7 chords of the key. We number them with roman numerals: capitals for major, small for minor.\n\nI   ii   iii   IV   V   vi   vii°\nC   Dm  Em   F   G   Am   Bdim\n\nIn the Chords + Melody layout each one is a single key on the bottom row.",
                    ),
                    Listen {
                        text: "All seven.",
                        sound: &[
                            &[0, 4, 7],
                            &[2, 5, 9],
                            &[4, 7, 11],
                            &[5, 9, 12],
                            &[7, 11, 14],
                            &[9, 12, 16],
                            &[11, 14, 17],
                            &[12, 16, 19],
                        ],
                        gap_ms: 600,
                    },
                    Play {
                        text: "Play them in order.",
                        targets: &[
                            Chord("I"),
                            Chord("ii"),
                            Chord("iii"),
                            Chord("IV"),
                            Chord("V"),
                            Chord("vi"),
                            Chord("vii°"),
                        ],
                        done: "That's every chord of C major. The numerals work in every key: V is always the 5th chord.",
                    },
                    Read(
                        "Change key with {a:transpose-} / {a:transpose+}. The chord keys follow, so I is always home.",
                    ),
                ],
            },
            Lesson {
                id: "sevenths",
                title: "7th chords",
                summary: "Add one more 3rd on top",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "Stack one more 3rd on a triad and you get a 7th chord. It adds colour: dreamy, jazzy or bluesy depending on the chord.",
                    ),
                    Play {
                        text: "G7: G B D F.",
                        targets: &[Together(&[7, 11, 14, 17])],
                        done: "Bluesy and restless. The B and F inside form a tritone, which wants to resolve.",
                    },
                    Play {
                        text: "Resolve it to C.",
                        targets: &[Together(&[0, 4, 7])],
                        done: "Ahh. Tension → release.",
                    },
                    Play {
                        text: "Cmaj7: C E G B.",
                        targets: &[Together(&[0, 4, 7, 11])],
                        done: "Soft and dreamy: the lo-fi chord.",
                    },
                    Play {
                        text: "Am7: A C E G.",
                        targets: &[Together(&[9, 12, 16, 19])],
                        done: "Mellow minor. In the Chords + Melody layout, hold {a:mod 7th} with any chord key to add the 7th.",
                    },
                ],
            },
        ],
    },
    Chapter {
        title: "5 · Progressions",
        lessons: &[
            Lesson {
                id: "functions",
                title: "Home, away and tension",
                summary: "What each chord does",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "Chords have jobs.\n\n• I is home: rest.\n• IV goes away from home: open, hopeful.\n• V is tension: it really wants to go back to I.\n\nA progression is a journey: leave home, build tension, come back.",
                    ),
                    Listen {
                        text: "I → IV → V → I.",
                        sound: &[&[0, 4, 7], &[0, 5, 9], &[2, 7, 11], &[0, 4, 7]],
                        gap_ms: 900,
                    },
                    Play {
                        text: "Play I, IV, V, I.",
                        targets: &[Chord("I"), Chord("IV"), Chord("V"), Chord("I")],
                        done: "You just played the progression behind most of early rock'n'roll and blues.",
                    },
                ],
            },
            Lesson {
                id: "cadences",
                title: "Tension and release",
                summary: "Cadences: how phrases end",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "V → I is the strongest move in music. V contains B, the note a half step under home, and B wants to step up to C. Your ear feels that pull as tension, and landing on I as release. A chord move that ends a phrase is called a cadence.",
                    ),
                    Play {
                        text: "V → I.",
                        targets: &[Chord("V"), Chord("I")],
                        done: "Full stop. That's how most songs end.",
                    },
                    Play {
                        text: "Now trick your ear: V → vi.",
                        targets: &[Chord("V"), Chord("vi")],
                        done: "A 'deceptive cadence': your ear expected home and got its sad twin. Great for 'not over yet' moments.",
                    },
                    Play {
                        text: "IV → I, the gentle one.",
                        targets: &[Chord("IV"), Chord("I")],
                        done: "The 'Amen' ending from church music: softer than V → I.",
                    },
                    Play {
                        text: "Make V even tenser: hold {a:mod 7th} for V7, then go to I.",
                        targets: &[Chord("V"), Chord("I")],
                        done: "With the 7th the pull is even stronger. Watch the tension bar on the right.",
                    },
                ],
            },
            Lesson {
                id: "four-chords",
                title: "The four-chord song",
                summary: "I V vi IV",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "One progression powers a huge number of pop songs: I → V → vi → IV. In C: C G Am F. It goes home → tension → sad → hopeful, and loops forever.",
                    ),
                    Listen {
                        text: "I V vi IV.",
                        sound: &[&[0, 4, 7], &[-1, 2, 7], &[0, 4, 9], &[0, 5, 9]],
                        gap_ms: 900,
                    },
                    Play {
                        text: "Play it.",
                        targets: &[Chord("I"), Chord("V"), Chord("vi"), Chord("IV")],
                        done: "Now loop it a few times. Start the rhythm with {a:rhythm} to play along with a beat.",
                    },
                    Play {
                        text: "Start on vi instead: vi IV I V.",
                        targets: &[Chord("vi"), Chord("IV"), Chord("I"), Chord("V")],
                        done: "Same four chords, sadder feel, because it starts on the minor one.",
                    },
                ],
            },
            Lesson {
                id: "two-five-one",
                title: "ii – V – I",
                summary: "The jazz backbone",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "ii → V → I walks home in falling 5ths (D → G → C). Jazz standards are full of it, usually with 7ths: Dm7 G7 Cmaj7.",
                    ),
                    Play {
                        text: "ii, V, I.",
                        targets: &[Chord("ii"), Chord("V"), Chord("I")],
                        done: "Smooth. Try it again holding {a:mod 7th} the whole time.",
                    },
                ],
            },
            Lesson {
                id: "borrowed",
                title: "Borrowed chords",
                summary: "bVII, bVI and iv: colour from the minor key",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "You can borrow chords from the minor key with the same home. They sound surprising but still 'right'. In the Chords + Melody layout they're on the row above the normal chords.",
                    ),
                    Play {
                        text: "I → bVII → IV → I. The rock move.",
                        targets: &[Chord("I"), Chord("bVII"), Chord("IV"), Chord("I")],
                        done: "bVII (B♭ in C) is the sound of classic rock.",
                    },
                    Play {
                        text: "I → bVI → bVII → I. The epic one.",
                        targets: &[Chord("I"), Chord("bVI"), Chord("bVII"), Chord("I")],
                        done: "Film scores and video games love this climb.",
                    },
                    Play {
                        text: "IV → iv → I. Bittersweet.",
                        targets: &[Chord("IV"), Chord("iv"), Chord("I")],
                        done: "Turning IV minor right before home: instant nostalgia.",
                    },
                ],
            },
        ],
    },
    Chapter {
        title: "6 · Circle of fifths",
        lessons: &[
            Lesson {
                id: "circle",
                title: "The circle of fifths",
                summary: "A map of all 12 keys",
                layout: Layout::Notes,
                circle: true,
                steps: &[
                    Read(
                        "Start on C and keep going up by 5ths: C G D A E B F♯ … After 12 steps you're back at C. Drawn as a circle, that's the circle of fifths (look on the right).",
                    ),
                    Play {
                        text: "Climb in 5ths: C, G, D, A, E.",
                        targets: &[Note(0), Note(7), Note(14), Note(21), Note(28)],
                        done: "Each step clockwise adds one sharp to the key: C has none, G has F♯, D adds C♯…",
                    },
                    Read(
                        "Neighbours on the circle share 6 of their 7 notes. That's why C, F and G (C and its two neighbours) are the main chords of C major, and why changing to a neighbouring key sounds smooth.\n\nThe inner ring shows each key's relative minor: same notes, sadder home.",
                    ),
                    Read(
                        "Going anticlockwise (C F B♭ E♭ …) adds flats instead. Chords that move anticlockwise, falling by 5ths, sound the most natural of all.",
                    ),
                ],
            },
            Lesson {
                id: "circle-chords",
                title: "Chords around the circle",
                summary: "Falling 5ths: the strongest pull",
                layout: Layout::Chords,
                circle: true,
                steps: &[
                    Play {
                        text: "vi → ii → V → I (Am Dm G C): every root falls a 5th.",
                        targets: &[Chord("vi"), Chord("ii"), Chord("V"), Chord("I")],
                        done: "Each chord pulls into the next. Countless standards and pop songs use this chain.",
                    },
                    Play {
                        text: "Make it longer: iii → vi → ii → V → I.",
                        targets: &[
                            Chord("iii"),
                            Chord("vi"),
                            Chord("ii"),
                            Chord("V"),
                            Chord("I"),
                        ],
                        done: "Watch the dot move around the circle as you play.",
                    },
                    Read(
                        "Try changing key with {a:transpose+} and play the same chord keys: the circle shows where you are.",
                    ),
                ],
            },
        ],
    },
    Chapter {
        title: "7 · Modes",
        lessons: &[
            Lesson {
                id: "modes",
                title: "Modes: same notes, new home",
                summary: "Dorian, Phrygian, Lydian, Mixolydian…",
                layout: Layout::Notes,
                circle: false,
                steps: &[
                    Read(
                        "You saw that A to A on white keys is minor. Start on any other white key and you get another 'mode': same notes, different home, different mood. Each one has an old Greek name.",
                    ),
                    Play {
                        text: "D Dorian: D to D on white keys. Minor, but hopeful.",
                        targets: &[
                            Note(2),
                            Note(4),
                            Note(5),
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                            Note(14),
                        ],
                        done: "Dorian: jazz, funk, Celtic tunes.",
                    },
                    Play {
                        text: "E Phrygian: E to E. Dark, Spanish-sounding.",
                        targets: &[
                            Note(4),
                            Note(5),
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                            Note(14),
                            Note(16),
                        ],
                        done: "That half step right above home (E → F) gives the flamenco and metal sound.",
                    },
                    Play {
                        text: "F Lydian: F to F. Bright and magical.",
                        targets: &[
                            Note(5),
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                            Note(14),
                            Note(16),
                            Note(17),
                        ],
                        done: "The raised 4th (B against F) sounds like film-music wonder.",
                    },
                    Play {
                        text: "G Mixolydian: G to G. Major with a rock edge.",
                        targets: &[
                            Note(7),
                            Note(9),
                            Note(11),
                            Note(12),
                            Note(14),
                            Note(16),
                            Note(17),
                            Note(19),
                        ],
                        done: "The flat 7th (F against G) is the sound of rock and folk.",
                    },
                    Read(
                        "The full set, brightest to darkest: Lydian, Ionian (= major), Mixolydian, Dorian, Aeolian (= minor), Phrygian, Locrian (so unstable it's rarely used).\n\nThe Chords + Melody layout has Dorian and Mixolydian in its scale list: pick one and the melody keys stay in the mode.",
                    ),
                ],
            },
            Lesson {
                id: "mode-vamps",
                title: "Hearing the modes",
                summary: "Two-chord vamps",
                layout: Layout::Chords,
                circle: false,
                steps: &[
                    Read(
                        "A mode really shows itself when you sit on its home chord. A 'vamp' is two chords repeated. Each of these uses chords of C major, but treats a different chord as home.",
                    ),
                    Play {
                        text: "Dorian vamp: Dm → G, a few times. Hear Dm as home.",
                        targets: &[
                            Chord("ii"),
                            Chord("V"),
                            Chord("ii"),
                            Chord("V"),
                            Chord("ii"),
                        ],
                        done: "Minor, but the major chord on the 4th makes it hopeful (Dorian).",
                    },
                    Play {
                        text: "Mixolydian vamp: G → F.",
                        targets: &[Chord("V"), Chord("IV"), Chord("V"), Chord("IV"), Chord("V")],
                        done: "G major with F major: the rock 'bVII' sound.",
                    },
                    Play {
                        text: "Lydian vamp: F → G.",
                        targets: &[
                            Chord("IV"),
                            Chord("V"),
                            Chord("IV"),
                            Chord("V"),
                            Chord("IV"),
                        ],
                        done: "F as home with G floating above it: dreamy Lydian.",
                    },
                ],
            },
        ],
    },
];

/// Every lesson in order.
pub fn all() -> impl Iterator<Item = &'static Lesson> {
    CHAPTERS.iter().flat_map(|c| c.lessons.iter())
}

pub fn find(id: &str) -> Option<&'static Lesson> {
    all().find(|l| l.id == id)
}

/// The lesson after this one.
pub fn next_after(id: &str) -> Option<&'static Lesson> {
    let v: Vec<&Lesson> = all().collect();
    let i = v.iter().position(|l| l.id == id)?;
    v.get(i + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Action;
    use crate::theory::ChordRef;

    #[test]
    fn lessons_are_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for l in all() {
            assert!(ids.insert(l.id), "duplicate id {}", l.id);
            assert!(!l.steps.is_empty());
            for s in l.steps {
                let text = match s {
                    Step::Read(t) => *t,
                    Step::Listen { text, .. } | Step::Play { text, .. } => *text,
                    Step::Interval { semitones, .. } => {
                        assert!((1..=12).contains(semitones));
                        ""
                    }
                };
                if let Step::Play { targets, done, .. } = s {
                    assert!(!targets.is_empty());
                    check_placeholders(done);
                    for t in *targets {
                        if let Target::Chord(n) = t {
                            assert!(ChordRef::parse(n).is_some(), "{} bad numeral {n}", l.id);
                        }
                    }
                }
                check_placeholders(text);
            }
        }
        assert!(all().count() >= 20);
        assert_eq!(next_after("alphabet").unwrap().id, "octaves");
        assert!(find("modes").is_some());
    }

    fn check_placeholders(text: &str) {
        let mut rest = text;
        while let Some(i) = rest.find('{') {
            let end = rest[i..].find('}').expect("unclosed placeholder") + i;
            let inner = &rest[i + 1..end];
            let (kind, arg) = inner.split_once(':').expect("placeholder needs kind:arg");
            match kind {
                "n" => assert!(arg.parse::<i32>().is_ok(), "{inner}"),
                "c" => assert!(ChordRef::parse(arg).is_some(), "{inner}"),
                "a" => assert!(Action::parse(arg).is_some(), "{inner}"),
                _ => panic!("unknown placeholder {inner}"),
            }
            rest = &rest[end + 1..];
        }
    }
}
