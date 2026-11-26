//! Standard MIDI Files: read any `.mid` into a [`Song`], and write a song's
//! parts back out (the note editor keeps `song.mid` and `midi/<part>.mid`
//! in step with `song.json`).

use super::chords::{estimate_chords, estimate_key, key_from};
use super::{
    Beat, BeatGrid, ChordSpan, Metadata, Note, Origin, Part, Section, Song, Source, Tempo,
};
use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::collections::HashMap;
use std::path::Path;

/// Tempo map: (tick, microseconds per quarter note), sorted.
struct TempoMap {
    ticks_per_beat: f64,
    /// (tick, seconds at that tick, seconds per tick from there).
    segments: Vec<(u64, f64, f64)>,
    /// SMPTE files: fixed seconds per tick.
    fixed: Option<f64>,
}

impl TempoMap {
    fn new(timing: Timing, mut changes: Vec<(u64, u32)>) -> TempoMap {
        let tpb = match timing {
            Timing::Metrical(t) => t.as_int().max(1) as f64,
            Timing::Timecode(fps, sub) => {
                let spt = 1.0 / (fps.as_f32() as f64 * sub.max(1) as f64);
                return TempoMap {
                    ticks_per_beat: 0.5 / spt,
                    segments: vec![(0, 0.0, spt)],
                    fixed: Some(spt),
                };
            }
        };
        changes.sort_by_key(|c| c.0);
        let mut segments = vec![(0u64, 0.0f64, 0.5 / tpb)];
        for (tick, us) in changes {
            let (t0, s0, spt) = *segments.last().unwrap();
            let secs = s0 + (tick - t0) as f64 * spt;
            let new_spt = us as f64 / 1e6 / tpb;
            if tick == t0 {
                segments.last_mut().unwrap().2 = new_spt;
            } else {
                segments.push((tick, secs, new_spt));
            }
        }
        TempoMap {
            ticks_per_beat: tpb,
            segments,
            fixed: None,
        }
    }

    fn seconds(&self, tick: u64) -> f64 {
        if let Some(spt) = self.fixed {
            return tick as f64 * spt;
        }
        let i = self.segments.partition_point(|s| s.0 <= tick).max(1) - 1;
        let (t0, s0, spt) = self.segments[i];
        s0 + (tick - t0) as f64 * spt
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim().to_string()
}

/// General MIDI instrument families, for naming parts.
const GM_FAMILIES: [&str; 16] = [
    "Piano",
    "Chromatic percussion",
    "Organ",
    "Guitar",
    "Bass",
    "Strings",
    "Ensemble",
    "Brass",
    "Reed",
    "Pipe",
    "Synth lead",
    "Synth pad",
    "Synth effects",
    "Ethnic",
    "Percussive",
    "Sound effects",
];

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "part".into() } else { out }
}

/// Read a MIDI file into a song.
pub fn load_file(path: &Path) -> Result<Song, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let mut song = parse(&bytes)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if song.metadata.title.is_empty() || song.metadata.title == "Untitled" {
        song.metadata.title = stem;
    }
    song.source = Source {
        kind: "midi".into(),
        file: path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        path: path.display().to_string(),
        sha256: String::new(),
    };
    song.origin = Origin::Midi(path.to_path_buf());
    Ok(song)
}

