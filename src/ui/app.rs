//! The main window: input handling, state, the chord engine and the tabs.
//! The Play and Keybinds tabs live in `app/play.rs` and `app/keybinds.rs`.

mod keybinds;
mod learn;
mod listen;
mod looper;
mod play;
mod record;
mod song;

use crate::audio::patterns::Pattern;
use crate::audio::sequencer::{ChordNotes, StyleKind, StyleParams};
use crate::audio::{Command, Engine};
use crate::config::{self, Config, DeviceProfile, FileWatch};
use crate::devices::{self, OutputInfo};
use crate::keymap::{Action, BUILTIN_LAYOUTS, KeyId, Keymap};
use crate::latency::{TARGET_TAPS, TEST_BPM, TapResult, TapTest};
use crate::music::{BASS_SOUNDS, INSTRUMENTS, note_name};
use crate::theme::{self, Palette};
use crate::theory::{Chord, ChordMod, ChordRef, build_chord, voice_chord};
use crate::ui::widgets::{CapLabel, CapStyle, KeyboardView, card, chip};
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Color32, Event, Key, RichText};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VELOCITIES: [(i32, &str); 3] = [(60, "Soft"), (95, "Normal"), (122, "Loud")];
const BUFFER_CHOICES: [u32; 5] = [32, 64, 128, 256, 512];
/// MIDI note that `note 0` plays at octave 0 (C3).
const BASE_NOTE: i32 = 48;
/// Styles that aren't patterns, as stored in config.toml.
const FIXED_STYLES: [(&str, &str); 3] = [
    ("block", "Block chords"),
    ("strum", "Strum"),
    ("arpeggio", "Arpeggio"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Play,
    Learn,
    Song,
    Listen,
    Keybinds,
    Audio,
    KeyTest,
}

pub struct QwertoneApp {
    palette: Palette,
    theme_file: Option<PathBuf>,
    theme_watch: Option<FileWatch>,

    cfg: Config,
    cfg_watch: FileWatch,
    cfg_dirty_since: Option<Instant>,

    keymap: Keymap,
    keymap_file: String,
    keymap_watch: FileWatch,
    keymap_warnings: Vec<String>,

    engine: Option<Engine>,
    engine_error: Option<String>,
    sf_tx: Sender<Result<String, String>>,
    sf_rx: Receiver<Result<String, String>>,
    sf_status: String,
    sf_path_input: String,

    device: Arc<Mutex<Option<OutputInfo>>>,
    current_device: Option<OutputInfo>,

    tab: Tab,
    /// Keys physically held right now (any key, bound or not).
    held: HashSet<KeyId>,
    /// Key -> note it started, so releasing always stops the right note
    /// even if you changed octave in between.
    sounding: HashMap<KeyId, i32>,
    sustain_on: bool,

    // Chords.
    /// Chord keys held, oldest first; the last one plays.
    chord_keys: Vec<(KeyId, ChordRef)>,
    /// Chord modifier keys held.
    mod_keys: Vec<(KeyId, ChordMod)>,
    /// The chord that last played (kept while latched).
    last_chord_ref: Option<ChordRef>,
    /// What's sounding / shown: the chord, its notes, its bass note.
    chord: Option<Chord>,
    voicing: Vec<i32>,
    bass_note: Option<i32>,
    /// A chord keeps sounding after its key was released (latch / pedal).
    chord_latched: bool,
    patterns: Vec<Arc<Pattern>>,
    /// Patterns replaced by a reload, kept so the audio thread never frees one.
    retired_patterns: Vec<Arc<Pattern>>,
    pattern_warnings: Vec<String>,
    last_drum: Option<(u8, Instant)>,
    /// When each drum was last hit, for the drum-pad picture.
    drum_hits: HashMap<u8, Instant>,

    // Keybind editor.
    selected_key: Option<KeyId>,
    /// Waiting for a key press to bind: (key it's moving from, action).
    capture_for: Option<(Option<KeyId>, Action)>,
    new_layout_name: String,

    // Saved devices list: (sink being renamed, new name), and the sink
    // waiting for a second "remove" click.
    renaming: Option<(String, String)>,
    confirm_remove: Option<String>,
    /// Focus the rename box on the next frame.
    rename_focus: bool,

    // Latency test.
    tap: TapTest,
    rec: record::RecordUi,
    /// Looper + MIDI out (Part 4b).
    loop_ui: looper::LoopUi,
    tap_result: Option<TapResult>,

    // Keyboard ghosting test.
    max_held: usize,
    best_combo: Vec<KeyId>,

    /// Learn mode: lessons, ear training, theory overlay.
    learn: learn::LearnUi,
    /// Song mode (Part 5) and live listen.
    song_ui: song::SongUi,
    listen: listen::ListenUi,

    toast: Option<(String, Instant)>,
    last_poll: Instant,
}

impl QwertoneApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        let missing_fonts = theme::install_fonts(ctx);
        let (cfg, cfg_err) = Config::load();
        let (palette, theme_file) = if cfg.follow_system_theme {
            match theme::load_system_palette() {
                Some((p, path)) => (p, Some(path)),
                None => (Palette::default(), None),
            }
        } else {
            (Palette::default(), None)
        };
        theme::apply(ctx, &palette);

        let keymap_file = cfg.keymap.clone();
        let (keymap, keymap_warnings) = config::load_keymap(&keymap_file);
        let (sf_tx, sf_rx) = unbounded();
        let (patterns, pattern_warnings) = config::load_patterns();

        let mut app = Self {
            palette,
            theme_watch: theme_file.clone().map(FileWatch::new),
            theme_file,
            cfg_watch: FileWatch::new(config::config_path()),
            cfg,
            cfg_dirty_since: None,
            keymap_watch: FileWatch::new(config::keymap_path(&keymap_file)),
            keymap,
            keymap_file,
            keymap_warnings,
            engine: None,
            engine_error: None,
            sf_tx,
            sf_rx,
            sf_status: String::new(),
            sf_path_input: String::new(),
            device: devices::spawn_watcher(ctx.clone()),
            current_device: None,
            tab: Tab::Play,
            held: HashSet::new(),
            sounding: HashMap::new(),
            sustain_on: false,
            chord_keys: Vec::new(),
            mod_keys: Vec::new(),
            last_chord_ref: None,
            chord: None,
            voicing: Vec::new(),
            bass_note: None,
            chord_latched: false,
            patterns: patterns.into_iter().map(Arc::new).collect(),
            retired_patterns: Vec::new(),
            pattern_warnings,
            last_drum: None,
            drum_hits: HashMap::new(),
            selected_key: None,
            capture_for: None,
            new_layout_name: String::new(),
            renaming: None,
            confirm_remove: None,
            rename_focus: false,
            tap: TapTest::default(),
            rec: record::RecordUi::default(),
            loop_ui: looper::LoopUi::default(),
            tap_result: None,
            max_held: 0,
            best_combo: Vec::new(),
            learn: learn::LearnUi::new(&Default::default()),
            song_ui: song::SongUi::new(),
            listen: listen::ListenUi::default(),
            toast: None,
            last_poll: Instant::now(),
        };
        app.learn = learn::LearnUi::new(&app.cfg.learn);
        app.set_missing_fonts(missing_fonts);
        if let Some(e) = cfg_err {
            app.notify(e);
        }
        app.start_engine();
        app
    }

    // ---------------------------------------------------------------- engine

    fn start_engine(&mut self) {
        self.engine = None; // drop the old stream first
        self.sounding.clear();
        self.reset_chord_state();
        let program = self.instrument().program;
        match Engine::start(self.cfg.buffer_frames, program, self.cfg.volume) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.engine_error = None;
                self.send_programs();
                self.send_style();
                self.load_soundfont(self.cfg.soundfont.clone());
                // A new audio thread has no song: hand it over again.
                self.song_resend();
                // ... nor MIDI out (a loop doesn't survive a restart).
                self.apply_midi_out();
            }
            Err(e) => {
                self.engine_error = Some(format!("{e:#}"));
            }
        }
    }

    fn load_soundfont(&mut self, path: Option<PathBuf>) {
        let Some(engine) = &self.engine else { return };
        match config::find_soundfont(path.as_deref()) {
            Some(p) => {
                self.sf_status = format!(
                    "Loading {}…",
                    p.file_name().unwrap_or_default().to_string_lossy()
                );
                self.sf_path_input = p.display().to_string();
                engine.load_soundfont_async(&p, self.sf_tx.clone());
            }
            None => {
                self.sf_status = "No SoundFont found, using the built-in synth. \
                                  Install one with: sudo pacman -S soundfont-fluid"
                    .into();
            }
        }
    }

    fn send(&self, cmd: Command) {
        if let Some(e) = &self.engine {
            e.send(cmd);
        }
    }

    fn instrument(&self) -> crate::music::Instrument {
        INSTRUMENTS[self.cfg.instrument.min(INSTRUMENTS.len() - 1)]
    }

    fn set_instrument(&mut self, idx: usize) {
        self.cfg.instrument = idx % INSTRUMENTS.len();
        self.send(Command::Program(self.instrument().program));
        self.send_programs();
        self.mark_dirty();
    }

    /// Chord and bass channel sounds.
    fn send_programs(&self) {
        let chord = self
            .cfg
            .chord_instrument
            .map(|i| INSTRUMENTS[i.min(INSTRUMENTS.len() - 1)])
            .unwrap_or_else(|| self.instrument());
        let bass = BASS_SOUNDS[self.cfg.bass_instrument.min(BASS_SOUNDS.len() - 1)];
        self.send(Command::ChannelProgram {
            channel: 1,
            program: chord.program,
        });
        self.send(Command::ChannelProgram {
            channel: 2,
            program: bass.program,
        });
    }

    fn base_note(&self) -> i32 {
        BASE_NOTE + self.cfg.octave * 12 + self.cfg.transpose
    }

    fn velocity(&self) -> i32 {
        VELOCITIES[self.cfg.velocity.min(2)].0
    }

    fn all_notes_off(&mut self) {
        self.sounding.clear();
        self.song_stop_sounds();
        self.sustain_on = false;
        self.reset_chord_state();
        self.send(Command::Sustain(false));
        self.send(Command::AllOff);
    }

    fn reset_chord_state(&mut self) {
        self.chord_keys.clear();
        self.chord = None;
        self.bass_note = None;
        self.chord_latched = false;
    }

    fn notify(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn mark_dirty(&mut self) {
        self.cfg_dirty_since.get_or_insert_with(Instant::now);
    }

    // ---------------------------------------------------------------- styles

    /// Every style: the fixed ones, then each pattern. (config id, label)
    fn styles(&self) -> Vec<(String, String)> {
        FIXED_STYLES
            .iter()
            .map(|(id, label)| (id.to_string(), label.to_string()))
            .chain(
                self.patterns
                    .iter()
                    .map(|p| (p.name.to_lowercase(), p.name.clone())),
            )
            .collect()
    }

    fn style_index(&self) -> usize {
        self.styles()
            .iter()
            .position(|(id, _)| id.eq_ignore_ascii_case(&self.cfg.style))
            .unwrap_or(0)
    }

    fn current_pattern(&self) -> Option<Arc<Pattern>> {
        self.patterns
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(&self.cfg.style))
            .cloned()
    }

    fn style_kind(&self) -> StyleKind {
        match self.cfg.style.as_str() {
            "block" => StyleKind::Block,
            "strum" => StyleKind::Strum,
            "arpeggio" => StyleKind::Arp,
            _ if self.current_pattern().is_some() => StyleKind::Pattern,
            _ => StyleKind::Block,
        }
    }

    fn rhythmic(&self) -> bool {
        matches!(self.style_kind(), StyleKind::Arp | StyleKind::Pattern)
    }

    /// Extra time allowed for a late chord change to still land on the beat:
    /// about the measured output delay on Bluetooth, since you react to
    /// what you hear.
    fn grace_ms(&self) -> f32 {
        let measured = self
            .current_device
            .as_ref()
            .and_then(|d| self.cfg.devices.get(&d.sink))
            .map(|p| p.latency_ms)
            .unwrap_or(0.0);
        (80.0 + measured * 0.8).min(320.0)
    }

    fn send_style(&self) {
        // Song mode: chord keys play plain block chords (a running pattern
        // would fight the song's own rhythm).
        let song = self.song_ui.style_overridden;
        let params = StyleParams {
            kind: if song {
                StyleKind::Block
            } else {
                self.style_kind()
            },
            strum_ms: self.cfg.strum_ms.clamp(5.0, 120.0),
            arp_mode: self.cfg.arp_mode,
            arp_steps_per_beat: self.cfg.arp_rate.clamp(1, 8),
            arp_octaves: self.cfg.arp_octaves.clamp(1, 3),
            gate: 0.8,
            bass: self.cfg.chord_bass,
            drums: self.cfg.drums,
            latch: self.cfg.latch && !song,
            grace_ms: self.grace_ms(),
        };
        self.send(Command::Tempo(self.cfg.tempo.clamp(40.0, 240.0)));
        self.send(Command::Pattern(self.current_pattern()));
        self.send(Command::Style(params));
    }

    fn set_style(&mut self, idx: usize) {
        let styles = self.styles();
        let idx = idx % styles.len();
        self.cfg.style = styles[idx].0.clone();
        self.mark_dirty();
        self.send_style();
        if !self.rhythmic() && self.chord_keys.is_empty() && !self.sustain_on {
            self.chord_latched = false;
            self.chord = None;
        }
        let desc = self
            .current_pattern()
            .map(|p| format!(" · {}", p.description))
            .unwrap_or_default();
        self.notify(format!("Style: {}{desc}", styles[idx].1));
    }

    fn set_tempo(&mut self, bpm: f32) {
        self.cfg.tempo = bpm.clamp(40.0, 240.0).round();
        self.send(Command::Tempo(self.cfg.tempo));
        self.mark_dirty();
    }

    fn rhythm_running(&self) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|e| e.stats.seq_running.load(Ordering::Relaxed))
    }

    fn toggle_rhythm(&mut self) {
        if self.rhythm_running() {
            self.send(Command::Rhythm(false));
            self.chord_latched = false;
            if self.chord_keys.is_empty() {
                self.chord = None;
            }
            return;
        }
        if !self.rhythmic() {
            // Beginner shortcut: pick the first pattern so something happens.
            let idx = FIXED_STYLES.len();
            if idx < self.styles().len() {
                self.set_style(idx);
            }
        }
        self.send(Command::Rhythm(true));
        if let (Some(c), true) = (self.last_chord_ref, self.chord_keys.is_empty()) {
            // Restart with the last chord so the rhythm isn't just drums.
            if self.style_kind() == StyleKind::Arp
                || self.current_pattern().is_some_and(|p| !p.has_drums)
            {
                self.play_chord(c, true);
                self.release_chord_engine();
            }
        }
    }

    fn reload_patterns(&mut self) {
        let (list, warnings) = config::load_patterns();
        let old = std::mem::replace(&mut self.patterns, list.into_iter().map(Arc::new).collect());
        self.retired_patterns.extend(old);
        self.pattern_warnings = warnings;
        self.send_style();
        self.notify(format!("{} patterns loaded", self.patterns.len()));
    }

    // ---------------------------------------------------------------- chords

    fn tonic_pc(&self) -> i32 {
        self.cfg.transpose.rem_euclid(12)
    }

    fn chord_mods(&self) -> u8 {
        self.mod_keys.iter().fold(0, |a, (_, m)| a | m.bit())
    }

    /// Where chords sit: around G3 at octave 0, whatever the key.
    fn chord_center(&self) -> i32 {
        BASE_NOTE + 12 * self.cfg.octave + 7
    }

    /// Work out and send a chord. `retrigger` = a fresh key press.
    fn play_chord(&mut self, cref: ChordRef, retrigger: bool) {
        let chord = build_chord(cref, self.chord_mods(), self.tonic_pc(), self.cfg.scale);
        let voicing = voice_chord(
            &chord,
            &self.voicing,
            self.chord_center(),
            self.cfg.voice_leading,
        );
        let bass = self
            .cfg
            .chord_bass
            .then(|| BASE_NOTE - 12 + 12 * self.cfg.octave + chord.root_pc);
        self.send(Command::Chord {
            chord: Some(ChordNotes::new(&voicing, bass)),
            retrigger,
            velocity: self.velocity() as u8,
        });
        self.voicing = voicing;
        self.bass_note = bass;
        self.chord = Some(chord);
        self.last_chord_ref = Some(cref);
    }

    /// Tell the engine no chord key is held any more.
    fn release_chord_engine(&mut self) {
        self.send(Command::Chord {
            chord: None,
            retrigger: false,
            velocity: self.velocity() as u8,
        });
        self.chord_latched = (self.rhythmic() && self.cfg.latch) || self.sustain_on;
        if !self.chord_latched {
            self.chord = None;
        }
    }

    /// Re-work the sounding chord after a modifier, key, scale or octave
    /// change, without re-striking it.
    fn refresh_chord(&mut self) {
        if let Some((_, c)) = self.chord_keys.last().copied() {
            self.play_chord(c, false);
        } else if self.chord_latched
            && let Some(c) = self.last_chord_ref
        {
            self.play_chord(c, false);
            self.send(Command::Chord {
                chord: None,
                retrigger: false,
                velocity: self.velocity() as u8,
            });
        }
    }

    fn change_key(&mut self, delta: i32) {
        self.cfg.transpose = (self.cfg.transpose + delta).clamp(-12, 12);
        self.mark_dirty();
        self.refresh_chord();
    }

    fn key_name(&self) -> String {
        format!(
            "{} {}",
            crate::theory::pc_name(
                self.tonic_pc(),
                self.cfg.scale.prefers_flats(self.tonic_pc())
            ),
            self.cfg.scale.name().to_lowercase()
        )
    }

    /// The note a `note`/`melody` action plays right now.
    fn action_note(&self, a: Action) -> Option<i32> {
        match a {
            Action::Note(n) => {
                let raw = self.base_note() + n;
                Some(if self.cfg.scale_lock {
                    self.cfg.scale.snap(self.tonic_pc(), raw)
                } else {
                    raw
                })
            }
            Action::Melody(n) => Some(self.cfg.scale.note(self.base_note() + 12, n)),
            _ => None,
        }
    }

    /// What a key cap shows for an action, in the current key and with the
    /// modifiers held right now.
    fn cap_label(&self) -> impl Fn(Action) -> CapLabel + '_ {
        let tonic = self.tonic_pc();
        let scale = self.cfg.scale;
        let mods = self.chord_mods();
        let base = self.base_note();
        let show_scale = self.cfg.show_scale;
        move |a: Action| match a {
            Action::Note(_) | Action::Melody(_) => {
                let note = self.action_note(a).unwrap_or(base);
                let style = CapStyle::Note {
                    black: crate::music::is_black(note),
                };
                let sub = match a {
                    // Scale step number (1 = home note of the key).
                    Action::Melody(n) => (n.rem_euclid(scale.steps().len() as i32) + 1).to_string(),
                    // Dot = in the scale, star = the key's home note.
                    _ if show_scale && (note - tonic).rem_euclid(12) == 0 => "★".into(),
                    _ if show_scale && scale.contains(tonic, note) => "•".into(),
                    _ => String::new(),
                };
                CapLabel {
                    main: note_name(note),
                    sub,
                    style,
                }
            }
            Action::Chord(c) => {
                let ch = build_chord(c, mods, tonic, scale);
                CapLabel {
                    main: ch.name,
                    sub: ch.numeral,
                    style: match c {
                        ChordRef::Diatonic(_) | ChordRef::Named { .. } => CapStyle::Chord,
                        ChordRef::Fixed { .. } => CapStyle::Borrowed,
                    },
                }
            }
            Action::Mod(m) => CapLabel {
                main: m.short().into(),
                sub: String::new(),
                style: CapStyle::Mod {
                    on: mods & m.bit() != 0,
                },
            },
            Action::Drum(d) => CapLabel {
                main: crate::drums::short(d),
                sub: String::new(),
                style: CapStyle::Drum,
            },
            other => CapLabel::simple(other, base),
        }
    }

    /// Key names bound to an action, e.g. "Shift / 1", for help text.
    fn keys_text(&self, a: Action) -> String {
        let keys = self.keymap.keys_for(a);
        if keys.is_empty() {
            "(no key)".into()
        } else {
            keys.iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(" / ")
        }
    }

    // ----------------------------------------------------------------- input

    /// Handle one key event as an instrument. Returns true if consumed.
    fn play_key(&mut self, key: KeyId, pressed: bool, repeat: bool) -> bool {
        // Releases always go through so notes never get stuck, using what
        // the key started rather than what it's bound to now.
        if !pressed {
            let mut used = false;
            if let Some(note) = self.sounding.remove(&key) {
                if let Some(e) = &self.engine {
                    e.note_off(note);
                }
                used = true;
            }
            if let Some(i) = self.chord_keys.iter().position(|(k, _)| *k == key) {
                let was_top = i + 1 == self.chord_keys.len();
                self.chord_keys.remove(i);
                if was_top {
                    match self.chord_keys.last().copied() {
                        Some((_, c)) => self.play_chord(c, true),
                        None => self.release_chord_engine(),
                    }
                }
                used = true;
            }
            if let Some(i) = self.mod_keys.iter().position(|(k, _)| *k == key) {
                self.mod_keys.remove(i);
                self.refresh_chord();
                used = true;
            }
            if self.keymap.get(key) == Some(Action::Sustain) {
                self.sustain_on = false;
                self.send(Command::Sustain(false));
                if self.chord_keys.is_empty() && !(self.rhythmic() && self.cfg.latch) {
                    self.chord_latched = false;
                    self.chord = None;
                }
            }
            return used || self.keymap.get(key).is_some();
        }
        let Some(action) = self.keymap.get(key) else {
            return false;
        };
        if repeat {
            return true;
        }
        match action {
            Action::Note(_) | Action::Melody(_) => {
                if self.sounding.contains_key(&key) {
                    return true;
                }
                let note = self.action_note(action).unwrap_or(0).clamp(0, 127);
                if let Some(e) = &self.engine {
                    e.note_on(note, self.velocity());
                }
                self.sounding.insert(key, note);
                self.learn_event(crate::learn::ear::Played::Note(note));
                self.song_event(crate::song::practice::Played::Note(note));
            }
            Action::Chord(c) => {
                if self.chord_keys.iter().any(|(k, _)| *k == key) {
                    return true;
                }
                self.chord_keys.push((key, c));
                self.chord_latched = false;
                self.play_chord(c, true);
                if let Some(ev) = self.chord_event() {
                    self.learn_event(ev);
                }
                if let Some(ev) = self.chord_played() {
                    self.song_event(ev);
                }
            }
            Action::Mod(m) => {
                if !self.mod_keys.iter().any(|(k, _)| *k == key) {
                    self.mod_keys.push((key, m));
                    self.refresh_chord();
                }
            }
            Action::Drum(d) => {
                self.send(Command::Drum {
                    note: d,
                    velocity: self.velocity() as u8,
                });
                self.last_drum = Some((d, Instant::now()));
                self.drum_hits.insert(d, Instant::now());
                self.song_event(crate::song::practice::Played::Drum(d));
            }
            Action::Sustain => {
                self.sustain_on = true;
                self.send(Command::Sustain(true));
            }
            Action::OctaveUp => {
                self.cfg.octave = (self.cfg.octave + 1).min(3);
                self.mark_dirty();
                self.refresh_chord();
            }
            Action::OctaveDown => {
                self.cfg.octave = (self.cfg.octave - 1).max(-3);
                self.mark_dirty();
                self.refresh_chord();
            }
            Action::TransposeUp => self.change_key(1),
            Action::TransposeDown => self.change_key(-1),
            Action::InstrumentNext => self.set_instrument(self.cfg.instrument + 1),
            Action::InstrumentPrev => {
                self.set_instrument(self.cfg.instrument + INSTRUMENTS.len() - 1)
            }
            Action::Velocity => {
                self.cfg.velocity = (self.cfg.velocity + 1) % 3;
                self.mark_dirty();
            }
            Action::Panic => {
                self.all_notes_off();
                self.send(Command::Loop(crate::audio::looper::LoopCommand::Stop));
            }
            Action::StyleNext => self.set_style(self.style_index() + 1),
            Action::StylePrev => {
                let n = self.styles().len();
                self.set_style(self.style_index() + n - 1)
            }
            Action::Rhythm => self.toggle_rhythm(),
            Action::TempoUp => {
                self.set_tempo(self.cfg.tempo + 5.0);
                self.notify(format!("Tempo {:.0}", self.cfg.tempo));
            }
            Action::TempoDown => {
                self.set_tempo(self.cfg.tempo - 5.0);
                self.notify(format!("Tempo {:.0}", self.cfg.tempo));
            }
            Action::ScaleNext => {
                self.cfg.scale = self.cfg.scale.next();
                self.mark_dirty();
                self.refresh_chord();
                self.notify(format!(
                    "Scale: {} ({})",
                    self.key_name(),
                    self.cfg.scale.mood()
                ));
            }
            Action::ScaleLock => {
                self.cfg.scale_lock = !self.cfg.scale_lock;
                self.mark_dirty();
                self.notify(if self.cfg.scale_lock {
                    "Scale lock on: every key plays a note of the scale"
                } else {
                    "Scale lock off"
                });
            }
            Action::LayoutNext => self.next_layout(),
            Action::Metronome => self.toggle_metronome(),
            Action::TapTempo => self.tap_tempo(),
            Action::Record => self.toggle_record(),
            Action::Loop => self.loop_rec(),
            Action::LoopPlay => self.loop_play(),
            Action::LoopUndo => self.loop_undo(),
            Action::LoopClear => self.loop_clear(),
            Action::LoopMute(n) => self.loop_mute(n),
        }
        true
    }

    fn next_layout(&mut self) {
        let i = BUILTIN_LAYOUTS
            .iter()
            .position(|(f, _)| *f == self.keymap_file)
            .map(|i| (i + 1) % BUILTIN_LAYOUTS.len())
            .unwrap_or(0);
        let (file, name) = BUILTIN_LAYOUTS[i];
        self.switch_keymap(file);
        self.notify(format!("Layout: {name}"));
    }

    /// Rebind capture: the next key pressed gets `action`. `from` is the key
    /// that had it before; if the new key already did something else, the
    /// two keys swap so nothing silently loses its key.
    fn capture_key(&mut self, key: KeyId, from: Option<KeyId>, action: Action) {
        self.capture_for = None;
        if key == KeyId::Key(Key::Escape) {
            self.notify("Rebind cancelled");
            return;
        }
        let previous = self.keymap.bind(key, action);
        let msg = match (from, previous) {
            (Some(old), Some(prev)) if old != key && prev != action => {
                self.keymap.bind(old, prev);
                format!(
                    "{} is now “{}”. It used to be “{}”, so that moved to {}.",
                    key.name(),
                    action.describe(),
                    prev.describe(),
                    old.name()
                )
            }
            (Some(old), _) if old != key => {
                self.keymap.unbind(old);
                format!(
                    "{} → {} (moved from {})",
                    key.name(),
                    action.describe(),
                    old.name()
                )
            }
            (None, Some(prev)) if prev != action => format!(
                "{} is now “{}” (was “{}”, which no longer has a key).",
                key.name(),
                action.describe(),
                prev.describe()
            ),
            _ => format!("{} → {}", key.name(), action.describe()),
        };
        self.notify(msg);
        self.selected_key = Some(key);
        self.save_keymap();
    }

    fn save_keymap(&mut self) {
        if let Err(e) = config::save_keymap(&self.keymap_file, &self.keymap) {
            self.notify(format!("Couldn't save keymap: {e}"));
        }
        self.keymap_watch.sync();
    }

    fn switch_keymap(&mut self, file: &str) {
        self.all_notes_off();
        self.mod_keys.clear();
        let (km, w) = config::load_keymap(file);
        self.keymap = km;
        self.keymap_warnings = w;
        self.keymap_file = file.to_string();
        self.keymap_watch.set_path(config::keymap_path(file));
        self.cfg.keymap = file.to_string();
        self.selected_key = None;
        self.mark_dirty();
    }

    /// Generate press/release events for Shift / Ctrl / Alt, which egui only
    /// reports as a modifier state.
    fn sync_modifiers(&mut self, m: egui::Modifiers, typing: bool) {
        for (id, down) in [
            (KeyId::Shift, m.shift),
            (KeyId::Ctrl, m.ctrl || m.command),
            (KeyId::Alt, m.alt),
        ] {
            if down == self.held.contains(&id) {
                continue;
            }
            self.handle_input(id, down, false, egui::Modifiers::NONE, typing);
        }
    }

    /// Route one key press/release. Returns true if it was used (so egui
    /// shouldn't see it).
    fn handle_input(
        &mut self,
        k: KeyId,
        pressed: bool,
        repeat: bool,
        modifiers: egui::Modifiers,
        typing: bool,
    ) -> bool {
        if pressed {
            self.held.insert(k);
        } else {
            self.held.remove(&k);
        }
        if self.tab == Tab::KeyTest {
            if self.held.len() > self.max_held {
                self.max_held = self.held.len();
                self.best_combo = self.held.iter().copied().collect();
                self.best_combo.sort_by_key(|k| k.name());
            }
            // Still release notes that were playing before switching tab.
            if !pressed {
                self.play_key(k, false, false);
            }
            return true;
        }
        if let Some((from, action)) = self.capture_for {
            if pressed && !repeat {
                self.capture_key(k, from, action);
            }
            return true;
        }
        if self.tap.running && k == KeyId::Key(Key::Space) {
            if pressed && !repeat {
                self.tap.tap(Instant::now());
            }
            return true;
        }
        // Learn tab: F2 hear again, F3 next (unless the layout uses them).
        if self.tab == Tab::Learn
            && pressed
            && self.keymap.get(k).is_none()
            && matches!(k, KeyId::Key(Key::F2 | Key::F3))
        {
            if !repeat {
                if k == KeyId::Key(Key::F2) {
                    self.learn_replay();
                } else {
                    self.learn_next();
                }
            }
            return true;
        }
        // Song tab: play/pause and friends, one-key mode, the note editor.
        if !typing && self.song_key(k, pressed, repeat, modifiers) {
            return true;
        }
        // Ctrl/Alt + key is a shortcut unless this layout uses Ctrl/Alt.
        let shortcut = ((modifiers.ctrl || modifiers.command)
            && self.keymap.get(KeyId::Ctrl).is_none())
            || (modifiers.alt && self.keymap.get(KeyId::Alt).is_none());
        if typing || shortcut {
            // Let shortcuts and text fields work, but never leave notes hanging.
            if !pressed {
                self.play_key(k, false, false);
            }
            return false;
        }
        self.play_key(k, pressed, repeat)
    }

    // -------------------------------------------------------------- polling

    fn poll(&mut self, ctx: &egui::Context) {
        // SoundFont load results.
        while let Ok(res) = self.sf_rx.try_recv() {
            match res {
                Ok(name) => {
                    self.sf_status = format!("Using SoundFont: {name}");
                    self.send(Command::Program(self.instrument().program));
                    self.send_programs();
                    self.send_style();
                }
                Err(e) => {
                    self.sf_status = format!("SoundFont failed ({e}); using the built-in synth")
                }
            }
        }
        // Metronome clicks for the latency test.
        if let Some(engine) = &self.engine {
            while let Ok(at) = engine.clicks.try_recv() {
                self.tap.click(at);
            }
        }
        if self.tap.running && self.tap.done() {
            self.finish_tap_test();
        }
        // Recording (Part 4a): collect what the audio thread heard.
        if self.drain_recording() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // Lesson / ear-training examples.
        if self.tick_player() {
            ctx.request_repaint_after(Duration::from_millis(10));
        }
        // Song mode: clock, scoring, analyzer progress.
        if self.song_poll(ctx) {
            ctx.request_repaint_after(Duration::from_millis(15));
        }

        // Debounced config save.
        if let Some(since) = self.cfg_dirty_since
            && since.elapsed() > Duration::from_millis(400)
        {
            self.cfg_dirty_since = None;
            if let Err(e) = self.cfg.save() {
                self.notify(format!("Couldn't save settings: {e}"));
            }
            self.cfg_watch.sync();
        }

        // Device changes.
        let dev = self.device.lock().unwrap().clone();
        if dev.as_ref().map(|d| &d.sink) != self.current_device.as_ref().map(|d| &d.sink) {
            self.on_device_changed(dev.clone());
        }
        self.current_device = dev;

        // File watching, once a second.
        if self.last_poll.elapsed() > Duration::from_secs(1) {
            self.last_poll = Instant::now();
            if self.cfg_watch.changed() {
                let (cfg, err) = Config::load();
                if let Some(e) = err {
                    self.notify(e);
                } else {
                    let restart = cfg.buffer_frames != self.cfg.buffer_frames;
                    let keymap_changed = cfg.keymap != self.keymap_file;
                    let midi_changed = (cfg.midi_out, cfg.midi_out_song, cfg.midi_out_local)
                        != (
                            self.cfg.midi_out,
                            self.cfg.midi_out_song,
                            self.cfg.midi_out_local,
                        );
                    self.cfg = cfg;
                    if midi_changed && !restart {
                        self.apply_midi_out();
                    }
                    self.send(Command::Program(self.instrument().program));
                    self.send(Command::Volume(self.cfg.volume));
                    self.send_programs();
                    self.send_style();
                    if keymap_changed {
                        let k = self.cfg.keymap.clone();
                        self.switch_keymap(&k);
                    }
                    if restart {
                        self.start_engine();
                    }
                    self.notify("Reloaded config.toml");
                }
            }
            if self.keymap_watch.changed() {
                let (km, w) = config::load_keymap(&self.keymap_file);
                self.all_notes_off();
                self.keymap = km;
                self.keymap_warnings = w;
                self.notify("Reloaded keymap");
            }
            if self.cfg.follow_system_theme {
                let changed = match &mut self.theme_watch {
                    Some(w) => w.changed(),
                    None => theme::find_system_palette_file().is_some(),
                };
                if changed && let Some((p, path)) = theme::load_system_palette() {
                    self.palette = p;
                    theme::apply(ctx, &p);
                    if self.theme_file.as_ref() != Some(&path) {
                        self.theme_watch = Some(FileWatch::new(path.clone()));
                        self.theme_file = Some(path);
                    }
                }
            }
        }
    }

    fn on_device_changed(&mut self, dev: Option<OutputInfo>) {
        let Some(d) = dev else { return };
        // The chord-change grace window follows the device's delay.
        self.current_device = Some(d.clone());
        self.send_style();
        let name = self.device_name(&d);
        match self.cfg.devices.get(&d.sink).cloned() {
            Some(profile) => {
                if d.bluetooth
                    && self.cfg.bluetooth_auto_instrument
                    && let Some(i) = profile.instrument
                {
                    self.set_instrument(i);
                }
                self.notify(format!(
                    "Output: {name} (measured delay {:.0} ms)",
                    profile.latency_ms
                ));
            }
            None if d.bluetooth => {
                self.notify(format!(
                    "Bluetooth: {name}. Run the latency test in the Audio tab once for this device."
                ));
            }
            None => self.notify(format!("Output: {name}")),
        }
    }

    /// Your name for a device if you renamed it, else what PipeWire calls it.
    fn device_name(&self, d: &OutputInfo) -> String {
        match self.cfg.devices.get(&d.sink) {
            Some(prof) if !prof.label.is_empty() => prof.label.clone(),
            _ if !d.description.is_empty() => d.description.clone(),
            _ => d.sink.clone(),
        }
    }

    fn start_tap_test(&mut self) {
        self.all_notes_off();
        self.tap.start();
        self.tap_result = None;
        self.send(Command::Metronome {
            bpm: TEST_BPM,
            on: true,
        });
    }

    fn finish_tap_test(&mut self) {
        self.tap.stop();
        self.send(Command::Metronome {
            bpm: TEST_BPM,
            on: false,
        });
        self.tap_result = self.tap.result();
        match self.tap_result {
            Some(r) => self.notify(format!(
                "Measured {:.0} ms (±{:.0})",
                r.latency_ms, r.spread_ms
            )),
            None => self.notify("Not enough taps to measure, try again"),
        }
    }

    // ------------------------------------------------------------------- UI

    fn header(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("♪ Qwertone")
                    .size(22.0)
                    .strong()
                    .color(p.primary),
            );
            ui.add_space(16.0);
            for (tab, label) in [
                (Tab::Play, "Play"),
                (Tab::Learn, "Learn"),
                (Tab::Song, "Song"),
                (Tab::Listen, "Listen"),
                (Tab::Keybinds, "Keybinds"),
                (Tab::Audio, "Audio & latency"),
                (Tab::KeyTest, "Keyboard test"),
            ] {
                let selected = self.tab == tab;
                let text = RichText::new(label).size(15.0).color(if selected {
                    p.on_secondary_container
                } else {
                    p.on_surface_variant
                });
                let btn = egui::Button::new(text)
                    .fill(if selected {
                        p.secondary_container
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .corner_radius(egui::CornerRadius::same(255));
                if ui.add(btn).clicked() {
                    if self.tab != tab {
                        self.stop_player();
                    }
                    self.tab = tab;
                    self.capture_for = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(d) = &self.current_device {
                    let mut text = if d.bluetooth {
                        "Bluetooth · ".to_string()
                    } else {
                        String::new()
                    };
                    text += &self.device_name(d);
                    if let Some(c) = &d.codec {
                        text += &format!(" · {}", c.to_uppercase());
                    }
                    if let Some(prof) = self.cfg.devices.get(&d.sink) {
                        text += &format!(" · {:.0} ms", prof.latency_ms);
                    }
                    let (fill, fg) = if d.bluetooth {
                        (p.tertiary_container, p.on_surface)
                    } else {
                        (p.surface_container_high, p.on_surface)
                    };
                    chip(ui, fill, fg, text);
                }
                if self.engine.is_none() {
                    chip(ui, p.error, p.surface, "audio off");
                }
                if self.loop_ui.midi.is_some() {
                    chip(ui, p.surface_container_high, p.on_surface, "MIDI out");
                }
            });
        });
    }

    fn audio_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(2, |cols| {
                // ---- Output device
                card(&mut cols[0], &p, "Output", |ui| match self.current_device.clone() {
                    None => {
                        ui.label("Couldn't ask PipeWire about the output (is `pactl` installed? It comes with `libpulse`).");
                    }
                    Some(d) => {
                        ui.label(RichText::new(self.device_name(&d)).size(18.0).strong());
                        ui.label(RichText::new(&d.sink).small().color(p.on_surface_variant));
                        ui.horizontal(|ui| {
                            chip(ui, p.surface_container_high, p.on_surface, if d.bluetooth { "Bluetooth" } else { "Wired / built-in" });
                            if let Some(c) = &d.codec {
                                chip(ui, p.surface_container_high, p.on_surface, format!("Codec {}", c.to_uppercase()));
                            }
                            chip(ui, p.surface_container_high, p.on_surface, format!("Typical delay ~{:.0} ms", d.typical_latency_ms()));
                        });
                        if d.bluetooth {
                            if let (Some(card_name), Some(best)) = (d.card.clone(), d.lowest_latency_profile()) {
                                ui.label("A lower-delay codec is available for these earbuds.");
                                if ui.button(format!("Switch to {best}")).clicked() {
                                    match devices::set_profile(&card_name, best) {
                                        Ok(()) => self.notify(format!("Switched to {best}. Re-run the latency test.")),
                                        Err(e) => self.notify(format!("Couldn't switch codec: {e}")),
                                    }
                                }
                            } else if d.codec.is_some() {
                                ui.label(RichText::new("Already on the lowest-delay codec these earbuds offer.").color(p.on_surface_variant));
                            }
                            ui.label(
                                RichText::new("LDAC sounds best for music but has the most delay. SBC is quickest for playing.")
                                    .small()
                                    .color(p.on_surface_variant),
                            );
                        }
                    }
                });

                // ---- Engine
                card(&mut cols[1], &p, "Audio engine", |ui| {
                    if let Some(err) = &self.engine_error {
                        ui.label(RichText::new(format!("Audio failed to start: {err}")).color(p.error));
                    }
                    if let Some(e) = &self.engine {
                        let s = &e.stats;
                        egui::Grid::new("engine").num_columns(2).show(ui, |ui| {
                            ui.label("Backend");
                            ui.label(s.host.lock().unwrap().clone());
                            ui.end_row();
                            ui.label("Device");
                            ui.label(s.device.lock().unwrap().clone());
                            ui.end_row();
                            ui.label("Sample rate");
                            ui.label(format!("{} Hz", s.sample_rate.load(std::sync::atomic::Ordering::Relaxed)));
                            ui.end_row();
                            ui.label("Buffer");
                            ui.label(format!(
                                "{} frames = {:.1} ms",
                                s.buffer_frames.load(std::sync::atomic::Ordering::Relaxed),
                                s.buffer_ms()
                            ));
                            ui.end_row();
                            ui.label("Reported output delay");
                            ui.label(format!("{:.1} ms", s.reported_ms()));
                            ui.end_row();
                            ui.label("Glitches / errors");
                            ui.label(s.errors.load(std::sync::atomic::Ordering::Relaxed).to_string());
                            ui.end_row();
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Buffer size");
                        let mut b = self.cfg.buffer_frames;
                        egui::ComboBox::from_id_salt("buf").selected_text(b.to_string()).show_ui(ui, |ui| {
                            for c in BUFFER_CHOICES {
                                ui.selectable_value(&mut b, c, format!("{c}{}", if c <= 128 { "  (low latency)" } else { "  (safer)" }));
                            }
                        });
                        if b != self.cfg.buffer_frames {
                            self.cfg.buffer_frames = b;
                            self.mark_dirty();
                            self.start_engine();
                        }
                        if ui.button("Restart audio").clicked() {
                            self.start_engine();
                        }
                    });
                    ui.label(RichText::new("Hear crackles? Pick a bigger buffer.").small().color(p.on_surface_variant));
                });
            });
            ui.add_space(8.0);

            ui.columns(2, |cols| {
                // ---- Latency test
                card(&mut cols[0], &p, "Latency test (tap along)", |ui| {
                    ui.label(
                        "Put on the headphones you want to measure. Press Start, then tap Space exactly when you \
                         HEAR each click. Keep going until it finishes. The app uses this number to line up songs and visuals \
                         with what you hear (later parts).",
                    );
                    ui.add_space(4.0);
                    if self.tap.running {
                        let n = self.tap.taps();
                        ui.add(egui::ProgressBar::new((n as f32 / (TARGET_TAPS + 3) as f32).min(1.0)).text(format!("{n} taps")));
                        if ui.button("Stop").clicked() {
                            self.finish_tap_test();
                        }
                    } else if ui.button("▶  Start test").clicked() {
                        self.start_tap_test();
                    }
                    if let Some(r) = self.tap_result {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{:.0} ms", r.latency_ms)).size(32.0).strong().color(p.primary));
                        let quality = if r.spread_ms < 25.0 { "steady taps, good result" } else if r.spread_ms < 50.0 { "okay, maybe run it again" } else { "taps were uneven, try again" };
                        ui.label(format!("±{:.0} ms · {} taps · {quality}", r.spread_ms, r.taps_used));
                        if let Some(d) = self.current_device.clone()
                            && ui.button(format!("Save for {}", self.device_name(&d))).clicked() {
                                let entry = self.cfg.devices.entry(d.sink.clone()).or_default();
                                entry.latency_ms = r.latency_ms;
                                if entry.label.is_empty() {
                                    entry.label = d.description.clone();
                                }
                                entry.bluetooth = d.bluetooth;
                                self.mark_dirty();
                                self.send_style();
                                self.notify("Saved");
                            }
                    }
                });

                // ---- Bluetooth profile + SoundFont
                card(&mut cols[1], &p, "Sound & device profile", |ui| {
                    ui.label(RichText::new(&self.sf_status).color(p.on_surface_variant));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.sf_path_input).hint_text("/path/to/sound.sf2").desired_width(260.0));
                        if ui.button("Load").clicked() {
                            let path = PathBuf::from(self.sf_path_input.trim());
                            self.cfg.soundfont = Some(path.clone());
                            self.mark_dirty();
                            self.load_soundfont(Some(path));
                        }
                        if ui.button("Built-in synth").clicked() {
                            self.send(Command::UseBuiltin);
                            self.sf_status = "Using the built-in synth".into();
                        }
                    });
                    ui.separator();
                    let mut auto = self.cfg.bluetooth_auto_instrument;
                    if ui.checkbox(&mut auto, "Switch instrument automatically for each device").changed() {
                        self.cfg.bluetooth_auto_instrument = auto;
                        self.mark_dirty();
                    }
                    if let Some(d) = self.current_device.clone() {
                        let prof = self.cfg.devices.get(&d.sink).cloned().unwrap_or_default();
                        ui.label(format!(
                            "This device: {}",
                            if prof.latency_ms > 0.0 { format!("{:.0} ms measured", prof.latency_ms) } else { "not measured yet".into() }
                        ));
                        let mut inst = prof.instrument;
                        egui::ComboBox::from_id_salt("devinst")
                            .selected_text(inst.map(|i| INSTRUMENTS[i.min(INSTRUMENTS.len() - 1)].name).unwrap_or("Keep current"))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut inst, None, "Keep current");
                                for (i, ins) in INSTRUMENTS.iter().enumerate() {
                                    ui.selectable_value(&mut inst, Some(i), format!("{}{}", ins.name, if ins.bt_friendly { "  ✦" } else { "" }));
                                }
                            });
                        if inst != prof.instrument {
                            let e = self.cfg.devices.entry(d.sink.clone()).or_insert_with(|| DeviceProfile {
                                label: d.description.clone(),
                                bluetooth: d.bluetooth,
                                ..Default::default()
                            });
                            e.instrument = inst;
                            self.mark_dirty();
                        }
                    }
                    let mut follow = self.cfg.follow_system_theme;
                    ui.separator();
                    if ui.checkbox(&mut follow, "Match end4 / wallpaper colours").changed() {
                        self.cfg.follow_system_theme = follow;
                        self.mark_dirty();
                        if !follow {
                            self.palette = Palette::default();
                            theme::apply(ui.ctx(), &self.palette);
                        } else {
                            self.theme_watch = None;
                            self.last_poll = Instant::now() - Duration::from_secs(5);
                        }
                    }
                    ui.label(
                        RichText::new(match &self.theme_file {
                            Some(f) if self.cfg.follow_system_theme => format!("Colours from {}", f.display()),
                            _ => "Using the built-in colours".into(),
                        })
                        .small()
                        .color(p.on_surface_variant),
                    );
                });
            });
            ui.add_space(8.0);
            self.midi_card(ui);
            ui.add_space(8.0);
            card(ui, &p, "Saved devices", |ui| self.saved_devices(ui));
        });
    }

    /// Every device in config.toml [devices], with rename / re-test / remove.
    fn saved_devices(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let connected = self.current_device.as_ref().map(|d| d.sink.clone());
        if self.cfg.devices.is_empty() {
            ui.label(
                RichText::new(
                    "No devices saved yet. Run the latency test above with your headphones on and \
                     press Save; each pair you save shows up here.",
                )
                .color(p.on_surface_variant),
            );
        }
        if let Some(sink) = &connected
            && !self.cfg.devices.contains_key(sink)
            && !self.cfg.devices.is_empty()
        {
            ui.label(
                RichText::new(
                    "The device you're using now isn't saved yet: run the test and press Save.",
                )
                .color(p.tertiary),
            );
        }
        let mut remove = None;
        let mut retest = false;
        let mut rename_done: Option<(String, String)> = None;
        let want_focus = std::mem::take(&mut self.rename_focus);
        let mut start_rename = None;
        let devices: Vec<(String, DeviceProfile)> = self
            .cfg
            .devices
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        egui::Grid::new("saved-devices")
            .striped(true)
            .num_columns(5)
            .spacing([18.0, 8.0])
            .show(ui, |ui| {
                for h in ["Device", "Type", "Delay", "Instrument", ""] {
                    ui.label(RichText::new(h).small().color(p.on_surface_variant));
                }
                ui.end_row();
                for (sink, prof) in devices {
                    let is_now = connected.as_deref() == Some(sink.as_str());
                    // Name (or rename box).
                    ui.vertical(|ui| {
                        match &mut self.renaming {
                            Some((s, text)) if *s == sink => {
                                ui.horizontal(|ui| {
                                    let r = ui
                                        .add(egui::TextEdit::singleline(text).desired_width(180.0));
                                    if want_focus {
                                        r.request_focus();
                                    }
                                    let enter =
                                        r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                                    if ui.small_button("Save").clicked() || enter {
                                        rename_done = Some((sink.clone(), text.trim().to_string()));
                                    }
                                    if ui.small_button("Cancel").clicked() {
                                        rename_done = Some((sink.clone(), prof.label.clone()));
                                    }
                                });
                            }
                            _ => {
                                ui.horizontal(|ui| {
                                    let name = if prof.label.is_empty() {
                                        &sink
                                    } else {
                                        &prof.label
                                    };
                                    ui.label(RichText::new(name).strong());
                                    if is_now {
                                        chip(
                                            ui,
                                            p.primary_container,
                                            p.on_primary_container,
                                            "● connected",
                                        );
                                    }
                                });
                            }
                        }
                        ui.add(
                            egui::Label::new(RichText::new(&sink).small().color(p.outline))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                    ui.label(if prof.bluetooth {
                        "Bluetooth"
                    } else {
                        "Wired / built-in"
                    });
                    ui.label(if prof.latency_ms > 0.0 {
                        format!("{:.0} ms", prof.latency_ms)
                    } else {
                        "not measured".into()
                    });
                    ui.label(
                        prof.instrument
                            .map(|i| INSTRUMENTS[i.min(INSTRUMENTS.len() - 1)].name)
                            .unwrap_or("keep current"),
                    );
                    ui.horizontal(|ui| {
                        if ui.small_button("Rename").clicked() {
                            let current = if prof.label.is_empty() {
                                sink.clone()
                            } else {
                                prof.label.clone()
                            };
                            start_rename = Some((sink.clone(), current));
                        }
                        let retest_btn = ui.add_enabled(
                            is_now && !self.tap.running,
                            egui::Button::new("Re-test").small(),
                        );
                        if retest_btn
                            .on_disabled_hover_text(
                                "Connect this device first (or wait for the running test)",
                            )
                            .on_hover_text("Runs the tap-along latency test above again")
                            .clicked()
                        {
                            retest = true;
                        }
                        let confirming = self.confirm_remove.as_deref() == Some(sink.as_str());
                        let label = if confirming {
                            "Really remove?"
                        } else {
                            "Remove"
                        };
                        let btn = egui::Button::new(RichText::new(label).color(if confirming {
                            p.surface
                        } else {
                            p.error
                        }))
                        .small()
                        .fill(if confirming {
                            p.error
                        } else {
                            Color32::TRANSPARENT
                        });
                        if ui.add(btn).clicked() {
                            if confirming {
                                remove = Some(sink.clone());
                            } else {
                                self.confirm_remove = Some(sink.clone());
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        if let Some(r) = start_rename {
            self.renaming = Some(r);
            self.rename_focus = true;
        }
        if let Some((sink, name)) = rename_done {
            if let Some(prof) = self.cfg.devices.get_mut(&sink) {
                prof.label = name;
                self.mark_dirty();
            }
            self.renaming = None;
        }
        if let Some(sink) = remove {
            if let Some(prof) = self.cfg.devices.remove(&sink) {
                let name = if prof.label.is_empty() {
                    sink
                } else {
                    prof.label
                };
                self.notify(format!(
                    "Removed {name}. It'll use defaults until you test it again."
                ));
                self.mark_dirty();
                self.send_style();
            }
            self.confirm_remove = None;
        }
        if retest {
            self.start_tap_test();
            self.notify("Latency test started: tap Space each time you hear a click");
        }
    }

    fn keytest_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        card(ui, &p, "Keyboard test (ghosting)", |ui| {
            ui.label(
                "Laptop keyboards can't always detect several keys held at once. Hold keys down one at a time: \
                 if a key you're holding doesn't light up, your keyboard can't do that combination. \
                 That's why the Chords + Melody layout plays a whole chord from one key.",
            );
            ui.horizontal(|ui| {
                chip(
                    ui,
                    p.primary_container,
                    p.on_primary_container,
                    format!("Holding {} keys", self.held.len()),
                );
                chip(
                    ui,
                    p.surface_container_high,
                    p.on_surface,
                    format!("Most at once: {}", self.max_held),
                );
                if !self.best_combo.is_empty() {
                    let names: Vec<&str> = self.best_combo.iter().map(|k| k.name()).collect();
                    chip(
                        ui,
                        p.surface_container_high,
                        p.on_surface,
                        format!("Best combo: {}", names.join(" + ")),
                    );
                }
                if ui.button("Reset").clicked() {
                    self.max_held = 0;
                    self.best_combo.clear();
                }
            });
        });
        ui.add_space(8.0);
        let label = self.cap_label();
        KeyboardView {
            keymap: &self.keymap,
            palette: &p,
            held: &self.held,
            selected: None,
            show_notes: false,
            label: &label,
            hints: &HashMap::new(),
        }
        .show(ui);
        ui.label(
            RichText::new(
                "Sound is muted on this tab. Shift, Ctrl and Alt are tested (left and right \
                 count as the same key); Caps and Super aren't.",
            )
            .color(p.on_surface_variant),
        );
    }
}

impl eframe::App for QwertoneApp {
    /// Runs before the UI each frame, with the raw events. Instrument keys
    /// are handled here and removed so egui never sees them (no Tab focus
    /// jumps, no Space clicking buttons).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let typing = ctx.egui_wants_keyboard_input();
        let mut keep = Vec::with_capacity(raw.events.len());
        for ev in std::mem::take(&mut raw.events) {
            match &ev {
                Event::WindowFocused(false) => {
                    // Alt-tab with keys held would leave notes stuck.
                    self.held.clear();
                    self.mod_keys.clear();
                    self.all_notes_off();
                    self.stop_player();
                    keep.push(ev);
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => {
                    // Shift/Ctrl/Alt first, so Shift+B plays B's chord as a 7th.
                    self.sync_modifiers(*modifiers, typing);
                    let k = KeyId::Key(physical_key.unwrap_or(*key));
                    if !self.handle_input(k, *pressed, *repeat, *modifiers, typing) {
                        keep.push(ev);
                    }
                }
                Event::ModifiersChanged(m) => {
                    // Shift / Ctrl / Alt pressed or released on their own.
                    self.sync_modifiers(*m, typing);
                    keep.push(ev);
                }
                _ => keep.push(ev),
            }
        }
        raw.events = keep;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        if self.update_overlay() {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        let p = self.palette;

        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(18, 12)),
            )
            .show(ui, |ui| self.header(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(22, 10)),
            )
            .show(ui, |ui| match self.tab {
                Tab::Play => self.play_tab(ui),
                Tab::Learn => self.learn_tab(ui),
                Tab::Song => self.song_tab(ui),
                Tab::Listen => self.listen_tab(ui),
                Tab::Keybinds => self.keybinds_tab(ui),
                Tab::Audio => self.audio_tab(ui),
                Tab::KeyTest => self.keytest_tab(ui),
            });

        // Toast.
        if let Some((msg, at)) = self.toast.clone() {
            if at.elapsed() < Duration::from_secs(5) {
                egui::Area::new(egui::Id::unique("qwertone-toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
                    .show(&ctx, |ui| {
                        egui::Frame::new()
                            .fill(p.surface_container_highest)
                            .corner_radius(egui::CornerRadius::same(16))
                            .inner_margin(egui::Margin::symmetric(18, 12))
                            .show(ui, |ui| {
                                // Size to the text, wrapping only long messages.
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let w = ui
                                    .painter()
                                    .layout_no_wrap(msg.clone(), font, p.on_surface)
                                    .size()
                                    .x;
                                ui.set_width(w.min(640.0));
                                ui.label(RichText::new(msg).color(p.on_surface))
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                self.toast = None;
            }
        }

        // Keep polling files/devices/clicks without burning CPU; redraw
        // often while the beat display is moving.
        let next = if self.tap.running || self.rhythm_running() {
            30
        } else {
            500
        };
        ctx.request_repaint_after(Duration::from_millis(next));
    }

    fn on_exit(&mut self) {
        let _ = self.cfg.save();
    }
}
