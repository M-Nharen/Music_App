//! Custom-drawn widgets: the laptop keyboard, the piano strip and cards.

use crate::keymap::{Action, Keymap, key_label};
use crate::music::{is_black, note_name, pitch_class_name};
use crate::theme::Palette;
use egui::{
    Align2, Color32, CornerRadius, FontId, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use std::collections::HashSet;

/// One key cap on the drawn keyboard. `key` is None for keys the app can't
/// see (Shift, Caps…), which are drawn greyed out for orientation.
struct Cap {
    key: Option<Key>,
    label: &'static str,
    width: f32,
}

const fn k(key: Key) -> Cap {
    Cap {
        key: Some(key),
        label: "",
        width: 1.0,
    }
}
const fn kw(key: Key, label: &'static str, width: f32) -> Cap {
    Cap {
        key: Some(key),
        label,
        width,
    }
}
const fn dead(label: &'static str, width: f32) -> Cap {
    Cap {
        key: None,
        label,
        width,
    }
}

fn rows() -> [Vec<Cap>; 5] {
    use Key::*;
    [
        vec![
            k(Backtick),
            k(Num1),
            k(Num2),
            k(Num3),
            k(Num4),
            k(Num5),
            k(Num6),
            k(Num7),
            k(Num8),
            k(Num9),
            k(Num0),
            k(Minus),
            k(Equals),
            kw(Backspace, "⌫", 2.0),
        ],
        vec![
            kw(Tab, "Tab", 1.5),
            k(Q),
            k(W),
            k(E),
            k(R),
            k(T),
            k(Y),
            k(U),
            k(I),
            k(O),
            k(P),
            k(OpenBracket),
            k(CloseBracket),
            kw(Backslash, "\\", 1.5),
        ],
        vec![
            dead("Caps", 1.75),
            k(A),
            k(S),
            k(D),
            k(F),
            k(G),
            k(H),
            k(J),
            k(K),
            k(L),
            k(Semicolon),
            k(Quote),
            kw(Enter, "Enter", 2.25),
        ],
        vec![
            dead("Shift", 2.25),
            k(Z),
            k(X),
            k(C),
            k(V),
            k(B),
            k(N),
            k(M),
            k(Comma),
            k(Period),
            k(Slash),
            dead("Shift", 2.75),
        ],
        vec![
            dead("Ctrl", 1.5),
            dead("Alt", 1.5),
            kw(Space, "Space", 6.5),
            dead("Alt", 1.5),
            kw(ArrowLeft, "←", 1.0),
            kw(ArrowUp, "↑", 1.0),
            kw(ArrowDown, "↓", 1.0),
            kw(ArrowRight, "→", 1.0),
        ],
    ]
}

pub struct KeyboardView<'a> {
    pub keymap: &'a Keymap,
    pub palette: &'a Palette,
    /// MIDI note that `note 0` plays right now (octave + transpose applied).
    pub base_note: i32,
    pub held: &'a HashSet<Key>,
    pub selected: Option<Key>,
    /// Show note names on keys (Play) or raw key names only (Key test).
    pub show_notes: bool,
}