/// Parse MIDI bytes into a song (chords and key guessed if the file
/// doesn't say).
pub fn parse(bytes: &[u8]) -> Result<Song, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("not a MIDI file: {e}"))?;
    let mut tempos: Vec<(u64, u32)> = Vec::new();
    let mut time_sigs: Vec<(u64, u32, u32)> = Vec::new();
    let mut key_sig: Option<(i8, bool)> = None;
    let mut markers: Vec<(u64, String)> = Vec::new();
    let mut chord_texts: Vec<(u64, String)> = Vec::new();
    let mut title = String::new();

    struct Raw {
        track: usize,
        channel: u8,
        pitch: u8,
        vel: u8,
        on: u64,
        off: u64,
    }
    let mut raw: Vec<Raw> = Vec::new();
    let mut track_names: HashMap<usize, String> = HashMap::new();
    let mut programs: HashMap<(usize, u8), u8> = HashMap::new();
    let mut end_tick = 0u64;

    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut tick = 0u64;
        let mut pending: HashMap<(u8, u8), Vec<(u64, u8)>> = HashMap::new();
        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Meta(m) => match m {
                    MetaMessage::Tempo(us) => tempos.push((tick, us.as_int())),
                    MetaMessage::TimeSignature(n, d, ..) => {
                        time_sigs.push((tick, n as u32, 1u32 << d.min(6)))
                    }
                    MetaMessage::KeySignature(sf, minor) => {
                        key_sig.get_or_insert((sf, minor));
                    }
                    MetaMessage::TrackName(t) => {
                        let t = text(t);
                        if ti == 0 && title.is_empty() && smf.tracks.len() > 1 {
                            // Track 0's name is often the song title.
                            title = t.clone();
                        }
                        track_names.insert(ti, t);
                    }
                    MetaMessage::Marker(t) => markers.push((tick, text(t))),
                    MetaMessage::Text(t) => {
                        let t = text(t);
                        if let Some(c) = t.strip_prefix("chord:") {
                            chord_texts.push((tick, c.trim().to_string()));
                        }
                    }
                    _ => {}
                },
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            pending
                                .entry((ch, key.as_int()))
                                .or_default()
                                .push((tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(list) = pending.get_mut(&(ch, key.as_int()))
                                && !list.is_empty()
                            {
                                let (on, vel) = list.remove(0);
                                raw.push(Raw {
                                    track: ti,
                                    channel: ch,
                                    pitch: key.as_int(),
                                    vel,
                                    on,
                                    off: tick.max(on + 1),
                                });
                            }
                        }
                        MidiMessage::ProgramChange { program } => {
                            programs.entry((ti, ch)).or_insert(program.as_int());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // Notes never switched off end with the track.
        for ((ch, key), list) in pending {
            for (on, vel) in list {
                raw.push(Raw {
                    track: ti,
                    channel: ch,
                    pitch: key,
                    vel,
                    on,
                    off: tick.max(on + 1),
                });
            }
        }
        end_tick = end_tick.max(tick);
    }
    if raw.is_empty() {
        return Err("this MIDI file has no notes".into());
    }

    let tm = TempoMap::new(smf.header.timing, tempos.clone());
    let last_note_tick = raw.iter().map(|r| r.off).max().unwrap_or(0);
    let duration = tm.seconds(last_note_tick) + 0.5;

    // ---- beats: one per quarter note, bars from the time signatures.
    time_sigs.sort_by_key(|t| t.0);
    if time_sigs.first().is_none_or(|t| t.0 > 0) {
        time_sigs.insert(0, (0, 4, 4));
    }
    let tpb = tm.ticks_per_beat;
    let beats_per_bar_of = |num: u32, den: u32| -> u32 {
        (if den == 8 && num.is_multiple_of(3) && num > 3 {
            num / 3 * 3 / 2 // 6/8 → 3 quarter notes
        } else {
            num * 4 / den.max(1)
        })
        .max(1)
    };
    let mut beats: Vec<Beat> = Vec::new();
    let mut bar = 0i32;
    let mut beat_in_bar = 0u32;
    let last_tick = (last_note_tick as f64 / tpb).ceil() as u64 + 1;
    let mut ts_i = 0;
    let mut cur_bpb = beats_per_bar_of(time_sigs[0].1, time_sigs[0].2);
    for q in 0..=last_tick {
        let tick = (q as f64 * tpb) as u64;
        while ts_i + 1 < time_sigs.len() && time_sigs[ts_i + 1].0 <= tick {
            ts_i += 1;
            cur_bpb = beats_per_bar_of(time_sigs[ts_i].1, time_sigs[ts_i].2);
            beat_in_bar = 0;
        }
        if beat_in_bar == 0 {
            bar += 1;
        }
        beats.push(Beat {
            time: tm.seconds(tick),
            bar,
            beat: beat_in_bar + 1,
            downbeat: beat_in_bar == 0,
        });
        beat_in_bar = (beat_in_bar + 1) % cur_bpb;
    }
    let bpm0 = tempos
        .iter()
        .min_by_key(|t| t.0)
        .map(|t| 60e6 / t.1 as f64)
        .unwrap_or(120.0);
    // The meter that lasts longest (a pickup bar often has its own).
    let main_ts = (0..time_sigs.len())
        .max_by_key(|&i| {
            let end = time_sigs.get(i + 1).map(|t| t.0).unwrap_or(last_note_tick);
            end.saturating_sub(time_sigs[i].0)
        })
        .map(|i| time_sigs[i])
        .unwrap_or((0, 4, 4));
    let main_bpb = beats_per_bar_of(main_ts.1, main_ts.2);
    let grid = BeatGrid::new(&beats, bpm0, main_bpb);

    // ---- parts, one per (track, channel).
    let mut groups: Vec<((usize, u8), Vec<Note>)> = Vec::new();
    for r in &raw {
        let start = tm.seconds(r.on);
        let end = tm.seconds(r.off);
        let note = Note {
            pitch: r.pitch.min(127),
            start,
            end,
            start_beat: grid.beat_at(start),
            end_beat: grid.beat_at(end),
            velocity: r.vel.max(1),
            confidence: 1.0,
            low_confidence: false,
        };
        match groups.iter_mut().find(|g| g.0 == (r.track, r.channel)) {
            Some(g) => g.1.push(note),
            None => groups.push(((r.track, r.channel), vec![note])),
        }
    }
    groups.sort_by_key(|g| g.0);
    let multi_channel = |track: usize| groups.iter().filter(|g| g.0.0 == track).count() > 1;
    let mut parts: Vec<Part> = Vec::new();
    for ((track, ch), mut notes) in groups.clone() {
        notes.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap()
                .then(a.pitch.cmp(&b.pitch))
        });
        let is_drum = ch == 9;
        let program = programs.get(&(track, ch)).copied().unwrap_or(0);
        let tname = track_names.get(&track).cloned().unwrap_or_default();
        let family = if is_drum {
            "Drums"
        } else {
            GM_FAMILIES[(program / 8) as usize]
        };
        let mut name = if tname.is_empty() || (track == 0 && tname == title) {
            format!("Track {} · {family}", track + 1)
        } else {
            tname.clone()
        };
        if multi_channel(track) {
            name = format!("{name} (ch {})", ch + 1);
        }
        let avg = notes.iter().map(|n| n.pitch as f64).sum::<f64>() / notes.len() as f64;
        let role = if is_drum {
            "drums"
        } else if (32..40).contains(&program) || avg < 45.0 {
            "bass"
        } else if program < 8 {
            "piano"
        } else {
            "other"
        };
        // Analyzer files name their tracks by part id ("melody", "chords").
        let known = [
            "melody",
            "chords",
            "bass",
            "drums",
            "piano",
            "vocals",
            "guitar",
            "arrangement",
        ];
        let role = known
            .iter()
            .find(|k| slug(&tname).starts_with(*k))
            .copied()
            .unwrap_or(role);
        let mut id = if tname.is_empty() {
            format!("track-{}", track + 1)
        } else {
            slug(&tname)
        };
        if multi_channel(track) {
            id = format!("{id}-ch{}", ch + 1);
        }
        while parts.iter().any(|p| p.id == id) {
            id.push_str("-2");
        }
        parts.push(Part {
            id,
            name,
            role: role.into(),
            method: "midi".into(),
            stem: None,
            is_drum,
            program,
            transpose: 0,
            notes,
        });
    }
    // The highest part that's mostly one note at a time is the melody.
    if !parts.iter().any(|p| p.role == "melody")
        && let Some(i) = (0..parts.len())
            .filter(|&i| !parts[i].is_drum && parts[i].role != "bass")
            .filter(|&i| monophonic(&parts[i].notes) > 0.8)
            .max_by(|&a, &b| {
                avg_pitch(&parts[a].notes)
                    .partial_cmp(&avg_pitch(&parts[b].notes))
                    .unwrap()
            })
    {
        parts[i].role = "melody".into();
    }

    // ---- key.
    let pitched: Vec<&Note> = parts
        .iter()
        .filter(|p| !p.is_drum)
        .flat_map(|p| p.notes.iter())
        .collect();
    let key = match key_sig {
        Some((sf, minor)) => {
            let major_tonic = (sf as i32 * 7).rem_euclid(12);
            let tonic = if minor {
                (major_tonic + 9) % 12
            } else {
                major_tonic
            };
            Some(key_from(tonic, minor, 1.0))
        }
        None => {
            let c = super::chords::chroma_of_notes(pitched.iter().copied(), 0.0, duration);
            estimate_key(&c).map(|(t, m, conf)| key_from(t, m, conf))
        }
    };

    // ---- chords: from "chord:X" texts (analyzer files) or guessed.
    let chords = if !chord_texts.is_empty() {
        chord_texts.sort_by_key(|c| c.0);
        let mut v: Vec<ChordSpan> = Vec::new();
        for (i, (tick, sym)) in chord_texts.iter().enumerate() {
            let start = tm.seconds(*tick);
            let end = chord_texts
                .get(i + 1)
                .map(|c| tm.seconds(c.0))
                .unwrap_or(duration);
            v.push(span_from_symbol(sym, start, end, &grid, key.as_ref()));
        }
        v
    } else {
        let k = key.clone().unwrap_or_default();
        estimate_chords(&pitched, &grid, duration, &k, 2.0, 0.5)
    };

    // ---- sections from "A:verse" markers.
    markers.sort_by_key(|m| m.0);
    let mut sections = Vec::new();
    for (i, (tick, m)) in markers.iter().enumerate() {
        let start = tm.seconds(*tick);
        let end = markers
            .get(i + 1)
            .map(|n| tm.seconds(n.0))
            .unwrap_or(duration);
        let (label, name) = match m.split_once(':') {
            Some((l, n)) if l.len() == 1 => (l.to_string(), n.to_string()),
            _ => (String::new(), m.clone()),
        };
        let (b0, _) = grid.bar_of(grid.beat_at(start));
        let (b1, _) = grid.bar_of(grid.beat_at(end) - 0.01);
        sections.push(Section {
            start,
            end,
            start_bar: b0,
            end_bar: b1,
            label,
            name,
            confidence: 1.0,
        });
    }

    let song = Song {
        format: "qwertone-song".into(),
        format_version: 1,
        analyzer: Default::default(),
        source: Source::default(),
        metadata: Metadata {
            title,
            artist: None,
            album: None,
        },
        duration,
        low_confidence_threshold: 0.5,
        stems: vec![],
        tempo: Tempo {
            bpm: bpm0,
            beats_per_bar: main_bpb,
            time_signature: [main_ts.1, main_ts.2],
            confidence: 1.0,
        },
        beats,
        key,
        chords,
        sections,
        parts,
        simplified: None,
        origin: Origin::Memory,
    };
    let _ = end_tick;
    Ok(song.normalised())
}

