//! The Listen tab: live chords and notes from any audio input.

use super::*;
use crate::listen::input::{self, InputInfo, Listener};
use crate::listen::{Heard, Vocab};
use crate::song::chords::{key_from, roman};
use crate::theory::chord_symbol;
use crate::ui::widgets::{Hint, HintKind, piano_strip};
use egui::vec2;

#[derive(Default)]
pub(super) struct ListenUi {
    pub listener: Option<Listener>,
    inputs: Vec<InputInfo>,
    listed: bool,
    error: Option<String>,
}

impl QwertoneApp {
    fn listen_start(&mut self, ctx: &egui::Context) {
        self.listen.listener = None;
        let id =
            (!self.cfg.song.listen_input.is_empty()).then(|| self.cfg.song.listen_input.clone());
        let c = ctx.clone();
        match input::start(id.as_deref(), move || c.request_repaint()) {
            Ok(l) => {
                *l.settings.lock().unwrap() = (
                    self.cfg.song.listen_gate_db,
                    if self.cfg.song.listen_basic {
                        Vocab::Basic
                    } else {
                        Vocab::Extended
                    },
                );
                self.listen.error = None;
                self.listen.listener = Some(l);
            }
            Err(e) => self.listen.error = Some(format!("{e:#}")),
        }
    }

    /// Keys of your layout that play what's heard.
    fn listen_hints(&self, h: &Heard) -> HashMap<KeyId, Hint> {
        let mut out = HashMap::new();
        if let Some((root, kind, _)) = h.chord {
            let pcs: Vec<i32> = kind.intervals().iter().map(|t| (root + t) % 12).collect();
            let chord_keys = self.song_chord_keys();
            let key = chord_keys
                .iter()
                .find(|(_, r, p)| *r == root && crate::song::practice::chord_fits(&pcs, p, true));
            if let Some((k, ..)) = key {
                out.insert(
                    *k,
                    Hint {
                        kind: HintKind::Now,
                        tag: chord_symbol(root, kind),
                    },
                );
            } else {
                // Show the chord's notes instead.
                let note_keys = self.note_keys();
                for pc in &pcs {
                    if let Some((n, k)) = note_keys.iter().find(|(n, _)| n.rem_euclid(12) == *pc) {
                        out.entry(*k).or_insert(Hint {
                            kind: HintKind::Option,
                            tag: note_name(*n),
                        });
                    }
                }
            }
        }
        if let Some((p, clarity)) = h.pitch
            && clarity > 0.8
        {
            let n = p.round() as i32;
            let note_keys = self.note_keys();
            let (keys, _) = Self::keys_for_note(&note_keys, n);
            if let Some(k) = Self::pick_key(&keys, None) {
                out.insert(
                    k,
                    Hint {
                        kind: HintKind::Next,
                        tag: note_name(n),
                    },
                );
            }
        }
        out
    }