impl KeyboardView<'_> {
    /// Draws the keyboard; returns the key that was clicked, if any.
    pub fn show(&self, ui: &mut Ui) -> Option<Key> {
        let p = self.palette;
        let total_units = 15.0;
        let width = ui.available_width().min(1100.0);
        let unit = width / total_units;
        let gap = (unit * 0.08).max(3.0);
        let height = unit * 5.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let painter = ui.painter_at(rect.expand(2.0));
        let mut clicked = None;

        for (r, row) in rows().iter().enumerate() {
            let mut x = rect.left();
            let y = rect.top() + r as f32 * unit;
            for cap in row {
                let w = cap.width * unit;
                let cap_rect = Rect::from_min_size(
                    pos2(x + gap / 2.0, y + gap / 2.0),
                    vec2(w - gap, unit - gap),
                );
                x += w;
                let action = cap.key.and_then(|k| self.keymap.get(k));
                let held = cap.key.is_some_and(|k| self.held.contains(&k));
                let selected = cap.key.is_some() && cap.key == self.selected;

                let (fill, text) = if held {
                    (p.primary, p.on_primary)
                } else {
                    match action {
                        _ if !self.show_notes => (p.surface_container_high, p.on_surface),
                        Some(Action::Note(n)) if is_black(self.base_note + n) => {
                            (p.secondary_container, p.on_secondary_container)
                        }
                        Some(Action::Note(_)) => (p.surface_container_highest, p.on_surface),
                        Some(_) => (p.tertiary_container, p.on_surface),
                        None if cap.key.is_none() => (p.surface_container_low, p.outline_variant),
                        None => (p.surface_container, p.outline),
                    }
                };
                let radius = CornerRadius::same((unit * 0.18) as u8);
                if let Some(key) = cap.key {
                    let resp = ui.interact(
                        cap_rect,
                        ui.scope_id().with(("cap", key.name())),
                        Sense::click(),
                    );
                    if resp.clicked() {
                        clicked = Some(key);
                    }
                    let hover = resp.hovered();
                    painter.rect_filled(
                        cap_rect,
                        radius,
                        if hover && !held {
                            lighten(fill, 0.08)
                        } else {
                            fill
                        },
                    );
                    if let Some(Action::Note(_)) = action
                        && self.show_notes
                        && !held
                    {
                        // Little accent bar at the bottom of playable keys.
                        let bar = Rect::from_min_max(
                            pos2(cap_rect.left() + unit * 0.2, cap_rect.bottom() - unit * 0.1),
                            pos2(
                                cap_rect.right() - unit * 0.2,
                                cap_rect.bottom() - unit * 0.06,
                            ),
                        );
                        painter.rect_filled(
                            bar,
                            CornerRadius::same(2),
                            p.primary.gamma_multiply(0.7),
                        );
                    }
                } else {
                    painter.rect_filled(cap_rect, radius, fill);
                }
                if selected {
                    painter.rect_stroke(
                        cap_rect,
                        radius,
                        Stroke::new(2.5, p.primary),
                        StrokeKind::Outside,
                    );
                }

                // Labels: key name top-left, what it does in the middle.
                let name = if !cap.label.is_empty() {
                    cap.label
                } else {
                    cap.key.map(key_label).unwrap_or("")
                };
                let small = FontId::proportional((unit * 0.22).clamp(9.0, 14.0));
                let big = FontId::proportional((unit * 0.30).clamp(10.0, 19.0));
                match (self.show_notes, action) {
                    (true, Some(a)) => {
                        painter.text(
                            cap_rect.left_top() + vec2(unit * 0.12, unit * 0.08),
                            Align2::LEFT_TOP,
                            name,
                            small,
                            text.gamma_multiply(0.75),
                        );
                        let main = match a {
                            Action::Note(n) => note_name(self.base_note + n),
                            other => control_icon(other).to_string(),
                        };
                        painter.text(
                            cap_rect.center() + vec2(0.0, unit * 0.1),
                            Align2::CENTER_CENTER,
                            main,
                            big,
                            text,
                        );
                    }
                    _ => {
                        painter.text(cap_rect.center(), Align2::CENTER_CENTER, name, big, text);
                    }
                }
            }
        }
        clicked
    }
}