fn monophonic(notes: &[Note]) -> f64 {
    if notes.len() < 2 {
        return 1.0;
    }
    let overlapping = notes
        .windows(2)
        .filter(|w| w[1].start < w[0].end - 0.03)
        .count();
    1.0 - overlapping as f64 / (notes.len() - 1) as f64
}

fn avg_pitch(notes: &[Note]) -> f64 {
    notes.iter().map(|n| n.pitch as f64).sum::<f64>() / notes.len().max(1) as f64
}

/// A chord-sheet entry from a symbol like "Am7".
pub fn span_from_symbol(
    sym: &str,
    start: f64,
    end: f64,
    grid: &BeatGrid,
    key: Option<&super::Key>,
) -> ChordSpan {
    let parsed = crate::theory::parse_chord_symbol(sym);
    let bass = sym
        .split_once('/')
        .map(|(_, b)| b.trim().to_string())
        .filter(|b| super::chords::pc_from_name(b).is_some());
    let (root, quality, pcs, roman) = match parsed {
        Some((r, k)) => (
            Some(crate::theory::pc_name(r, false).to_string()),
            Some(super::chords::kind_quality(k).to_string()),
            k.intervals().iter().map(|t| (r + t) % 12).collect(),
            key.map(|k2| super::chords::roman(r, k, k2).0),
        ),
        None => (None, None, vec![], None),
    };
    ChordSpan {
        start,
        end,
        start_beat: grid.beat_at(start),
        end_beat: grid.beat_at(end),
        symbol: if parsed.is_some() {
            sym.to_string()
        } else {
            "N".into()
        },
        root,
        quality,
        bass,
        pitch_classes: pcs,
        roman,
        confidence: 1.0,
        low_confidence: false,
    }
}

