//! The Keybinds tab: pick a layout, click a key, change what it does.

use super::*;
use crate::keymap::ActionKind;
use crate::theory::{ChordMod, ChordRef};

impl QwertoneApp {
    pub(super) fn keybinds_tab(&mut self, ui: &mut egui::Ui) {
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
            let mut reset_to = None;
            ui.menu_button("Reset to…", |ui| {
                for (file, name) in BUILTIN_LAYOUTS {
                    if ui.button(name).clicked() {
                        reset_to = Some((file, name));
                        ui.close();
                    }
                }
            });
            if let Some((file, label)) = reset_to
                && let Some(km) = Keymap::builtin(file)
            {
                self.all_notes_off();
                self.mod_keys.clear();
                let name = self.keymap.name.clone();
                self.keymap = km;
                if !BUILTIN_LAYOUTS.iter().any(|(f, _)| *f == self.keymap_file) {
                    self.keymap.name = name;
                }
                self.save_keymap();
                self.notify(format!("Layout reset to the {label} defaults"));
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

        let label = self.cap_label();
        let clicked = KeyboardView {
            keymap: &self.keymap,
            palette: &p,
            held: &self.held,
            selected: self.selected_key,
            show_notes: true,
            label: &label,
            hints: &HashMap::new(),
        }
        .show(ui);
        drop(label);
        if let Some(k) = clicked {
            self.selected_key = Some(k);
        }
        ui.label(
            RichText::new(
                "Click a key to edit it (Shift, Ctrl and Alt too), or use “Rebind” below and press the new key.",
            )
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
                let changed = self.edit_binding(ui, key);
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
                                    let desc = self.binding_text(action);
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

    /// Plain text for a binding in the list.
    fn binding_text(&self, a: Action) -> String {
        match a {
            Action::Note(n) => {
                format!("{} (note {n})", note_name(self.action_note(a).unwrap_or(n)))
            }
            Action::Melody(n) => format!(
                "Melody {} (scale step {n})",
                note_name(self.action_note(a).unwrap_or(0))
            ),
            Action::Chord(c) => {
                let ch = build_chord(c, 0, self.tonic_pc(), self.cfg.scale);
                format!("Chord {} = {} now", c.code(), ch.name)
            }
            other => other.describe(),
        }
    }

    /// The "Selected key" editor. Returns Some(new binding) if changed.
    fn edit_binding(&mut self, ui: &mut egui::Ui, key: KeyId) -> Option<Option<Action>> {
        let p = self.palette;
        let current = self.keymap.get(key);
        let kind = current.map(|a| a.kind());
        let mut changed: Option<Option<Action>> = None;
        ui.horizontal_wrapped(|ui| {
            for (k, label, default) in [
                (ActionKind::Note, "Note", Action::Note(0)),
                (ActionKind::Melody, "Melody (in key)", Action::Melody(0)),
                (
                    ActionKind::Chord,
                    "Chord",
                    Action::Chord(ChordRef::Diatonic(1)),
                ),
                (
                    ActionKind::Mod,
                    "Chord modifier",
                    Action::Mod(ChordMod::Seventh),
                ),
                (ActionKind::Drum, "Drum", Action::Drum(36)),
                (ActionKind::Control, "Control", Action::Sustain),
            ] {
                if ui.radio(kind == Some(k), label).clicked() && kind != Some(k) {
                    changed = Some(Some(default));
                }
            }
            if ui.radio(current.is_none(), "Nothing").clicked() && current.is_some() {
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
                        RichText::new(format!(
                            "= {} now",
                            note_name(self.action_note(Action::Note(n)).unwrap_or(0))
                        ))
                        .color(p.primary),
                    );
                });
            }
            Some(Action::Melody(mut n)) => {
                ui.horizontal(|ui| {
                    ui.label("Scale step (0 = home note of the key):");
                    if ui
                        .add(egui::DragValue::new(&mut n).range(-14..=28))
                        .changed()
                    {
                        changed = Some(Some(Action::Melody(n)));
                    }
                    ui.label(
                        RichText::new(format!(
                            "= {} now",
                            note_name(self.action_note(Action::Melody(n)).unwrap_or(0))
                        ))
                        .color(p.primary),
                    );
                });
            }
            Some(Action::Chord(c)) => {
                let mut sel = c;
                let name = |r: ChordRef| {
                    let ch = build_chord(r, 0, self.tonic_pc(), self.cfg.scale);
                    match r {
                        ChordRef::Diatonic(d) => format!("Step {d}: {} ({})", ch.numeral, ch.name),
                        ChordRef::Fixed { .. } => format!("Borrowed {} ({})", r.code(), ch.name),
                        ChordRef::Named { .. } => {
                            format!("Song chord {} ({})", ch.name, ch.numeral)
                        }
                    }
                };
                egui::ComboBox::from_id_salt("chordref")
                    .width(240.0)
                    .selected_text(name(sel))
                    .show_ui(ui, |ui| {
                        for r in ChordRef::presets() {
                            ui.selectable_value(&mut sel, r, name(r));
                        }
                    });
                ui.label(
                    RichText::new(
                        "Steps follow the key and scale. Borrowed chords are fixed (bVII is always \
                         a whole step below the key). Edit the file for any roman numeral.",
                    )
                    .small()
                    .color(p.on_surface_variant),
                );
                if sel != c {
                    changed = Some(Some(Action::Chord(sel)));
                }
            }
            Some(Action::Mod(m)) => {
                let mut sel = m;
                egui::ComboBox::from_id_salt("chordmod")
                    .selected_text(sel.describe())
                    .show_ui(ui, |ui| {
                        for x in ChordMod::ALL {
                            ui.selectable_value(&mut sel, x, x.describe());
                        }
                    });
                if sel != m {
                    changed = Some(Some(Action::Mod(sel)));
                }
            }
            Some(Action::Drum(d)) => {
                let mut sel = d;
                egui::ComboBox::from_id_salt("drum")
                    .selected_text(crate::drums::name(sel))
                    .show_ui(ui, |ui| {
                        for x in crate::drums::DRUMS {
                            ui.selectable_value(&mut sel, x.note, x.name);
                        }
                    });
                if sel != d {
                    changed = Some(Some(Action::Drum(sel)));
                }
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
        changed
    }
}