    pub(super) fn listen_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if !self.listen.listed {
            self.listen.inputs = input::list_inputs();
            self.listen.listed = true;
        }
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            card(ui, &p, "Listen", |ui| {
                ui.label(
                    "Play or sing something, or play a song on the computer: Qwertone shows the chord, the notes and \
                     the key as it hears them, and lights the keys that play along.",
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Input");
                    let cur = self.cfg.song.listen_input.clone();
                    let name = if cur.is_empty() {
                        "Default (microphone)".to_string()
                    } else {
                        self.listen.inputs.iter().find(|i| i.id == cur).map(|i| i.name.clone()).unwrap_or(cur.clone())
                    };
                    let mut sel = cur.clone();
                    egui::ComboBox::from_id_salt("listen-in")
                        .width(300.0)
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut sel, String::new(), "Default (microphone)");
                            for i in &self.listen.inputs {
                                let mut label = i.name.clone();
                                if i.monitor {
                                    label += "  · what the computer plays";
                                }
                                if i.bluetooth {
                                    label += "  · Bluetooth mic";
                                }
                                ui.selectable_value(&mut sel, i.id.clone(), label);
                            }
                        });
                    if sel != cur {
                        self.cfg.song.listen_input = sel;
                        self.mark_dirty();
                        if self.listen.listener.is_some() {
                            self.listen_start(&ctx);
                        }
                    }
                    if ui.small_button("⟳").on_hover_text("Look for inputs again").clicked() {
                        self.listen.inputs = input::list_inputs();
                    }
                    let on = self.listen.listener.is_some();
                    let (label, fill, fg) = if on {
                        ("■  Stop", p.primary, p.on_primary)
                    } else {
                        ("🎤  Start listening", p.secondary_container, p.on_secondary_container)
                    };
                    if ui
                        .add(egui::Button::new(RichText::new(label).color(fg)).fill(fill).corner_radius(egui::CornerRadius::same(255)))
                        .clicked()
                    {
                        if on {
                            self.listen.listener = None;
                        } else {
                            self.listen_start(&ctx);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Ignore sounds quieter than");
                    let mut g = self.cfg.song.listen_gate_db;
                    let mut basic = self.cfg.song.listen_basic;
                    let mut changed = ui.add(egui::Slider::new(&mut g, -80.0..=-20.0).suffix(" dB").integer()).changed();
                    changed |= ui
                        .checkbox(&mut basic, "Only major / minor chords")
                        .on_hover_text("Simpler answers, fewer surprises")
                        .changed();
                    if changed {
                        self.cfg.song.listen_gate_db = g;
                        self.cfg.song.listen_basic = basic;
                        self.mark_dirty();
                        if let Some(l) = &self.listen.listener {
                            *l.settings.lock().unwrap() = (g, if basic { Vocab::Basic } else { Vocab::Extended });
                        }
                    }
                });
                let sel_bt = self
                    .listen
                    .inputs
                    .iter()
                    .any(|i| i.id == self.cfg.song.listen_input && i.bluetooth);
                if sel_bt {
                    ui.label(
                        RichText::new(
                            "Careful: using the earbuds' microphone switches Bluetooth to “call mode”, and music sounds \
                             much worse until you stop. The laptop's microphone is better.",
                        )
                        .color(p.tertiary),
                    );
                }
                ui.label(
                    RichText::new(
                        "To follow a song playing on this computer (Spotify, a video…), pick the input that says \
                         “what the computer plays” (a monitor). No microphone needed.",
                    )
                    .small()
                    .color(p.on_surface_variant),
                );
                if let Some(e) = &self.listen.error {
                    ui.label(RichText::new(format!("⚠ {e}")).color(p.error));
                }
            });
            ui.add_space(8.0);

            let Some(l) = &self.listen.listener else {
                return;
            };
            let h = l.latest.lock().unwrap().clone();
            let hist: Vec<(f64, String)> = l.history.lock().unwrap().iter().rev().take(16).rev().cloned().collect();
            let trail: Vec<(f64, f32)> = l.trail.lock().unwrap().clone();
            let src = l.name.clone();
            let rate = l.sample_rate;

            ui.columns(2, |cols| {
                card(&mut cols[0], &p, "", |ui| {
                    ui.vertical_centered(|ui| {
                        let key = h.key.map(|(t, m, c)| key_from(t, m, c));
                        match h.chord {
                            Some((r, k, conf)) => {
                                ui.label(RichText::new(chord_symbol(r, k)).size(52.0).strong().color(p.primary));
                                let mut sub = format!("{} · {:.0}% sure", kind_words(k), conf * 100.0);
                                if let Some(key) = &key {
                                    sub = format!("{} in {} · {sub}", roman(r, k, key).0, key.name);
                                }
                                ui.label(RichText::new(sub).color(p.on_surface));
                            }
                            None => {
                                ui.label(RichText::new("—").size(52.0).strong().color(p.outline_variant));
                                ui.label(RichText::new(if h.level_db < self.cfg.song.listen_gate_db { "listening… (it's quiet)" } else { "no clear chord" }).color(p.on_surface_variant));
                            }
                        }
                        if let Some((t, m, conf)) = h.key {
                            let k = key_from(t, m, conf);
                            ui.label(
                                RichText::new(format!("Key so far: probably {} ({:.0}% sure)", k.name, conf * 100.0))
                                    .color(p.on_surface_variant),
                            );
                        }
                    });
                    level_meter(ui, &p, h.level_db, self.cfg.song.listen_gate_db);
                    ui.label(RichText::new(format!("from {src} · {} Hz", rate)).small().color(p.outline));
                    chroma_bars(ui, &p, &h.chroma);
                });
                card(&mut cols[1], &p, "", |ui| {
                    ui.label(RichText::new("Notes heard").strong().color(p.primary));
                    let notes: Vec<String> = h.notes.iter().map(|(n, _)| note_name(*n)).collect();
                    ui.label(if notes.is_empty() { "—".to_string() } else { notes.join("  ") });
                    match h.pitch {
                        Some((m, clarity)) if clarity > 0.75 => {
                            let n = m.round() as i32;
                            let cents = ((m - n as f32) * 100.0).round();
                            ui.label(RichText::new(format!("Melody note: {}", note_name(n))).size(20.0).strong());
                            tuner(ui, &p, cents);
                        }
                        _ => {
                            ui.label(RichText::new("Melody note: —").color(p.on_surface_variant));
                        }
                    }
                    pitch_trail(ui, &p, &trail, h.time);
                });
            });
            ui.add_space(6.0);
            card(ui, &p, "", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Chords so far").color(p.on_surface_variant));
                    for (_, c) in &hist {
                        chip(ui, p.surface_container_high, p.on_surface, c);
                    }
                    if ui.small_button("Clear").clicked() {
                        l.history.lock().unwrap().clear();
                    }
                });
                let lit: HashSet<i32> = h.notes.iter().map(|n| n.0).collect();
                let chord_notes: HashSet<i32> = HashSet::new();
                piano_strip(ui, &p, 36, 5, &lit, &chord_notes, None);
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Lit keys play along: the chord (or its notes) and the melody note.").color(p.on_surface_variant));
            let hints = self.listen_hints(&h);
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
        });
    }
}