// ------------------------------------------------------------------ writing

const TPB: u16 = 480;

/// Ticks for a time, with every beat of the song exactly 480 ticks (like
/// the analyzer's files, so bar lines line up with the recording).
fn tick_of(grid: &BeatGrid, t: f64) -> u64 {
    let origin = grid.beat_at(0.0);
    ((grid.beat_at(t) - origin) * TPB as f64).round().max(0.0) as u64
}

fn meta(tick: u64, m: MetaMessage<'_>) -> (u64, TrackEventKind<'_>) {
    (tick, TrackEventKind::Meta(m))
}

fn finish(mut evs: Vec<(u64, TrackEventKind<'_>)>) -> Vec<TrackEvent<'_>> {
    // Note-offs before note-ons at the same tick.
    evs.sort_by_key(|(t, k)| {
        let order = match k {
            TrackEventKind::Meta(_) => 0,
            TrackEventKind::Midi {
                message: MidiMessage::ProgramChange { .. },
                ..
            } => 1,
            TrackEventKind::Midi {
                message: MidiMessage::NoteOff { .. },
                ..
            } => 2,
            _ => 3,
        };
        (*t, order)
    });
    let mut out = Vec::with_capacity(evs.len() + 1);
    let mut last = 0u64;
    for (t, kind) in evs {
        out.push(TrackEvent {
            delta: u28::new((t - last).min(0x0FFF_FFFF) as u32),
            kind,
        });
        last = t;
    }
    out.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}

/// Conductor track: tempo map following the beats, meter, key, sections,
/// chord symbols.
fn conductor<'a>(
    song: &Song,
    grid: &BeatGrid,
    texts: &'a [(u64, Vec<u8>, bool)],
) -> Vec<TrackEvent<'a>> {
    let mut evs: Vec<(u64, TrackEventKind)> = vec![meta(0, MetaMessage::TrackName(b"conductor"))];
    // One tempo per beat interval.
    let origin = grid.beat_at(0.0);
    let first_full = origin.ceil() as i64;
    let mut tempos = vec![(0u64, grid.time_at(origin + 1.0) - grid.time_at(origin))];
    let last_beat = grid.beat_at(song.duration).ceil() as i64 + 1;
    for b in first_full.max(0)..last_beat {
        let secs = grid.time_at(b as f64 + 1.0) - grid.time_at(b as f64);
        let tick = ((b as f64 - origin) * TPB as f64).round() as u64;
        tempos.push((tick, secs));
    }
    let mut last_us = 0u32;
    for (tick, secs) in tempos {
        let us = (secs * 1e6).round().clamp(1.0, 16_777_215.0) as u32;
        if us != last_us {
            evs.push(meta(tick, MetaMessage::Tempo(u24::new(us))));
            last_us = us;
        }
    }
    let bpb = song.tempo.beats_per_bar.clamp(1, 32) as u8;
    let first_down = grid
        .bars_between(origin, origin + 64.0)
        .into_iter()
        .find(|(b, _)| *b >= origin)
        .map(|(b, _)| ((b - origin) * TPB as f64).round() as u64)
        .unwrap_or(0);
    evs.push(meta(first_down, MetaMessage::TimeSignature(bpb, 2, 24, 8)));
    if let Some(k) = &song.key {
        // Sharps/flats count from the tonic.
        let major_tonic = if k.minor() {
            (k.tonic_pc + 3) % 12
        } else {
            k.tonic_pc
        };
        let sf = [0i8, -5, 2, -3, 4, -1, 6, 1, -4, 3, -2, 5][major_tonic.rem_euclid(12) as usize];
        evs.push(meta(0, MetaMessage::KeySignature(sf, k.minor())));
    }
    for (tick, bytes, marker) in texts {
        evs.push((
            *tick,
            TrackEventKind::Meta(if *marker {
                MetaMessage::Marker(bytes)
            } else {
                MetaMessage::Text(bytes)
            }),
        ));
    }
    finish(evs)
}

