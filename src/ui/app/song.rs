//! The Song tab (Part 5): the song library, practising a song (falling
//! notes, chord sheet, wait mode, loops, scoring, one-key mode, the auto
//! song layout), starting the analyzer, and the note editor.

use super::*;
use crate::audio::songplay::{SongClock, SongCommand, SongData, SongEvent};
use crate::song::analyze::{self, Job, Options};
use crate::song::edit::Editor;
use crate::song::library::{self, Entry};
use crate::song::practice::{
    self, ClockSnapshot, Hit, Judgement, Lane, Outcome, Played, Practice, Score, Settings, Target,
    Want,
};
use crate::song::{BeatGrid, Origin, Song, chords::span_kind, layout};
use crate::ui::song_view::{
    self, FallView, NoteDraw, NoteState, RollAction, RollNote, RollState, SheetChord,
};
use crate::ui::widgets::{Hint, HintKind};
use egui::vec2;

mod jam;
use jam::JamUi;

/// Channels for the song's parts (0–2 are yours, 9 is drums).
const SONG_CHANNELS: [u8; 12] = [3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15];
/// Track number of the metronome clicks.
const CLICK_TRACK: u8 = 63;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SongView {
    Library,
    Practice,
    Edit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowseFor {
    Open,
    Analyze,
}

/// A key's label for a target: (key, plays exactly this note?).
type KeyLabel = Option<(KeyId, bool)>;

pub(super) struct SongUi {
    pub view: SongView,
    // Library.
    library: Vec<Entry>,
    midi_files: Vec<Entry>,
    scanned: bool,
    open_path: String,
    analyze_path: String,
    browse: Option<(BrowseFor, Receiver<Option<PathBuf>>)>,
    analyzer: Option<PathBuf>,
    job: Option<Job>,
    job_error: Option<(String, Vec<String>)>,
    opts: Options,
    // The song.
    song: Option<Song>,
    grid: BeatGrid,
    data: Option<Arc<SongData>>,
    /// Old song data kept alive so the audio thread never frees it.
    retired: Vec<Arc<SongData>>,
    load_error: Option<String>,
    // What you play.
    play_parts: Vec<usize>,
    play_chords: bool,
    shift: i32,
    hear: Vec<bool>,
    practice: Option<Practice>,
    labels: Vec<KeyLabel>,
    signature: String,
    // Transport.
    pub speed: f64,
    looping: Option<(f64, f64)>,
    counting_in: Option<f64>,
    sent_wait: Option<f64>,
    sent_mute: u64,
    // Clock.
    clock: SongClock,
    pub heard: f64,
    seeks_seen: u32,
    loops_seen: u32,
    ended_seen: bool,
    // Feedback.
    flash: Option<(bool, Instant)>,
    last_hit: Option<(Hit, Instant)>,
    passes: Vec<f32>,
    result: Option<(Score, bool)>,
    onekey: HashMap<KeyId, Vec<(u8, u8)>>,
    pending_offs: Vec<(Instant, u8, u8)>,
    // Song layout.
    pub layout: Option<layout::SongLayout>,
    prev_keymap: Option<String>,
    pub style_overridden: bool,
    // Editor.
    editor: Option<Editor>,
    roll: RollState,
    snap: f64,
    edit_hear_all: bool,
    confirm_leave: bool,
    pub was_tab: bool,
    // Jam mode (Part 7).
    jam: JamUi,
}

impl SongUi {
    pub fn new() -> SongUi {
        SongUi {
            view: SongView::Library,
            library: Vec::new(),
            midi_files: Vec::new(),
            scanned: false,
            open_path: String::new(),
            analyze_path: String::new(),
            browse: None,
            analyzer: None,
            job: None,
            job_error: None,
            opts: Options::default(),
            song: None,
            grid: BeatGrid::fixed(120.0, 4),
            data: None,
            retired: Vec::new(),
            load_error: None,
            play_parts: Vec::new(),
            play_chords: false,
            shift: 0,
            hear: Vec::new(),
            practice: None,
            labels: Vec::new(),
            signature: String::new(),
            speed: 1.0,
            looping: None,
            counting_in: None,
            sent_wait: None,
            sent_mute: 0,
            clock: SongClock::default(),
            heard: 0.0,
            seeks_seen: 0,
            loops_seen: 0,
            ended_seen: false,
            flash: None,
            last_hit: None,
            passes: Vec::new(),
            result: None,
            onekey: HashMap::new(),
            pending_offs: Vec::new(),
            layout: None,
            prev_keymap: None,
            style_overridden: false,
            editor: None,
            roll: RollState::default(),
            snap: 0.25,
            edit_hear_all: true,
            confirm_leave: false,
            was_tab: false,
            jam: JamUi::new(),
        }
    }
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:02}", (t / 60.0) as u32, (t % 60.0) as u32)
}

impl QwertoneApp {
    // ------------------------------------------------------------ timing

    /// How long sound takes to reach your ears on this output, in ms: the
    /// measured Bluetooth / headphone delay if you ran the test, else a
    /// typical value, plus the Sync nudge.
    pub(super) fn heard_delay_ms(&self) -> f32 {
        (self.output_delay_ms() + self.cfg.song.offset_ms).max(0.0)
    }

    /// How far behind the audio clock the falling notes (and scoring) run.
    /// Normally what you hear. With "line up" (Part 7) only the nudge: the
    /// notes run ahead of the sound by the output delay, so a key pressed
    /// on the line comes out of the earbuds together with the backing —
    /// the same as delaying the backing and the notes by that delay.
    pub(super) fn visual_delay_ms(&self) -> f32 {
        if self.cfg.song.line_up {
            self.cfg.song.offset_ms
        } else {
            self.heard_delay_ms()
        }
    }

    /// The output's delay without the nudge (measured, typical or reported).
    pub(super) fn output_delay_ms(&self) -> f32 {
        let measured = self
            .current_device
            .as_ref()
            .and_then(|d| self.cfg.devices.get(&d.sink))
            .map(|p| p.latency_ms)
            .filter(|l| *l > 0.0);
        let base = match (measured, &self.current_device, &self.engine) {
            (Some(m), _, _) => m,
            (None, Some(d), _) if d.bluetooth => d.typical_latency_ms(),
            (None, _, Some(e)) => e.stats.reported_ms() + e.stats.buffer_ms(),
            _ => 20.0,
        };
        base.max(0.0)
    }

    fn practice_settings(&self) -> Settings {
        Settings {
            wait: self.cfg.song.wait,
            any_octave: self.cfg.song.any_octave,
            ..Default::default()
        }
        .with_latency(self.heard_delay_ms(), self.grace_ms())
    }

    fn song_send(&self, c: SongCommand) {
        self.send(Command::Song(c));
    }

    /// The heard song time right now.
    fn song_heard(&self) -> f64 {
        self.song_ui.heard
    }

    fn halted(&self) -> bool {
        let c = &self.song_ui.clock;
        c.playing && !c.moving && !c.ended
    }

    // ------------------------------------------------------------ loading

    pub(super) fn song_rescan(&mut self) {
        self.song_ui.library = library::scan_songs(&crate::song::songs_dir());
        self.song_ui.midi_files = library::scan_midi(&library::midi_dirs(), 3, 60);
        self.song_ui.analyzer = analyze::find_analyzer();
        self.song_ui.scanned = true;
    }

    pub(super) fn open_song(&mut self, path: &std::path::Path) {
        match Song::load(path) {
            Ok(song) => {
                let title = song.display_name();
                self.set_song(song);
                let p = path.to_path_buf();
                self.cfg.song.recent.retain(|r| *r != p);
                self.cfg.song.recent.insert(0, p);
                self.cfg.song.recent.truncate(12);
                self.mark_dirty();
                self.song_ui.view = SongView::Practice;
                self.notify(format!("Opened {title}"));
            }
            Err(e) => {
                self.song_ui.load_error = Some(e.clone());
                self.notify(format!("Couldn't open that song: {e}"));
            }
        }
    }

    fn set_song(&mut self, song: Song) {
        self.song_stop_sounds();
        let su = &mut self.song_ui;
        su.grid = song.grid();
        su.load_error = None;
        su.looping = None;
        su.result = None;
        su.passes.clear();
        su.editor = None;
        su.speed = 1.0;
        su.heard = song.beats.first().map(|b| b.time).unwrap_or(0.0).min(0.0);
        // Default: the easy melody if there is one, else the melody, else
        // the first part with notes; plus the chord sheet.
        let pick = |id: &str| {
            song.parts
                .iter()
                .position(|p| p.id == id && !p.notes.is_empty())
        };
        let melody = pick("melody-easy")
            .or_else(|| pick("melody"))
            .or_else(|| {
                song.parts
                    .iter()
                    .position(|p| p.role == "melody" && !p.notes.is_empty())
            })
            .or_else(|| {
                song.parts
                    .iter()
                    .position(|p| !p.is_drum && !p.notes.is_empty())
            });
        su.play_parts = melody.into_iter().collect();
        su.play_chords = !song.chords.iter().all(|c| c.is_none());
        su.roll = RollState::default();
        su.song = Some(song);
        self.song_default_hear();
        self.song_rebuild(true);
        self.song_send_data();
        if self.cfg.song.song_layout {
            self.song_apply_layout();
        }
    }

