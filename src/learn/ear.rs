//! Ear-training games: hear something, answer by playing it.
//!
//! Questions are built from what the current layout can actually play, so
//! the answer is always on a key you have (and the Learn tab lights it up).

use super::Rng;
use super::analysis::{Family, numeral, role};
use super::intervals::interval;
use crate::theory::{Scale, pc_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Game {
    /// Hear C, then a mystery note: find it.
    FindNote,
    /// Hear two notes (the first one is lit): play the second.
    Interval,
    /// Happy or sad? Hear a chord, answer with its 3rd or the matching chord key.
    Quality,
    /// Hear home, then a chord of the key: play that chord (or its root).
    Chord,
}

impl Game {
    pub const ALL: [Game; 4] = [Game::FindNote, Game::Interval, Game::Quality, Game::Chord];

    pub fn name(self) -> &'static str {
        match self {
            Game::FindNote => "Find the note",
            Game::Interval => "Intervals",
            Game::Quality => "Happy or sad?",
            Game::Chord => "Name that chord",
        }
    }

    /// Key for saved best scores.
    pub fn code(self) -> &'static str {
        match self {
            Game::FindNote => "note",
            Game::Interval => "interval",
            Game::Quality => "quality",
            Game::Chord => "chord",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Game::FindNote => "You hear C (home), then a mystery note. Find it on the keyboard.",
            Game::Interval => "You hear two notes. The first one is lit: play the second.",
            Game::Quality => {
                "You hear a chord. Was it major (happy) or minor (sad)? Answer on the lit keys."
            }
            Game::Chord => {
                "You hear the home chord, then another chord of the key. Play that chord."
            }
        }
    }

    pub fn level_names(self) -> [&'static str; 3] {
        match self {
            Game::FindNote => ["C D E F G", "White keys", "All 12 notes"],
            Game::Interval => ["3rd, 5th, octave", "Common intervals", "All intervals"],
            Game::Quality => ["Major / minor on C", "+ sus4", "Any root"],
            Game::Chord => ["I IV V", "I ii IV V vi", "Every chord key"],
        }
    }
}

/// A chord key of the current layout (with no modifiers held).
#[derive(Clone, Debug, PartialEq)]
pub struct ChordKey {
    pub root_pc: i32,
    pub family: Family,
    pub name: String,
    pub numeral: String,
    /// Scale step for diatonic chord keys (1–7), None for borrowed ones.
    pub degree: Option<u8>,
    /// Notes to play it with.
    pub notes: Vec<i32>,
}

/// What the layout can play right now.
pub struct Context<'a> {
    /// Every note a key plays (sorted, no duplicates).
    pub notes: &'a [i32],
    pub chords: &'a [ChordKey],
    /// "Lesson C": the C at the bottom of the Piano layout.
    pub c: i32,
    pub tonic_pc: i32,
    pub scale: Scale,
}

impl Context<'_> {
    fn has(&self, n: i32) -> bool {
        self.notes.binary_search(&n).is_ok()
    }
    /// The playable note with this pitch class nearest to `near`.
    fn nearest_pc(&self, pc: i32, near: i32) -> Option<i32> {
        self.notes
            .iter()
            .copied()
            .filter(|n| n.rem_euclid(12) == pc.rem_euclid(12))
            .min_by_key(|n| ((n - near).abs(), *n))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// This exact note.
    Note(i32),
    /// This note name, any octave.
    Pc(i32),
    Chord {
        root_pc: i32,
        family: Family,
    },
}

/// Something the player did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Played {
    Note(i32),
    Chord { root_pc: i32, family: Family },
}

