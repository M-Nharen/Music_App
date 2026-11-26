//! The note editor: fix the notes of one part, then save them back into
//! `song.json`. Every note you touch (or confirm) gets confidence 1.0.
//!
//! Edits work in beats (so they snap to the song's grid); seconds are
//! worked out from the beat grid. Saving rewrites only that part's notes in
//! the JSON (every other field, known or not, is kept), keeps a one-time
//! `song.json.bak`, and rewrites `song.mid` and `midi/<part>.mid` to match.

use super::{BeatGrid, Note, Song};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Editor {
    pub part: usize,
    pub notes: Vec<Note>,
    grid: BeatGrid,
    pub selected: Vec<usize>,
    undo: Vec<Vec<Note>>,
    redo: Vec<Vec<Note>>,
    /// Changed since the last save.
    pub dirty: bool,
}

impl Editor {
    pub fn new(song: &Song, part: usize) -> Editor {
        Editor {
            part,
            notes: song.parts[part].notes.clone(),
            grid: song.grid(),
            selected: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
        }
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.notes.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.notes, prev));
            self.selected.retain(|&i| i < self.notes.len());
            self.dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.notes, next));
            self.selected.retain(|&i| i < self.notes.len());
            self.dirty = true;
        }
    }

    /// The user has checked this note: confidence 1.0.
    fn confirm(n: &mut Note) {
        n.confidence = 1.0;
        n.low_confidence = false;
    }

    fn set_beats(&self, n: &mut Note, start_beat: f64, end_beat: f64) {
        let end_beat = end_beat.max(start_beat + 1.0 / 32.0);
        n.start_beat = start_beat;
        n.end_beat = end_beat;
        n.start = self.grid.time_at(start_beat).max(0.0);
        n.end = self.grid.time_at(end_beat).max(n.start + 0.01);
    }

    /// Keep notes in time order (keeping the selection pointing at the
    /// same notes).
    fn sort(&mut self) {
        let mut idx: Vec<usize> = (0..self.notes.len()).collect();
        idx.sort_by(|&a, &b| {
            self.notes[a]
                .start_beat
                .partial_cmp(&self.notes[b].start_beat)
                .unwrap()
                .then(self.notes[a].pitch.cmp(&self.notes[b].pitch))
        });
        let old = std::mem::take(&mut self.notes);
        let mut new_pos = vec![0; old.len()];
        for (new, &o) in idx.iter().enumerate() {
            new_pos[o] = new;
        }
        self.notes = idx.iter().map(|&i| old[i]).collect();
        for s in self.selected.iter_mut() {
            *s = new_pos[*s];
        }
    }

    /// Move notes by beats and semitones.
    pub fn move_notes(&mut self, which: &[usize], d_beats: f64, d_pitch: i32) {
        if which.is_empty() || (d_beats == 0.0 && d_pitch == 0) {
            return;
        }
        self.checkpoint();
        for &i in which {
            let Some(mut n) = self.notes.get(i).copied() else {
                continue;
            };
            n.pitch = (n.pitch as i32 + d_pitch).clamp(0, 127) as u8;
            let (s, e) = (n.start_beat + d_beats, n.end_beat + d_beats);
            self.set_beats(&mut n, s, e);
            Self::confirm(&mut n);
            self.notes[i] = n;
        }
        self.sort();
    }

    /// Set where one note ends.
    pub fn set_end(&mut self, i: usize, end_beat: f64) {
        let Some(mut n) = self.notes.get(i).copied() else {
            return;
        };
        self.checkpoint();
        let s = n.start_beat;
        self.set_beats(&mut n, s, end_beat);
        Self::confirm(&mut n);
        self.notes[i] = n;
    }

    /// Add a note; returns its index.
    pub fn add(&mut self, start_beat: f64, pitch: u8, len_beats: f64, velocity: u8) -> usize {
        self.checkpoint();
        let mut n = Note {
            pitch: pitch.min(127),
            start: 0.0,
            end: 0.0,
            start_beat: 0.0,
            end_beat: 0.0,
            velocity: velocity.clamp(1, 127),
            confidence: 1.0,
            low_confidence: false,
        };
        self.set_beats(&mut n, start_beat, start_beat + len_beats.max(1.0 / 16.0));
        self.notes.push(n);
        self.selected = vec![self.notes.len() - 1];
        self.sort();
        self.selected[0]
    }

    pub fn delete(&mut self, which: &[usize]) {
        if which.is_empty() {
            return;
        }
        self.checkpoint();
        let mut w: Vec<usize> = which.to_vec();
        w.sort_unstable();
        w.dedup();
        for &i in w.iter().rev() {
            if i < self.notes.len() {
                self.notes.remove(i);
            }
        }
        self.selected.clear();
    }

    /// "This note is right": confidence 1.0 without changing it.
    pub fn confirm_notes(&mut self, which: &[usize]) {
        if which
            .iter()
            .all(|&i| self.notes.get(i).is_none_or(|n| !n.low_confidence))
        {
            return;
        }
        self.checkpoint();
        for &i in which {
            if let Some(n) = self.notes.get_mut(i) {
                Self::confirm(n);
            }
        }
    }

    /// Snap selected notes' starts and ends to a grid (in beats).
    pub fn quantize(&mut self, which: &[usize], step: f64) {
        if which.is_empty() {
            return;
        }
        self.checkpoint();
        for &i in which {
            let Some(mut n) = self.notes.get(i).copied() else {
                continue;
            };
            let s = BeatGrid::snap(n.start_beat, step);
            let e = BeatGrid::snap(n.end_beat, step).max(s + step);
            self.set_beats(&mut n, s, e);
            Self::confirm(&mut n);
            self.notes[i] = n;
        }
        self.sort();
    }

    pub fn low_confidence_count(&self) -> usize {
        self.notes.iter().filter(|n| n.low_confidence).count()
    }

    /// The next (or previous) unsure note after `from`, wrapping around.
    pub fn next_unsure(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let n = self.notes.len();
        if n == 0 {
            return None;
        }
        let start = from.unwrap_or(if forward { n - 1 } else { 0 });
        (1..=n)
            .map(|k| {
                if forward {
                    (start + k) % n
                } else {
                    (start + n - k % n) % n
                }
            })
            .find(|&i| self.notes[i].low_confidence)
    }

    /// The notes at a beat/pitch: the topmost one there.
    #[cfg(test)]
    pub fn note_at(&self, beat: f64, pitch: i32) -> Option<usize> {
        self.notes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, n)| n.pitch as i32 == pitch && beat >= n.start_beat && beat < n.end_beat)
            .map(|(i, _)| i)
    }

    /// Put the edited notes into a song (for playing them back, or saving).
    pub fn apply(&self, song: &mut Song) {
        if let Some(p) = song.parts.get_mut(self.part) {
            p.notes = self.notes.clone();
        }
    }

    /// Save into the song folder. `song` must already have the notes applied.
    pub fn save(&mut self, song: &Song) -> Result<PathBuf, String> {
        let dir = song
            .dir()
            .ok_or("this song isn't a song folder (save a MIDI copy instead)")?;
        save_part(dir, song, self.part)?;
        self.dirty = false;
        Ok(dir.join("song.json"))
    }
}

