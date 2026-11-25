//! The instrument picture under the keyboard. It follows the instrument:
//! keyboard for pianos/organs, fretboard for guitars, fingerboard for
//! strings, finger holes for the flute, pads for drums and a glowing
//! waveform for synths and pads. Every one lights up the notes playing.

use crate::music::note_name;
use crate::theme::Palette;
use crate::ui::widgets::piano_strip;
use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visual {
    Keyboard,
    Guitar,
    Strings,
    Flute,
    Drums,
    Wave,
}

impl Visual {
    pub const ALL: [Visual; 6] = [
        Visual::Keyboard,
        Visual::Guitar,
        Visual::Strings,
        Visual::Flute,
        Visual::Drums,
        Visual::Wave,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Visual::Keyboard => "Keyboard",
            Visual::Guitar => "Guitar fretboard",
            Visual::Strings => "Violin / cello",
            Visual::Flute => "Flute holes",
            Visual::Drums => "Drum pads",
            Visual::Wave => "Waveform",
        }
    }

    /// Name stored in config.toml.
    pub fn code(self) -> &'static str {
        match self {
            Visual::Keyboard => "keyboard",
            Visual::Guitar => "guitar",
            Visual::Strings => "strings",
            Visual::Flute => "flute",
            Visual::Drums => "drums",
            Visual::Wave => "wave",
        }
    }

    pub fn from_code(s: &str) -> Option<Visual> {
        Visual::ALL.into_iter().find(|v| v.code() == s)
    }

    /// The natural picture for a General MIDI program.
    pub fn for_program(program: u8) -> Visual {
        match program {
            46 => Visual::Keyboard, // harp: strings laid out like a keyboard
            24..=39 => Visual::Guitar,
            40..=51 => Visual::Strings,
            72..=79 => Visual::Flute,
            52..=55 | 80..=103 => Visual::Wave,
            _ => Visual::Keyboard,
        }
    }
}

/// Everything the pictures need to know.
pub struct VisualInput<'a> {
    /// Melody / single notes (drawn in the primary colour).
    pub melody: &'a HashSet<i32>,
    /// Chord + bass notes (drawn in the tertiary colour).
    pub chord: &'a HashSet<i32>,
    /// Keyboard range.
    pub low: i32,
    pub octaves: i32,
    pub in_scale: Option<&'a dyn Fn(i32) -> bool>,
    /// Drums to show as pads, and when each was last hit.
    pub drum_pads: &'a [u8],
    pub drum_hits: &'a HashMap<u8, Instant>,
}

impl VisualInput<'_> {
    fn any(&self) -> bool {
        !self.melody.is_empty() || !self.chord.is_empty()
    }
    fn color(&self, p: &Palette, note: i32) -> Color32 {
        if self.melody.contains(&note) {
            p.primary
        } else {
            p.tertiary
        }
    }
}

pub fn show(ui: &mut Ui, p: &Palette, v: Visual, input: &VisualInput) {
    match v {
        Visual::Keyboard => piano_strip(
            ui,
            p,
            input.low,
            input.octaves,
            input.melody,
            input.chord,
            input.in_scale,
        ),
        Visual::Guitar => fingerboard(
            ui,
            p,
            &Board {
                name: "Guitar (standard tuning)",
                open: &[40, 45, 50, 55, 59, 64],
                frets: 15,
                fretted: true,
                markers: &[3, 5, 7, 9, 12, 15],
            },
            input,
        ),
        Visual::Strings => {
            // Cello for low music, violin otherwise. The melody decides when
            // there is one (chords usually come with a low bass note).
            let lowest = if input.melody.is_empty() {
                input.chord.iter().min()
            } else {
                input.melody.iter().min()
            }
            .copied();
            let cello = lowest.is_some_and(|n| n < 55);
            let board = if cello {
                Board {
                    name: "Cello (C G D A)",
                    open: &[36, 43, 50, 57],
                    frets: 12,
                    fretted: false,
                    markers: &[2, 4, 5, 7],
                }
            } else {
                Board {
                    name: "Violin (G D A E)",
                    open: &[55, 62, 69, 76],
                    frets: 12,
                    fretted: false,
                    markers: &[2, 4, 5, 7],
                }
            };
            fingerboard(ui, p, &board, input);
        }
        Visual::Flute => flute(ui, p, input),
        Visual::Drums => drum_pads(ui, p, input),
        Visual::Wave => waveform(ui, p, input),
    }
}

