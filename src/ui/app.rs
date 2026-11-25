//! The main window: input handling, state, and the four tabs.

use crate::audio::{Command, Engine};
use crate::config::{self, Config, DeviceProfile, FileWatch};
use crate::devices::{self, OutputInfo};
use crate::keymap::{Action, Keymap};
use crate::latency::{TARGET_TAPS, TEST_BPM, TapResult, TapTest};
use crate::music::{INSTRUMENTS, note_name};
use crate::theme::{self, Palette};
use crate::ui::widgets::{KeyboardView, card, chip, now_playing, piano_strip};
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Event, Key, RichText};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VELOCITIES: [(i32, &str); 3] = [(60, "Soft"), (95, "Normal"), (122, "Loud")];
const BUFFER_CHOICES: [u32; 5] = [32, 64, 128, 256, 512];
/// MIDI note that `note 0` plays at octave 0 (C3).
const BASE_NOTE: i32 = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Play,
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
    held: HashSet<Key>,
    /// Key -> note it started, so releasing always stops the right note
    /// even if you changed octave in between.
    sounding: HashMap<Key, i32>,
    sustain_on: bool,

    // Keybind editor.
    selected_key: Option<Key>,
    /// Waiting for a key press to bind: (key it's moving from, action).
    capture_for: Option<(Option<Key>, Action)>,
    new_layout_name: String,

    // Latency test.
    tap: TapTest,
    tap_result: Option<TapResult>,

    // Keyboard ghosting test.
    max_held: usize,
    best_combo: Vec<Key>,

    toast: Option<(String, Instant)>,
    last_poll: Instant,
}