fn r4(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

/// One note as the analyzer writes it.
pub fn note_json(n: &Note) -> Value {
    json!({
        "pitch": n.pitch,
        "name": crate::music::note_name(n.pitch as i32),
        "start": r4(n.start),
        "end": r4(n.end),
        "start_beat": r4(n.start_beat),
        "end_beat": r4(n.end_beat),
        "velocity": n.velocity.clamp(1, 127),
        "confidence": (n.confidence as f64 * 1000.0).round() / 1000.0,
        "low_confidence": n.low_confidence,
    })
}

/// Write a part's notes into `dir/song.json` (keeping everything else),
/// then refresh the MIDI files.
pub fn save_part(dir: &Path, song: &Song, part: usize) -> Result<(), String> {
    let path = dir.join("song.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading song.json: {e}"))?;
    let mut doc: Value = serde_json::from_str(&text).map_err(|e| format!("song.json: {e}"))?;
    let p = &song.parts[part];
    let parts = doc
        .get_mut("parts")
        .and_then(|v| v.as_array_mut())
        .ok_or("song.json has no parts")?;
    let entry = parts
        .iter_mut()
        .find(|v| v.get("id").and_then(|x| x.as_str()) == Some(p.id.as_str()))
        .ok_or_else(|| format!("part {} isn't in song.json", p.id))?;
    let obj = entry.as_object_mut().ok_or("bad part")?;
    obj.insert(
        "notes".into(),
        Value::Array(p.notes.iter().map(note_json).collect()),
    );
    obj.insert("note_count".into(), json!(p.notes.len()));
    obj.insert(
        "low_confidence_count".into(),
        json!(p.low_confidence_count()),
    );
    obj.insert(
        "pitch_range".into(),
        match p.pitch_range() {
            Some((a, b)) => json!([a, b]),
            None => json!([null, null]),
        },
    );
    // Say it was edited, without breaking the schema (options is free-form).
    if let Some(opts) = doc
        .get_mut("analyzer")
        .and_then(|a| a.get_mut("options"))
        .and_then(|o| o.as_object_mut())
    {
        let edited = opts.entry("edited_parts").or_insert_with(|| json!([]));
        if let Some(a) = edited.as_array_mut()
            && !a.iter().any(|x| x.as_str() == Some(p.id.as_str()))
        {
            a.push(json!(p.id));
        }
    }
    let backup = dir.join("song.json.bak");
    if !backup.exists() {
        std::fs::copy(&path, &backup).map_err(|e| format!("backup: {e}"))?;
    }
    let out = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    write_atomic(&path, out.as_bytes())?;

    // MIDI files: the whole song and this part. Best effort.
    let midi_name = doc
        .get("midi")
        .and_then(|m| m.as_str())
        .unwrap_or("song.mid")
        .to_string();
    write_atomic(&dir.join(midi_name), &super::midi::song_to_bytes(song))?;
    let part_file = doc["parts"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == json!(p.id)))
        .and_then(|v| v["midi_file"].as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("midi/{}.mid", p.id));
    let pf = dir.join(part_file);
    if let Some(parent) = pf.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    write_atomic(&pf, &super::midi::part_to_bytes(song, p))?;
    Ok(())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp-qwertone");
    std::fs::write(&tmp, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("saving {}: {e}", path.display()))
}