/// A chord type in plain words.
fn kind_words(k: crate::theory::ChordKind) -> &'static str {
    use crate::theory::ChordKind::*;
    match k {
        Major => "major: bright",
        Minor => "minor: soft, a bit sad",
        Dom7 => "seventh: wants to move on",
        Maj7 => "major 7th: dreamy",
        Min7 => "minor 7th: mellow",
        Dim => "diminished: tense",
        Aug => "augmented: floating",
        Sus2 | Sus4 => "suspended: open, unresolved",
    }
}

fn level_meter(ui: &mut egui::Ui, p: &crate::theme::Palette, db: f32, gate: f32) {
    let w = ui.available_width().min(360.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 10.0), egui::Sense::hover());
    let painter = ui.painter_at(r);
    painter.rect_filled(r, egui::CornerRadius::same(5), p.surface_container_high);
    let f = ((db + 80.0) / 80.0).clamp(0.0, 1.0);
    let fill = egui::Rect::from_min_size(r.min, vec2(r.width() * f, r.height()));
    painter.rect_filled(
        fill,
        egui::CornerRadius::same(5),
        if db > -3.0 { p.error } else { p.primary },
    );
    let gx = r.left() + r.width() * ((gate + 80.0) / 80.0).clamp(0.0, 1.0);
    painter.line_segment(
        [
            egui::pos2(gx, r.top() - 2.0),
            egui::pos2(gx, r.bottom() + 2.0),
        ],
        egui::Stroke::new(2.0, p.outline),
    );
}