    /// Which parts you hear by default: not the ones you play, and not
    /// another version of them (melody vs melody-easy).
    fn song_default_hear(&mut self) {
        let Some(song) = &self.song_ui.song else {
            return;
        };
        let mine: Vec<&crate::song::Part> = self
            .song_ui
            .play_parts
            .iter()
            .map(|&i| &song.parts[i])
            .collect();
        let my_roles: Vec<&str> = mine.iter().map(|p| p.role.as_str()).collect();
        let playing_easy = mine.iter().any(|p| p.is_easy());
        let chords_mine = self.song_ui.play_chords;
        self.song_ui.hear = song
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if self.song_ui.play_parts.contains(&i) {
                    return false;
                }
                if p.is_easy() != playing_easy
                    && (p.role == "melody" || p.role == "chords" || p.role == "arrangement")
                {
                    // Don't double up: easy and normal versions of the same thing.
                    return !my_roles.contains(&p.role.as_str())
                        && !(chords_mine && p.role == "chords")
                        && p.role != "arrangement"
                        && !p.is_easy();
                }
                if p.role == "arrangement" {
                    return false;
                }
                if my_roles.contains(&p.role.as_str()) && p.role != "other" {
                    return false;
                }
                if chords_mine && p.role == "chords" {
                    return false;
                }
                true
            })
            .collect();
    }

    /// The transpose of what you play (easy parts may be moved to an easier key).
    fn song_transpose(&self) -> i32 {
        let Some(song) = &self.song_ui.song else {
            return 0;
        };
        self.song_ui
            .play_parts
            .iter()
            .map(|&i| song.parts[i].transpose)
            .find(|t| *t != 0)
            .unwrap_or(0)
    }

    /// Build the audio data: every part (the mute mask picks what you hear)
    /// plus metronome clicks.
    fn song_send_data(&mut self) {
        let Some(song) = &self.song_ui.song else {
            return;
        };
        let mine_t = self.song_transpose();
        let mut events = Vec::new();
        let mut k = 0;
        let mut channels = 0u16;
        for (i, p) in song.parts.iter().enumerate() {
            let track = i.min(62) as u8;
            let ch = if p.is_drum {
                9
            } else {
                let c = SONG_CHANNELS[k % SONG_CHANNELS.len()];
                k += 1;
                channels |= 1 << c;
                self.send(Command::ChannelProgram {
                    channel: c,
                    program: p.program,
                });
                c
            };
            let shift = if p.is_drum { 0 } else { mine_t - p.transpose };
            for n in &p.notes {
                let note = (n.pitch as i32 + shift).clamp(0, 127) as u8;
                events.push(SongEvent {
                    time: n.start,
                    ch,
                    note,
                    vel: n.velocity.max(1),
                    track,
                });
                events.push(SongEvent {
                    time: n.end.max(n.start + 0.02),
                    ch,
                    note,
                    vel: 0,
                    track,
                });
            }
        }
        let g = &self.song_ui.grid;
        let mut b = (g.beat_at(0.0).floor() - 8.0).min(-8.0);
        let end = g.beat_at(song.duration) + 1.0;
        let bpb = g.beats_per_bar().max(1) as f64;
        let first_bar = g
            .bars_between(-1000.0, 1000.0)
            .first()
            .map(|x| x.0)
            .unwrap_or(0.0);
        while b <= end {
            let t = g.time_at(b);
            let down = ((b - first_bar).rem_euclid(bpb)).abs() < 1e-6;
            let note = if down { 76 } else { 77 };
            events.push(SongEvent {
                time: t,
                ch: 9,
                note,
                vel: if down { 110 } else { 80 },
                track: CLICK_TRACK,
            });
            events.push(SongEvent {
                time: t + 0.08,
                ch: 9,
                note,
                vel: 0,
                track: CLICK_TRACK,
            });
            b += 1.0;
        }
        // New loop layers keep off the song's channels (Part 4b note).
        self.send(Command::Loop(crate::audio::looper::LoopCommand::Reserve(
            channels,
        )));
        let data = Arc::new(SongData::new(events, song.duration + 0.3));
        if let Some(old) = self.song_ui.data.replace(data.clone()) {
            self.song_ui.retired.push(old);
        }
        self.song_send(SongCommand::Load(Some(data)));
        self.song_send(SongCommand::Speed(self.song_ui.speed));
        self.song_send(SongCommand::Gain(self.cfg.song.backing));
        self.song_ui.sent_wait = None;
        self.song_ui.sent_mute = u64::MAX;
        self.song_send_loop();
        self.song_update_mute();
    }

    /// After the audio engine restarts.
    pub(super) fn song_resend(&mut self) {
        if self.song_ui.song.is_some() {
            self.song_send_data();
        }
        self.jam_resend();
    }

    fn song_mute_mask(&self) -> u64 {
        let su = &self.song_ui;
        let mut m = 0u64;
        let jam = self.jam_active();
        for (i, h) in su.hear.iter().enumerate() {
            // Jam mode: the stems replace every MIDI part.
            if !h || jam {
                m |= 1u64 << i.min(62);
            }
        }
        if !(self.cfg.song.metronome || su.counting_in.is_some()) {
            m |= 1u64 << CLICK_TRACK;
        }
        m
    }

    fn song_update_mute(&mut self) {
        let m = self.song_mute_mask();
        if m != self.song_ui.sent_mute {
            self.song_ui.sent_mute = m;
            self.song_send(SongCommand::Mute(m));
        }
    }

    fn song_send_loop(&self) {
        self.song_send(SongCommand::Loop(self.song_ui.looping));
    }

    // ---------------------------------------------------- keys and targets

    /// Chord keys of the current keymap: (key, root, pitch classes).
    pub(super) fn song_chord_keys(&self) -> Vec<(KeyId, i32, Vec<i32>)> {
        let mut v: Vec<(KeyId, i32, Vec<i32>)> = self
            .keymap
            .bindings
            .iter()
            .filter_map(|(k, a)| match a {
                Action::Chord(c) => {
                    let ch = build_chord(*c, 0, self.tonic_pc(), self.cfg.scale);
                    let mut pcs: Vec<i32> = ch
                        .tones
                        .iter()
                        .map(|t| (ch.root_pc + t).rem_euclid(12))
                        .collect();
                    pcs.sort();
                    pcs.dedup();
                    Some((*k, ch.root_pc, pcs))
                }
                _ => None,
            })
            .collect();
        v.sort_by_key(|(k, ..)| crate::ui::widgets::key_position(*k).map(|(r, c)| (4 - r, c)));
        v
    }

    fn song_drum_keys(&self) -> Vec<(u8, KeyId)> {
        self.keymap
            .bindings
            .iter()
            .filter_map(|(k, a)| match a {
                Action::Drum(d) => Some((*d, *k)),
                _ => None,
            })
            .collect()
    }

    /// What decides the targets' keys: the keymap, octave, key, choices.
    fn song_signature(&self) -> String {
        let mut b: Vec<String> = self
            .keymap
            .bindings
            .iter()
            .map(|(k, a)| format!("{}={a}", k.name()))
            .collect();
        b.sort();
        format!(
            "{}|{}|{}|{:?}|{}|{:?}|{}|{}",
            b.join(","),
            self.base_note(),
            self.tonic_pc(),
            self.cfg.scale,
            self.cfg.song.any_octave,
            self.song_ui.play_parts,
            self.song_ui.play_chords,
            self.cfg.song.wait,
        )
    }

    fn chord_key_for(
        chord_keys: &[(KeyId, i32, Vec<i32>)],
        root: i32,
        pcs: &[i32],
    ) -> Option<(KeyId, bool)> {
        let mut want: Vec<i32> = pcs.to_vec();
        want.sort();
        if let Some((k, ..)) = chord_keys.iter().find(|(_, r, p)| *r == root && *p == want) {
            return Some((*k, true));
        }
        chord_keys
            .iter()
            .find(|(_, r, p)| *r == root && practice::chord_fits(pcs, p, true))
            .map(|(k, ..)| (*k, false))
    }

    /// Work out targets, their keys, and a fresh practice run.
    pub(super) fn song_rebuild(&mut self, pick_shift: bool) {
        let note_keys = self.note_keys();
        let chord_keys = self.song_chord_keys();
        let drum_keys = self.song_drum_keys();
        let any = self.cfg.song.any_octave;
        let settings = self.practice_settings();
        let sig = self.song_signature();
        let su = &mut self.song_ui;
        let Some(song) = &su.song else {
            su.practice = None;
            return;
        };
        let exact = |p: i32| note_keys.iter().any(|(n, _)| *n == p);
        let has_note =
            |p: i32| exact(p) || any && note_keys.iter().any(|(n, _)| (n - p).rem_euclid(12) == 0);
        if pick_shift {
            let pitches: Vec<i32> = su
                .play_parts
                .iter()
                .filter(|&&i| !song.parts[i].is_drum)
                .flat_map(|&i| song.parts[i].notes.iter().map(|n| n.pitch as i32))
                .collect();
            su.shift = practice::best_octave_shift(&pitches, exact);
        }
        let mut targets: Vec<Target> = Vec::new();
        for &i in &su.play_parts {
            let is_drum = song.parts[i].is_drum;
            targets.extend(practice::part_targets(song, i, su.shift, |p| {
                if is_drum {
                    drum_keys.iter().any(|(d, _)| *d as i32 == p)
                } else {
                    has_note(p)
                }
            }));
        }
        if su.play_chords {
            let part = su.play_parts.first().map(|&i| &song.parts[i]);
            targets.extend(practice::chord_targets(
                song.chords_for(part),
                |root, pcs| {
                    Self::chord_key_for(&chord_keys, root, pcs).is_some()
                        || pcs
                            .iter()
                            .all(|pc| note_keys.iter().any(|(n, _)| n.rem_euclid(12) == *pc))
                },
            ));
        }
        let practice = Practice::new(targets, settings);
        su.labels = practice
            .targets
            .iter()
            .map(|t| match &t.want {
                Want::Note(p) => {
                    let (keys, exact) = Self::keys_for_note(&note_keys, *p);
                    Self::pick_key(&keys, None).map(|k| (k, exact))
                }
                Want::Chord { root_pc, pcs, .. } => Self::chord_key_for(&chord_keys, *root_pc, pcs),
                Want::Drum(d) => drum_keys
                    .iter()
                    .find(|(x, _)| x == d)
                    .map(|(_, k)| (*k, true)),
            })
            .collect();
        su.practice = Some(practice);
        su.signature = sig;
        su.result = None;
        su.passes.clear();
    }

    /// Make (or remove) the layout made for this song.
    pub(super) fn song_apply_layout(&mut self) {
        let Some(song) = &self.song_ui.song else {
            return;
        };
        let part = self.song_ui.play_parts.first().map(|&i| &song.parts[i]);
        let chords = if self.song_ui.play_chords {
            song.chords_for(part).to_vec()
        } else {
            vec![]
        };
        let mut pitches = Vec::new();
        let mut drums = Vec::new();
        for &i in &self.song_ui.play_parts {
            let p = &song.parts[i];
            if p.is_drum {
                drums.extend(p.notes.iter().map(|n| n.pitch));
            } else {
                pitches.extend(p.notes.iter().map(|n| n.pitch as i32));
            }
        }
        let l = layout::build(
            song.title(),
            &chords,
            song.key_for(part),
            &pitches,
            &drums,
            self.base_note(),
        );
        if let Err(e) = config::save_keymap("song", &l.keymap) {
            self.notify(format!("Couldn't save the song layout: {e}"));
            return;
        }
        if self.keymap_file != "song" {
            self.song_ui.prev_keymap = Some(self.keymap_file.clone());
        }
        let summary = l.summary();
        self.song_ui.layout = Some(l);
        self.switch_keymap("song");
        self.keymap_watch.sync();
        self.song_rebuild(true);
        self.notify(format!("Song layout: {summary}"));
    }

    fn song_remove_layout(&mut self) {
        self.song_ui.layout = None;
        if self.keymap_file == "song" {
            let back = self
                .song_ui
                .prev_keymap
                .take()
                .unwrap_or_else(|| "piano".into());
            self.switch_keymap(&back);
        }
        self.song_rebuild(true);
    }

    // ------------------------------------------------------------ transport

    fn song_playing(&self) -> bool {
        self.song_ui.clock.playing
    }

    pub(super) fn song_play_pause(&mut self) {
        if self.song_ui.song.is_none() {
            return;
        }
        if self.song_playing() {
            self.song_send(SongCommand::Play(false));
            self.song_ui.counting_in = None;
            self.song_update_mute();
            return;
        }
        self.song_ui.result = None;
        self.stop_rhythm_for_song();
        let mut start = self.song_ui.clock.pos;
        if self.song_ui.clock.ended {
            // A new run from the top.
            start = self
                .song_ui
                .looping
                .map(|l| l.0)
                .unwrap_or(0.0)
                .min(self.song_first_note());
            if let Some(p) = &mut self.song_ui.practice {
                p.reset();
            }
            self.song_ui.passes.clear();
            self.song_ui.sent_wait = Some(f64::NAN);
        }
        if self.cfg.song.count_in {
            let g = &self.song_ui.grid;
            let from = g.time_at(g.beat_at(start) - g.beats_per_bar() as f64);
            self.song_ui.counting_in = Some(start);
            self.song_update_mute();
            self.song_send(SongCommand::Seek(from));
        } else if self.song_ui.clock.ended {
            self.song_send(SongCommand::Seek(start));
        }
        self.song_send(SongCommand::Play(true));
    }

    fn song_first_note(&self) -> f64 {
        self.song_ui
            .practice
            .as_ref()
            .and_then(|p| p.targets.first().map(|t| t.start))
            .unwrap_or(0.0)
            .min(0.0)
    }

    /// Jump to a time; practice results from there on are forgotten.
    pub(super) fn song_seek(&mut self, t: f64) {
        let t = t.clamp(
            -4.0,
            self.song_ui
                .song
                .as_ref()
                .map(|s| s.duration)
                .unwrap_or(0.0),
        );
        self.song_ui.counting_in = None;
        self.song_update_mute();
        self.song_send(SongCommand::Seek(t));
        self.song_restart_practice(t);
        self.song_ui.heard = t;
        self.song_ui.clock.pos = t;
    }

    fn song_restart_practice(&mut self, t: f64) {
        let lp = self.song_ui.looping;
        if let Some(p) = &mut self.song_ui.practice {
            p.restart_from(t, lp.map(|l| l.1).filter(|b| t < *b));
        }
        self.song_ui.sent_wait = Some(f64::NAN);
    }

    /// Back to the loop start (or the beginning) and play.
    pub(super) fn song_restart(&mut self) {
        let start = self
            .song_ui
            .looping
            .map(|l| l.0)
            .unwrap_or(self.song_first_note());
        let was = self.song_playing();
        self.song_send(SongCommand::Play(false));
        self.song_seek(start);
        self.song_ui.clock.playing = false;
        self.song_ui.clock.ended = false;
        if was || self.cfg.song.count_in {
            self.song_play_pause();
        }
    }

    pub(super) fn song_set_speed(&mut self, s: f64) {
        let s = (s * 20.0).round() / 20.0;
        self.song_ui.speed = s.clamp(0.25, 1.5);
        self.song_send(SongCommand::Speed(self.song_ui.speed));
    }

    fn song_set_loop(&mut self, l: Option<(f64, f64)>) {
        self.song_ui.looping = l.filter(|(a, b)| b - a > 0.2);
        self.song_send_loop();
        self.song_ui.passes.clear();
        if let Some((a, b)) = self.song_ui.looping {
            let h = self.song_ui.clock.pos;
            if h < a || h >= b {
                self.song_seek(a);
            } else {
                self.song_restart_practice(a);
            }
        }
    }

    /// Bar start at or before a time, and the bar end after another.
    fn bar_span(&self, t0: f64, t1: f64) -> (f64, f64) {
        let g = &self.song_ui.grid;
        let (b0, b1) = (g.beat_at(t0), g.beat_at(t1));
        let bars = g.bars_between(b0 - 16.0, b1 + 16.0);
        let start = bars
            .iter()
            .rev()
            .find(|(b, _)| *b <= b0 + 0.01)
            .map(|x| x.0)
            .unwrap_or(b0.floor());
        let end = bars
            .iter()
            .find(|(b, _)| *b >= b1 - 0.01 && *b > start)
            .map(|x| x.0)
            .unwrap_or(b1.ceil());
        (g.time_at(start), g.time_at(end))
    }

    fn stop_rhythm_for_song(&mut self) {
        if self.rhythm_running() {
            self.send(Command::Rhythm(false));
        }
    }

    /// Chords from chord keys play as plain block chords in Song mode (a
    /// running arpeggio or pattern would fight the song's own rhythm).
    fn song_style_override(&mut self, on: bool) {
        if on == self.song_ui.style_overridden {
            return;
        }
        self.song_ui.style_overridden = on;
        if on {
            self.send(Command::Rhythm(false));
        }
        self.send_style();
    }

    pub(super) fn song_stop_sounds(&mut self) {
        for (_, list) in std::mem::take(&mut self.song_ui.onekey) {
            for (ch, note) in list {
                self.send(Command::ChannelNote {
                    channel: ch,
                    note,
                    velocity: 0,
                });
            }
        }
    }

    // ------------------------------------------------------------ polling

    /// Every frame: clock, scoring, wait mode, analyzer progress.
    pub(super) fn song_poll(&mut self, ctx: &egui::Context) -> bool {
        let mut busy = false;
        // Entering / leaving the tab.
        let on_tab = self.tab == Tab::Song;
        if on_tab != self.song_ui.was_tab {
            self.song_ui.was_tab = on_tab;
            if on_tab {
                if !self.song_ui.scanned {
                    self.song_rescan();
                }
            } else if self.song_playing() {
                self.song_send(SongCommand::Play(false));
            }
        }
        self.song_style_override(on_tab && self.song_ui.song.is_some());

        // Analyzer.
        if let Some(job) = &mut self.song_ui.job {
            busy = true;
            if job.poll() {
                ctx.request_repaint();
            }
            if let Some(res) = job.result.clone() {
                let log = job.log.clone();
                self.song_ui.job = None;
                match res {
                    Ok(dir) => {
                        self.song_rescan();
                        self.notify("Analysis done. Opening the song…");
                        self.open_song(&dir);
                    }
                    Err(e) => self.song_ui.job_error = Some((e, log)),
                }
            }
        }
        // File picker.
        if let Some((what, rx)) = &self.song_ui.browse
            && let Ok(res) = rx.try_recv()
        {
            let what = *what;
            self.song_ui.browse = None;
            if let Some(p) = res {
                match what {
                    BrowseFor::Open => self.open_song(&p),
                    BrowseFor::Analyze => self.song_ui.analyze_path = p.display().to_string(),
                }
            }
        }
        // Files dropped on the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            if library::is_midi(&p)
                || p.join("song.json").exists()
                || p.file_name().is_some_and(|n| n == "song.json")
            {
                self.tab = Tab::Song;
                self.open_song(&p);
            } else if library::is_audio(&p) {
                self.tab = Tab::Song;
                self.song_ui.view = SongView::Library;
                self.song_ui.analyze_path = p.display().to_string();
                self.notify("Press Analyze to turn this recording into a song you can play.");
            }
        }
        // Audition notes from the editor.
        let now = Instant::now();
        let due: Vec<(Instant, u8, u8)> = self
            .song_ui
            .pending_offs
            .iter()
            .copied()
            .filter(|x| x.0 <= now)
            .collect();
        self.song_ui.pending_offs.retain(|x| x.0 > now);
        for (_, ch, note) in due {
            self.send(Command::ChannelNote {
                channel: ch,
                note,
                velocity: 0,
            });
        }
        if !self.song_ui.pending_offs.is_empty() {
            busy = true;
        }

        busy |= self.jam_poll(ctx);
        if self.song_ui.song.is_none() {
            return busy;
        }
        // Keys changed (layout, octave, key…): new targets.
        if self.song_signature() != self.song_ui.signature {
            self.song_rebuild(false);
        }
        self.song_clock_tick();
        busy || self.song_ui.clock.playing
    }

    fn song_clock_tick(&mut self) {
        let Some(engine) = &self.engine else { return };
        let c = *engine.stats.song_clock.lock().unwrap();
        let now = Instant::now();
        let since = match now.checked_duration_since(c.at) {
            Some(d) => d.as_secs_f64(),
            None => -(c.at - now).as_secs_f64(),
        };
        let delay = self.visual_delay_ms() as f64 / 1000.0;
        let mut heard = practice::heard_time(
            ClockSnapshot {
                pos: c.pos,
                since,
                speed: c.speed,
                moving: c.moving,
            },
            delay,
        );
        let su = &mut self.song_ui;
        let jumped = c.seeks != su.seeks_seen || c.loops != su.loops_seen;
        if !jumped && c.playing {
            // Only forward: after a pause the first sound is still on its way.
            heard = heard.max(su.heard);
        }
        if !c.playing && !c.moving && c.seeks == su.seeks_seen {
            heard = heard.min(c.pos).max(su.heard.min(c.pos));
        }
        let new_loop = c.loops != su.loops_seen;
        su.seeks_seen = c.seeks;
        su.loops_seen = c.loops;
        su.clock = c;
        su.heard = heard;
        // Count-in over: back to the normal clicks.
        if let Some(start) = su.counting_in
            && c.pos >= start - 0.05
        {
            su.counting_in = None;
        }
        self.song_update_mute();

        if new_loop {
            self.song_loop_pass();
        }
        let speed = self.song_ui.speed;
        let wait = self.cfg.song.wait;
        if let Some(p) = &mut self.song_ui.practice {
            if c.playing && !wait {
                let missed = p.update(heard, speed);
                if !missed.is_empty() {
                    self.song_ui.flash = Some((false, Instant::now()));
                }
            }
            // Wait mode: stop the audio at the next notes.
            let wp = p.wait_point();
            let same = match (wp, self.song_ui.sent_wait) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                (None, None) => true,
                _ => false,
            };
            if !same {
                self.song_ui.sent_wait = wp;
                self.send(Command::Song(SongCommand::Wait(wp)));
            }
        }
        if c.ended && !self.song_ui.ended_seen {
            self.song_finished();
        }
        self.song_ui.ended_seen = c.ended;
    }

    /// The loop came round: score the pass, maybe speed up.
    fn song_loop_pass(&mut self) {
        let Some((a, b)) = self.song_ui.looping else {
            return;
        };
        let Some(p) = &mut self.song_ui.practice else {
            return;
        };
        p.update(b + 10.0, self.song_ui.speed);
        let pass = p.restart_from(a, Some(b));
        self.song_ui.sent_wait = Some(f64::NAN);
        if pass.judged() == 0 {
            return;
        }
        let acc = pass.accuracy();
        self.song_ui.passes.push(acc);
        if self.song_ui.passes.len() > 12 {
            self.song_ui.passes.remove(0);
        }
        if self.cfg.song.auto_speed && acc >= 90.0 && self.song_ui.speed < 1.0 {
            let s = (self.song_ui.speed + 0.05).min(1.0);
            self.song_set_speed(s);
            self.notify(format!("{acc:.0}% — nice! Speed up to {:.0}%", s * 100.0));
        } else {
            self.notify(format!("Loop pass: {acc:.0}% ({})", pass.grade()));
        }
    }

    fn song_finished(&mut self) {
        let id = self
            .song_ui
            .song
            .as_ref()
            .map(|s| s.id())
            .unwrap_or_default();
        let Some(p) = &mut self.song_ui.practice else {
            return;
        };
        p.update(f64::MAX / 4.0, 1.0);
        let score = p.score;
        if score.judged() == 0 {
            return;
        }
        let acc = score.accuracy().round() as u32;
        let best = self.cfg.song.best.get(&id).copied().unwrap_or(0);
        let new_best = acc > best && self.song_ui.speed >= 0.999 && !self.cfg.song.one_key;
        if new_best {
            self.cfg.song.best.insert(id, acc);
            self.mark_dirty();
        }
        self.song_ui.result = Some((score, new_best));
    }

    // ------------------------------------------------------------ input

    /// A note / chord / drum you played (from `play_key`).
    pub(super) fn song_event(&mut self, played: Played) {
        if self.tab != Tab::Song || self.song_ui.view != SongView::Practice {
            return;
        }
        let playing = self.song_playing();
        let halted = self.halted();
        let held = self.held_pcs();
        let heard = self.song_heard();
        let speed = self.song_ui.speed;
        let Some(p) = &mut self.song_ui.practice else {
            return;
        };
        if !playing {
            return;
        }
        match p.press(&played, heard, speed, &held, halted) {
            Outcome::Hit(h) => {
                self.song_ui.flash = Some((true, Instant::now()));
                if let Some((_, hit)) = h.first() {
                    self.song_ui.last_hit = Some((*hit, Instant::now()));
                }
            }
            Outcome::Wrong => self.song_ui.flash = Some((false, Instant::now())),
            Outcome::Partial => {}
        }
    }

    /// The chord a chord key just played, for scoring.
    pub(super) fn chord_played(&self) -> Option<Played> {
        let ch = self.chord.as_ref()?;
        let mut pcs: Vec<i32> = ch
            .tones
            .iter()
            .map(|t| (ch.root_pc + t).rem_euclid(12))
            .collect();
        pcs.sort();
        pcs.dedup();
        Some(Played::Chord {
            root_pc: ch.root_pc,
            pcs,
        })
    }

    fn held_pcs(&self) -> Vec<i32> {
        let mut v: Vec<i32> = self.sounding.values().map(|n| n.rem_euclid(12)).collect();
        if self.chord.is_some() && !self.chord_keys.is_empty() {
            v.extend(self.voicing.iter().map(|n| n.rem_euclid(12)));
        }
        v.sort();
        v.dedup();
        v
    }

    /// Keys on the Song tab before they reach the instrument. Returns true
    /// if used.
    pub(super) fn song_key(
        &mut self,
        k: KeyId,
        pressed: bool,
        repeat: bool,
        mods: egui::Modifiers,
    ) -> bool {
        if self.tab != Tab::Song {
            return false;
        }
        // One-key mode: releasing a key stops what it started.
        if !pressed {
            if let Some(list) = self.song_ui.onekey.remove(&k) {
                for (ch, note) in list {
                    self.send(Command::ChannelNote {
                        channel: ch,
                        note,
                        velocity: 0,
                    });
                }
                return true;
            }
            return false;
        }
        let free = |a: Option<Action>| a.is_none();
        let bound = self.keymap.get(k);
        // Shortcuts (only when the layout doesn't use the key).
        if let KeyId::Key(key) = k
            && free(bound)
        {
            let used = match key {
                Key::F5 => {
                    if !repeat {
                        if self.song_ui.view == SongView::Edit {
                            self.editor_play_toggle();
                        } else {
                            self.song_play_pause();
                        }
                    }
                    true
                }
                Key::F6 => {
                    if !repeat {
                        self.song_restart();
                    }
                    true
                }
                Key::F7 => {
                    self.song_set_speed(self.song_ui.speed - 0.05);
                    self.notify(format!("Speed {:.0}%", self.song_ui.speed * 100.0));
                    true
                }
                Key::F8 => {
                    self.song_set_speed(self.song_ui.speed + 0.05);
                    self.notify(format!("Speed {:.0}%", self.song_ui.speed * 100.0));
                    true
                }
                Key::F9 => {
                    if !repeat {
                        self.cfg.song.wait = !self.cfg.song.wait;
                        self.mark_dirty();
                        self.notify(if self.cfg.song.wait {
                            "Wait mode on"
                        } else {
                            "Wait mode off"
                        });
                    }
                    true
                }
                _ => false,
            };
            if used {
                return true;
            }
        }
        if self.song_ui.view == SongView::Edit {
            return self.editor_key(k, repeat, mods);
        }
        // One-key mode: any playing key (not controls) plays the next notes.
        let control = bound.is_some_and(|a| a.kind() == crate::keymap::ActionKind::Control);
        let modifier = matches!(k, KeyId::Shift | KeyId::Ctrl | KeyId::Alt);
        let fkey =
            matches!(k, KeyId::Key(key) if key.name().starts_with('F') && key.name().len() <= 3);
        if self.cfg.song.one_key
            && self.song_ui.view == SongView::Practice
            && self.song_ui.song.is_some()
            && !control
            && !modifier
            && !fkey
            && !matches!(k, KeyId::Key(Key::Escape | Key::Tab | Key::Enter))
        {
            if !repeat && !self.song_ui.onekey.contains_key(&k) {
                self.onekey_press(k);
            }
            return true;
        }
        false
    }

    fn onekey_press(&mut self, k: KeyId) {
        if !self.song_playing() {
            self.song_play_pause();
            return;
        }
        let heard = self.song_heard();
        let speed = self.song_ui.speed;
        let halted = self.halted();
        let center = self.chord_center();
        let bass_base = BASE_NOTE - 12 + 12 * self.cfg.octave;
        let with_bass = self.cfg.chord_bass;
        let Some(p) = &mut self.song_ui.practice else {
            return;
        };
        let Some(g) = p.next_group(heard, speed) else {
            return;
        };
        let idx = p.groups[g].targets.clone();
        let vel_song = idx
            .iter()
            .map(|&i| p.targets[i].velocity as u32)
            .max()
            .unwrap_or(96);
        let hits = p.hit_group(g, heard, speed, halted);
        let mut notes: Vec<(u8, u8)> = Vec::new();
        for i in idx {
            match &p.targets[i].want {
                Want::Note(n) => notes.push((0, (*n).clamp(0, 127) as u8)),
                Want::Drum(d) => notes.push((9, *d)),
                Want::Chord { root_pc, pcs, .. } => {
                    for pc in pcs {
                        let mut n = center - 7 + (pc - (center - 7)).rem_euclid(12);
                        if n > center + 6 {
                            n -= 12;
                        }
                        notes.push((1, n.clamp(0, 127) as u8));
                    }
                    if with_bass {
                        notes.push((2, (bass_base + root_pc).clamp(0, 127) as u8));
                    }
                }
            }
        }
        // The song's own dynamics, scaled by your velocity setting.
        let vel = (self.velocity() as f32 * vel_song as f32 / 96.0).clamp(20.0, 127.0) as u8;
        for (ch, n) in &notes {
            self.send(Command::ChannelNote {
                channel: *ch,
                note: *n,
                velocity: vel,
            });
        }
        self.song_ui.onekey.insert(k, notes);
        if let Some((_, hit)) = hits.first() {
            self.song_ui.last_hit = Some((*hit, Instant::now()));
            self.song_ui.flash = Some((true, Instant::now()));
        }
    }

    /// Keys to light: what's due now and what comes next.
    fn song_hints(&self) -> HashMap<KeyId, Hint> {
        let mut h = HashMap::new();
        let su = &self.song_ui;
        let Some(p) = &su.practice else { return h };
        let beat_len = su.grid.beat_len(su.heard);
        let up = p.upcoming(su.heard - 0.3, beat_len * 3.0);
        for (i, dt) in up {
            let Some((k, _)) = su.labels.get(i).copied().flatten() else {
                continue;
            };
            let dt = dt - 0.3;
            let kind = if dt <= 0.2 || (self.cfg.song.wait && self.halted() && dt <= 0.35) {
                HintKind::Now
            } else {
                HintKind::Next
            };
            let tag = match &p.targets[i].want {
                Want::Chord { symbol, .. } => symbol.clone(),
                Want::Note(n) => note_name(*n),
                Want::Drum(d) => crate::drums::short(*d),
            };
            let e = h.entry(k).or_insert(Hint { kind, tag });
            if kind == HintKind::Now {
                e.kind = HintKind::Now;
            }
        }
        h
    }

    // ------------------------------------------------------------ the tab

    pub(super) fn song_tab(&mut self, ui: &mut egui::Ui) {
        if self.engine.is_none() {
            let p = self.palette;
            ui.label(
                RichText::new("Audio is off, so songs can't play. Check the Audio tab.")
                    .color(p.error),
            );
        }
        match self.song_ui.view {
            SongView::Library => self.song_library(ui),
            SongView::Practice if self.song_ui.song.is_some() => self.song_practice(ui),
            SongView::Edit if self.song_ui.song.is_some() => self.song_editor(ui),
            _ => {
                self.song_ui.view = SongView::Library;
                self.song_library(ui)
            }
        }
    }

    fn browse(&mut self, what: BrowseFor) {
        let (tx, rx) = unbounded();
        let filter = match what {
            BrowseFor::Open => "Songs | *.mid *.midi *.kar song.json",
            BrowseFor::Analyze => {
                "Audio | *.mp3 *.flac *.wav *.m4a *.ogg *.opus *.aac *.mid *.midi"
            }
        };
        let start = dirs::audio_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let filter = filter.to_string();
        std::thread::spawn(move || {
            let try_run = |cmd: &str, args: &[&str]| -> Option<Option<PathBuf>> {
                let out = std::process::Command::new(cmd).args(args).output().ok()?;
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                Some((out.status.success() && !s.is_empty()).then(|| PathBuf::from(s)))
            };
            let start_s = format!("{}/", start.display());
            let res = try_run(
                "zenity",
                &[
                    "--file-selection",
                    "--title=Choose a song",
                    &format!("--file-filter={filter}"),
                    "--file-filter=All files | *",
                    &format!("--filename={start_s}"),
                ],
            )
            .or_else(|| try_run("kdialog", &["--getopenfilename", &start_s]))
            .or_else(|| try_run("yad", &["--file", "--title=Choose a song"]));
            let _ = tx.send(res.flatten());
        });
        self.song_ui.browse = Some((what, rx));
    }

    fn song_library(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(song) = &self.song_ui.song {
                let name = song.display_name();
                ui.horizontal(|ui| {
                    if ui.button(format!("▶  Back to {name}")).clicked() {
                        self.song_ui.view = SongView::Practice;
                    }
                });
                ui.add_space(6.0);
            }
            ui.columns(2, |cols| {
                card(&mut cols[0], &p, "Your songs", |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(crate::song::songs_dir().display().to_string()).small().color(p.outline));
                        if ui.small_button("Refresh").clicked() {
                            self.song_rescan();
                        }
                        if ui.small_button("Open folder").clicked() {
                            let d = crate::song::songs_dir();
                            let _ = std::fs::create_dir_all(&d);
                            let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                        }
                    });
                    if self.song_ui.library.is_empty() {
                        ui.label(
                            RichText::new(
                                "No analysed songs yet. Analyse a recording on the right, or open a MIDI file below.",
                            )
                            .color(p.on_surface_variant),
                        );
                    }
                    let mut open = None;
                    let mut edit = None;
                    for e in self.song_ui.library.clone() {
                        egui::Frame::new()
                            .fill(p.surface_container_high)
                            .corner_radius(egui::CornerRadius::same(12))
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        let mut t = RichText::new(&e.title).strong().size(15.0);
                                        if e.error.is_some() {
                                            t = t.color(p.outline);
                                        }
                                        ui.label(t);
                                        let mut sub = Vec::new();
                                        if let Some(a) = &e.artist {
                                            sub.push(a.clone());
                                        }
                                        if e.duration > 0.0 {
                                            sub.push(fmt_time(e.duration));
                                        }
                                        if !e.mode.is_empty() {
                                            sub.push(e.mode.clone());
                                        }
                                        if !e.parts.is_empty() {
                                            sub.push(format!("{} parts", e.parts.len()));
                                        }
                                        if e.has_stems {
                                            sub.push("stems".into());
                                        }
                                        if e.low_confidence > 0 {
                                            sub.push(format!("{} unsure notes", e.low_confidence));
                                        }
                                        if let Some(err) = &e.error {
                                            sub.push(err.clone());
                                        }
                                        ui.label(RichText::new(sub.join(" · ")).small().color(p.on_surface_variant));
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if e.error.is_none() {
                                            if ui.button("Practise").clicked() {
                                                open = Some(e.path.clone());
                                            }
                                            if ui.small_button("Edit notes").clicked() {
                                                edit = Some(e.path.clone());
                                            }
                                        }
                                    });
                                });
                            });
                        ui.add_space(4.0);
                    }
                    if let Some(path) = open {
                        self.open_song(&path);
                    }
                    if let Some(path) = edit {
                        self.open_song(&path);
                        self.editor_open(None);
                    }
                });
                cols[0].add_space(8.0);
                card(&mut cols[0], &p, "MIDI files", |ui| {
                    ui.label(
                        RichText::new("Any .mid file works: notes, chords (guessed) and the beat grid come from the file.")
                            .color(p.on_surface_variant),
                    );
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.song_ui.open_path).hint_text("/path/to/song.mid").desired_width(260.0));
                        if ui.button("Open").clicked() {
                            let path = PathBuf::from(self.song_ui.open_path.trim());
                            self.open_song(&path);
                        }
                        if ui.add_enabled(self.song_ui.browse.is_none(), egui::Button::new("Browse…")).clicked() {
                            self.browse(BrowseFor::Open);
                        }
                    });
                    ui.label(RichText::new("Tip: you can also drag a file onto this window.").small().color(p.outline));
                    let mut open = None;
                    for e in self.song_ui.midi_files.iter().take(30) {
                        let folder = e.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
                        if ui
                            .link(format!("♪ {}", e.title))
                            .on_hover_text(folder)
                            .clicked()
                        {
                            open = Some(e.path.clone());
                        }
                    }
                    if let Some(path) = open {
                        self.open_song(&path);
                    }
                    let recent: Vec<PathBuf> = self.cfg.song.recent.iter().filter(|r| library::is_midi(r)).take(5).cloned().collect();
                    if !recent.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Opened lately").small().color(p.on_surface_variant));
                        for r in recent {
                            if ui.link(r.file_name().unwrap_or_default().to_string_lossy()).clicked() {
                                self.open_song(&r);
                            }
                        }
                    }
                });

                card(&mut cols[1], &p, "Analyse a recording", |ui| self.analyze_card(ui));
            });
            if let Some(e) = &self.song_ui.load_error {
                ui.label(RichText::new(format!("⚠ {e}")).color(p.error));
            }
        });
    }

    fn analyze_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.label(
            "Pick an mp3, flac, wav… The analyzer finds the beat, key, chords and melody, and makes a song you can \
             practise here. It runs on your computer; the first time takes a few minutes.",
        );
        let Some(program) = self.song_ui.analyzer.clone() else {
            ui.add_space(4.0);
            ui.label(RichText::new("The analyzer isn't installed yet.").color(p.tertiary));
            ui.label("Install it once from the app's folder:");
            ui.code("cd analyzer && ./setup.sh        # add --gpu for an NVIDIA card");
            if ui.button("Look again").clicked() {
                self.song_ui.analyzer = analyze::find_analyzer();
            }
            return;
        };
        if let Some(job) = &self.song_ui.job {
            let name = job
                .file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let mode = analyze::MODES
                .iter()
                .find(|m| m.0 == job.opts.mode)
                .map(|m| m.1)
                .unwrap_or("");
            ui.label(RichText::new(format!("Analysing {name}")).strong());
            ui.label(RichText::new(mode).small().color(p.on_surface_variant));
            let frac = job.fraction();
            ui.add(egui::ProgressBar::new(frac).show_percentage().animate(true));
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    job.message,
                    fmt_time(job.started.elapsed().as_secs_f64())
                ))
                .color(p.on_surface_variant),
            );
            if job.stage.as_deref() == Some("stems") {
                ui.label(RichText::new("Separating the instruments is the slow part (minutes on a CPU, done once per song).").small().color(p.outline));
            }
            ui.ctx().request_repaint_after(Duration::from_millis(200));
            if ui.button("Cancel").clicked()
                && let Some(j) = &mut self.song_ui.job
            {
                j.cancel();
            }
            return;
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.song_ui.analyze_path)
                    .hint_text("/path/to/song.mp3")
                    .desired_width(260.0),
            );
            if ui
                .add_enabled(self.song_ui.browse.is_none(), egui::Button::new("Browse…"))
                .clicked()
            {
                self.browse(BrowseFor::Analyze);
            }
        });
        let o = &mut self.song_ui.opts;
        egui::Grid::new("an-opts").num_columns(2).show(ui, |ui| {
            ui.label("What to get");
            egui::ComboBox::from_id_salt("an-mode")
                .width(240.0)
                .selected_text(
                    analyze::MODES
                        .iter()
                        .find(|m| m.0 == o.mode)
                        .map(|m| m.1)
                        .unwrap_or("?"),
                )
                .show_ui(ui, |ui| {
                    for (m, label) in analyze::MODES {
                        ui.selectable_value(&mut o.mode, m.to_string(), label);
                    }
                });
            ui.end_row();
            ui.label("Easy version");
            ui.horizontal(|ui| {
                let mut on = o.simplify.is_some() || o.mode == "simplified";
                ui.add_enabled_ui(o.mode != "simplified", |ui| ui.checkbox(&mut on, ""));
                let mut d = o.simplify.unwrap_or(2);
                ui.add_enabled(on, egui::Slider::new(&mut d, 1..=5).text("difficulty"));
                o.simplify = on.then_some(d);
            });
            ui.end_row();
            ui.label("Quick");
            ui.checkbox(
                &mut o.no_separate,
                "skip separating instruments (fast, less accurate)",
            );
            ui.end_row();
        });
        let path = PathBuf::from(self.song_ui.analyze_path.trim());
        let ok = path.is_file();
        if ui
            .add_enabled(ok, egui::Button::new(RichText::new("✨  Analyze").strong()))
            .on_disabled_hover_text("Pick a file first")
            .clicked()
        {
            self.song_ui.job_error = None;
            match Job::start(&program, &path, self.song_ui.opts.clone()) {
                Ok(j) => self.song_ui.job = Some(j),
                Err(e) => self.song_ui.job_error = Some((e, vec![])),
            }
        }
        ui.label(
            RichText::new(format!("Runs {}", program.display()))
                .small()
                .color(p.outline),
        );
        if let Some((e, log)) = &self.song_ui.job_error {
            ui.label(RichText::new(format!("⚠ {e}")).color(p.error));
            if !log.is_empty() {
                egui::CollapsingHeader::new("Details")
                    .id_salt("an-log")
                    .show(ui, |ui| {
                        for l in log.iter().rev().take(12).rev() {
                            ui.label(RichText::new(l).small().monospace());
                        }
                    });
            }
        }
    }

    // ------------------------------------------------------------ practice

    fn song_practice(&mut self, ui: &mut egui::Ui) {
        // Room for the drawn keyboard (5 rows of 1/15 of the width) + help.
        let kb_h = ui.available_width().min(1100.0) / 15.0 * 5.0 + 70.0;
        let total_h = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("song-practice")
            .auto_shrink([false, false])
            .show(ui, |ui| self.song_practice_inner(ui, kb_h, total_h));
    }

    fn song_practice_inner(&mut self, ui: &mut egui::Ui, kb_h: f32, total_h: f32) {
        let p = self.palette;
        let top_y = ui.cursor().min.y;
        self.song_top_bar(ui);
        ui.add_space(4.0);
        self.song_transport(ui);
        self.song_jam(ui);
        ui.add_space(4.0);
        // Timeline.
        let (dur, sections, marks) = {
            let song = self.song_ui.song.as_ref().unwrap();
            let secs: Vec<(f64, f64, String)> = song
                .sections
                .iter()
                .map(|s| (s.start, s.end, s.title()))
                .collect();
            let mut marks = Vec::new();
            if let Some(pr) = &self.song_ui.practice {
                for (t, r) in pr.targets.iter().zip(&pr.results) {
                    if let Some(h) = r {
                        let c = match h.judgement {
                            Judgement::Miss => p.error,
                            Judgement::Good => p.on_secondary_container,
                            Judgement::Perfect => p.primary,
                        };
                        marks.push((t.start, c));
                    }
                }
            }
            (song.duration, secs, marks)
        };
        let tl = song_view::timeline(
            ui,
            &p,
            dur,
            self.song_ui.heard,
            &sections,
            self.song_ui.looping,
            &marks,
        );
        if let Some(t) = tl.seek {
            self.song_seek(t);
        }
        if let Some((a, b)) = tl.selected {
            let (a, b) = self.bar_span(a, b);
            self.song_set_loop(Some((a, b)));
        }
        ui.add_space(6.0);

        let used = ui.cursor().min.y - top_y;
        let fall_h = (total_h - used - kb_h).clamp(240.0, 560.0);
        let width = ui.available_width();
        let right_w = (width * 0.3).clamp(240.0, 380.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(width - right_w - 12.0);
                self.song_falling(ui, fall_h);
            });
            ui.vertical(|ui| {
                ui.set_width(right_w);
                self.song_score_card(ui);
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(p.surface_container)
                    .corner_radius(egui::CornerRadius::same(16))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Chord sheet").strong().color(p.primary));
                        egui::ScrollArea::vertical()
                            .id_salt("sheet")
                            .max_height((fall_h - 150.0).max(120.0))
                            .show(ui, |ui| self.song_sheet(ui));
                    });
            });
        });
        ui.add_space(6.0);
        let hints = self.song_hints();
        let label = self.cap_label();
        KeyboardView {
            keymap: &self.keymap,
            palette: &p,
            held: &self.held,
            selected: None,
            show_notes: true,
            label: &label,
            hints: &hints,
        }
        .show(ui);
        drop(label);
        self.song_help(ui);
    }

    fn song_top_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut rebuild = false;
        let mut relayout = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button("← Songs").clicked() {
                self.song_ui.view = SongView::Library;
                if self.song_playing() {
                    self.song_play_pause();
                }
            }
            let song = self.song_ui.song.as_ref().unwrap();
            ui.label(RichText::new(song.display_name()).size(19.0).strong().color(p.primary));
            let part = self.song_ui.play_parts.first().map(|&i| &song.parts[i]);
            if let Some(k) = song.key_for(part) {
                chip(ui, p.surface_container_high, p.on_surface, &k.name);
            }
            chip(ui, p.surface_container_high, p.on_surface, format!("{:.0} BPM", song.tempo.bpm * self.song_ui.speed));
            if let Some(best) = self.cfg.song.best.get(&song.id()) {
                chip(ui, p.primary_container, p.on_primary_container, format!("Best {best}%"));
            }
            ui.add_space(8.0);
            ui.label(RichText::new("I play").color(p.on_surface_variant));
            let names: Vec<String> = song.parts.iter().map(|x| x.label()).collect();
            let summary = {
                let mut v: Vec<String> = self.song_ui.play_parts.iter().map(|&i| names[i].clone()).collect();
                if self.song_ui.play_chords {
                    v.push("Chords".into());
                }
                if v.is_empty() { "nothing (listen)".to_string() } else { v.join(" + ") }
            };
            let has_chords = !song.chords.iter().all(|c| c.is_none());
            let parts_info: Vec<(usize, String, usize, bool)> = song
                .parts
                .iter()
                .enumerate()
                .map(|(i, x)| (i, x.label(), x.notes.len(), x.transpose != 0))
                .collect();
            egui::ComboBox::from_id_salt("i-play")
                .width(220.0)
                .selected_text(summary)
                .show_ui(ui, |ui| {
                    for (i, name, n, transposed) in parts_info {
                        let mut on = self.song_ui.play_parts.contains(&i);
                        let label = format!("{name} ({n} notes){}", if transposed { " · easier key" } else { "" });
                        if ui.checkbox(&mut on, label).changed() {
                            if on {
                                self.song_ui.play_parts.push(i);
                            } else {
                                self.song_ui.play_parts.retain(|x| *x != i);
                            }
                            rebuild = true;
                        }
                    }
                    if has_chords && ui.checkbox(&mut self.song_ui.play_chords, "Chords (from the chord sheet)").changed() {
                        rebuild = true;
                    }
                });
            let mut sl = self.cfg.song.song_layout;
            if ui
                .checkbox(&mut sl, "Song layout")
                .on_hover_text("Switch to a keyboard layout made for this song: one key per chord on the bottom rows, the song's notes on the top rows. The fewest keys possible.")
                .changed()
            {
                self.cfg.song.song_layout = sl;
                self.mark_dirty();
                relayout = true;
            }
            if ui.button("✎ Edit notes").clicked() {
                self.editor_open(None);
            }
        });
        if rebuild {
            self.song_ui.jam.parts_changed();
            self.song_default_hear();
            self.song_rebuild(true);
            self.song_send_data();
            if self.cfg.song.song_layout {
                relayout = true;
            }
        }
        if relayout {
            if self.cfg.song.song_layout {
                self.song_apply_layout();
            } else {
                self.song_remove_layout();
            }
        }
        // What doesn't fit, said plainly.
        let mut notes = Vec::new();
        if let Some(pr) = &self.song_ui.practice {
            let unplayable = pr.targets.len() - pr.playable_count();
            if unplayable > 0 {
                notes.push(format!(
                    "{unplayable} notes have no key in this layout (drawn as outlines). Try the Song layout or another octave."
                ));
            }
            if pr.targets.is_empty() {
                notes.push("Pick what you want to play in “I play”.".into());
            }
        }
        if self.song_ui.shift != 0 {
            notes.push(format!(
                "Notes moved {} {} octave{} to fit your keys.",
                if self.song_ui.shift > 0 { "up" } else { "down" },
                (self.song_ui.shift / 12).abs(),
                if self.song_ui.shift.abs() > 12 {
                    "s"
                } else {
                    ""
                }
            ));
        }
        if self.song_transpose() != 0 && !self.jam_active() {
            notes.push(format!(
                "This easy part is in an easier key ({:+} semitones), so the backing is moved too and won't match the original recording.",
                self.song_transpose()
            ));
        }
        for n in notes {
            ui.label(RichText::new(n).small().color(p.tertiary));
        }
    }

    fn song_transport(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("⏮")
                .on_hover_text("Back to the start (F6)")
                .clicked()
            {
                self.song_restart();
            }
            let playing = self.song_playing();
            let (label, fill, fg) = if playing {
                ("⏸  Pause", p.primary, p.on_primary)
            } else {
                ("▶  Play", p.secondary_container, p.on_secondary_container)
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).color(fg).strong())
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(255)),
                )
                .on_hover_text("F5")
                .clicked()
            {
                self.song_play_pause();
            }
            let g = &self.song_ui.grid;
            let (bar, beat) = g.bar_of(g.beat_at(self.song_ui.heard));
            let dur = self
                .song_ui
                .song
                .as_ref()
                .map(|s| s.duration)
                .unwrap_or(0.0);
            ui.label(
                RichText::new(format!(
                    "{} / {}  ·  bar {bar} beat {}",
                    fmt_time(self.song_ui.heard),
                    fmt_time(dur),
                    (beat + 1e-6).floor().max(1.0) as i32
                ))
                .monospace(),
            );
            if self.song_ui.counting_in.is_some() {
                chip(ui, p.tertiary_container, p.on_surface, "count-in…");
            }
            if self.halted() && self.cfg.song.wait {
                chip(ui, p.tertiary, p.surface, "waiting for you");
            }
            ui.separator();
            ui.label("Speed");
            let mut s = self.song_ui.speed * 100.0;
            if ui
                .add(
                    egui::Slider::new(&mut s, 25.0..=150.0)
                        .suffix("%")
                        .integer()
                        .step_by(5.0),
                )
                .on_hover_text("F7 slower · F8 faster")
                .changed()
            {
                self.song_set_speed(s / 100.0);
            }
            for v in [50, 75, 100] {
                if ui.small_button(format!("{v}%")).clicked() {
                    self.song_set_speed(v as f64 / 100.0);
                }
            }
            ui.separator();
            let mut changed = false;
            changed |= ui
                .checkbox(&mut self.cfg.song.wait, "Wait for me")
                .on_hover_text("The song stops at each note until you press the right key (F9).")
                .changed();
            changed |= ui
                .checkbox(&mut self.cfg.song.one_key, "One-key mode")
                .on_hover_text("Press any key and the next note plays: just feel the rhythm.")
                .changed();
            if changed {
                self.mark_dirty();
                self.song_rebuild(false);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Loop").color(p.on_surface_variant));
            let sections: Vec<(f64, f64, String)> = self
                .song_ui
                .song
                .as_ref()
                .unwrap()
                .sections
                .iter()
                .map(|s| (s.start, s.end, s.title()))
                .collect();
            for (s, e, name) in sections {
                let on = self
                    .song_ui
                    .looping
                    .is_some_and(|l| (l.0 - s).abs() < 0.01 && (l.1 - e).abs() < 0.01);
                if ui
                    .selectable_label(on, name)
                    .on_hover_text("Practise just this part, over and over")
                    .clicked()
                {
                    self.song_set_loop(if on { None } else { Some((s, e)) });
                }
            }
            let h = self.song_ui.heard.max(0.0);
            if ui
                .small_button("This bar ×2")
                .on_hover_text("Loop the bar you're in and the next one")
                .clicked()
            {
                let (a, b) = self.bar_span(h, h + 0.01);
                let g = &self.song_ui.grid;
                let b2 = g.time_at(g.beat_at(b) + (g.beat_at(b) - g.beat_at(a)));
                self.song_set_loop(Some((a, b2)));
            }
            if let Some((a, b)) = self.song_ui.looping {
                let g = &self.song_ui.grid;
                let (ba, _) = g.bar_of(g.beat_at(a) + 0.01);
                let (bb, _) = g.bar_of(g.beat_at(b) - 0.01);
                chip(
                    ui,
                    p.tertiary_container,
                    p.on_surface,
                    format!("bars {ba}–{bb}"),
                );
                if ui.small_button("✕ no loop").clicked() {
                    self.song_set_loop(None);
                }
            } else {
                ui.label(
                    RichText::new("or drag across the timeline")
                        .small()
                        .color(p.outline),
                );
            }
            let mut auto = self.cfg.song.auto_speed;
            if ui
                .checkbox(&mut auto, "Speed up when I get it")
                .on_hover_text("After a loop pass of 90% or more, go 5% faster (up to 100%).")
                .changed()
            {
                self.cfg.song.auto_speed = auto;
                self.mark_dirty();
            }
        });
        egui::CollapsingHeader::new("Sound & sync")
            .id_salt("song-sound")
            .show(ui, |ui| self.song_sound(ui));
    }

    fn song_sound(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut hear_changed = false;
        if self.jam_active() {
            ui.label(
                RichText::new("Jam is on: you hear the recording (see the mixer above), not these MIDI parts.")
                    .small()
                    .color(p.tertiary),
            );
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Hear:");
            let song = self.song_ui.song.as_ref().unwrap();
            let names: Vec<(usize, String)> = song
                .parts
                .iter()
                .enumerate()
                .map(|(i, x)| (i, x.label()))
                .collect();
            for (i, name) in names {
                if let Some(h) = self.song_ui.hear.get_mut(i) {
                    let mine = self.song_ui.play_parts.contains(&i);
                    let label = if mine {
                        format!("{name} (yours)")
                    } else {
                        name
                    };
                    hear_changed |= ui.checkbox(h, label).changed();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut m = self.cfg.song.metronome;
            if ui.checkbox(&mut m, "Metronome").changed() {
                self.cfg.song.metronome = m;
                self.mark_dirty();
                hear_changed = true;
            }
            let mut ci = self.cfg.song.count_in;
            if ui
                .checkbox(&mut ci, "Count-in")
                .on_hover_text("One bar of clicks before it starts")
                .changed()
            {
                self.cfg.song.count_in = ci;
                self.mark_dirty();
            }
            let mut ao = self.cfg.song.any_octave;
            if ui
                .checkbox(&mut ao, "Any octave counts")
                .on_hover_text(
                    "The laptop keyboard has about two octaves; a C anywhere counts for a C.",
                )
                .changed()
            {
                self.cfg.song.any_octave = ao;
                self.mark_dirty();
            }
            ui.label("Song volume");
            let mut g = self.cfg.song.backing;
            if ui
                .add(egui::Slider::new(&mut g, 0.0..=1.5).show_value(false))
                .changed()
            {
                self.cfg.song.backing = g;
                self.send(Command::Song(SongCommand::Gain(g)));
                self.mark_dirty();
            }
            ui.label("Falling notes");
            let mut bv = self.cfg.song.beats_visible;
            if ui
                .add(
                    egui::Slider::new(&mut bv, 3.0..=16.0)
                        .suffix(" beats")
                        .integer(),
                )
                .changed()
            {
                self.cfg.song.beats_visible = bv;
                self.mark_dirty();
            }
        });
        ui.horizontal_wrapped(|ui| {
            let bt = self.current_device.as_ref().is_some_and(|d| d.bluetooth);
            let measured = self
                .current_device
                .as_ref()
                .and_then(|d| self.cfg.devices.get(&d.sink))
                .is_some_and(|x| x.latency_ms > 0.0);
            let source = if bt && !measured { " (a guess: run the latency test in the Audio tab)" } else if measured { " (your measured delay)" } else { "" };
            if self.cfg.song.line_up {
                ui.label(format!(
                    "Sync: the notes run {:.0} ms ahead of the sound{source}, so what you press lands with the backing.",
                    self.output_delay_ms() - self.cfg.song.offset_ms
                ));
            } else {
                ui.label(format!(
                    "Sync: notes and scoring are shifted by {:.0} ms so they match what you hear{source}.",
                    self.heard_delay_ms(),
                ));
            }
            ui.label("Nudge");
            let mut o = self.cfg.song.offset_ms;
            let tip = if self.cfg.song.line_up {
                "If your notes sound late against the backing, move left; if early, move right."
            } else {
                "If notes reach the line before you hear them, move right; if after, move left."
            };
            if ui
                .add(egui::Slider::new(&mut o, -200.0..=200.0).suffix(" ms").integer())
                .on_hover_text(tip)
                .changed()
            {
                self.cfg.song.offset_ms = o;
                self.mark_dirty();
            }
        });
        self.line_up_toggle(ui);
        let shift = self.song_ui.shift;
        ui.horizontal(|ui| {
            ui.label("Octave of the notes");
            if ui.small_button("−").clicked() {
                self.song_ui.shift = shift - 12;
                self.song_rebuild(false);
            }
            ui.label(format!("{:+}", shift / 12));
            if ui.small_button("+").clicked() {
                self.song_ui.shift = shift + 12;
                self.song_rebuild(false);
            }
            ui.label(
                RichText::new("(only changes which keys you press, not the song)")
                    .small()
                    .color(p.outline),
            );
        });
        if hear_changed {
            self.song_update_mute();
        }
    }

    fn song_falling(&mut self, ui: &mut egui::Ui, height: f32) {
        let p = self.palette;
        let su = &self.song_ui;
        let Some(song) = &su.song else { return };
        let Some(pr) = &su.practice else { return };
        let grid = &su.grid;
        let now_beat = grid.beat_at(su.heard);
        let vis = self.cfg.song.beats_visible as f64;
        let mut lo = i32::MAX;
        let mut hi = i32::MIN;
        let drums =
            su.play_parts.iter().all(|&i| song.parts[i].is_drum) && !su.play_parts.is_empty();
        let mut draws = Vec::with_capacity(pr.targets.len());
        for (i, t) in pr.targets.iter().enumerate() {
            let pitch = match t.want {
                Want::Note(n) => n,
                Want::Drum(d) => d as i32,
                Want::Chord { .. } => 0,
            };
            if t.lane == Lane::Notes {
                lo = lo.min(pitch);
                hi = hi.max(pitch);
            }
            if t.end_beat < now_beat - 2.0 || t.start_beat > now_beat + vis + 1.0 {
                continue;
            }
            let label = su.labels.get(i).copied().flatten();
            let state = match pr.results[i] {
                Some(h) => NoteState::Hit(h.judgement),
                None if !t.playable => NoteState::Unplayable,
                None => NoteState::Pending,
            };
            draws.push(NoteDraw {
                start_beat: t.start_beat,
                end_beat: t.end_beat,
                lane: t.lane,
                pitch,
                key: label.map(|l| l.0.label().to_string()).unwrap_or_default(),
                name: match &t.want {
                    Want::Chord { symbol, .. } => symbol.clone(),
                    Want::Note(n) => note_name(*n),
                    Want::Drum(d) => crate::drums::short(*d),
                },
                state,
                low_confidence: t.low_confidence,
                other_octave: label.is_some_and(|l| !l.1) && t.lane == Lane::Notes,
            });
        }
        if lo > hi {
            lo = 60;
            hi = 72;
        }
        // At least an octave wide, and keep lanes readable.
        if hi - lo < 12 {
            let pad = (12 - (hi - lo)) / 2;
            lo -= pad;
            hi = lo + 12;
        }
        let note_keys = self.note_keys();
        let mut lane_keys = HashMap::new();
        for pitch in lo..=hi {
            if drums {
                if let Some((_, k)) = self
                    .song_drum_keys()
                    .into_iter()
                    .find(|(d, _)| *d as i32 == pitch)
                {
                    lane_keys.insert(pitch, k.label().to_string());
                }
            } else if let Some((_, k)) = note_keys.iter().find(|(n, _)| *n == pitch) {
                lane_keys.insert(pitch, k.label().to_string());
            }
        }
        let held: HashSet<i32> = self.sounding.values().copied().collect();
        let sections: Vec<(f64, String)> = song
            .sections
            .iter()
            .map(|s| (grid.beat_at(s.start), s.title()))
            .collect();
        let flash = su.flash.and_then(|(good, at)| {
            let a = 1.0 - at.elapsed().as_secs_f32() / 0.35;
            (a > 0.0).then_some((if good { p.primary } else { p.error }, a))
        });
        let chord_lane = pr.targets.iter().any(|t| t.lane == Lane::Chords);
        let view = FallView {
            notes: &draws,
            grid,
            now_beat,
            beats_visible: vis,
            pitch_lo: lo,
            pitch_hi: hi,
            lane_keys: &lane_keys,
            held: &held,
            chord_lane,
            chord_held: self
                .chord
                .as_ref()
                .filter(|_| !self.chord_keys.is_empty())
                .map(|c| c.name.clone()),
            loop_beats: su.looping.map(|(a, b)| (grid.beat_at(a), grid.beat_at(b))),
            sections: &sections,
            drums,
            height,
            flash,
        };
        let top = ui.cursor().min;
        let clicked = view.show(ui, &p);
        if let Some(b) = clicked
            && !self.song_playing()
        {
            let t = self.song_ui.grid.time_at(b);
            self.song_seek(t);
        }
        // Timing feedback and the end-of-song card, over the view.
        let painter = ui.painter();
        if let Some((hit, at)) = self.song_ui.last_hit
            && at.elapsed() < Duration::from_millis(900)
        {
            let (text, c) = match hit.judgement {
                Judgement::Perfect if self.cfg.song.wait => ("Yes!".to_string(), p.primary),
                Judgement::Perfect => ("Perfect".to_string(), p.primary),
                Judgement::Good if hit.offset < 0.0 => (
                    format!("Good · {:.0} ms early", -hit.offset * 1000.0),
                    p.on_secondary_container,
                ),
                Judgement::Good => (
                    format!("Good · {:.0} ms late", hit.offset * 1000.0),
                    p.on_secondary_container,
                ),
                Judgement::Miss => ("Missed".to_string(), p.error),
            };
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(18.0), c);
            let pos = top + vec2(ui.available_width() * 0.5 - galley.size().x / 2.0, 14.0);
            let bg =
                egui::Rect::from_min_size(pos - vec2(12.0, 5.0), galley.size() + vec2(24.0, 10.0));
            painter.rect_filled(
                bg,
                egui::CornerRadius::same(255),
                p.surface_container_highest,
            );
            painter.galley(pos, galley, c);
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if self
            .song_ui
            .flash
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(400))
        {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
        if let Some((score, best)) = self.song_ui.result {
            let rect = egui::Rect::from_center_size(
                top + vec2(ui.available_width() * 0.5, height * 0.45),
                vec2(340.0, 190.0),
            );
            let mut again = false;
            let mut slower = false;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                egui::Frame::new()
                    .fill(p.surface_container_highest)
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(score.grade())
                                    .size(24.0)
                                    .strong()
                                    .color(p.primary),
                            );
                            ui.label(
                                RichText::new(format!("{:.0}%", score.accuracy()))
                                    .size(34.0)
                                    .strong(),
                            );
                            if best {
                                ui.label(RichText::new("★ New best!").color(p.tertiary));
                            }
                            ui.label(format!(
                                "{} perfect · {} good · {} missed · {} wrong keys · best streak {}",
                                score.perfect,
                                score.good,
                                score.missed,
                                score.wrong,
                                score.best_streak
                            ));
                            ui.horizontal(|ui| {
                                again = ui.button("Play again").clicked();
                                slower = ui.button("Again, slower").clicked();
                            });
                        });
                    });
            });
            if again || slower {
                if slower {
                    self.song_set_speed(self.song_ui.speed - 0.15);
                }
                if let Some(pr) = &mut self.song_ui.practice {
                    pr.reset();
                }
                self.song_ui.result = None;
                self.song_restart();
            }
        }
    }

    fn song_sheet(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let su = &self.song_ui;
        let Some(song) = &su.song else { return };
        let part = su.play_parts.first().map(|&i| &song.parts[i]);
        let chord_keys = self.song_chord_keys();
        let chords: Vec<SheetChord> = song
            .chords_for(part)
            .iter()
            .filter(|c| !c.is_none())
            .map(|c| SheetChord {
                start_beat: c.start_beat,
                end_beat: c.end_beat,
                symbol: c.symbol.clone(),
                roman: c.roman.clone(),
                key: span_kind(c)
                    .and_then(|(r, k)| {
                        let pcs: Vec<i32> = k.intervals().iter().map(|t| (r + t) % 12).collect();
                        Self::chord_key_for(&chord_keys, r, &pcs)
                    })
                    .map(|(k, _)| k.label().to_string()),
                low_confidence: c.low_confidence,
            })
            .collect();
        let g = su.grid.clone();
        let sections: Vec<(f64, String)> = song
            .sections
            .iter()
            .map(|s| (g.beat_at(s.start), s.title()))
            .collect();
        let end = g.beat_at(song.duration);
        let now = g.beat_at(su.heard);
        let lp = su.looping.map(|(a, b)| (g.beat_at(a), g.beat_at(b)));
        let follow = self.song_playing();
        if chords.is_empty() {
            ui.label(RichText::new("This song has no chord sheet.").color(p.on_surface_variant));
            return;
        }
        if let Some(b) =
            song_view::chord_sheet(ui, &p, &chords, &g, now, end, &sections, lp, follow)
        {
            self.song_seek(g.time_at(b));
        }
    }

    fn song_score_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(pr) = &self.song_ui.practice else {
            return;
        };
        let s = if self.song_ui.looping.is_some() {
            pr.pass
        } else {
            pr.score
        };
        egui::Frame::new()
            .fill(p.surface_container)
            .corner_radius(egui::CornerRadius::same(16))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:.0}%", s.accuracy()))
                            .size(30.0)
                            .strong()
                            .color(p.primary),
                    );
                    ui.vertical(|ui| {
                        ui.label(format!("streak {}  ·  best {}", s.streak, s.best_streak));
                        ui.label(
                            RichText::new(format!(
                                "{} perfect · {} good · {} missed · {} wrong",
                                s.perfect, s.good, s.missed, s.wrong
                            ))
                            .small()
                            .color(p.on_surface_variant),
                        );
                    });
                });
                if !self.song_ui.passes.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("passes").small().color(p.outline));
                        for a in &self.song_ui.passes {
                            let (r, _) =
                                ui.allocate_exact_size(vec2(10.0, 24.0), egui::Sense::hover());
                            let h = r.height() * (a / 100.0).clamp(0.05, 1.0);
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(r.left(), r.bottom() - h),
                                r.max,
                            );
                            ui.painter().rect_filled(
                                bar,
                                egui::CornerRadius::same(2),
                                if *a >= 90.0 { p.primary } else { p.outline },
                            );
                        }
                    });
                }
                if self.cfg.song.wait {
                    ui.label(
                        RichText::new("Wait mode: no rush, timing isn't scored.")
                            .small()
                            .color(p.outline),
                    );
                }
            });
    }

    fn song_help(&self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut text = String::from(
            "Notes fall towards the line; the letter on each note is the key to press when it reaches the line. \
             The strip under the line shows which key plays which note. Dashed notes with a “?” are ones the analyzer \
             wasn't sure about: trust your ears. Click the chord sheet or the timeline to jump. ",
        );
        if self.cfg.song.one_key {
            text += "One-key mode: tap any letter key in time and the right notes play. ";
        } else if self.cfg.song.wait {
            text += "Wait mode: the song waits on each note until you press its key. ";
        }
        text += "F5 play/pause · F6 restart · F7/F8 slower/faster · F9 wait mode.";
        ui.label(RichText::new(text).color(p.on_surface_variant));
        if self.current_device.as_ref().is_some_and(|d| d.bluetooth) {
            let text = if self.cfg.song.line_up {
                format!(
                    "Bluetooth: your earbuds play everything about {:.0} ms late, your notes too. The falling notes run \
                     that much ahead of the sound, so press when a note reaches the line (not when you hear it) and it \
                     comes out together with the backing.",
                    self.output_delay_ms()
                )
            } else {
                format!(
                    "Bluetooth: your earbuds play sound about {:.0} ms late. The falling notes and the scoring are moved by \
                     the same amount, so press when you hear it, and a slightly late press still counts.",
                    self.heard_delay_ms()
                )
            };
            ui.label(RichText::new(text).color(p.tertiary));
        }
    }

    // ------------------------------------------------------------ editor

    fn editor_open(&mut self, part: Option<usize>) {
        if self
            .song_ui
            .song
            .as_ref()
            .is_none_or(|s| s.parts.is_empty())
        {
            self.notify("This song has no notes to edit.");
            return;
        }
        if self.song_playing() {
            self.song_play_pause();
        }
        let Some(song) = &self.song_ui.song else {
            return;
        };
        let part = part
            .or_else(|| {
                // The part with the most unsure notes, else what you play.
                song.parts
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.low_confidence_count() > 0)
                    .max_by_key(|(_, p)| p.low_confidence_count())
                    .map(|(i, _)| i)
            })
            .or_else(|| self.song_ui.play_parts.first().copied())
            .unwrap_or(0);
        let ed = Editor::new(song, part);
        if let Some((lo, hi)) = song.parts[part].pitch_range() {
            // The part's notes in the middle of the view.
            let mid = (lo as i32 + hi as i32) / 2;
            self.song_ui.roll.pitch_top = (mid + 18).max(hi as i32 + 3).min(127);
        }
        if let Some(n) = ed.notes.first() {
            self.song_ui.roll.beat0 = n.start_beat.floor() - 1.0;
        }
        self.song_ui.editor = Some(ed);
        self.song_ui.view = SongView::Edit;
        self.song_ui.confirm_leave = false;
    }

    /// Put the editor's notes into the song so playback / practice use them.
    fn editor_apply(&mut self) {
        if let (Some(ed), Some(song)) = (&self.song_ui.editor, &mut self.song_ui.song) {
            ed.apply(song);
        }
    }

    fn editor_play_toggle(&mut self) {
        if self.song_playing() {
            self.song_send(SongCommand::Play(false));
            return;
        }
        self.editor_apply();
        let part = self.song_ui.editor.as_ref().map(|e| e.part).unwrap_or(0);
        // Hear the part being edited (plus the rest if asked).
        let n = self
            .song_ui
            .song
            .as_ref()
            .map(|s| s.parts.len())
            .unwrap_or(0);
        let saved = self.song_ui.hear.clone();
        self.song_send_data();
        self.song_ui.hear = (0..n)
            .map(|i| {
                i == part || (self.song_ui.edit_hear_all && saved.get(i).copied().unwrap_or(true))
            })
            .collect();
        self.song_update_mute();
        self.song_ui.hear = saved;
        let g = &self.song_ui.grid;
        let from = g.time_at(self.song_ui.roll.beat0.max(g.beat_at(0.0)));
        self.song_send(SongCommand::Wait(None));
        self.song_ui.sent_wait = None;
        self.song_send(SongCommand::Seek(from));
        self.song_send(SongCommand::Play(true));
    }

    fn audition(&mut self, pitch: u8) {
        self.send(Command::ChannelNote {
            channel: 0,
            note: pitch,
            velocity: 90,
        });
        self.song_ui
            .pending_offs
            .push((Instant::now() + Duration::from_millis(380), 0, pitch));
    }

    fn editor_key(&mut self, k: KeyId, _repeat: bool, mods: egui::Modifiers) -> bool {
        let KeyId::Key(key) = k else { return false };
        let ctrl = mods.ctrl || mods.command;
        let snap = if self.song_ui.snap > 0.0 {
            self.song_ui.snap
        } else {
            0.25
        };
        let Some(ed) = &mut self.song_ui.editor else {
            return false;
        };
        let sel = ed.selected.clone();
        let mut audition = None;
        match key {
            Key::Delete | Key::Backspace => ed.delete(&sel),
            Key::ArrowUp | Key::ArrowDown if !sel.is_empty() => {
                let d = if key == Key::ArrowUp { 1 } else { -1 } * if mods.shift { 12 } else { 1 };
                ed.move_notes(&sel, 0.0, d);
                audition = ed.selected.first().map(|&i| ed.notes[i].pitch);
            }
            Key::ArrowLeft | Key::ArrowRight if !sel.is_empty() => {
                let d = if key == Key::ArrowRight { snap } else { -snap };
                ed.move_notes(&sel, d, 0);
            }
            Key::Enter => ed.confirm_notes(&sel),
            Key::Tab => {
                if let Some(i) = ed.next_unsure(sel.first().copied(), !mods.shift) {
                    ed.selected = vec![i];
                    let n = ed.notes[i];
                    self.song_ui
                        .roll
                        .reveal(n.start_beat, n.pitch as i32, 900.0, 26);
                    audition = Some(n.pitch);
                }
            }
            Key::Z if ctrl && mods.shift => ed.redo(),
            Key::Z if ctrl => ed.undo(),
            Key::Y if ctrl => ed.redo(),
            Key::S if ctrl => {
                self.editor_save();
                return true;
            }
            _ => return false,
        }
        if let Some(pitch) = audition {
            self.audition(pitch);
        }
        true
    }

    fn editor_save(&mut self) {
        self.editor_apply();
        let Some(song) = self.song_ui.song.clone() else {
            return;
        };
        let Some(ed) = &mut self.song_ui.editor else {
            return;
        };
        let res = match &song.origin {
            Origin::Folder(_) => ed.save(&song).map(|p| format!("Saved {}", p.display())),
            Origin::Midi(path) => crate::song::edit::save_midi_copy(&song, path).map(|p| {
                ed.dirty = false;
                format!("Saved a copy: {}", p.display())
            }),
            Origin::Memory => Err("nowhere to save".into()),
        };
        match res {
            Ok(m) => {
                self.notify(m);
                self.song_rebuild(false);
                self.song_send_data();
                self.song_ui.scanned = false;
            }
            Err(e) => self.notify(format!("Couldn't save: {e}")),
        }
    }

    fn song_editor(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut leave = false;
        let mut switch_part = None;
        ui.horizontal_wrapped(|ui| {
            let dirty = self.song_ui.editor.as_ref().is_some_and(|e| e.dirty);
            if ui.button("← Practise").clicked() {
                if dirty {
                    self.song_ui.confirm_leave = true;
                } else {
                    leave = true;
                }
            }
            let song = self.song_ui.song.as_ref().unwrap();
            ui.label(
                RichText::new(format!("Edit notes · {}", song.display_name()))
                    .size(18.0)
                    .strong()
                    .color(p.primary),
            );
            let ed = self.song_ui.editor.as_ref().unwrap();
            let cur = ed.part;
            egui::ComboBox::from_id_salt("ed-part")
                .width(200.0)
                .selected_text(song.parts[cur].label())
                .show_ui(ui, |ui| {
                    for (i, part) in song.parts.iter().enumerate() {
                        let unsure = part.low_confidence_count();
                        let label = if unsure > 0 {
                            format!("{} ({unsure} unsure)", part.label())
                        } else {
                            part.label()
                        };
                        if ui.selectable_label(i == cur, label).clicked() && i != cur {
                            switch_part = Some(i);
                        }
                    }
                });
            let unsure = ed.low_confidence_count();
            chip(
                ui,
                if unsure > 0 {
                    p.tertiary_container
                } else {
                    p.primary_container
                },
                p.on_surface,
                if unsure > 0 {
                    format!("{unsure} unsure notes")
                } else {
                    "every note checked ✓".into()
                },
            );
        });
        if let Some(i) = switch_part {
            if self.song_ui.editor.as_ref().is_some_and(|e| e.dirty) {
                self.editor_apply();
                self.notify("Changes kept in memory; press Save to write them.");
            }
            self.editor_open(Some(i));
        }
        if self.song_ui.confirm_leave {
            ui.horizontal(|ui| {
                ui.label(RichText::new("You have unsaved changes.").color(p.tertiary));
                if ui.button("Save").clicked() {
                    self.editor_save();
                    leave = true;
                }
                if ui
                    .button("Keep them for now")
                    .on_hover_text("Practise with the edited notes without saving")
                    .clicked()
                {
                    self.editor_apply();
                    self.song_rebuild(false);
                    self.song_send_data();
                    leave = true;
                }
                if ui.button("Throw away").clicked() {
                    if let Some(path) = self.song_ui.song.as_ref().and_then(|s| match &s.origin {
                        Origin::Folder(d) => Some(d.clone()),
                        Origin::Midi(m) => Some(m.clone()),
                        _ => None,
                    }) && let Ok(s) = Song::load(&path)
                    {
                        self.set_song(s);
                    }
                    leave = true;
                }
            });
        }
        if leave {
            self.song_ui.view = SongView::Practice;
            self.song_ui.confirm_leave = false;
            if self.song_playing() {
                self.song_send(SongCommand::Play(false));
            }
            return;
        }

        // Tool row.
        let mut save = false;
        ui.horizontal_wrapped(|ui| {
            let ed = self.song_ui.editor.as_mut().unwrap();
            let sel = ed.selected.clone();
            if ui
                .button("◀ unsure")
                .on_hover_text("Previous unsure note (Shift+Tab)")
                .clicked()
                && let Some(i) = ed.next_unsure(sel.first().copied(), false)
            {
                ed.selected = vec![i];
                let n = ed.notes[i];
                self.song_ui
                    .roll
                    .reveal(n.start_beat, n.pitch as i32, 900.0, 26);
            }
            let ed = self.song_ui.editor.as_mut().unwrap();
            if ui
                .button("unsure ▶")
                .on_hover_text("Next unsure note (Tab)")
                .clicked()
                && let Some(i) = ed.next_unsure(sel.first().copied(), true)
            {
                ed.selected = vec![i];
                let n = ed.notes[i];
                self.song_ui
                    .roll
                    .reveal(n.start_beat, n.pitch as i32, 900.0, 26);
            }
            let ed = self.song_ui.editor.as_mut().unwrap();
            if ui
                .add_enabled(!sel.is_empty(), egui::Button::new("✓ Looks right"))
                .on_hover_text("Mark as checked (Enter)")
                .clicked()
            {
                ed.confirm_notes(&sel);
            }
            if ui
                .add_enabled(!sel.is_empty(), egui::Button::new("Delete"))
                .clicked()
            {
                ed.delete(&sel);
            }
            if ui
                .add_enabled(!sel.is_empty(), egui::Button::new("Snap to grid"))
                .clicked()
            {
                let s = if self.song_ui.snap > 0.0 {
                    self.song_ui.snap
                } else {
                    0.25
                };
                ed.quantize(&sel, s);
            }
            if ui
                .add_enabled(ed.can_undo(), egui::Button::new("↶ Undo"))
                .clicked()
            {
                ed.undo();
            }
            if ui
                .add_enabled(ed.can_redo(), egui::Button::new("↷ Redo"))
                .clicked()
            {
                ed.redo();
            }
            ui.separator();
            ui.label("Grid");
            egui::ComboBox::from_id_salt("snap")
                .width(80.0)
                .selected_text(match (self.song_ui.snap * 8.0).round() as i32 {
                    8 => "beat",
                    4 => "1/2 beat",
                    2 => "1/4 beat",
                    1 => "1/8 beat",
                    _ => "off",
                })
                .show_ui(ui, |ui| {
                    for (v, l) in [
                        (1.0, "beat"),
                        (0.5, "1/2 beat"),
                        (0.25, "1/4 beat"),
                        (0.125, "1/8 beat"),
                        (0.0, "off"),
                    ] {
                        ui.selectable_value(&mut self.song_ui.snap, v, l);
                    }
                });
            ui.separator();
            let playing = self.song_playing();
            if ui
                .button(if playing {
                    "■ Stop"
                } else {
                    "▶ Play from here"
                })
                .on_hover_text("F5")
                .clicked()
            {
                self.editor_play_toggle();
            }
            ui.checkbox(&mut self.song_ui.edit_hear_all, "with the other parts");
            ui.separator();
            let dirty = self.song_ui.editor.as_ref().is_some_and(|e| e.dirty);
            let is_folder = self
                .song_ui
                .song
                .as_ref()
                .is_some_and(|s| s.dir().is_some());
            if ui
                .add_enabled(
                    dirty,
                    egui::Button::new(
                        RichText::new(if is_folder {
                            "💾 Save"
                        } else {
                            "💾 Save a copy"
                        })
                        .strong(),
                    ),
                )
                .on_hover_text(if is_folder {
                    "Writes song.json (Ctrl+S). Edited notes are marked as checked."
                } else {
                    "Writes <name>-edited.mid next to the file"
                })
                .clicked()
            {
                save = true;
            }
        });
        if save {
            self.editor_save();
        }

        // The roll.
        let su = &mut self.song_ui;
        let (Some(ed), Some(song)) = (&su.editor, &su.song) else {
            return;
        };
        let notes: Vec<RollNote> = ed
            .notes
            .iter()
            .map(|n| RollNote {
                start_beat: n.start_beat,
                end_beat: n.end_beat,
                pitch: n.pitch as i32,
                low_confidence: n.low_confidence,
                confidence: n.confidence,
            })
            .collect();
        let part = &song.parts[ed.part];
        let thr = song.low_confidence_threshold;
        let chords: Vec<(f64, f64, String)> = song
            .chords_for(Some(part))
            .iter()
            .filter(|c| !c.is_none())
            .map(|c| (c.start_beat, c.end_beat, c.symbol.clone()))
            .collect();
        let playhead = su.clock.playing.then(|| su.grid.beat_at(su.heard));
        let selected = ed.selected.clone();
        let h = (ui.available_height() - 110.0).max(260.0);
        let grid = su.grid.clone();
        let actions = song_view::piano_roll(
            ui,
            &p,
            &mut su.roll,
            &notes,
            &selected,
            &grid,
            su.snap,
            playhead,
            &chords,
            h,
        );
        let mut audition = None;
        let mut seek = None;
        let ed = su.editor.as_mut().unwrap();
        for a in actions {
            match a {
                RollAction::Select { idx, add } => match idx {
                    Some(i) => {
                        if add {
                            if let Some(pos) = ed.selected.iter().position(|x| *x == i) {
                                ed.selected.remove(pos);
                            } else {
                                ed.selected.push(i);
                            }
                        } else {
                            ed.selected = vec![i];
                        }
                        audition = Some(ed.notes[i].pitch);
                    }
                    None if !add => ed.selected.clear(),
                    None => {}
                },
                RollAction::Move { d_beats, d_pitch } => {
                    let sel = ed.selected.clone();
                    ed.move_notes(&sel, d_beats, d_pitch);
                    if d_pitch != 0 {
                        audition = ed.selected.first().map(|&i| ed.notes[i].pitch);
                    }
                }
                RollAction::Resize { idx, end_beat } => ed.set_end(idx, end_beat),
                RollAction::Add { beat, pitch } => {
                    let len = if su.snap > 0.0 { su.snap.max(0.5) } else { 1.0 };
                    ed.add(beat, pitch.clamp(0, 127) as u8, len, 96);
                    audition = Some(pitch.clamp(0, 127) as u8);
                }
                RollAction::Seek(b) => seek = Some(b),
            }
        }
        // Selected note, in words.
        let info = ed.selected.first().map(|&i| {
            let n = ed.notes[i];
            let (bar, beat) = grid.bar_of(n.start_beat);
            format!(
                "{} · bar {bar} beat {:.2} · {:.2} beats long · {}",
                note_name(n.pitch as i32),
                beat,
                n.end_beat - n.start_beat,
                if n.low_confidence {
                    format!(
                        "unsure: {:.0}% sure (under {:.0}% counts as unsure)",
                        n.confidence * 100.0,
                        thr * 100.0
                    )
                } else if n.confidence >= 0.999 {
                    "checked".into()
                } else {
                    format!("{:.0}% sure", n.confidence * 100.0)
                }
            )
        });
        if let Some(pitch) = audition {
            self.audition(pitch);
        }
        if let Some(b) = seek {
            self.song_ui.roll.beat0 = b - 1.0;
        }
        ui.label(
            RichText::new(info.unwrap_or_else(|| {
                "Click a note to select it (it plays so you can check it by ear).".into()
            }))
            .color(p.on_surface),
        );
        ui.label(
            RichText::new(
                "Drag a note to move it, drag its right edge to make it longer, double-click empty space to add one. \
                 Tab jumps to the next unsure note · ↑↓ change pitch (Shift = octave) · ←→ move · Enter = looks right · \
                 Delete removes · Ctrl+Z undo · Ctrl+S save. Scroll to move in time, Shift+scroll for pitch, Ctrl+scroll to zoom, \
                 drag empty space to pan. Your letter keys still play notes, so you can try a note before fixing it.",
            )
            .small()
            .color(p.on_surface_variant),
        );
        if self.song_ui.clock.playing {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fmt_time;

    #[test]
    fn times() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(83.4), "1:23");
        assert_eq!(fmt_time(-3.0), "0:00");
    }
}