fn part_track<'a>(part: &'a Part, grid: &BeatGrid, channel: u8) -> Vec<TrackEvent<'a>> {
    let ch = u4::new(channel & 15);
    let mut evs: Vec<(u64, TrackEventKind)> =
        vec![meta(0, MetaMessage::TrackName(part.id.as_bytes()))];
    if !part.is_drum {
        evs.push((
            0,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::ProgramChange {
                    program: u7::new(part.program & 127),
                },
            },
        ));
    }
    for n in &part.notes {
        let on = tick_of(grid, n.start);
        let off = tick_of(grid, n.end).max(on + 1);
        let key = u7::new(n.pitch & 127);
        evs.push((
            on,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::NoteOn {
                    key,
                    vel: u7::new(n.velocity.clamp(1, 127)),
                },
            },
        ));
        evs.push((
            off,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
    }
    finish(evs)
}

fn texts(song: &Song, grid: &BeatGrid) -> Vec<(u64, Vec<u8>, bool)> {
    let mut v = Vec::new();
    for s in &song.sections {
        v.push((
            tick_of(grid, s.start),
            format!("{}:{}", s.label, s.name).into_bytes(),
            true,
        ));
    }
    for c in &song.chords {
        v.push((
            tick_of(grid, c.start),
            format!("chord:{}", c.symbol).into_bytes(),
            false,
        ));
    }
    v
}