/// How much of each note name is in the sound right now.
fn chroma_bars(ui: &mut egui::Ui, p: &crate::theme::Palette, chroma: &[f32; 12]) {
    let w = ui.available_width().min(360.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 46.0), egui::Sense::hover());
    let painter = ui.painter_at(r);
    let bw = r.width() / 12.0;
    for (i, v) in chroma.iter().enumerate() {
        let x = r.left() + i as f32 * bw;
        let h = (r.height() - 13.0) * v.clamp(0.0, 1.0);
        let bar = egui::Rect::from_min_max(
            egui::pos2(x + 2.0, r.bottom() - 13.0 - h),
            egui::pos2(x + bw - 2.0, r.bottom() - 13.0),
        );
        painter.rect_filled(
            bar,
            egui::CornerRadius::same(2),
            if *v > 0.6 {
                p.primary
            } else {
                p.outline_variant
            },
        );
        painter.text(
            egui::pos2(x + bw / 2.0, r.bottom()),
            egui::Align2::CENTER_BOTTOM,
            crate::theory::pc_name(i as i32, false),
            egui::FontId::proportional(9.0),
            p.on_surface_variant,
        );
    }
}

/// In tune or not: −50..+50 cents.
fn tuner(ui: &mut egui::Ui, p: &crate::theme::Palette, cents: f32) {
    let w = ui.available_width().min(300.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), egui::Sense::hover());
    let painter = ui.painter_at(r);
    painter.rect_filled(r, egui::CornerRadius::same(9), p.surface_container_high);
    let cx = r.center().x;
    painter.line_segment(
        [egui::pos2(cx, r.top()), egui::pos2(cx, r.bottom())],
        egui::Stroke::new(1.5, p.outline),
    );
    let x = cx + (cents / 50.0).clamp(-1.0, 1.0) * r.width() / 2.0;
    let c = if cents.abs() < 10.0 {
        p.primary
    } else {
        p.tertiary
    };
    painter.circle_filled(egui::pos2(x, r.center().y), 6.0, c);
    let txt = if cents.abs() < 10.0 {
        "in tune".to_string()
    } else if cents > 0.0 {
        format!("{cents:+.0} cents (a bit high)")
    } else {
        format!("{cents:+.0} cents (a bit low)")
    };
    ui.label(RichText::new(txt).small().color(p.on_surface_variant));
}

/// The melody's last few seconds as a line (pitch over time).
fn pitch_trail(ui: &mut egui::Ui, p: &crate::theme::Palette, trail: &[(f64, f32)], now: f64) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 110.0), egui::Sense::hover());
    let painter = ui.painter_at(r);
    painter.rect_filled(r, egui::CornerRadius::same(10), p.surface_container_low);
    if trail.is_empty() {
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            "sing or play a tune",
            egui::FontId::proportional(12.0),
            p.outline,
        );
        return;
    }
    let lo = trail.iter().map(|x| x.1).fold(f32::MAX, f32::min).floor() - 2.0;
    let hi = (trail.iter().map(|x| x.1).fold(f32::MIN, f32::max).ceil() + 2.0).max(lo + 12.0);
    let span = 8.0;
    let x_of = |t: f64| r.right() - ((now - t) / span) as f32 * r.width();
    let y_of = |m: f32| r.bottom() - (m - lo) / (hi - lo) * r.height();
    for m in (lo as i32)..=(hi as i32) {
        if m.rem_euclid(12) == 0 {
            let y = y_of(m as f32);
            painter.line_segment(
                [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                egui::Stroke::new(1.0, p.outline_variant.gamma_multiply(0.5)),
            );
            painter.text(
                egui::pos2(r.left() + 4.0, y - 1.0),
                egui::Align2::LEFT_BOTTOM,
                note_name(m),
                egui::FontId::proportional(9.0),
                p.outline,
            );
        }
    }
    let mut prev: Option<(f64, egui::Pos2)> = None;
    for &(t, m) in trail {
        if now - t > span {
            continue;
        }
        let pt = egui::pos2(x_of(t), y_of(m));
        if let Some((pt0, pp)) = prev
            && t - pt0 < 0.15
        {
            painter.line_segment([pp, pt], egui::Stroke::new(2.5, p.primary));
        } else {
            painter.circle_filled(pt, 1.5, p.primary);
        }
        prev = Some((t, pt));
    }
}