impl QwertoneApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        theme::install_fonts(ctx);
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
            selected_key: None,
            capture_for: None,
            new_layout_name: String::new(),
            tap: TapTest::default(),
            tap_result: None,
            max_held: 0,
            best_combo: Vec::new(),
            toast: None,
            last_poll: Instant::now(),
        };
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
        let program = self.instrument().program;
        match Engine::start(self.cfg.buffer_frames, program, self.cfg.volume) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.engine_error = None;
                self.load_soundfont(self.cfg.soundfont.clone());
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
        self.mark_dirty();
    }

    fn base_note(&self) -> i32 {
        BASE_NOTE + self.cfg.octave * 12 + self.cfg.transpose
    }

    fn all_notes_off(&mut self) {
        self.sounding.clear();
        self.sustain_on = false;
        self.send(Command::Sustain(false));
        self.send(Command::AllOff);
    }

    fn notify(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn mark_dirty(&mut self) {
        self.cfg_dirty_since.get_or_insert_with(Instant::now);
    }

    // ----------------------------------------------------------------- input

    /// Handle one key event as an instrument. Returns true if consumed.
    fn play_key(&mut self, key: Key, pressed: bool, repeat: bool) -> bool {
        // Releases always go through so notes never get stuck.
        if !pressed {
            if let Some(note) = self.sounding.remove(&key)
                && let Some(e) = &self.engine
            {
                e.note_off(note);
            }
            if self.keymap.get(key) == Some(Action::Sustain) {
                self.sustain_on = false;
                self.send(Command::Sustain(false));
            }
            return self.keymap.get(key).is_some();
        }
        let Some(action) = self.keymap.get(key) else {
            return false;
        };
        if repeat {
            return true;
        }
        match action {
            Action::Note(n) => {
                if self.sounding.contains_key(&key) {
                    return true;
                }
                let note = (self.base_note() + n).clamp(0, 127);
                let vel = VELOCITIES[self.cfg.velocity.min(2)].0;
                if let Some(e) = &self.engine {
                    e.note_on(note, vel);
                }
                self.sounding.insert(key, note);
            }
            Action::Sustain => {
                self.sustain_on = true;
                self.send(Command::Sustain(true));
            }
            Action::OctaveUp => {
                self.cfg.octave = (self.cfg.octave + 1).min(3);
                self.mark_dirty();
            }
            Action::OctaveDown => {
                self.cfg.octave = (self.cfg.octave - 1).max(-3);
                self.mark_dirty();
            }
            Action::TransposeUp => {
                self.cfg.transpose = (self.cfg.transpose + 1).min(12);
                self.mark_dirty();
            }
            Action::TransposeDown => {
                self.cfg.transpose = (self.cfg.transpose - 1).max(-12);
                self.mark_dirty();
            }
            Action::InstrumentNext => self.set_instrument(self.cfg.instrument + 1),
            Action::InstrumentPrev => {
                self.set_instrument(self.cfg.instrument + INSTRUMENTS.len() - 1)
            }
            Action::Velocity => {
                self.cfg.velocity = (self.cfg.velocity + 1) % 3;
                self.mark_dirty();
            }
            Action::Panic => self.all_notes_off(),
        }
        true
    }

    /// Rebind capture: the next key pressed gets `action`. `from` is the key
    /// that had it before; if the new key already did something else, the
    /// two keys swap so nothing silently loses its key.
    fn capture_key(&mut self, key: Key, from: Option<Key>, action: Action) {
        self.capture_for = None;
        if key == Key::Escape {
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
        let (km, w) = config::load_keymap(file);
        self.keymap = km;
        self.keymap_warnings = w;
        self.keymap_file = file.to_string();
        self.keymap_watch.set_path(config::keymap_path(file));
        self.cfg.keymap = file.to_string();
        self.selected_key = None;
        self.mark_dirty();
    }

    // -------------------------------------------------------------- polling

    fn poll(&mut self, ctx: &egui::Context) {
        // SoundFont load results.
        while let Ok(res) = self.sf_rx.try_recv() {
            match res {
                Ok(name) => {
                    self.sf_status = format!("Using SoundFont: {name}");
                    self.send(Command::Program(self.instrument().program));
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
                    self.cfg = cfg;
                    self.send(Command::Program(self.instrument().program));
                    self.send(Command::Volume(self.cfg.volume));
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
        let name = if d.description.is_empty() {
            d.sink.clone()
        } else {
            d.description.clone()
        };
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
                    text += if d.description.is_empty() {
                        &d.sink
                    } else {
                        &d.description
                    };
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
            });
        });
    }

    fn play_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let sounding: Vec<i32> = self.sounding.values().copied().collect();
        ui.horizontal_wrapped(|ui| {
            // Instrument picker.
            let inst = self.instrument();
            let mut idx = self.cfg.instrument;
            egui::ComboBox::from_id_salt("instrument")
                .width(190.0)
                .selected_text(format!(
                    "{}{}",
                    inst.name,
                    if inst.bt_friendly { "  ✦" } else { "" }
                ))
                .show_ui(ui, |ui| {
                    for (i, ins) in INSTRUMENTS.iter().enumerate() {
                        let label = format!(
                            "{}{}",
                            ins.name,
                            if ins.bt_friendly {
                                "  ✦ Bluetooth-friendly"
                            } else {
                                ""
                            }
                        );
                        ui.selectable_value(&mut idx, i, label);
                    }
                });
            if idx != self.cfg.instrument {
                self.set_instrument(idx);
            }
            chip(
                ui,
                p.surface_container_high,
                p.on_surface,
                format!("Octave {:+}", self.cfg.octave),
            );
            chip(
                ui,
                p.surface_container_high,
                p.on_surface,
                format!(
                    "Key {}{}",
                    crate::music::pitch_class_name(self.cfg.transpose),
                    if self.cfg.transpose != 0 {
                        format!(" ({:+})", self.cfg.transpose)
                    } else {
                        String::new()
                    }
                ),
            );
            chip(
                ui,
                p.surface_container_high,
                p.on_surface,
                format!("Velocity: {}", VELOCITIES[self.cfg.velocity.min(2)].1),
            );
            let (sf, sc) = if self.sustain_on {
                (p.primary, p.on_primary)
            } else {
                (p.surface_container_high, p.on_surface_variant)
            };
            chip(
                ui,
                sf,
                sc,
                if self.sustain_on {
                    "Pedal ON"
                } else {
                    "Pedal off"
                },
            );
            ui.add_space(8.0);
            ui.label("Volume");
            let mut vol = self.cfg.volume;
            if ui
                .add(egui::Slider::new(&mut vol, 0.0..=1.5).show_value(false))
                .changed()
            {
                self.cfg.volume = vol;
                self.send(Command::Volume(vol));
                self.mark_dirty();
            }
        });
        ui.add_space(6.0);

        card(ui, &p, "", |ui| now_playing(ui, &p, &sounding));
        ui.add_space(6.0);

        let base = self.base_note();
        KeyboardView {
            keymap: &self.keymap,
            palette: &p,
            base_note: base,
            held: &self.held,
            selected: None,
            show_notes: true,
        }
        .show(ui);
        ui.add_space(8.0);
        let set: HashSet<i32> = sounding.iter().copied().collect();
        piano_strip(ui, &p, BASE_NOTE + self.cfg.octave * 12, 3, &set);
        ui.add_space(6.0);

        let bt = self.current_device.as_ref().is_some_and(|d| d.bluetooth);
        ui.label(
            RichText::new(format!(
                "Z-row + A-row = lower octave (starts at {}), Q-row + number row = upper octave. \
                 Space = sustain pedal · ↑↓ octave · ←→ change key · PgUp/PgDn instrument · Tab velocity · Esc stop all.",
                note_name(base)
            ))
            .color(p.on_surface_variant),
        );
        if bt {
            ui.label(
                RichText::new(
                    "On Bluetooth: sounds marked ✦ (pads, strings, organ) hide the delay best. \
                     Wired earphones remove it almost completely.",
                )
                .color(p.tertiary),
            );
        }
    }

    fn keybinds_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal_wrapped(|ui| {
            ui.label("Layout");
            let mut chosen = self.keymap_file.clone();
            egui::ComboBox::from_id_salt("layout")
                .selected_text(format!("{} ({}.toml)", self.keymap.name, self.keymap_file))
                .show_ui(ui, |ui| {
                    for name in config::list_keymaps() {
                        ui.selectable_value(&mut chosen, name.clone(), name);
                    }
                });
            if chosen != self.keymap_file {
                self.switch_keymap(&chosen);
            }
            ui.add_space(12.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.new_layout_name)
                    .hint_text("new layout name")
                    .desired_width(150.0),
            );
            if ui.button("Save as new layout").clicked() {
                let name = self.new_layout_name.trim().to_string();
                if name.is_empty() {
                    self.notify("Type a name for the new layout first");
                } else {
                    let file = config::slug(&name);
                    self.keymap.name = name;
                    self.keymap_file = file.clone();
                    self.save_keymap();
                    self.switch_keymap(&file);
                    self.new_layout_name.clear();
                    self.notify(format!("Saved as {file}.toml"));
                }
            }
            if ui.button("Reset to Piano").clicked() {
                let name = self.keymap.name.clone();
                self.keymap = Keymap::piano();
                self.keymap.name = name;
                self.save_keymap();
                self.notify("Layout reset to the Piano defaults");
            }
            if ui.button("Open folder").clicked() {
                let _ = std::process::Command::new("xdg-open")
                    .arg(config::keymaps_dir())
                    .spawn();
            }
        });
        for w in &self.keymap_warnings {
            ui.label(RichText::new(format!("⚠ {w}")).color(p.error));
        }
        ui.add_space(6.0);

        if let Some((_, a)) = self.capture_for {
            card(ui, &p, "", |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!("Press a key for “{}”", a.describe()))
                            .size(20.0)
                            .color(p.primary),
                    );
                    ui.label("Esc to cancel");
                });
            });
            ui.add_space(6.0);
        }

        let clicked = KeyboardView {
            keymap: &self.keymap,
            palette: &p,
            base_note: self.base_note(),
            held: &self.held,
            selected: self.selected_key,
            show_notes: true,
        }
        .show(ui);
        if let Some(k) = clicked {
            self.selected_key = Some(k);
        }
        ui.label(
            RichText::new("Click a key to edit it, or use “Rebind” below and press the new key.")
                .color(p.on_surface_variant),
        );
        ui.add_space(6.0);

        ui.columns(2, |cols| {
            // Left: edit the selected key.
            card(&mut cols[0], &p, "Selected key", |ui| {
                let Some(key) = self.selected_key else {
                    ui.label("Nothing selected. Click a key on the keyboard above.");
                    return;
                };
                ui.label(RichText::new(key.name()).size(20.0).strong());
                let current = self.keymap.get(key);
                let mut changed: Option<Option<Action>> = None;
                let mut is_note = matches!(current, Some(Action::Note(_)));
                ui.horizontal(|ui| {
                    if ui.radio(is_note, "Plays a note").clicked() && !is_note {
                        is_note = true;
                        changed = Some(Some(Action::Note(0)));
                    }
                    if ui
                        .radio(
                            matches!(current, Some(a) if !matches!(a, Action::Note(_))),
                            "Control",
                        )
                        .clicked()
                    {
                        changed = Some(Some(Action::Sustain));
                    }
                    if ui.radio(current.is_none(), "Nothing").clicked() {
                        changed = Some(None);
                    }
                });
                match current {
                    Some(Action::Note(mut n)) => {
                        ui.horizontal(|ui| {
                            ui.label("Semitones above the layout's lowest C:");
                            if ui
                                .add(egui::DragValue::new(&mut n).range(-24..=48))
                                .changed()
                            {
                                changed = Some(Some(Action::Note(n)));
                            }
                            ui.label(
                                RichText::new(format!("= {} now", note_name(self.base_note() + n)))
                                    .color(p.primary),
                            );
                        });
                    }
                    Some(a) => {
                        let mut sel = a;
                        egui::ComboBox::from_id_salt("control")
                            .selected_text(sel.describe())
                            .show_ui(ui, |ui| {
                                for c in Action::CONTROLS {
                                    ui.selectable_value(&mut sel, c, c.describe());
                                }
                            });
                        if sel != a {
                            changed = Some(Some(sel));
                        }
                    }
                    None => {}
                }
                if let Some(new) = changed {
                    match new {
                        Some(a) => {
                            self.keymap.bind(key, a);
                        }
                        None => self.keymap.unbind(key),
                    }
                    self.save_keymap();
                }
            });
            // Right: list of everything, with rebind buttons.
            card(&mut cols[1], &p, "All bindings", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        let mut rebind = None;
                        let mut missing: Vec<Action> = Action::CONTROLS
                            .iter()
                            .copied()
                            .filter(|a| self.keymap.keys_for(*a).is_empty())
                            .collect();
                        egui::Grid::new("bindings")
                            .striped(true)
                            .num_columns(3)
                            .min_col_width(ui.available_width() / 3.3)
                            .show(ui, |ui| {
                                for (key, action) in self.keymap.sorted() {
                                    ui.label(RichText::new(key.name()).strong());
                                    let desc = match action {
                                        Action::Note(n) => format!(
                                            "{} (note {n})",
                                            note_name(self.base_note() + n)
                                        ),
                                        a => a.describe(),
                                    };
                                    ui.label(desc);
                                    if ui.small_button("Rebind").clicked() {
                                        rebind = Some((Some(key), action));
                                    }
                                    ui.end_row();
                                }
                                for a in missing.drain(..) {
                                    ui.label(RichText::new("—").color(p.outline));
                                    ui.label(RichText::new(a.describe()).color(p.outline));
                                    if ui.small_button("Assign").clicked() {
                                        rebind = Some((None, a));
                                    }
                                    ui.end_row();
                                }
                            });
                        if let Some(a) = rebind {
                            self.all_notes_off();
                            self.capture_for = Some(a);
                        }
                    });
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
                        ui.label(RichText::new(if d.description.is_empty() { &d.sink } else { &d.description }).size(18.0).strong());
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
                        self.all_notes_off();
                        self.tap.start();
                        self.tap_result = None;
                        self.send(Command::Metronome { bpm: TEST_BPM, on: true });
                    }
                    if let Some(r) = self.tap_result {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{:.0} ms", r.latency_ms)).size(32.0).strong().color(p.primary));
                        let quality = if r.spread_ms < 25.0 { "steady taps, good result" } else if r.spread_ms < 50.0 { "okay, maybe run it again" } else { "taps were uneven, try again" };
                        ui.label(format!("±{:.0} ms · {} taps · {quality}", r.spread_ms, r.taps_used));
                        if let Some(d) = self.current_device.clone()
                            && ui.button(format!("Save for {}", if d.description.is_empty() { &d.sink } else { &d.description })).clicked() {
                                let entry = self.cfg.devices.entry(d.sink.clone()).or_default();
                                entry.latency_ms = r.latency_ms;
                                entry.label = d.description.clone();
                                entry.bluetooth = d.bluetooth;
                                self.mark_dirty();
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
        });
    }

    fn keytest_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        card(ui, &p, "Keyboard test (ghosting)", |ui| {
            ui.label(
                "Laptop keyboards can't always detect several keys held at once. Hold keys down one at a time: \
                 if a key you're holding doesn't light up, your keyboard can't do that combination. \
                 That's why the chord layout (Part 2) plays a whole chord from one key.",
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
        KeyboardView {
            keymap: &self.keymap,
            palette: &p,
            base_note: self.base_note(),
            held: &self.held,
            selected: None,
            show_notes: false,
        }
        .show(ui);
        ui.label(
            RichText::new(
                "Sound is muted on this tab. Shift, Ctrl, Alt and Caps aren't tested here.",
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
                    self.all_notes_off();
                    keep.push(ev);
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => {
                    let k = physical_key.unwrap_or(*key);
                    let (pressed, repeat) = (*pressed, *repeat);
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
                        continue;
                    }
                    if let Some((from, action)) = self.capture_for {
                        if pressed && !repeat {
                            self.capture_key(k, from, action);
                        }
                        continue;
                    }
                    if self.tap.running && k == Key::Space {
                        if pressed && !repeat {
                            self.tap.tap(Instant::now());
                        }
                        continue;
                    }
                    if typing || modifiers.ctrl || modifiers.command || modifiers.alt {
                        // Let shortcuts and text fields work, but never leave notes hanging.
                        if !pressed {
                            self.play_key(k, false, false);
                        }
                        keep.push(ev);
                        continue;
                    }
                    if !self.play_key(k, pressed, repeat) {
                        keep.push(ev);
                    }
                }
                _ => keep.push(ev),
            }
        }
        raw.events = keep;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
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
                            .show(ui, |ui| ui.label(RichText::new(msg).color(p.on_surface)));
                    });
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                self.toast = None;
            }
        }

        // Keep polling files/devices/clicks without burning CPU.
        let next = if self.tap.running { 30 } else { 500 };
        ctx.request_repaint_after(Duration::from_millis(next));
    }

    fn on_exit(&mut self) {
        let _ = self.cfg.save();
    }
}