pub fn control_icon(a: Action) -> &'static str {
    match a {
        Action::Sustain => "Pedal",
        Action::OctaveUp => "Oct+",
        Action::OctaveDown => "Oct−",
        Action::TransposeUp => "Key+",
        Action::TransposeDown => "Key−",
        Action::InstrumentNext => "Inst+",
        Action::InstrumentPrev => "Inst−",
        Action::Velocity => "Vel",
        Action::Panic => "Stop",
        Action::Note(_) => "",
    }
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let f = |v: u8| (v as f32 + (255.0 - v as f32) * t) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// A piano strip from `low` covering `octaves`, lighting `sounding` notes.
pub fn piano_strip(ui: &mut Ui, p: &Palette, low: i32, octaves: i32, sounding: &HashSet<i32>) {
    let width = ui.available_width().min(1100.0);
    let whites: Vec<i32> = (low..low + octaves * 12 + 1)
        .filter(|n| !is_black(*n))
        .collect();
    let ww = width / whites.len() as f32;
    let height = (ww * 4.2).clamp(70.0, 130.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    let white_fill = Color32::from_rgb(0xF4, 0xF1, 0xF6);
    let black_fill = Color32::from_rgb(0x1E, 0x1C, 0x22);

    let mut x_of = std::collections::HashMap::new();
    for (i, n) in whites.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + i as f32 * ww, rect.top()),
            vec2(ww - 2.0, height),
        );
        x_of.insert(*n, r.left());
        let on = sounding.contains(n);
        painter.rect_filled(
            r,
            CornerRadius {
                nw: 0,
                ne: 0,
                sw: 8,
                se: 8,
            },
            if on { p.primary } else { white_fill },
        );
        if n.rem_euclid(12) == 0 {
            painter.text(
                pos2(r.center().x, r.bottom() - 10.0),
                Align2::CENTER_CENTER,
                note_name(*n),
                FontId::proportional(11.0),
                if on {
                    p.on_primary
                } else {
                    Color32::from_gray(110)
                },
            );
        }
    }
    for n in low..low + octaves * 12 {
        if !is_black(n) {
            continue;
        }
        let Some(left_white) = x_of.get(&(n - 1)) else {
            continue;
        };
        let bw = ww * 0.62;
        let r = Rect::from_min_size(
            pos2(left_white + ww - bw / 2.0 - 1.0, rect.top()),
            vec2(bw, height * 0.6),
        );
        let on = sounding.contains(&n);
        painter.rect_filled(
            r,
            CornerRadius {
                nw: 0,
                ne: 0,
                sw: 6,
                se: 6,
            },
            if on { p.primary } else { black_fill },
        );
    }
    // Notes outside the strip still deserve a hint.
    let out_of_range = sounding
        .iter()
        .filter(|n| **n < low || **n > low + octaves * 12)
        .count();
    if out_of_range > 0 {
        painter.text(
            rect.right_top() + vec2(-6.0, 6.0),
            Align2::RIGHT_TOP,
            format!("+{out_of_range} off-screen"),
            FontId::proportional(11.0),
            p.error,
        );
    }
}

/// A rounded "card" section with a title.
pub fn card<R>(ui: &mut Ui, p: &Palette, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.surface_container)
        .corner_radius(CornerRadius::same(20))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(
                    egui::RichText::new(title)
                        .size(17.0)
                        .strong()
                        .color(p.primary),
                );
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner
}

/// Small rounded label like Material "assist chips".
pub fn chip(ui: &mut Ui, fill: Color32, text_color: Color32, text: impl Into<String>) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(12, 5))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text.into())
                    .color(text_color)
                    .size(13.5),
            );
        });
}

/// The big "what you're playing" readout.
pub fn now_playing(ui: &mut Ui, p: &Palette, sounding: &[i32]) {
    let chord = crate::music::chord_name(sounding);
    let notes: Vec<&str> = {
        let mut s = sounding.to_vec();
        s.sort();
        s.iter().map(|n| pitch_class_name(*n)).collect()
    };
    ui.vertical_centered(|ui| {
        let big = match (&chord, notes.len()) {
            (Some(c), _) => c.clone(),
            (None, 0) => "—".into(),
            (None, _) => notes.join(" "),
        };
        ui.label(
            egui::RichText::new(big)
                .size(46.0)
                .strong()
                .color(if sounding.is_empty() {
                    p.outline_variant
                } else {
                    p.primary
                }),
        );
        let sub = if chord.is_some() {
            format!("notes: {}", notes.join(" · "))
        } else if sounding.is_empty() {
            "press some keys".into()
        } else {
            "notes".into()
        };
        ui.label(egui::RichText::new(sub).color(p.on_surface_variant));
    });
}