impl Answer {
    pub fn accepts(&self, p: Played) -> bool {
        match (self, p) {
            (Answer::Note(n), Played::Note(m)) => *n == m,
            (Answer::Pc(pc), Played::Note(m)) => m.rem_euclid(12) == *pc,
            (Answer::Pc(pc), Played::Chord { root_pc, .. }) => root_pc == *pc,
            (
                Answer::Chord { root_pc, family },
                Played::Chord {
                    root_pc: r,
                    family: f,
                },
            ) => *root_pc == r && *family == f,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    /// Moments to play; each is the notes that start together (empty = rest).
    pub sound: Vec<Vec<i32>>,
    pub gap_ms: u32,
    pub answer: Answer,
    /// Reference notes to light ("home", "first note").
    pub home: Vec<(i32, &'static str)>,
    /// Possible answers to light, with a small label.
    pub options: Vec<(i32, String)>,
    /// Chord keys to light as possible answers.
    pub chord_options: Vec<(i32, Family)>,
    pub prompt: String,
    /// Shown after answering.
    pub reveal: String,
}

const DIATONIC: [(i32, Family); 7] = [
    (0, Family::Major),
    (2, Family::Minor),
    (4, Family::Minor),
    (5, Family::Major),
    (7, Family::Major),
    (9, Family::Minor),
    (11, Family::Dim),
];

/// Make a question. `level` is 0, 1 or 2. Errors explain what the layout
/// is missing.
pub fn question(
    game: Game,
    level: usize,
    ctx: &Context,
    rng: &mut Rng,
) -> Result<Question, String> {
    let level = level.min(2);
    let name = |n: i32| pc_name(n, false);
    match game {
        Game::FindNote => {
            let offs: &[i32] = match level {
                0 => &[0, 2, 4, 5, 7],
                1 => &[0, 2, 4, 5, 7, 9, 11],
                _ => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            };
            let cands: Vec<i32> = offs
                .iter()
                .filter_map(|o| ctx.nearest_pc(ctx.c + o, ctx.c + o))
                .collect();
            if cands.len() < 3 {
                return Err("This layout doesn't have enough notes. Switch to Piano.".into());
            }
            let home = ctx.nearest_pc(ctx.c, ctx.c).unwrap_or(cands[0]);
            let target = *rng.pick(&cands).unwrap();
            let semis = (target - home).rem_euclid(12);
            let iv = interval(semis);
            Ok(Question {
                sound: vec![vec![home], vec![target]],
                gap_ms: 700,
                answer: Answer::Pc(target.rem_euclid(12)),
                home: vec![(home, "home")],
                options: if level < 2 {
                    cands.iter().map(|n| (*n, String::new())).collect()
                } else {
                    Vec::new()
                },
                chord_options: Vec::new(),
                prompt: "First C (home), then a mystery note. Find it! Any octave counts.".into(),
                reveal: if semis == 0 {
                    "It was C itself, the home note.".into()
                } else {
                    format!(
                        "It was {}: {semis} half steps above C ({}).",
                        name(target),
                        iv.name.to_lowercase()
                    )
                },
            })
        }
        Game::Interval => {
            let pool: Vec<i32> = match level {
                0 => vec![4, 7, 12],
                1 => vec![2, 3, 4, 5, 7, 9, 12],
                _ => (1..=12).collect(),
            };
            // Roots near lesson C where both notes are playable.
            let mut tries = 0;
            loop {
                tries += 1;
                let semis = *rng.pick(&pool).unwrap();
                let roots: Vec<i32> = ctx
                    .notes
                    .iter()
                    .copied()
                    .filter(|r| ctx.has(r + semis) && (ctx.c - 7..=ctx.c + 14).contains(r))
                    .collect();
                if let Some(&root) = rng.pick(&roots) {
                    let iv = interval(semis);
                    let options = if level == 0 {
                        pool.iter()
                            .filter(|s| ctx.has(root + **s))
                            .map(|s| (root + s, interval(*s).short.to_string()))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    return Ok(Question {
                        sound: vec![
                            vec![root],
                            vec![root + semis],
                            vec![],
                            vec![root, root + semis],
                        ],
                        gap_ms: 600,
                        answer: Answer::Note(root + semis),
                        home: vec![(root, "1st")],
                        options,
                        chord_options: Vec::new(),
                        prompt: "The lit key is the first note. Play the second note you heard."
                            .into(),
                        reveal: format!(
                            "{} ({} → {}): {}. Like {}.",
                            iv.name,
                            name(root),
                            name(root + semis),
                            iv.mood,
                            iv.songs[0]
                        ),
                    });
                }
                if tries > 40 {
                    return Err("This layout can't play these intervals. Switch to Piano.".into());
                }
            }
        }
        Game::Quality => {
            // Chord keys: find a root that has both a major and a minor key.
            let pairs: Vec<(&ChordKey, &ChordKey)> = ctx
                .chords
                .iter()
                .filter(|a| a.family == Family::Major)
                .filter_map(|a| {
                    ctx.chords
                        .iter()
                        .find(|b| b.family == Family::Minor && b.root_pc == a.root_pc)
                        .map(|b| (a, b))
                })
                .collect();
            if let Some(&(maj, min)) = rng.pick(&pairs) {
                let pick = if rng.below(2) == 0 { maj } else { min };
                let happy = pick.family == Family::Major;
                return Ok(Question {
                    sound: vec![pick.notes.clone()],
                    gap_ms: 600,
                    answer: Answer::Chord {
                        root_pc: pick.root_pc,
                        family: pick.family,
                    },
                    home: Vec::new(),
                    options: Vec::new(),
                    chord_options: vec![(maj.root_pc, Family::Major), (min.root_pc, Family::Minor)],
                    prompt: format!(
                        "Happy or sad? Press {} for happy (major) or {} for sad (minor).",
                        maj.name, min.name
                    ),
                    reveal: if happy {
                        format!(
                            "{}: major. The 3rd is 4 half steps up, so it sounds bright.",
                            maj.name
                        )
                    } else {
                        format!(
                            "{}: minor. The 3rd is only 3 half steps up, so it sounds sad.",
                            min.name
                        )
                    },
                });
            }
            // Note keys: answer with the chord's 3rd.
            let roots: Vec<i32> = if level < 2 {
                ctx.nearest_pc(ctx.c, ctx.c).into_iter().collect()
            } else {
                ctx.notes
                    .iter()
                    .copied()
                    .filter(|r| (ctx.c - 5..=ctx.c + 12).contains(r))
                    .collect()
            };
            let kinds: &[(i32, &str, &str)] = if level == 0 {
                &[(4, "happy", "major"), (3, "sad", "minor")]
            } else {
                &[
                    (4, "happy", "major"),
                    (3, "sad", "minor"),
                    (5, "sus", "sus4"),
                ]
            };
            let roots: Vec<i32> = roots
                .into_iter()
                .filter(|r| kinds.iter().all(|(t, ..)| ctx.has(r + t)))
                .collect();
            let Some(&root) = rng.pick(&roots) else {
                return Err(
                    "This game needs every note (or one-key chords). Switch to Piano.".into(),
                );
            };
            let &(third, mood, kind) = rng.pick(kinds).unwrap();
            let chord = vec![root, root + third, root + 7];
            Ok(Question {
                sound: vec![
                    chord.clone(),
                    vec![],
                    vec![root],
                    vec![root + third],
                    vec![root + 7],
                ],
                gap_ms: 550,
                answer: Answer::Note(root + third),
                home: vec![(root, "root")],
                options: kinds
                    .iter()
                    .map(|(t, m, _)| (root + t, m.to_string()))
                    .collect(),
                chord_options: Vec::new(),
                prompt: format!(
                    "Happy or sad{}? Play the middle note you heard: the lit keys are labelled.",
                    if level == 0 { "" } else { " (or sus)" }
                ),
                reveal: format!(
                    "{}{}: {mood}. The middle note was {} ({} half steps above the root).",
                    name(root),
                    match kind {
                        "major" => "",
                        "minor" => "m",
                        _ => "sus4",
                    },
                    name(root + third),
                    third
                ),
            })
        }
        Game::Chord => {
            let degrees: &[u8] = match level {
                0 => &[1, 4, 5],
                1 => &[1, 2, 4, 5, 6],
                _ => &[1, 2, 3, 4, 5, 6, 7],
            };
            // One-key chords.
            if let Some(home) = ctx.chords.iter().find(|c| c.degree == Some(1)) {
                let pool: Vec<&ChordKey> = ctx
                    .chords
                    .iter()
                    .filter(|c| match c.degree {
                        Some(d) => degrees.contains(&d),
                        None => level == 2,
                    })
                    .collect();
                if pool.len() >= 2 {
                    let pick = *rng.pick(&pool).unwrap();
                    let off = (pick.root_pc - ctx.tonic_pc).rem_euclid(12);
                    let (label, ..) = role(off, pick.family, ctx.scale);
                    return Ok(Question {
                        sound: vec![home.notes.clone(), vec![], pick.notes.clone()],
                        gap_ms: 750,
                        answer: Answer::Chord {
                            root_pc: pick.root_pc,
                            family: pick.family,
                        },
                        home: Vec::new(),
                        options: Vec::new(),
                        chord_options: if level == 0 {
                            pool.iter().map(|c| (c.root_pc, c.family)).collect()
                        } else {
                            Vec::new()
                        },
                        prompt:
                            "First the home chord, then a mystery chord. Play the mystery chord."
                                .into(),
                        reveal: format!(
                            "{} ({}): {}.",
                            pick.name,
                            pick.numeral,
                            label.to_lowercase()
                        ),
                    });
                }
            }
            // Piano: chords of C major; answer with the root.
            let home = ctx
                .nearest_pc(ctx.c, ctx.c)
                .ok_or("This layout has no C. Switch to Piano.")?;
            let pool: Vec<(i32, Family)> = DIATONIC
                .iter()
                .copied()
                .filter(|(o, _)| {
                    let deg = DIATONIC.iter().position(|(x, _)| x == o).unwrap() as u8 + 1;
                    degrees.contains(&deg) && ctx.nearest_pc(ctx.c + o, ctx.c + o).is_some()
                })
                .collect();
            if pool.len() < 2 {
                return Err(
                    "This layout doesn't have enough notes. Switch to Piano or Chords.".into(),
                );
            }
            let triad = |o: i32, f: Family| -> Vec<i32> {
                let (t, fi) = match f {
                    Family::Major => (4, 7),
                    Family::Minor => (3, 7),
                    _ => (3, 6),
                };
                let r = ctx.c + o;
                vec![r, r + t, r + fi]
            };
            let &(o, f) = rng.pick(&pool).unwrap();
            let suffix = match f {
                Family::Major => "",
                Family::Minor => "m",
                _ => "dim",
            };
            let num = numeral(o, f, "");
            let (label, ..) = role(o, f, Scale::Major);
            Ok(Question {
                sound: vec![triad(0, Family::Major), vec![], triad(o, f)],
                gap_ms: 750,
                answer: Answer::Pc((ctx.c + o).rem_euclid(12)),
                home: vec![(home, "home")],
                options: if level == 0 {
                    pool.iter()
                        .filter_map(|(o, f)| {
                            ctx.nearest_pc(ctx.c + o, ctx.c + o)
                                .map(|n| (n, numeral(*o, *f, "")))
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                chord_options: Vec::new(),
                prompt: "First C major (home), then a mystery chord of C major. Press its root note (the chord's letter).".into(),
                reveal: format!("{}{suffix} ({num}): {}.", name(ctx.c + o), label.to_lowercase()),
            })
        }
    }
}

/// Score keeping for one game session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Score {
    pub asked: u32,
    pub right_first_try: u32,
    pub streak: u32,
    pub best_streak: u32,
}

impl Score {
    /// Record a finished question.
    pub fn record(&mut self, first_try: bool) {
        self.asked += 1;
        if first_try {
            self.right_first_try += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
        } else {
            self.streak = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piano_notes() -> Vec<i32> {
        (48..=79).collect()
    }

    fn ctx<'a>(notes: &'a [i32], chords: &'a [ChordKey]) -> Context<'a> {
        Context {
            notes,
            chords,
            c: 48,
            tonic_pc: 0,
            scale: Scale::Major,
        }
    }

    #[test]
    fn piano_questions_are_answerable() {
        let notes = piano_notes();
        let c = ctx(&notes, &[]);
        let mut rng = Rng::new(7);
        for game in Game::ALL {
            for level in 0..3 {
                for _ in 0..30 {
                    let q = question(game, level, &c, &mut rng).unwrap();
                    // The answer is playable.
                    let ok = notes.iter().any(|n| q.answer.accepts(Played::Note(*n)));
                    assert!(ok, "{game:?} {level} {q:?}");
                    assert!(q.options.iter().all(|(n, _)| notes.contains(n)));
                    assert!(q.home.iter().all(|(n, _)| notes.contains(n)));
                    if !q.options.is_empty() {
                        assert!(
                            q.options
                                .iter()
                                .any(|(n, _)| q.answer.accepts(Played::Note(*n))),
                            "{q:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn chord_key_questions() {
        let notes: Vec<i32> = vec![60, 62, 64, 65, 67, 69, 71, 72];
        let mk = |root_pc, family, deg| ChordKey {
            root_pc,
            family,
            name: String::new(),
            numeral: String::new(),
            degree: deg,
            notes: vec![48 + root_pc],
        };
        let chords = vec![
            mk(0, Family::Major, Some(1)),
            mk(2, Family::Minor, Some(2)),
            mk(5, Family::Major, Some(4)),
            mk(7, Family::Major, Some(5)),
            mk(9, Family::Minor, Some(6)),
            mk(0, Family::Minor, None),
        ];
        let c = ctx(&notes, &chords);
        let mut rng = Rng::new(3);
        for _ in 0..30 {
            let q = question(Game::Chord, 0, &c, &mut rng).unwrap();
            let Answer::Chord { root_pc, .. } = q.answer else {
                panic!()
            };
            assert!([0, 5, 7].contains(&root_pc));
            let q = question(Game::Quality, 0, &c, &mut rng).unwrap();
            assert_eq!(q.chord_options.len(), 2);
        }
        assert!(
            Answer::Chord {
                root_pc: 7,
                family: Family::Major
            }
            .accepts(Played::Chord {
                root_pc: 7,
                family: Family::Major
            })
        );
        assert!(
            !Answer::Chord {
                root_pc: 7,
                family: Family::Major
            }
            .accepts(Played::Chord {
                root_pc: 7,
                family: Family::Minor
            })
        );
    }

    #[test]
    fn drums_layout_errors() {
        let c = ctx(&[], &[]);
        let mut rng = Rng::new(1);
        for game in Game::ALL {
            assert!(question(game, 0, &c, &mut rng).is_err());
        }
    }

    #[test]
    fn scoring() {
        let mut s = Score::default();
        s.record(true);
        s.record(true);
        s.record(false);
        s.record(true);
        assert_eq!(
            (s.asked, s.right_first_try, s.streak, s.best_streak),
            (4, 3, 1, 2)
        );
    }
}
