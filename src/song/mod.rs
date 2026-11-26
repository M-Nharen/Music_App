//! Songs for Song mode (Part 5): the analyzer's `song.json` folders and
//! plain MIDI files, read into one [`Song`] model.
//!
//! Everything in here is pure logic (no egui, no audio) so it can be tested:
//! - [`midi`] reads / writes Standard MIDI Files,
//! - [`chords`] chord symbols, chord / key estimation from pitch classes
//!   (shared with live listen mode),
//! - [`library`] finds songs on disk,
//! - [`practice`] targets, timing, wait mode, loops and scoring,
//! - [`layout`] the auto song layout (song chords → fewest keys),
//! - [`analyze`] runs `qwertone-analyze` and reads its progress,
//! - [`edit`] the note editor's model and saving back to `song.json`.
//!
//! The format is described in `analyzer/SONG_FORMAT.md`.

pub mod analyze;
pub mod chords;
pub mod edit;
pub mod layout;
pub mod library;
pub mod midi;
pub mod practice;

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Newest `format_version` this app understands.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// A separated instrument track (Part 7 plays these).
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct Stem {
    pub name: String,
    pub file: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Tempo {
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub time_signature: [u32; 2],
    pub confidence: f32,
}

impl Default for Tempo {
    fn default() -> Self {
        Tempo {
            bpm: 120.0,
            beats_per_bar: 4,
            time_signature: [4, 4],
            confidence: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct Beat {
    pub time: f64,
    #[serde(default)]
    pub bar: i32,
    #[serde(default = "one")]
    pub beat: u32,
    #[serde(default)]
    pub downbeat: bool,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Key {
    pub tonic: String,
    pub tonic_pc: i32,
    pub mode: String,
    pub name: String,
    pub confidence: f32,
}

impl Default for Key {
    fn default() -> Self {
        Key {
            tonic: "C".into(),
            tonic_pc: 0,
            mode: "major".into(),
            name: "C major".into(),
            confidence: 0.0,
        }
    }
}

impl Key {
    pub fn minor(&self) -> bool {
        self.mode == "minor"
    }
}

/// One chord of the chord sheet.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ChordSpan {
    pub start: f64,
    pub end: f64,
    pub start_beat: f64,
    pub end_beat: f64,
    /// "G", "Am7", "C/E"; "N" = no chord.
    pub symbol: String,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub bass: Option<String>,
    #[serde(default)]
    pub pitch_classes: Vec<i32>,
    #[serde(default)]
    pub roman: Option<String>,
    #[serde(default = "full")]
    pub confidence: f32,
    #[serde(default)]
    pub low_confidence: bool,
}

fn full() -> f32 {
    1.0
}

impl ChordSpan {
    pub fn is_none(&self) -> bool {
        self.symbol == "N" || self.root.is_none() && self.pitch_classes.is_empty()
    }
    pub fn root_pc(&self) -> Option<i32> {
        self.root.as_deref().and_then(chords::pc_from_name)
    }
}

#[allow(dead_code)] // every field of the format, some for later parts
#[derive(Clone, Debug, Deserialize)]
pub struct Section {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub start_bar: i32,
    #[serde(default)]
    pub end_bar: i32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "full")]
    pub confidence: f32,
}

impl Section {
    /// "Verse (A)", "Chorus (B)".
    pub fn title(&self) -> String {
        let mut n = self.name.clone();
        if let Some(c) = n.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        if n.is_empty() || n == "Section" {
            format!("Section {}", self.label)
        } else if self.label.is_empty() {
            n
        } else {
            format!("{n} ({})", self.label)
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct Note {
    pub pitch: u8,
    pub start: f64,
    pub end: f64,
    pub start_beat: f64,
    pub end_beat: f64,
    #[serde(default = "vel")]
    pub velocity: u8,
    #[serde(default = "full")]
    pub confidence: f32,
    #[serde(default)]
    pub low_confidence: bool,
}

fn vel() -> u8 {
    96
}

#[allow(dead_code)] // method / stem: for Part 7's stem mixer
#[derive(Clone, Debug, Deserialize)]
pub struct Part {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub stem: Option<String>,
    #[serde(default)]
    pub is_drum: bool,
    #[serde(default)]
    pub program: u8,
    #[serde(default)]
    pub transpose: i32,
    #[serde(default)]
    pub notes: Vec<Note>,
}

impl Part {
    pub fn low_confidence_count(&self) -> usize {
        self.notes.iter().filter(|n| n.low_confidence).count()
    }
    pub fn pitch_range(&self) -> Option<(u8, u8)> {
        let lo = self.notes.iter().map(|n| n.pitch).min()?;
        let hi = self.notes.iter().map(|n| n.pitch).max()?;
        Some((lo, hi))
    }
    /// Friendly name: "Melody (easy)".
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            self.id.clone()
        } else {
            self.name.clone()
        }
    }
    pub fn is_easy(&self) -> bool {
        self.id.ends_with("-easy") || self.role == "arrangement"
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Simplified {
    pub difficulty: u32,
    pub transpose: i32,
    pub max_keys: u32,
    pub key: Option<Key>,
    pub chords: Vec<ChordSpan>,
    pub parts: Vec<String>,
}

impl Default for Simplified {
    fn default() -> Self {
        Simplified {
            difficulty: 2,
            transpose: 0,
            max_keys: 2,
            key: None,
            chords: Vec::new(),
            parts: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnalyzerInfo {
    pub version: String,
    pub mode: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Source {
    pub kind: String,
    pub file: String,
    pub path: String,
    pub sha256: String,
}

/// Where a song came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Origin {
    /// An analyzer song folder (has `song.json`).
    Folder(PathBuf),
    /// A plain MIDI file.
    Midi(PathBuf),
    #[default]
    Memory,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Song {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub analyzer: AnalyzerInfo,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub metadata: Metadata,
    pub duration: f64,
    #[serde(default = "half")]
    pub low_confidence_threshold: f32,
    #[serde(default)]
    pub stems: Vec<Stem>,
    #[serde(default)]
    pub tempo: Tempo,
    #[serde(default)]
    pub beats: Vec<Beat>,
    #[serde(default)]
    pub key: Option<Key>,
    #[serde(default)]
    pub chords: Vec<ChordSpan>,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub parts: Vec<Part>,
    #[serde(default)]
    pub simplified: Option<Simplified>,
    #[serde(skip)]
    pub origin: Origin,
}

fn half() -> f32 {
    0.5
}

impl Song {
    /// Parse `song.json` text. Rejects other formats and newer versions.
    pub fn from_json(text: &str) -> Result<Song, String> {
        let song: Song = serde_json::from_str(text).map_err(|e| format!("not a song file: {e}"))?;
        if song.format != "qwertone-song" {
            return Err(format!("not a Qwertone song (format \"{}\")", song.format));
        }
        if song.format_version > FORMAT_VERSION {
            return Err(format!(
                "made by a newer analyzer (format {}); update Qwertone",
                song.format_version
            ));
        }
        Ok(song.normalised())
    }

    /// Load a song folder (or a `song.json` path).
    pub fn load_dir(dir: &Path) -> Result<Song, String> {
        let (dir, file) = if dir.is_file() {
            (
                dir.parent().unwrap_or(Path::new(".")).to_path_buf(),
                dir.to_path_buf(),
            )
        } else {
            (dir.to_path_buf(), dir.join("song.json"))
        };
        let text = std::fs::read_to_string(&file)
            .map_err(|e| format!("couldn't read {}: {e}", file.display()))?;
        let mut song = Song::from_json(&text)?;
        song.origin = Origin::Folder(dir);
        Ok(song)
    }

    /// Load a song folder, a `song.json` or a MIDI file, by looking at the path.
    pub fn load(path: &Path) -> Result<Song, String> {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "mid" || ext == "midi" || ext == "kar" {
            midi::load_file(path)
        } else {
            Song::load_dir(path)
        }
    }

    /// Sort notes, fix up missing beat positions, drop broken notes.
    pub fn normalised(mut self) -> Song {
        self.beats.retain(|b| b.time.is_finite());
        self.beats.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let fix_beats = self.beats.is_empty();
        let grid = self.grid();
        for p in &mut self.parts {
            p.notes
                .retain(|n| n.start.is_finite() && n.end.is_finite() && n.pitch < 128);
            for n in &mut p.notes {
                if n.end <= n.start {
                    n.end = n.start + 0.05;
                }
                if fix_beats || !n.start_beat.is_finite() || !n.end_beat.is_finite() {
                    n.start_beat = grid.beat_at(n.start);
                    n.end_beat = grid.beat_at(n.end);
                }
            }
            p.notes.sort_by(|a, b| {
                a.start
                    .partial_cmp(&b.start)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.pitch.cmp(&b.pitch))
            });
        }
        if self.metadata.title.is_empty() {
            self.metadata.title = "Untitled".into();
        }
        if self.duration <= 0.0 {
            self.duration = self
                .parts
                .iter()
                .flat_map(|p| p.notes.iter().map(|n| n.end))
                .fold(0.0, f64::max);
        }
        self
    }

    pub fn title(&self) -> &str {
        &self.metadata.title
    }

    /// "Title · Artist".
    pub fn display_name(&self) -> String {
        match &self.metadata.artist {
            Some(a) if !a.is_empty() => format!("{} · {a}", self.metadata.title),
            _ => self.metadata.title.clone(),
        }
    }

    pub fn grid(&self) -> BeatGrid {
        BeatGrid::new(&self.beats, self.tempo.bpm, self.tempo.beats_per_bar)
    }

    #[allow(dead_code)]
    pub fn part(&self, id: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }

    /// The chords that go with a part: the transposed simplified chords for
    /// a transposed easy part, else the song's chords.
    pub fn chords_for(&self, part: Option<&Part>) -> &[ChordSpan] {
        match (part, &self.simplified) {
            (Some(p), Some(s)) if p.transpose != 0 && !s.chords.is_empty() => &s.chords,
            _ => &self.chords,
        }
    }

    /// Key of a part (the simplified key for transposed easy parts).
    pub fn key_for(&self, part: Option<&Part>) -> Option<&Key> {
        match (part, &self.simplified) {
            (Some(p), Some(s)) if p.transpose != 0 && s.key.is_some() => s.key.as_ref(),
            _ => self.key.as_ref(),
        }
    }

    /// The song folder, if it's one.
    pub fn dir(&self) -> Option<&Path> {
        match &self.origin {
            Origin::Folder(d) => Some(d),
            _ => None,
        }
    }

    /// A stable id for remembering scores: the analyzer fingerprint +
    /// source hash, or the MIDI file path.
    pub fn id(&self) -> String {
        match &self.origin {
            Origin::Midi(p) => format!("midi:{}", p.display()),
            _ if !self.source.sha256.is_empty() => {
                format!(
                    "{}:{}",
                    &self.source.sha256[..self.source.sha256.len().min(16)],
                    self.analyzer.fingerprint
                )
            }
            Origin::Folder(d) => format!("dir:{}", d.display()),
            Origin::Memory => format!("mem:{}", self.metadata.title),
        }
    }
}

/// Converts between seconds and beat positions (fractional indices into the
/// song's beats, see SONG_FORMAT.md), and knows where bars start.
#[derive(Clone, Debug)]
pub struct BeatGrid {
    times: Vec<f64>,
    /// Beat index of each downbeat that's in `times`.
    downbeats: Vec<usize>,
    first_interval: f64,
    last_interval: f64,
    beats_per_bar: u32,
}

impl BeatGrid {
    pub fn new(beats: &[Beat], bpm: f64, beats_per_bar: u32) -> BeatGrid {
        let default = 60.0 / if bpm > 1.0 { bpm } else { 120.0 };
        let mut times: Vec<f64> = beats.iter().map(|b| b.time).collect();
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        if times.is_empty() {
            times.push(0.0);
        }
        let first_interval = if times.len() > 1 {
            (times[1] - times[0]).max(0.05)
        } else {
            default
        };
        let last_interval = if times.len() > 1 {
            (times[times.len() - 1] - times[times.len() - 2]).max(0.05)
        } else {
            default
        };
        let mut downbeats: Vec<usize> = beats
            .iter()
            .enumerate()
            .filter(|(_, b)| b.downbeat)
            .map(|(i, _)| i)
            .collect();
        if downbeats.is_empty() {
            downbeats = (0..times.len())
                .step_by(beats_per_bar.max(1) as usize)
                .collect();
        }
        BeatGrid {
            times,
            downbeats,
            first_interval,
            last_interval,
            beats_per_bar: beats_per_bar.max(1),
        }
    }

    /// A plain grid at a fixed tempo, first beat at 0 s.
    pub fn fixed(bpm: f64, beats_per_bar: u32) -> BeatGrid {
        BeatGrid::new(&[], bpm, beats_per_bar)
    }

    pub fn beats_per_bar(&self) -> u32 {
        self.beats_per_bar
    }

    /// Seconds → beat position.
    pub fn beat_at(&self, t: f64) -> f64 {
        let ts = &self.times;
        if t <= ts[0] {
            return (t - ts[0]) / self.first_interval;
        }
        let last = ts.len() - 1;
        if t >= ts[last] {
            return last as f64 + (t - ts[last]) / self.last_interval;
        }
        let i = ts.partition_point(|x| *x <= t) - 1;
        i as f64 + (t - ts[i]) / (ts[i + 1] - ts[i]).max(1e-9)
    }

    /// Beat position → seconds.
    pub fn time_at(&self, beat: f64) -> f64 {
        let ts = &self.times;
        if beat <= 0.0 {
            return ts[0] + beat * self.first_interval;
        }
        let last = ts.len() - 1;
        if beat >= last as f64 {
            return ts[last] + (beat - last as f64) * self.last_interval;
        }
        let i = beat.floor() as usize;
        ts[i] + (beat - i as f64) * (ts[i + 1] - ts[i])
    }

    /// Seconds per beat around `t` (for windows measured in beats).
    pub fn beat_len(&self, t: f64) -> f64 {
        let b = self.beat_at(t);
        (self.time_at(b + 0.5) - self.time_at(b - 0.5)).max(0.05)
    }

    /// Beat positions of bar starts between `from` and `to` (beats), with the
    /// bar number (1-based, counted from the first downbeat).
    pub fn bars_between(&self, from: f64, to: f64) -> Vec<(f64, i32)> {
        let mut out = Vec::new();
        let first = self.downbeats[0] as f64;
        let bpb = self.beats_per_bar as f64;
        // Downbeats we know, then continue at beats_per_bar after the last.
        let last_known = *self.downbeats.last().unwrap() as f64;
        for (n, &d) in self.downbeats.iter().enumerate() {
            let d = d as f64;
            if d >= from && d <= to {
                out.push((d, n as i32 + 1));
            }
        }
        // Before the first downbeat (pickup bars, if any), and after the last.
        let mut b = first - bpb;
        let mut n = 0;
        while b >= from - bpb && b > -1000.0 {
            if b <= to && b >= from {
                out.push((b, n));
            }
            b -= bpb;
            n -= 1;
        }
        let mut b = last_known + bpb;
        let mut n = self.downbeats.len() as i32 + 1;
        while b <= to {
            if b >= from {
                out.push((b, n));
            }
            b += bpb;
            n += 1;
        }
        out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        out
    }

    /// Bar number and beat-in-bar (both 1-based) at a beat position.
    pub fn bar_of(&self, beat: f64) -> (i32, f64) {
        let bars = self.bars_between(beat - 4.0 * self.beats_per_bar as f64, beat + 0.0001);
        match bars.last() {
            Some((start, n)) => (*n, beat - start + 1.0),
            None => (0, beat + 1.0),
        }
    }

    /// Snap a beat position to a grid (`step` beats).
    pub fn snap(beat: f64, step: f64) -> f64 {
        if step <= 0.0 {
            beat
        } else {
            (beat / step).round() * step
        }
    }
}

/// `~/.local/share/qwertone/songs` (or `$XDG_DATA_HOME/qwertone/songs`).
pub fn songs_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qwertone")
        .join("songs")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    pub fn clip_song() -> Song {
        let text = std::fs::read_to_string(fixture("clip-midi.song.json")).unwrap();
        Song::from_json(&text).unwrap()
    }

    #[test]
    fn reads_analyzer_song_json() {
        let s = clip_song();
        assert_eq!(s.format_version, 1);
        assert_eq!(s.key.as_ref().unwrap().name, "G major");
        assert_eq!(s.tempo.beats_per_bar, 4);
        let ids: Vec<&str> = s.parts.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "melody",
                "chords",
                "melody-easy",
                "chords-easy",
                "arrangement"
            ]
        );
        assert_eq!(s.part("melody").unwrap().notes.len(), 48);
        let simp = s.simplified.as_ref().unwrap();
        assert_eq!(simp.transpose, 5);
        // The easy parts are transposed, so they get the transposed chords.
        let easy = s.part("melody-easy").unwrap();
        assert_eq!(s.chords_for(Some(easy)).len(), simp.chords.len());
        assert_eq!(s.key_for(Some(easy)).unwrap().name, "C major");
        assert_eq!(s.chords_for(s.part("melody")).len(), s.chords.len());
    }

    #[test]
    fn low_confidence_notes_come_through() {
        let text = std::fs::read_to_string(fixture("clip-audio.song.json")).unwrap();
        let s = Song::from_json(&text).unwrap();
        let mel = s.part("melody").unwrap();
        assert!(mel.low_confidence_count() > 0);
        assert!(s.chords.iter().any(|c| c.is_none()));
    }

    #[test]
    fn rejects_other_files() {
        assert!(Song::from_json(r#"{"format":"x","format_version":1,"duration":1}"#).is_err());
        assert!(
            Song::from_json(r#"{"format":"qwertone-song","format_version":99,"duration":1}"#)
                .unwrap_err()
                .contains("newer")
        );
        // Unknown keys are fine.
        let s = Song::from_json(
            r#"{"format":"qwertone-song","format_version":1,"duration":2,"future":[1,2]}"#,
        )
        .unwrap();
        assert_eq!(s.title(), "Untitled");
    }

    #[test]
    fn beat_grid_matches_the_analyzer() {
        let s = clip_song();
        let g = s.grid();
        // Every note's beat positions agree with the grid (within rounding).
        for p in &s.parts {
            for n in &p.notes {
                assert!((g.beat_at(n.start) - n.start_beat).abs() < 0.01, "{n:?}");
                assert!((g.time_at(n.start_beat) - n.start).abs() < 0.01);
            }
        }
        for c in &s.chords {
            assert!((g.beat_at(c.start) - c.start_beat).abs() < 0.01, "{c:?}");
        }
        // Before the first beat: negative, at the first interval.
        assert!(g.beat_at(-0.25) < 0.0);
        let audio =
            Song::from_json(&std::fs::read_to_string(fixture("clip-audio.song.json")).unwrap())
                .unwrap();
        assert!(audio.grid().beat_at(0.0) < 0.0);
        // Past the last beat: keeps going at the last tempo.
        let last = s.beats.last().unwrap().time;
        assert!((g.beat_at(last + 1.0) - (s.beats.len() as f64 - 1.0 + 2.0)).abs() < 0.01);
    }

    #[test]
    fn bars_are_numbered_from_the_first_downbeat() {
        let g = clip_song().grid();
        // The clip starts with a one-beat pickup bar.
        let bars = g.bars_between(0.0, 12.0);
        assert_eq!(bars, vec![(0.0, 1), (1.0, 2), (5.0, 3), (9.0, 4)]);
        assert_eq!(g.bar_of(6.5), (3, 2.5));
        // Beyond the known beats bars continue.
        let far = g.bars_between(200.0, 210.0);
        assert!(!far.is_empty());
        assert_eq!(BeatGrid::snap(1.3, 0.5), 1.5);
    }

    #[test]
    fn fixed_grid() {
        let g = BeatGrid::fixed(120.0, 4);
        assert!((g.beat_at(1.0) - 2.0).abs() < 1e-9);
        assert!((g.time_at(3.0) - 1.5).abs() < 1e-9);
        assert!((g.beat_len(1.0) - 0.5).abs() < 1e-9);
    }
}