/// The MIDI channel for each part, like the analyzer: drums on 9, the rest
/// in order skipping 9.
pub fn channels(song: &Song) -> Vec<u8> {
    let mut next = 0u8;
    song.parts
        .iter()
        .map(|p| {
            if p.is_drum {
                9
            } else {
                let c = next;
                next = (next + 1) % 16;
                if next == 9 {
                    next = 10;
                }
                c
            }
        })
        .collect()
}

/// The whole song as a type-1 MIDI file: conductor + one track per part.
pub fn song_to_bytes(song: &Song) -> Vec<u8> {
    let grid = song.grid();
    let t = texts(song, &grid);
    let chans = channels(song);
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPB)),
    ));
    smf.tracks.push(conductor(song, &grid, &t));
    for (p, ch) in song.parts.iter().zip(chans) {
        smf.tracks.push(part_track(p, &grid, ch));
    }
    let mut out = Vec::new();
    smf.write_std(&mut out).expect("writing to memory");
    out
}

/// One part on its own (plus the conductor track).
pub fn part_to_bytes(song: &Song, part: &Part) -> Vec<u8> {
    let grid = song.grid();
    let t = texts(song, &grid);
    let ch = if part.is_drum { 9 } else { 0 };
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPB)),
    ));
    smf.tracks.push(conductor(song, &grid, &t));
    smf.tracks.push(part_track(part, &grid, ch));
    let mut out = Vec::new();
    smf.write_std(&mut out).expect("writing to memory");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::tests::fixture;

    #[test]
    fn reads_the_test_clip() {
        let s = load_file(&fixture("clip.mid")).unwrap();
        assert!((s.tempo.bpm - 120.0).abs() < 0.5);
        assert_eq!(s.tempo.beats_per_bar, 4);
        // Notes come out in seconds that match the analyzer's reading.
        let reference = crate::song::tests::clip_song();
        let mel = reference.part("melody").unwrap();
        let ours = s
            .parts
            .iter()
            .find(|p| p.notes.len() == mel.notes.len() && !p.is_drum)
            .expect("a part with the melody's notes");
        for (a, b) in ours.notes.iter().zip(&mel.notes) {
            assert_eq!(a.pitch, b.pitch);
            assert!(
                (a.start - b.start).abs() < 0.01,
                "{} vs {}",
                a.start,
                b.start
            );
        }
        assert!(s.parts.iter().any(|p| p.is_drum));
        assert!(s.parts.iter().any(|p| p.role == "melody"));
        assert_eq!(
            s.key.as_ref().unwrap().tonic_pc % 12,
            7,
            "G major (or E minor)"
        );
    }

    #[test]
    fn guessed_chords_match_the_truth() {
        let s = load_file(&fixture("clip.mid")).unwrap();
        let truth: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(fixture("clip-truth.json")).unwrap())
                .unwrap();
        let mut right = 0;
        let list = truth["chords"].as_array().unwrap();
        for c in list {
            let mid = (c["start"].as_f64().unwrap() + c["end"].as_f64().unwrap()) / 2.0;
            let ours = s
                .chords
                .iter()
                .find(|x| mid >= x.start && mid < x.end)
                .unwrap();
            if ours.symbol == c["symbol"].as_str().unwrap() {
                right += 1;
            }
        }
        assert!(
            right * 10 >= list.len() * 8,
            "{right}/{} chords right",
            list.len()
        );
    }

    #[test]
    fn analyzer_song_mid_has_chords_and_sections() {
        // The analyzer's own song.mid carries chord texts and section markers.
        let reference = crate::song::tests::clip_song();
        let bytes = song_to_bytes(&reference);
        let s = parse(&bytes).unwrap();
        assert_eq!(s.sections.len(), reference.sections.len());
        let syms: Vec<&str> = s.chords.iter().map(|c| c.symbol.as_str()).collect();
        let want: Vec<&str> = reference.chords.iter().map(|c| c.symbol.as_str()).collect();
        assert_eq!(syms, want);
        assert_eq!(s.parts.len(), reference.parts.len());
        assert_eq!(s.parts[0].id, "melody");
        assert_eq!(s.parts[0].role, "melody");
    }

    #[test]
    fn round_trip_keeps_times() {
        let reference = crate::song::tests::clip_song();
        let s = parse(&song_to_bytes(&reference)).unwrap();
        for (a, b) in s.parts.iter().zip(&reference.parts) {
            assert_eq!(a.notes.len(), b.notes.len(), "{}", b.id);
            for (x, y) in a.notes.iter().zip(&b.notes) {
                assert_eq!(x.pitch, y.pitch);
                assert!(
                    (x.start - y.start).abs() < 0.005,
                    "{} {} vs {}",
                    b.id,
                    x.start,
                    y.start
                );
                assert!((x.end - y.end).abs() < 0.005);
            }
        }
        // One part on its own.
        let one = parse(&part_to_bytes(&reference, &reference.parts[1])).unwrap();
        assert_eq!(one.parts.len(), 1);
        assert_eq!(one.parts[0].notes.len(), reference.parts[1].notes.len());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"hello").is_err());
    }
}