fn area(ui: &mut Ui, p: &Palette, height: f32) -> (Rect, egui::Painter) {
    let width = ui.available_width().min(1100.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect_filled(rect, CornerRadius::same(16), p.surface_container_low);
    (rect, painter)
}

// ------------------------------------------------------------ fingerboards

struct Board {
    name: &'static str,
    /// Open-string notes, lowest string first.
    open: &'static [i32],
    frets: i32,
    /// Guitar has frets; violin/cello don't (we draw beginner finger tapes).
    fretted: bool,
    markers: &'static [i32],
}

/// Choose a (string, fret) for each note: every note on its own string,
/// frets as close together and as low as possible, like a real hand shape.
/// Notes that can't fit are left out (returned as None).
pub fn place_notes(open: &[i32], frets: i32, notes: &[i32]) -> Vec<Option<(usize, i32)>> {
    fn search(
        open: &[i32],
        frets: i32,
        notes: &[i32],
        i: usize,
        used: &mut Vec<bool>,
        cur: &mut Vec<Option<(usize, i32)>>,
        best: &mut (f32, Vec<Option<(usize, i32)>>),
    ) {
        if i == notes.len() {
            let fretted: Vec<i32> = cur
                .iter()
                .flatten()
                .map(|(_, f)| *f)
                .filter(|f| *f > 0)
                .collect();
            let span = match (fretted.iter().min(), fretted.iter().max()) {
                (Some(a), Some(b)) => (b - a) as f32,
                _ => 0.0,
            };
            let skipped = cur.iter().filter(|c| c.is_none()).count() as f32;
            let sum: i32 = cur.iter().flatten().map(|(_, f)| *f).sum();
            let stretch = if span > 4.0 { (span - 4.0) * 6.0 } else { 0.0 };
            let cost = skipped * 30.0 + span * 2.0 + stretch + sum as f32 * 0.25;
            if cost < best.0 {
                *best = (cost, cur.clone());
            }
            return;
        }
        let n = notes[i];
        for (s, &o) in open.iter().enumerate() {
            let f = n - o;
            if used[s] || f < 0 || f > frets {
                continue;
            }
            used[s] = true;
            cur.push(Some((s, f)));
            search(open, frets, notes, i + 1, used, cur, best);
            cur.pop();
            used[s] = false;
        }
        cur.push(None);
        search(open, frets, notes, i + 1, used, cur, best);
        cur.pop();
    }
    let mut best = (f32::MAX, vec![None; notes.len()]);
    search(
        open,
        frets,
        notes,
        0,
        &mut vec![false; open.len()],
        &mut Vec::new(),
        &mut best,
    );
    best.1
}

fn fingerboard(ui: &mut Ui, p: &Palette, b: &Board, input: &VisualInput) {
    let strings = b.open.len();
    let row = 24.0;
    let height = row * strings as f32 + 44.0;
    let (rect, painter) = area(ui, p, height);
    let left = rect.left() + 56.0; // open-string column + nut
    let right = rect.right() - 14.0;
    let top = rect.top() + 26.0;
    let fret_w = (right - left) / b.frets as f32;
    // High string at the top, like a guitar tab.
    let y_of = |s: usize| top + (strings - 1 - s) as f32 * row + row / 2.0;
    let x_of = |f: i32| {
        if f == 0 {
            left - 22.0
        } else {
            left + (f as f32 - 0.5) * fret_w
        }
    };

    painter.text(
        pos2(rect.left() + 14.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        b.name,
        FontId::proportional(12.0),
        p.on_surface_variant,
    );
    // Wood.
    let board = Rect::from_min_max(pos2(left, top), pos2(right, top + row * strings as f32));
    painter.rect_filled(board, CornerRadius::same(4), p.surface_container_high);
    // Inlay dots / finger tapes.
    for &m in b.markers {
        if m > b.frets {
            continue;
        }
        let x = left + (m as f32 - 0.5) * fret_w;
        if b.fretted {
            let dots: &[f32] = if m == 12 { &[-0.25, 0.25] } else { &[0.0] };
            for d in dots {
                painter.circle_filled(
                    pos2(x, board.center().y + d * board.height()),
                    4.0,
                    p.outline_variant,
                );
            }
        } else {
            painter.line_segment(
                [pos2(x, board.top()), pos2(x, board.bottom())],
                Stroke::new(3.0, p.outline_variant.gamma_multiply(0.6)),
            );
        }
    }
    // Frets, nut.
    if b.fretted {
        for f in 1..=b.frets {
            let x = left + f as f32 * fret_w;
            painter.line_segment(
                [pos2(x, board.top()), pos2(x, board.bottom())],
                Stroke::new(1.5, p.outline),
            );
        }
    }
    painter.line_segment(
        [pos2(left, board.top()), pos2(left, board.bottom())],
        Stroke::new(4.0, p.on_surface_variant),
    );
    // Fret numbers.
    for f in [1, 3, 5, 7, 9, 12, 15] {
        if f <= b.frets {
            painter.text(
                pos2(left + (f as f32 - 0.5) * fret_w, board.bottom() + 3.0),
                Align2::CENTER_TOP,
                f.to_string(),
                FontId::proportional(10.0),
                p.outline,
            );
        }
    }
    // Strings + names.
    for (s, &o) in b.open.iter().enumerate() {
        let y = y_of(s);
        painter.line_segment(
            [pos2(left - 34.0, y), pos2(right, y)],
            // Low strings are thicker.
            Stroke::new(1.0 + (strings - 1 - s) as f32 * 0.35, p.on_surface_variant),
        );
        painter.text(
            pos2(rect.left() + 12.0, y),
            Align2::LEFT_CENTER,
            crate::music::pitch_class_name(o),
            FontId::proportional(11.0),
            p.outline,
        );
    }
    // Faint scale map.
    if let Some(in_scale) = input.in_scale {
        for (s, &o) in b.open.iter().enumerate() {
            for f in 0..=b.frets {
                if in_scale(o + f) {
                    painter.circle_filled(
                        pos2(x_of(f), y_of(s)),
                        3.0,
                        p.primary.gamma_multiply(0.25),
                    );
                }
            }
        }
    }
    // The notes.
    let mut notes: Vec<i32> = input.melody.iter().chain(input.chord).copied().collect();
    notes.sort();
    notes.dedup();
    // More notes than strings: drop the lowest (usually the bass note).
    let extra = notes.len().saturating_sub(strings);
    let shown = &notes[extra..];
    let placed = place_notes(b.open, b.frets, shown);
    let mut missing = extra;
    for (n, pos) in shown.iter().zip(&placed) {
        match pos {
            Some((s, f)) => {
                let c = pos2(x_of(*f), y_of(*s));
                let col = input.color(p, *n);
                painter.circle_filled(c, 10.0, col.gamma_multiply(0.25));
                painter.circle_filled(c, 8.0, col);
                painter.text(
                    c,
                    Align2::CENTER_CENTER,
                    crate::music::pitch_class_name(*n),
                    FontId::proportional(9.0),
                    p.on_primary,
                );
            }
            None => missing += 1,
        }
    }
    if missing > 0 {
        painter.text(
            rect.right_top() + vec2(-12.0, 6.0),
            Align2::RIGHT_TOP,
            format!("+{missing} more notes than fit on this instrument"),
            FontId::proportional(11.0),
            p.error,
        );
    }
}

// ------------------------------------------------------------------ flute

/// Hole states for a 6-hole simple-system flute / whistle in C (holes top
/// to bottom; 1 = closed, 2 = half-covered, 0 = open), by semitone above C.
const FLUTE_HOLES: [[u8; 6]; 12] = [
    [1, 1, 1, 1, 1, 1], // C
    [1, 1, 1, 1, 1, 2], // C#
    [1, 1, 1, 1, 1, 0], // D
    [1, 1, 1, 1, 2, 0], // D#
    [1, 1, 1, 1, 0, 0], // E
    [1, 1, 1, 0, 0, 0], // F
    [1, 1, 2, 0, 0, 0], // F#
    [1, 1, 0, 0, 0, 0], // G
    [1, 2, 0, 0, 0, 0], // G#
    [1, 0, 0, 0, 0, 0], // A
    [0, 1, 1, 0, 0, 0], // A# (cross fingering)
    [0, 0, 0, 0, 0, 0], // B
];

pub fn flute_holes(note: i32) -> [u8; 6] {
    FLUTE_HOLES[note.rem_euclid(12) as usize]
}

fn flute(ui: &mut Ui, p: &Palette, input: &VisualInput) {
    let (rect, painter) = area(ui, p, 130.0);
    // A flute plays one note: the highest melody note, else the top of the chord.
    let note = input
        .melody
        .iter()
        .max()
        .or_else(|| input.chord.iter().max())
        .copied();
    painter.text(
        pos2(rect.left() + 14.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "6-hole flute fingering (simple-system flute / whistle in C) · ● covered  ◐ half  ○ open",
        FontId::proportional(12.0),
        p.on_surface_variant,
    );
    // Body.
    let body = Rect::from_min_max(
        pos2(rect.left() + 30.0, rect.center().y - 14.0),
        pos2(
            rect.left() + 30.0 + (rect.width() * 0.62).min(640.0),
            rect.center().y + 18.0,
        ),
    );
    painter.rect_filled(body, CornerRadius::same(16), p.surface_container_high);
    painter.rect_stroke(
        body,
        CornerRadius::same(16),
        Stroke::new(1.0, p.outline),
        StrokeKind::Inside,
    );
    // Mouthpiece.
    painter.circle_stroke(
        pos2(body.left() + 30.0, body.center().y),
        7.0,
        Stroke::new(2.0, p.on_surface_variant),
    );
    let holes = note.map(flute_holes);
    let col = note.map(|n| input.color(p, n)).unwrap_or(p.outline);
    let first = body.left() + body.width() * 0.32;
    let step = body.width() * 0.1;
    for i in 0..6 {
        let gap = if i >= 3 { step * 0.6 } else { 0.0 }; // left hand / right hand
        let c = pos2(first + i as f32 * step + gap, body.center().y);
        let r = 9.0;
        painter.circle_stroke(c, r, Stroke::new(2.0, col));
        match holes.map(|h| h[i]) {
            Some(1) => {
                painter.circle_filled(c, r - 1.0, col);
            }
            Some(2) => {
                // Left half covered.
                let pts: Vec<Pos2> = (0..=16)
                    .map(|k| {
                        let a =
                            std::f32::consts::FRAC_PI_2 + k as f32 / 16.0 * std::f32::consts::PI;
                        c + vec2(a.cos(), -a.sin()) * (r - 1.0)
                    })
                    .collect();
                painter.add(Shape::convex_polygon(pts, col, Stroke::NONE));
            }
            _ => {}
        }
    }
    let info_x = body.right() + 24.0;
    match note {
        Some(n) => {
            let octave = n.div_euclid(12) - 1;
            let how = match octave {
                4 => "low octave: blow gently",
                5 => "high octave: same holes, blow harder",
                o if o < 4 => "lower than this flute: shown for the same note name",
                _ => "higher than this flute: shown for the same note name",
            };
            painter.text(
                pos2(info_x, body.center().y - 10.0),
                Align2::LEFT_CENTER,
                note_name(n),
                FontId::proportional(28.0),
                col,
            );
            painter.text(
                pos2(info_x, body.center().y + 18.0),
                Align2::LEFT_CENTER,
                how,
                FontId::proportional(12.0),
                p.on_surface_variant,
            );
        }
        None => {
            painter.text(
                pos2(info_x, body.center().y),
                Align2::LEFT_CENTER,
                "play a note to see its fingering",
                FontId::proportional(13.0),
                p.outline,
            );
        }
    }
}

// ------------------------------------------------------------------ drums

/// Used when the layout has no drum keys.
const DEFAULT_KIT: [u8; 10] = [36, 38, 42, 46, 39, 45, 47, 50, 49, 51];

fn drum_pads(ui: &mut Ui, p: &Palette, input: &VisualInput) {
    let pads: Vec<u8> = if input.drum_pads.is_empty() {
        DEFAULT_KIT.to_vec()
    } else {
        input.drum_pads.to_vec()
    };
    let cols = pads.len().clamp(1, 10);
    let rows = pads.len().div_ceil(cols);
    let pad_h = 54.0;
    let (rect, painter) = area(ui, p, rows as f32 * (pad_h + 8.0) + 16.0);
    let pad_w = (rect.width() - 16.0) / cols as f32;
    let mut animating = false;
    for (i, d) in pads.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let pr = Rect::from_min_size(
            pos2(
                rect.left() + 8.0 + c as f32 * pad_w,
                rect.top() + 8.0 + r as f32 * (pad_h + 8.0),
            ),
            vec2(pad_w - 8.0, pad_h),
        );
        let glow = input
            .drum_hits
            .get(d)
            .map(|t| 1.0 - (t.elapsed().as_secs_f32() / 0.35).min(1.0))
            .unwrap_or(0.0);
        if glow > 0.0 {
            animating = true;
        }
        let fill = lerp(p.surface_container_high, p.primary, glow);
        painter.rect_filled(pr.expand(glow * 3.0), CornerRadius::same(12), fill);
        painter.text(
            pr.center(),
            Align2::CENTER_CENTER,
            crate::drums::name(*d),
            FontId::proportional(13.0),
            if glow > 0.5 {
                p.on_primary
            } else {
                p.on_surface
            },
        );
    }
    if animating {
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

// --------------------------------------------------------------- waveform

fn waveform(ui: &mut Ui, p: &Palette, input: &VisualInput) {
    let (rect, painter) = area(ui, p, 130.0);
    let time = ui.input(|i| i.time);
    let mut notes: Vec<i32> = input.melody.iter().chain(input.chord).copied().collect();
    notes.sort();
    notes.dedup();
    let wave_rect = Rect::from_min_max(rect.min + vec2(12.0, 10.0), rect.max - vec2(12.0, 30.0));
    let mid = wave_rect.center().y;
    let amp = wave_rect.height() * 0.42;
    // About 12 ms of the real sound, drifting slowly so it shimmers.
    let window = 0.012;
    let points = 360;
    let pts: Vec<Pos2> = (0..=points)
        .map(|k| {
            let x = k as f32 / points as f32;
            let t = x as f64 * window;
            let y = if notes.is_empty() {
                0.0
            } else {
                notes
                    .iter()
                    .map(|n| {
                        let f = crate::music::midi_to_hz(*n as f32) as f64;
                        (std::f64::consts::TAU * (f * t + time * 0.15 * (*n as f64 % 7.0 + 1.0)))
                            .sin()
                    })
                    .sum::<f64>()
                    / (notes.len() as f64).sqrt().max(1.0)
            };
            pos2(
                wave_rect.left() + x * wave_rect.width(),
                mid - (y as f32).clamp(-1.6, 1.6) / 1.6 * amp,
            )
        })
        .collect();
    let col = if input.melody.is_empty() {
        p.tertiary
    } else {
        p.primary
    };
    let col = if notes.is_empty() {
        p.outline_variant
    } else {
        col
    };
    // Glow: wide faint strokes under a sharp one.
    for (w, a) in [(14.0, 0.06), (8.0, 0.12), (4.0, 0.3)] {
        painter.add(Shape::line(
            pts.clone(),
            Stroke::new(w, col.gamma_multiply(a)),
        ));
    }
    painter.add(Shape::line(pts, Stroke::new(1.6, col)));
    // Note bars along the bottom, one per semitone of the range.
    let low = input.low;
    let n_keys = input.octaves * 12 + 1;
    let bw = rect.width() / n_keys as f32;
    for i in 0..n_keys {
        let n = low + i;
        let on = input.melody.contains(&n) || input.chord.contains(&n);
        let r = Rect::from_min_size(
            pos2(rect.left() + i as f32 * bw + 1.0, rect.bottom() - 22.0),
            vec2(bw - 2.0, 12.0),
        );
        let fill = if on {
            input.color(p, n)
        } else if crate::music::is_black(n) {
            p.surface_container_high
        } else {
            p.surface_container_highest
        };
        painter.rect_filled(r, CornerRadius::same(3), fill);
        if on {
            painter.rect_filled(
                r.expand(3.0),
                CornerRadius::same(5),
                fill.gamma_multiply(0.25),
            );
        }
    }
    if input.any() {
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_pick_pictures() {
        assert_eq!(Visual::for_program(0), Visual::Keyboard);
        assert_eq!(Visual::for_program(19), Visual::Keyboard);
        assert_eq!(Visual::for_program(25), Visual::Guitar);
        assert_eq!(Visual::for_program(48), Visual::Strings);
        assert_eq!(Visual::for_program(46), Visual::Keyboard);
        assert_eq!(Visual::for_program(73), Visual::Flute);
        assert_eq!(Visual::for_program(89), Visual::Wave);
        for v in Visual::ALL {
            assert_eq!(Visual::from_code(v.code()), Some(v));
        }
    }

    #[test]
    fn guitar_c_major_is_a_hand_shape() {
        let open = [40, 45, 50, 55, 59, 64];
        // C3 E3 G3 C4 E4 — the open C chord.
        let placed = place_notes(&open, 15, &[48, 52, 55, 60, 64]);
        assert!(placed.iter().all(|p| p.is_some()));
        let frets: Vec<i32> = placed.iter().flatten().map(|(_, f)| *f).collect();
        assert_eq!(frets, vec![3, 2, 0, 1, 0]);
        // A note below the lowest string can't be placed.
        assert_eq!(place_notes(&open, 15, &[30]), vec![None]);
    }

    #[test]
    fn flute_fingerings() {
        assert_eq!(flute_holes(60), [1, 1, 1, 1, 1, 1]);
        assert_eq!(flute_holes(67), [1, 1, 0, 0, 0, 0]);
        assert_eq!(flute_holes(71), [0, 0, 0, 0, 0, 0]);
        // The same holes an octave up.
        assert_eq!(flute_holes(74), flute_holes(62));
    }
}