/// For songs from a MIDI file: write the edited song next to it as
/// `<name>-edited.mid`.
pub fn save_midi_copy(song: &Song, original: &Path) -> Result<PathBuf, String> {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "song".into());
    let stem = stem.trim_end_matches("-edited");
    let out = original.with_file_name(format!("{stem}-edited.mid"));
    write_atomic(&out, &super::midi::song_to_bytes(song))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::tests::fixture;

    fn audio_song() -> Song {
        Song::from_json(&std::fs::read_to_string(fixture("clip-audio.song.json")).unwrap()).unwrap()
    }

    #[test]
    fn edits_confirm_notes_and_undo() {
        let s = audio_song();
        let mel = s.parts.iter().position(|p| p.id == "melody").unwrap();
        let mut e = Editor::new(&s, mel);
        let unsure = e.low_confidence_count();
        assert!(unsure > 0);
        let first = e.next_unsure(None, true).unwrap();
        assert_eq!(first, 0);
        let before = e.notes[first];
        e.move_notes(&[first], 0.0, 12);
        let moved = e
            .notes
            .iter()
            .position(|n| n.pitch == before.pitch + 12 && (n.start - before.start).abs() < 0.01)
            .unwrap();
        assert_eq!(e.notes[moved].confidence, 1.0);
        assert!(!e.notes[moved].low_confidence);
        assert_eq!(e.low_confidence_count(), unsure - 1);
        e.undo();
        assert_eq!(e.notes[first], before);
        assert_eq!(e.low_confidence_count(), unsure);
        e.redo();
        assert_eq!(e.low_confidence_count(), unsure - 1);

        // Moving in time keeps seconds and beats in step.
        let g = s.grid();
        e.move_notes(&[3], 1.0, 0);
        for n in &e.notes {
            assert!((g.time_at(n.start_beat) - n.start).abs() < 2e-3 || n.start == 0.0);
        }
        let n_before = e.notes.len();
        let i = e.add(8.0, 67, 1.0, 90);
        assert_eq!(e.notes[i].pitch, 67);
        assert!((e.notes[i].start - g.time_at(8.0)).abs() < 1e-9);
        assert_eq!(e.notes.len(), n_before + 1);
        assert_eq!(e.note_at(8.5, 67), Some(i));
        e.delete(&[i]);
        assert_eq!(e.notes.len(), n_before);
        e.quantize(&[0, 1], 0.5);
        assert_eq!(e.notes[0].start_beat % 0.5, 0.0);
        assert!(e.dirty);
    }

    #[test]
    fn saves_back_into_song_json() {
        let dir = std::env::temp_dir().join(format!("qwertone-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let original = std::fs::read_to_string(fixture("clip-audio.song.json")).unwrap();
        std::fs::write(dir.join("song.json"), &original).unwrap();
        let mut song = Song::load_dir(&dir).unwrap();
        let mel = song.parts.iter().position(|p| p.id == "melody").unwrap();
        let mut e = Editor::new(&song, mel);
        let all: Vec<usize> = (0..e.notes.len()).collect();
        e.confirm_notes(&all);
        e.delete(&[0]);
        e.apply(&mut song);
        e.save(&song).unwrap();
        assert!(!e.dirty);

        let back = Song::load_dir(&dir).unwrap();
        let p = back.part("melody").unwrap();
        assert_eq!(p.notes.len(), song.parts[mel].notes.len());
        assert!(
            p.notes
                .iter()
                .all(|n| n.confidence == 1.0 && !n.low_confidence)
        );
        // Counts updated, other parts and unknown fields untouched.
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("song.json")).unwrap()).unwrap();
        let orig: Value = serde_json::from_str(&original).unwrap();
        let mp = v["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "melody")
            .unwrap();
        assert_eq!(mp["note_count"], json!(p.notes.len()));
        assert_eq!(mp["low_confidence_count"], json!(0));
        assert_eq!(v["chords"], orig["chords"]);
        assert_eq!(v["parts"][1], orig["parts"][1]);
        assert_eq!(v["analyzer"]["options"]["edited_parts"], json!(["melody"]));
        assert!(dir.join("song.json.bak").exists());
        // MIDI files rewritten and readable.
        let m = crate::song::midi::load_file(&dir.join("song.mid")).unwrap();
        assert_eq!(m.parts[0].notes.len(), p.notes.len());
        assert!(dir.join("midi/melody.mid").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validates_against_the_schema_shape() {
        // The fields the JSON schema requires for a note.
        let n = audio_song().parts[0].notes[0];
        let v = note_json(&n);
        for k in [
            "pitch",
            "name",
            "start",
            "end",
            "start_beat",
            "end_beat",
            "velocity",
            "confidence",
            "low_confidence",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
    }
}
