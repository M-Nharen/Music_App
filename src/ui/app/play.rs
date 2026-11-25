//! The Play tab: layout picker, toolbar, chord / drum panels, keyboard.

use super::*;
use crate::audio::sequencer::ArpMode;
use crate::keymap::ActionKind;
use crate::theory::{Scale, pc_name};
use crate::ui::visuals::{self, Visual, VisualInput};
use crate::ui::widgets::{beat_dots, now_playing};

impl QwertoneApp {
    pub(super) fn play_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let has_chords = self.keymap.has_kind(ActionKind::Chord);
        let has_drums = self.keymap.has_kind(ActionKind::Drum);
        let has_notes = self.keymap.has_kind(ActionKind::Note)
            || self.keymap.has_kind(ActionKind::Melody)
            || has_chords;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.layout_bar(ui);
                ui.add_space(4.0);
                self.toolbar(ui, has_notes);
                ui.add_space(6.0);

                if has_chords {
                    self.chord_panel(ui);
                } else if has_drums && !has_notes {
                    self.drum_panel(ui);
                } else {
                    let sounding: Vec<i32> = self.sounding.values().copied().collect();
                    card(ui, &p, "", |ui| now_playing(ui, &p, &sounding));
                }
                ui.add_space(6.0);

                let label = self.cap_label();
                KeyboardView {
                    keymap: &self.keymap,
                    palette: &p,
                    held: &self.held,
                    selected: None,
                    show_notes: true,
                    label: &label,
                }
                .show(ui);
                drop(label);
                ui.add_space(8.0);

                self.instrument_visual(ui, has_chords, has_drums && !has_notes);
                ui.add_space(6.0);
                self.help_text(ui, has_chords);
            });
    }

    /// The picture that follows the instrument (or the one picked).
    fn visual(&self, drums_layout: bool) -> Visual {
        match Visual::from_code(&self.cfg.visual) {
            Some(v) => v,
            None if drums_layout => Visual::Drums,
            None => Visual::for_program(self.instrument().program),
        }
    }

    fn instrument_visual(&mut self, ui: &mut egui::Ui, has_chords: bool, drums_layout: bool) {
        let p = self.palette;
        let auto = Visual::from_code(&self.cfg.visual).is_none();
        let current = self.visual(drums_layout);
        ui.horizontal(|ui| {
            ui.label(RichText::new("View").color(p.on_surface_variant));
            let mut sel = if auto { None } else { Some(current) };
            let auto_name = if drums_layout {
                Visual::Drums
            } else {
                Visual::for_program(self.instrument().program)
            };
            egui::ComboBox::from_id_salt("visual")
                .width(220.0)
                .selected_text(match sel {
                    None => format!("Match instrument ({})", auto_name.name()),
                    Some(v) => v.name().to_string(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sel, None, "Match instrument");
                    for v in Visual::ALL {
                        ui.selectable_value(&mut sel, Some(v), v.name());
                    }
                });
            let code = sel.map(|v| v.code()).unwrap_or("auto");
            if code != self.cfg.visual {
                self.cfg.visual = code.to_string();
                self.mark_dirty();
            }
        });
        let melody: HashSet<i32> = self.sounding.values().copied().collect();
        let mut chord: HashSet<i32> = HashSet::new();
        if self.chord.is_some() {
            chord.extend(self.voicing.iter().copied());
            chord.extend(self.bass_note);
        }
        let (low, octaves) = if has_chords {
            (BASE_NOTE - 12 + self.cfg.octave * 12, 4)
        } else {
            (BASE_NOTE + self.cfg.octave * 12, 3)
        };
        let (tonic, scale) = (self.tonic_pc(), self.cfg.scale);
        let in_scale = move |n: i32| scale.contains(tonic, n);
        // Drum pads: the drums this layout has, in kit order.
        let pads: Vec<u8> = crate::drums::DRUMS
            .iter()
            .map(|d| d.note)
            .filter(|n| {
                self.keymap
                    .bindings
                    .values()
                    .any(|a| *a == Action::Drum(*n))
            })
            .collect();
        visuals::show(
            ui,
            &p,
            current,
            &VisualInput {
                melody: &melody,
                chord: &chord,
                low,
                octaves,
                in_scale: self
                    .cfg
                    .show_scale
                    .then_some(&in_scale as &dyn Fn(i32) -> bool),
                drum_pads: &pads,
                drum_hits: &self.drum_hits,
            },
        );
    }

    fn layout_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Layout").color(p.on_surface_variant));
            let mut chosen: Option<&str> = None;
            for (file, name) in BUILTIN_LAYOUTS {
                let selected = self.keymap_file == file;
                let text = RichText::new(name).color(if selected {
                    p.on_secondary_container
                } else {
                    p.on_surface
                });
                let btn = egui::Button::new(text)
                    .fill(if selected {
                        p.secondary_container
                    } else {
                        p.surface_container_high
                    })
                    .corner_radius(egui::CornerRadius::same(255));
                if ui.add(btn).clicked() && !selected {
                    chosen = Some(file);
                }
            }
            if !BUILTIN_LAYOUTS.iter().any(|(f, _)| *f == self.keymap_file) {
                chip(
                    ui,
                    p.secondary_container,
                    p.on_secondary_container,
                    format!("{} (your layout)", self.keymap.name),
                );
            }
            ui.label(
                RichText::new(format!("{} switches", self.keys_text(Action::LayoutNext)))
                    .small()
                    .color(p.on_surface_variant),
            );
            if let Some(f) = chosen {
                self.switch_keymap(f);
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, has_notes: bool) {
        let p = self.palette;
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
            if has_notes {
                self.key_picker(ui);
            }
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
    }

    /// Key + scale pickers.
    fn key_picker(&mut self, ui: &mut egui::Ui) {
        let tonic = self.tonic_pc();
        let flats = self.cfg.scale.prefers_flats(tonic);
        ui.label("Key");
        let mut new_tonic = tonic;
        egui::ComboBox::from_id_salt("key")
            .width(56.0)
            .selected_text(pc_name(tonic, flats))
            .show_ui(ui, |ui| {
                for pc in 0..12 {
                    let name = if crate::music::is_black(pc) {
                        format!("{} / {}", pc_name(pc, false), pc_name(pc, true))
                    } else {
                        pc_name(pc, false).to_string()
                    };
                    ui.selectable_value(&mut new_tonic, pc, name);
                }
            });
        if new_tonic != tonic {
            // Stay near C so the keyboard doesn't jump an octave.
            let t = if new_tonic <= 6 {
                new_tonic
            } else {
                new_tonic - 12
            };
            self.change_key(t - self.cfg.transpose);
        }
        let mut scale = self.cfg.scale;
        egui::ComboBox::from_id_salt("scale")
            .width(150.0)
            .selected_text(scale.name())
            .show_ui(ui, |ui| {
                for s in Scale::ALL {
                    ui.selectable_value(&mut scale, s, format!("{} · {}", s.name(), s.mood()));
                }
            });
        if scale != self.cfg.scale {
            self.cfg.scale = scale;
            self.mark_dirty();
            self.refresh_chord();
        }
        let mut lock = self.cfg.scale_lock;
        if ui
            .checkbox(&mut lock, "Scale lock")
            .on_hover_text(
                "Piano keys play the nearest note of the scale, so nothing sounds wrong. \
                 (Melody keys are always in the scale.)",
            )
            .changed()
        {
            self.cfg.scale_lock = lock;
            self.mark_dirty();
        }
        let mut show = self.cfg.show_scale;
        if ui
            .checkbox(&mut show, "Show scale")
            .on_hover_text("Marks notes of the scale on the keys (• in the scale, ★ home note).")
            .changed()
        {
            self.cfg.show_scale = show;
            self.mark_dirty();
        }
    }

    fn chord_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.columns(2, |cols| {
            // ---- What's playing
            card(&mut cols[0], &p, "", |ui| {
                ui.vertical_centered(|ui| {
                    let flats = self.cfg.scale.prefers_flats(self.tonic_pc());
                    match &self.chord {
                        Some(c) => {
                            ui.label(RichText::new(&c.name).size(46.0).strong().color(p.primary));
                            let notes: Vec<&str> =
                                self.voicing.iter().map(|n| pc_name(*n, flats)).collect();
                            let bass = self
                                .bass_note
                                .map(|b| format!(" · bass {}", pc_name(b, flats)))
                                .unwrap_or_default();
                            ui.label(
                                RichText::new(format!("{} in {}", c.numeral, self.key_name()))
                                    .size(17.0)
                                    .color(p.on_surface),
                            );
                            ui.label(
                                RichText::new(format!("notes {}{bass}", notes.join(" · ")))
                                    .color(p.on_surface_variant),
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new("—")
                                    .size(46.0)
                                    .strong()
                                    .color(p.outline_variant),
                            );
                            ui.label(
                                RichText::new(format!("Key of {}", self.key_name()))
                                    .size(17.0)
                                    .color(p.on_surface),
                            );
                            ui.label(
                                RichText::new("press a chord key (bottom row)")
                                    .color(p.on_surface_variant),
                            );
                        }
                    }
                    let mut melody: Vec<i32> = self.sounding.values().copied().collect();
                    melody.sort();
                    if !melody.is_empty() {
                        let names: Vec<String> = melody.iter().map(|n| note_name(*n)).collect();
                        ui.label(
                            RichText::new(format!("melody {}", names.join(" "))).color(p.tertiary),
                        );
                    }
                });
            });

            // ---- How chords are played
            card(&mut cols[1], &p, "", |ui| self.style_controls(ui, true));
        });
    }

    fn drum_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.columns(2, |cols| {
            card(&mut cols[0], &p, "", |ui| {
                ui.vertical_centered(|ui| {
                    let (text, color) = match self.last_drum {
                        Some((d, at)) if at.elapsed() < Duration::from_millis(900) => {
                            (crate::drums::name(d), p.primary)
                        }
                        _ => ("—".to_string(), p.outline_variant),
                    };
                    ui.label(RichText::new(text).size(40.0).strong().color(color));
                    ui.label(
                        RichText::new("Hit the keys. Enter plays a beat to drum along with.")
                            .color(p.on_surface_variant),
                    );
                });
                if self
                    .last_drum
                    .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(1))
                {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
            });
            card(&mut cols[1], &p, "", |ui| self.style_controls(ui, false));
        });
    }

    /// Style, tempo, start/stop and options. `chords` = full chord options.
    fn style_controls(&mut self, ui: &mut egui::Ui, chords: bool) {
        let p = self.palette;
        let styles = self.styles();
        let cur = self.style_index();
        ui.horizontal_wrapped(|ui| {
            ui.label("Style");
            let mut sel = cur;
            egui::ComboBox::from_id_salt("style")
                .width(150.0)
                .selected_text(styles[cur].1.clone())
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in styles.iter().enumerate() {
                        if !chords && i < FIXED_STYLES.len() {
                            continue;
                        }
                        ui.selectable_value(&mut sel, i, label);
                    }
                });
            if sel != cur {
                self.set_style(sel);
            }
            let running = self.rhythm_running();
            let label = if running { "■  Stop" } else { "▶  Start" };
            let btn = egui::Button::new(RichText::new(label).color(if running {
                p.on_primary
            } else {
                p.on_secondary_container
            }))
            .fill(if running {
                p.primary
            } else {
                p.secondary_container
            })
            .corner_radius(egui::CornerRadius::same(255));
            if ui
                .add(btn)
                .on_hover_text(format!(
                    "Start / stop the rhythm ({})",
                    self.keys_text(Action::Rhythm)
                ))
                .clicked()
            {
                self.toggle_rhythm();
            }
            if let Some(e) = &self.engine {
                let s = &e.stats;
                let cur = running.then(|| s.seq_step.load(Ordering::Relaxed));
                beat_dots(
                    ui,
                    &p,
                    s.seq_steps_per_bar.load(Ordering::Relaxed).max(1),
                    s.seq_steps_per_beat.load(Ordering::Relaxed).max(1),
                    cur,
                );
            }
        });
        if let Some(pat) = self.current_pattern() {
            ui.label(
                RichText::new(&pat.description)
                    .small()
                    .color(p.on_surface_variant),
            );
        }
        ui.horizontal(|ui| {
            ui.label("Tempo");
            let mut t = self.cfg.tempo;
            if ui
                .add(
                    egui::Slider::new(&mut t, 40.0..=240.0)
                        .suffix(" BPM")
                        .integer(),
                )
                .changed()
            {
                self.set_tempo(t);
            }
        });

        if chords {
            match self.style_kind() {
                StyleKind::Strum => {
                    ui.horizontal(|ui| {
                        ui.label("Strum speed");
                        let mut v = self.cfg.strum_ms;
                        if ui
                            .add(
                                egui::Slider::new(&mut v, 5.0..=120.0)
                                    .suffix(" ms")
                                    .integer(),
                            )
                            .changed()
                        {
                            self.cfg.strum_ms = v;
                            self.mark_dirty();
                            self.send_style();
                        }
                    });
                }
                StyleKind::Arp => {
                    ui.horizontal_wrapped(|ui| {
                        let mut mode = self.cfg.arp_mode;
                        egui::ComboBox::from_id_salt("arpmode")
                            .width(100.0)
                            .selected_text(mode.name())
                            .show_ui(ui, |ui| {
                                for m in ArpMode::ALL {
                                    ui.selectable_value(&mut mode, m, m.name());
                                }
                            });
                        let mut rate = self.cfg.arp_rate;
                        egui::ComboBox::from_id_salt("arprate")
                            .width(110.0)
                            .selected_text(rate_name(rate))
                            .show_ui(ui, |ui| {
                                for r in [1, 2, 3, 4] {
                                    ui.selectable_value(&mut rate, r, rate_name(r));
                                }
                            });
                        let mut oct = self.cfg.arp_octaves;
                        ui.label("Octaves");
                        ui.add(egui::DragValue::new(&mut oct).range(1..=3));
                        if mode != self.cfg.arp_mode
                            || rate != self.cfg.arp_rate
                            || oct != self.cfg.arp_octaves
                        {
                            self.cfg.arp_mode = mode;
                            self.cfg.arp_rate = rate;
                            self.cfg.arp_octaves = oct;
                            self.mark_dirty();
                            self.send_style();
                        }
                    });
                }
                _ => {}
            }
        }

        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            if chords {
                changed |= ui
                    .checkbox(&mut self.cfg.voice_leading, "Smooth chords")
                    .on_hover_text(
                        "Picks the chord shape closest to the last one, like a pianist does, \
                         instead of jumping around.",
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut self.cfg.chord_bass, "Bass note")
                    .on_hover_text("Adds the chord's root note low down.")
                    .changed();
            }
            if self.current_pattern().is_some_and(|p| p.has_drums) {
                changed |= ui.checkbox(&mut self.cfg.drums, "Drums").changed();
            }
            if chords && self.rhythmic() {
                changed |= ui
                    .checkbox(&mut self.cfg.latch, "Keep playing after I let go")
                    .on_hover_text(
                        "Tap a chord key once and the pattern keeps going until the next chord \
                         or Stop. Easier than holding keys.",
                    )
                    .changed();
            }
        });
        if changed {
            self.mark_dirty();
            self.send_style();
            if chords {
                self.refresh_chord();
            }
        }

        egui::CollapsingHeader::new("Sounds & patterns")
            .id_salt("sounds")
            .show(ui, |ui| {
                if chords {
                    ui.horizontal(|ui| {
                        ui.label("Chord sound");
                        let mut ci = self.cfg.chord_instrument;
                        egui::ComboBox::from_id_salt("chordinst")
                            .width(170.0)
                            .selected_text(
                                ci.map(|i| INSTRUMENTS[i.min(INSTRUMENTS.len() - 1)].name)
                                    .unwrap_or("Same as melody"),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut ci, None, "Same as melody");
                                for (i, ins) in INSTRUMENTS.iter().enumerate() {
                                    ui.selectable_value(&mut ci, Some(i), ins.name);
                                }
                            });
                        ui.label("Bass");
                        let mut bi = self.cfg.bass_instrument;
                        egui::ComboBox::from_id_salt("bassinst")
                            .width(140.0)
                            .selected_text(BASS_SOUNDS[bi.min(BASS_SOUNDS.len() - 1)].name)
                            .show_ui(ui, |ui| {
                                for (i, ins) in BASS_SOUNDS.iter().enumerate() {
                                    ui.selectable_value(&mut bi, i, ins.name);
                                }
                            });
                        if ci != self.cfg.chord_instrument || bi != self.cfg.bass_instrument {
                            self.cfg.chord_instrument = ci;
                            self.cfg.bass_instrument = bi;
                            self.mark_dirty();
                            self.send_programs();
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Reload patterns").clicked() {
                        self.reload_patterns();
                    }
                    if ui.button("Open patterns folder").clicked() {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(config::patterns_dir())
                            .spawn();
                    }
                });
                ui.label(
                    RichText::new(
                        "Patterns are small text files; see shuffle.toml in that folder for how \
                         to write your own.",
                    )
                    .small()
                    .color(p.on_surface_variant),
                );
                for w in &self.pattern_warnings {
                    ui.label(RichText::new(format!("⚠ {w}")).small().color(p.error));
                }
            });
    }

    fn help_text(&self, ui: &mut egui::Ui, has_chords: bool) {
        let p = self.palette;
        let k = |a: Action| self.keys_text(a);
        let text = if has_chords {
            use crate::theory::ChordMod::*;
            format!(
                "Left hand: one key = one chord. The bottom row has the chords that belong to {key}, \
                 the row above has 'borrowed' chords for extra colour. Hold {sev} for a 7th, {sus} for sus4, \
                 {add9} for add9, {swap} to swap major ↔ minor. Right hand: the top row plays the melody, \
                 always in key. {rhythm} start/stop · {style} next style · {tdn} / {tup} tempo · \
                 {kdn} {kup} change key · {space} pedal (holds the chord).",
                key = self.key_name(),
                sev = k(Action::Mod(Seventh)),
                sus = k(Action::Mod(Sus4)),
                add9 = k(Action::Mod(Add9)),
                swap = k(Action::Mod(Swap)),
                rhythm = k(Action::Rhythm),
                style = k(Action::StyleNext),
                tdn = k(Action::TempoDown),
                tup = k(Action::TempoUp),
                kdn = k(Action::TransposeDown),
                kup = k(Action::TransposeUp),
                space = k(Action::Sustain),
            )
        } else {
            format!(
                "{} · {} octave · {} {} change key · {} instrument · {} velocity · {} stop all.",
                self.keymap.description,
                format_args!("{} {}", k(Action::OctaveUp), k(Action::OctaveDown)),
                k(Action::TransposeDown),
                k(Action::TransposeUp),
                k(Action::InstrumentNext),
                k(Action::Velocity),
                k(Action::Panic),
            )
        };
        ui.label(RichText::new(text).color(p.on_surface_variant));
        let bt = self.current_device.as_ref().is_some_and(|d| d.bluetooth);
        if bt {
            ui.label(
                RichText::new(
                    "On Bluetooth: sounds marked ✦ (pads, strings, organ) hide the delay best, and \
                     the patterns keep perfect time even though you hear them late. Chord changes \
                     pressed a moment after the beat still land on it. Wired earphones remove the \
                     delay almost completely.",
                )
                .color(p.tertiary),
            );
        }
    }
}

fn rate_name(r: u32) -> &'static str {
    match r {
        1 => "Quarter notes",
        2 => "8th notes",
        3 => "Triplets",
        _ => "16th notes",
    }
}
