//! Drawing for Song mode: the falling notes, the chord sheet, the song
//! timeline and the note editor's piano roll.

use crate::music::{is_black, note_name};
use crate::song::BeatGrid;
use crate::song::practice::{Judgement, Lane};
use crate::theme::Palette;
use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use std::collections::{HashMap, HashSet};

/// How a note is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteState {
    Pending,
    Hit(Judgement),
    /// The app doesn't ask for it (no key plays it).
    Unplayable,
}

/// One block in the falling-notes view.
#[derive(Clone, Debug)]
pub struct NoteDraw {
    pub start_beat: f64,
    pub end_beat: f64,
    pub lane: Lane,
    /// Pitch for the notes lanes.
    pub pitch: i32,
    /// The key to press ("Q"), or empty.
    pub key: String,
    /// Note or chord name.
    pub name: String,
    pub state: NoteState,
    pub low_confidence: bool,
    /// The key plays the same note in another octave.
    pub other_octave: bool,
}

pub struct FallView<'a> {
    pub notes: &'a [NoteDraw],
    pub grid: &'a BeatGrid,
    pub now_beat: f64,
    pub beats_visible: f64,
    pub pitch_lo: i32,
    pub pitch_hi: i32,
    /// Pitch → key label, for the strip under the hit line.
    pub lane_keys: &'a HashMap<i32, String>,
    /// Pitches sounding now (lit on the strip).
    pub held: &'a HashSet<i32>,
    pub chord_lane: bool,
    /// The chord being held (lit in the chord lane).
    pub chord_held: Option<String>,
    pub loop_beats: Option<(f64, f64)>,
    pub sections: &'a [(f64, String)],
    pub drums: bool,
    pub height: f32,
    /// Flash colour on the hit line (right / wrong), 0..1 strength.
    pub flash: Option<(Color32, f32)>,
}

fn dashed_rect(painter: &egui::Painter, r: Rect, stroke: Stroke) {
    let dash = 5.0;
    let gap = 3.0;
    let seg = |a: Pos2, b: Pos2| {
        let len = (b - a).length();
        let dir = (b - a) / len.max(1e-3);
        let mut t = 0.0;
        while t < len {
            let e = (t + dash).min(len);
            painter.line_segment([a + dir * t, a + dir * e], stroke);
            t += dash + gap;
        }
    };
    seg(r.left_top(), r.right_top());
    seg(r.right_top(), r.right_bottom());
    seg(r.right_bottom(), r.left_bottom());
    seg(r.left_bottom(), r.left_top());
}

impl FallView<'_> {
    /// Draws the view. Returns the beat clicked (to seek), if any.
    pub fn show(&self, ui: &mut Ui, p: &Palette) -> Option<f64> {
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, self.height), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(16), p.surface_container_low);

        let cw = if self.chord_lane {
            (width * 0.17).clamp(90.0, 150.0)
        } else {
            0.0
        };
        let strip_h = 34.0;
        let hit_y = rect.bottom() - strip_h - 8.0;
        let top = rect.top() + 6.0;
        let ppb = ((hit_y - top) / self.beats_visible.max(1.0) as f32).max(4.0);
        let y_of = |beat: f64| hit_y - ((beat - self.now_beat) as f32) * ppb;
        let x0 = rect.left() + cw + if self.chord_lane { 8.0 } else { 10.0 };
        let x1 = rect.right() - 10.0;
        let lanes = (self.pitch_hi - self.pitch_lo + 1).max(1);
        let lw = (x1 - x0) / lanes as f32;
        let lane_x = |pitch: i32| {
            x0 + (pitch.clamp(self.pitch_lo, self.pitch_hi) - self.pitch_lo) as f32 * lw
        };

        // Lanes: black-key lanes a little darker (not for drums).
        for pitch in self.pitch_lo..=self.pitch_hi {
            if !self.drums && is_black(pitch) {
                let r =
                    Rect::from_min_max(pos2(lane_x(pitch), top), pos2(lane_x(pitch) + lw, hit_y));
                painter.rect_filled(r, 0.0, p.surface_container.gamma_multiply(0.9));
            }
            if !self.drums && pitch.rem_euclid(12) == 0 {
                painter.line_segment(
                    [pos2(lane_x(pitch), top), pos2(lane_x(pitch), hit_y)],
                    Stroke::new(1.0, p.outline_variant.gamma_multiply(0.4)),
                );
            }
        }
        if self.chord_lane {
            let r = Rect::from_min_max(pos2(rect.left() + 6.0, top), pos2(rect.left() + cw, hit_y));
            painter.rect_filled(r, CornerRadius::same(10), p.surface_container);
        }

        // Loop region.
        if let Some((a, b)) = self.loop_beats {
            let (ya, yb) = (y_of(a).min(hit_y + strip_h), y_of(b).max(top));
            if ya > top && yb < hit_y {
                let r = Rect::from_min_max(pos2(rect.left(), yb), pos2(rect.right(), ya));
                painter.rect_filled(r, 0.0, p.tertiary.gamma_multiply(0.07));
                for y in [ya, yb] {
                    if y > top && y < hit_y {
                        painter.line_segment(
                            [pos2(rect.left(), y), pos2(rect.right(), y)],
                            Stroke::new(1.5, p.tertiary.gamma_multiply(0.6)),
                        );
                    }
                }
            }
        }

        // Beat and bar lines.
        let from = self.now_beat - 2.0;
        let to = self.now_beat + self.beats_visible + 1.0;
        let mut b = from.floor();
        while b <= to {
            let y = y_of(b);
            if y > top && y < hit_y {
                painter.line_segment(
                    [pos2(x0, y), pos2(x1, y)],
                    Stroke::new(1.0, p.outline_variant.gamma_multiply(0.25)),
                );
            }
            b += 1.0;
        }
        let small = FontId::proportional(11.0);
        for (beat, n) in self.grid.bars_between(from, to) {
            let y = y_of(beat);
            if y > top && y < hit_y {
                painter.line_segment(
                    [pos2(rect.left() + 4.0, y), pos2(x1, y)],
                    Stroke::new(1.4, p.outline_variant.gamma_multiply(0.8)),
                );
                painter.text(
                    pos2(x1 - 2.0, y - 2.0),
                    Align2::RIGHT_BOTTOM,
                    format!("bar {n}"),
                    small.clone(),
                    p.outline,
                );
            }
        }
        for (beat, name) in self.sections {
            let y = y_of(*beat);
            if y > top + 10.0 && y < hit_y {
                painter.text(
                    pos2(x0 + 4.0, y - 3.0),
                    Align2::LEFT_BOTTOM,
                    name,
                    FontId::proportional(13.0),
                    p.tertiary,
                );
            }
        }

        // Notes (cut off at the hit line, as if they go into the keys).
        let note_painter = ui.painter_at(Rect::from_min_max(rect.min, pos2(rect.right(), hit_y)));
        let painter_full = painter.clone();
        let painter = note_painter;
        for n in self.notes {
            let (yb, yt) = (y_of(n.start_beat), y_of(n.end_beat));
            if yb < top || yt > hit_y + strip_h {
                continue;
            }
            let (l, r) = match n.lane {
                Lane::Chords => (rect.left() + 10.0, rect.left() + cw - 4.0),
                Lane::Notes => (lane_x(n.pitch) + 1.5, lane_x(n.pitch) + lw - 1.5),
            };
            let h = (yb - yt).max(8.0);
            let nr = Rect::from_min_max(pos2(l, yb - h), pos2(r, yb));
            let base = match n.lane {
                Lane::Chords => p.tertiary,
                Lane::Notes => p.primary,
            };
            let (fill, text) = match n.state {
                NoteState::Pending => (base, p.on_primary),
                NoteState::Hit(Judgement::Perfect) => (p.primary_container, p.on_primary_container),
                NoteState::Hit(Judgement::Good) => {
                    (p.secondary_container, p.on_secondary_container)
                }
                NoteState::Hit(Judgement::Miss) => (p.error.gamma_multiply(0.45), p.on_surface),
                NoteState::Unplayable => (Color32::TRANSPARENT, p.outline),
            };
            let fill = if n.low_confidence {
                fill.gamma_multiply(0.45)
            } else {
                fill
            };
            let radius = CornerRadius::same(6);
            painter.rect_filled(nr, radius, fill);
            if n.state == NoteState::Unplayable {
                painter.rect_stroke(nr, radius, Stroke::new(1.0, p.outline), StrokeKind::Inside);
            }
            if n.low_confidence {
                dashed_rect(
                    &painter,
                    nr.shrink(1.0),
                    Stroke::new(1.3, p.on_surface.gamma_multiply(0.7)),
                );
                if nr.height() > 14.0 && nr.width() > 12.0 {
                    painter.text(
                        nr.right_top() + vec2(-3.0, 2.0),
                        Align2::RIGHT_TOP,
                        "?",
                        FontId::proportional(11.0),
                        p.on_surface,
                    );
                }
            }
            // Label: the key to press, at the bottom where it'll be hit.
            let label = match n.lane {
                Lane::Chords if !n.key.is_empty() => format!("{} · {}", n.name, n.key),
                Lane::Chords => n.name.clone(),
                Lane::Notes => n.key.clone(),
            };
            if !label.is_empty() && nr.height() >= 10.0 {
                let size = (lw * 0.55).clamp(10.0, 16.0);
                let size = if n.lane == Lane::Chords { 14.0 } else { size };
                // Short notes get smaller letters rather than none.
                let size = size.min(nr.height() - 1.0).max(8.0);
                let galley = painter.layout_no_wrap(label, FontId::proportional(size), text);
                if galley.size().x <= nr.width() + 6.0 || n.lane == Lane::Chords {
                    let pos = pos2(
                        nr.center().x - galley.size().x / 2.0,
                        nr.bottom() - galley.size().y - 2.0,
                    );
                    painter.galley(pos, galley, text);
                }
                if n.other_octave && nr.height() > 26.0 {
                    painter.text(
                        pos2(nr.center().x, nr.bottom() - size - 6.0),
                        Align2::CENTER_BOTTOM,
                        "8va",
                        FontId::proportional(9.0),
                        text.gamma_multiply(0.8),
                    );
                }
            }
        }

        let painter = painter_full;
        // Hit line.
        let line_color = match self.flash {
            Some((c, a)) => c.gamma_multiply(0.5 + 0.5 * a),
            None => p.primary,
        };
        painter.rect_filled(
            Rect::from_min_max(
                pos2(rect.left() + 4.0, hit_y - 1.5),
                pos2(rect.right() - 4.0, hit_y + 1.5),
            ),
            CornerRadius::same(2),
            line_color,
        );
        if let Some((c, a)) = self.flash {
            painter.rect_filled(
                Rect::from_min_max(pos2(rect.left(), hit_y - 18.0), pos2(rect.right(), hit_y)),
                0.0,
                c.gamma_multiply(0.18 * a),
            );
        }

        // Strip: which laptop key is each lane.
        let sy = hit_y + 6.0;
        for pitch in self.pitch_lo..=self.pitch_hi {
            let r =
                Rect::from_min_size(pos2(lane_x(pitch) + 1.0, sy), vec2(lw - 2.0, strip_h - 4.0));
            let held = self.held.contains(&pitch);
            let key = self.lane_keys.get(&pitch);
            let fill = if held {
                p.primary
            } else if key.is_some() {
                if !self.drums && is_black(pitch) {
                    p.secondary_container
                } else {
                    p.surface_container_highest
                }
            } else {
                p.surface_container
            };
            painter.rect_filled(r, CornerRadius::same(5), fill);
            let tc = if held { p.on_primary } else { p.on_surface };
            if let Some(k) = key
                && lw > 9.0
            {
                painter.text(
                    r.center() - vec2(0.0, 4.0),
                    Align2::CENTER_CENTER,
                    k,
                    FontId::proportional((lw * 0.5).clamp(9.0, 15.0)),
                    tc,
                );
            }
            if lw > 22.0 && !self.drums {
                painter.text(
                    pos2(r.center().x, r.bottom() - 2.0),
                    Align2::CENTER_BOTTOM,
                    note_name(pitch),
                    FontId::proportional(8.5),
                    tc.gamma_multiply(0.7),
                );
            }
        }
        if self.chord_lane {
            let r =
                Rect::from_min_size(pos2(rect.left() + 8.0, sy), vec2(cw - 10.0, strip_h - 4.0));
            let on = self.chord_held.is_some();
            painter.rect_filled(
                r,
                CornerRadius::same(5),
                if on {
                    p.tertiary
                } else {
                    p.surface_container_high
                },
            );
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                self.chord_held.clone().unwrap_or_else(|| "chords".into()),
                FontId::proportional(13.0),
                if on { p.surface } else { p.on_surface_variant },
            );
        }

        // Click: seek to that beat.
        resp.clicked()
            .then(|| resp.interact_pointer_pos())
            .flatten()
            .filter(|pos| pos.y < hit_y)
            .map(|pos| self.now_beat + ((hit_y - pos.y) / ppb) as f64)
    }
}

/// One chord on the chord sheet.
#[derive(Clone, Debug)]
pub struct SheetChord {
    pub start_beat: f64,
    pub end_beat: f64,
    pub symbol: String,
    pub roman: Option<String>,
    pub key: Option<String>,
    pub low_confidence: bool,
}

/// The chord sheet: bars in rows of four, the current bar lit. Returns the
/// beat of a clicked bar.
#[allow(clippy::too_many_arguments)]
pub fn chord_sheet(
    ui: &mut Ui,
    p: &Palette,
    chords: &[SheetChord],
    grid: &BeatGrid,
    now_beat: f64,
    end_beat: f64,
    sections: &[(f64, String)],
    loop_beats: Option<(f64, f64)>,
    follow: bool,
) -> Option<f64> {
    let bars = grid.bars_between(grid.beat_at(0.0).min(0.0) - 0.01, end_beat);
    let bars: Vec<(f64, i32)> = bars.into_iter().filter(|(b, _)| *b < end_beat).collect();
    if bars.is_empty() {
        ui.label(egui::RichText::new("No chords in this song.").color(p.on_surface_variant));
        return None;
    }
    let per_row = 4;
    let width = ui.available_width();
    let cell_w = width / per_row as f32;
    let cell_h = 50.0;
    let mut clicked = None;
    let bar_end = |i: usize| {
        bars.get(i + 1)
            .map(|b| b.0)
            .unwrap_or(bars[i].0 + grid.beats_per_bar() as f64)
    };
    for (row_i, row) in bars.chunks(per_row).enumerate() {
        // Section name above the row where one starts.
        let row_start = row[0].0;
        let row_end = bar_end(row_i * per_row + row.len() - 1);
        if let Some((_, name)) = sections
            .iter()
            .find(|(b, _)| *b >= row_start - 0.01 && *b < row_end)
        {
            ui.label(egui::RichText::new(name).small().strong().color(p.tertiary));
        }
        let (rect, _) = ui.allocate_exact_size(vec2(width, cell_h), Sense::hover());
        let painter = ui.painter_at(rect.expand(2.0));
        let mut current_rect = None;
        for (ci, &(b0, num)) in row.iter().enumerate() {
            let idx = row_i * per_row + ci;
            let b1 = bar_end(idx);
            let r = Rect::from_min_size(
                pos2(rect.left() + ci as f32 * cell_w, rect.top()),
                vec2(cell_w - 4.0, cell_h - 4.0),
            );
            let id = ui.scope_id().with(("bar", idx));
            let resp = ui.interact(r, id, Sense::click());
            if resp.clicked() {
                clicked = Some(b0);
            }
            let now = now_beat >= b0 && now_beat < b1;
            let in_loop = loop_beats.is_some_and(|(a, b)| b0 >= a - 0.01 && b1 <= b + 0.01);
            let fill = if now {
                p.primary_container
            } else if resp.hovered() {
                p.surface_container_highest
            } else if in_loop {
                p.tertiary_container.gamma_multiply(0.6)
            } else {
                p.surface_container_high
            };
            painter.rect_filled(r, CornerRadius::same(8), fill);
            if now {
                current_rect = Some(r);
                let x = r.left() + (((now_beat - b0) / (b1 - b0)) as f32) * r.width();
                painter.line_segment(
                    [pos2(x, r.bottom() - 6.0), pos2(x, r.bottom() - 1.0)],
                    Stroke::new(3.0, p.primary),
                );
            }
            painter.text(
                r.left_top() + vec2(5.0, 3.0),
                Align2::LEFT_TOP,
                num.to_string(),
                FontId::proportional(9.0),
                p.outline,
            );
            // Chords starting in this bar, or the one carried over.
            let mut inside: Vec<&SheetChord> = chords
                .iter()
                .filter(|c| c.start_beat >= b0 - 0.01 && c.start_beat < b1 - 0.01)
                .collect();
            let carried = inside.is_empty() || inside[0].start_beat > b0 + 0.01;
            let carry = chords
                .iter()
                .rfind(|c| c.start_beat < b0 - 0.01 && c.end_beat > b0 + 0.01);
            if carried && let Some(c) = carry {
                inside.insert(0, c);
            }
            let n = inside.len().max(1) as f32;
            for (k, c) in inside.iter().enumerate() {
                let is_carry = carried && k == 0 && carry.is_some_and(|x| std::ptr::eq(x, *c));
                let x = r.left() + 6.0 + k as f32 * (r.width() - 8.0) / n;
                let active = now_beat >= c.start_beat && now_beat < c.end_beat && now;
                let color = if is_carry {
                    p.outline
                } else if c.low_confidence {
                    p.on_surface.gamma_multiply(0.55)
                } else if active {
                    p.on_primary_container
                } else {
                    p.on_surface
                };
                let text = if is_carry {
                    "–".to_string()
                } else {
                    c.symbol.clone()
                };
                let size = if n > 2.0 { 13.0 } else { 17.0 };
                painter.text(
                    pos2(x, r.top() + 13.0),
                    Align2::LEFT_TOP,
                    if c.low_confidence && !is_carry {
                        format!("{text}?")
                    } else {
                        text
                    },
                    FontId::proportional(size),
                    color,
                );
                if !is_carry {
                    let sub = match (&c.key, &c.roman) {
                        (Some(k), Some(rn)) => format!("{rn} · {k}"),
                        (Some(k), None) => k.clone(),
                        (None, Some(rn)) => rn.clone(),
                        _ => String::new(),
                    };
                    painter.text(
                        pos2(x, r.bottom() - 4.0),
                        Align2::LEFT_BOTTOM,
                        sub,
                        FontId::proportional(10.0),
                        color.gamma_multiply(0.75),
                    );
                }
            }
        }
        if follow && let Some(r) = current_rect {
            ui.scroll_to_rect(r.expand(30.0), None);
        }
    }
    clicked
}

/// What the user did on the timeline.
#[derive(Default)]
pub struct TimelineResponse {
    pub seek: Option<f64>,
    /// Dragged across a stretch (seconds).
    pub selected: Option<(f64, f64)>,
}

/// A strip for the whole song: sections, the loop, misses, the play head.
/// Click to jump, drag to pick a stretch to loop.
pub fn timeline(
    ui: &mut Ui,
    p: &Palette,
    duration: f64,
    now: f64,
    sections: &[(f64, f64, String)],
    looping: Option<(f64, f64)>,
    marks: &[(f64, Color32)],
) -> TimelineResponse {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(1.0));
    let d = duration.max(0.1);
    let x_of = |t: f64| rect.left() + ((t / d).clamp(0.0, 1.0) as f32) * rect.width();
    let t_of = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * d;
    painter.rect_filled(rect, CornerRadius::same(8), p.surface_container_high);
    let colors = [
        p.primary_container,
        p.secondary_container,
        p.tertiary_container,
    ];
    for (s, e, label) in sections {
        let r = Rect::from_min_max(
            pos2(x_of(*s) + 1.0, rect.top() + 2.0),
            pos2(x_of(*e) - 1.0, rect.bottom() - 2.0),
        );
        let ci = label.bytes().next().map(|b| b as usize).unwrap_or(0) % colors.len();
        painter.rect_filled(r, CornerRadius::same(6), colors[ci].gamma_multiply(0.7));
        if r.width() > 30.0 {
            painter.text(
                r.left_center() + vec2(5.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(10.5),
                p.on_surface,
            );
        }
    }
    for (t, c) in marks {
        let x = x_of(*t);
        painter.line_segment(
            [pos2(x, rect.bottom() - 6.0), pos2(x, rect.bottom() - 2.0)],
            Stroke::new(1.5, *c),
        );
    }
    if let Some((a, b)) = looping {
        let r = Rect::from_min_max(pos2(x_of(a), rect.top()), pos2(x_of(b), rect.bottom()));
        painter.rect_stroke(
            r,
            CornerRadius::same(6),
            Stroke::new(2.0, p.tertiary),
            StrokeKind::Inside,
        );
    }
    let mut out = TimelineResponse::default();
    if resp.dragged()
        && let (Some(start), Some(cur)) = (
            ui.input(|i| i.pointer.press_origin()),
            resp.interact_pointer_pos(),
        )
    {
        let (a, b) = (t_of(start.x.min(cur.x)), t_of(start.x.max(cur.x)));
        let r = Rect::from_min_max(pos2(x_of(a), rect.top()), pos2(x_of(b), rect.bottom()));
        painter.rect_filled(r, CornerRadius::same(6), p.tertiary.gamma_multiply(0.25));
    }
    if resp.drag_stopped()
        && let (Some(start), Some(cur)) = (
            ui.input(|i| i.pointer.press_origin()),
            resp.interact_pointer_pos(),
        )
        && (start.x - cur.x).abs() > 6.0
    {
        out.selected = Some((t_of(start.x.min(cur.x)), t_of(start.x.max(cur.x))));
    } else if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        out.seek = Some(t_of(pos.x));
    }
    let x = x_of(now);
    painter.line_segment(
        [pos2(x, rect.top() - 1.0), pos2(x, rect.bottom() + 1.0)],
        Stroke::new(2.5, p.primary),
    );
    out
}

// ------------------------------------------------------------ piano roll

/// View and drag state of the note editor.
pub struct RollState {
    pub beat0: f64,
    pub ppb: f32,
    pub pitch_top: i32,
    pub row_h: f32,
    drag: Option<RollDrag>,
}

impl Default for RollState {
    fn default() -> Self {
        RollState {
            beat0: -1.0,
            ppb: 60.0,
            pitch_top: 84,
            row_h: 14.0,
            drag: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum RollDrag {
    Move { origin: Pos2 },
    Resize { idx: usize },
    Pan { origin: Pos2, beat0: f64, top: i32 },
}

/// What happened in the piano roll this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum RollAction {
    /// Click on a note (index), `add` = Shift/Ctrl held.
    Select {
        idx: Option<usize>,
        add: bool,
    },
    Move {
        d_beats: f64,
        d_pitch: i32,
    },
    Resize {
        idx: usize,
        end_beat: f64,
    },
    /// Double-click on empty space.
    Add {
        beat: f64,
        pitch: i32,
    },
    Seek(f64),
}

/// A note for the piano roll.
pub struct RollNote {
    pub start_beat: f64,
    pub end_beat: f64,
    pub pitch: i32,
    pub low_confidence: bool,
    pub confidence: f32,
}

impl RollState {
    /// Scroll so a note is visible.
    pub fn reveal(&mut self, beat: f64, pitch: i32, width: f32, rows: i32) {
        let visible = (width / self.ppb) as f64;
        if beat < self.beat0 + 0.5 || beat > self.beat0 + visible - 1.0 {
            self.beat0 = beat - visible * 0.3;
        }
        if pitch > self.pitch_top - 1 || pitch < self.pitch_top - rows + 2 {
            self.pitch_top = pitch + rows / 2;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn piano_roll(
    ui: &mut Ui,
    p: &Palette,
    st: &mut RollState,
    notes: &[RollNote],
    selected: &[usize],
    grid: &BeatGrid,
    snap: f64,
    playhead: Option<f64>,
    chords: &[(f64, f64, String)],
    height: f32,
) -> Vec<RollAction> {
    let mut out = Vec::new();
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    // Panning moves the view before anything is drawn.
    let pointer = resp
        .interact_pointer_pos()
        .or(ui.input(|i| i.pointer.hover_pos()));
    if let (Some(RollDrag::Pan { origin, beat0, top }), Some(pos)) = (st.drag, pointer) {
        st.beat0 = beat0 - ((pos.x - origin.x) / st.ppb) as f64;
        st.pitch_top = (top + ((pos.y - origin.y) / st.row_h).round() as i32).clamp(12, 127);
    }
    let painter = ui.painter_at(rect);
    let keys_w = 44.0;
    let head_h = 22.0;
    let area = Rect::from_min_max(pos2(rect.left() + keys_w, rect.top() + head_h), rect.max);
    let rows = (area.height() / st.row_h).floor() as i32;
    let x_of = |b: f64| area.left() + ((b - st.beat0) as f32) * st.ppb;
    let b_of = |x: f32| st.beat0 + ((x - area.left()) / st.ppb) as f64;
    let y_of = |pitch: i32| area.top() + (st.pitch_top - pitch) as f32 * st.row_h;
    let pitch_of = |y: f32| st.pitch_top - ((y - area.top()) / st.row_h).floor() as i32;
    let snapf = |b: f64| {
        if snap > 0.0 {
            BeatGrid::snap(b, snap)
        } else {
            b
        }
    };

    painter.rect_filled(rect, CornerRadius::same(12), p.surface_container_low);
    // Rows.
    for r in 0..=rows {
        let pitch = st.pitch_top - r;
        let y = y_of(pitch);
        let row = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), st.row_h));
        if is_black(pitch) {
            painter.rect_filled(row, 0.0, p.surface_container.gamma_multiply(0.9));
        }
        let key = Rect::from_min_size(
            pos2(rect.left() + 2.0, y),
            vec2(keys_w - 4.0, st.row_h - 1.0),
        );
        painter.rect_filled(
            key,
            CornerRadius::same(2),
            if is_black(pitch) {
                Color32::from_gray(40)
            } else {
                Color32::from_gray(235)
            },
        );
        if pitch.rem_euclid(12) == 0 || st.row_h >= 16.0 {
            painter.text(
                key.right_center() - vec2(3.0, 0.0),
                Align2::RIGHT_CENTER,
                note_name(pitch),
                FontId::proportional(9.0),
                if is_black(pitch) {
                    Color32::from_gray(220)
                } else {
                    Color32::from_gray(60)
                },
            );
        }
        if pitch.rem_euclid(12) == 0 {
            painter.line_segment(
                [
                    pos2(area.left(), y + st.row_h),
                    pos2(area.right(), y + st.row_h),
                ],
                Stroke::new(1.0, p.outline_variant.gamma_multiply(0.6)),
            );
        }
    }
    // Beats, bars, chords along the top.
    let visible = (area.width() / st.ppb) as f64;
    let mut b = st.beat0.floor();
    while b < st.beat0 + visible + 1.0 {
        let x = x_of(b);
        if x >= area.left() {
            painter.line_segment(
                [pos2(x, area.top()), pos2(x, area.bottom())],
                Stroke::new(1.0, p.outline_variant.gamma_multiply(0.3)),
            );
        }
        b += 1.0;
    }
    for (beat, n) in grid.bars_between(st.beat0, st.beat0 + visible) {
        let x = x_of(beat);
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, area.bottom())],
            Stroke::new(1.4, p.outline_variant),
        );
        painter.text(
            pos2(x + 3.0, rect.top() + 2.0),
            Align2::LEFT_TOP,
            n.to_string(),
            FontId::proportional(10.0),
            p.outline,
        );
    }
    for (s, e, sym) in chords {
        let (xs, xe) = (x_of(*s).max(area.left()), x_of(*e));
        if xe < area.left() || xs > area.right() {
            continue;
        }
        painter.text(
            pos2(xs + 14.0, rect.top() + 3.0),
            Align2::LEFT_TOP,
            sym,
            FontId::proportional(11.0),
            p.tertiary,
        );
    }

    // Drag preview offsets.
    let (mut d_beats, mut d_pitch, mut resize_to) = (0.0, 0, None);
    if let (Some(drag), Some(pos)) = (st.drag, pointer) {
        match drag {
            RollDrag::Move { origin } => {
                d_beats = snapf(((pos.x - origin.x) / st.ppb) as f64);
                d_pitch = -((pos.y - origin.y) / st.row_h).round() as i32;
            }
            RollDrag::Resize { idx } => {
                resize_to = Some((
                    idx,
                    snapf(b_of(pos.x)).max(notes[idx].start_beat + snap.max(1.0 / 16.0)),
                ));
            }
            RollDrag::Pan { .. } => {}
        }
    }

    // Notes.
    let mut hit: Option<(usize, bool)> = None;
    for (i, n) in notes.iter().enumerate() {
        let sel = selected.contains(&i);
        let (mut s, mut e, mut pitch) = (n.start_beat, n.end_beat, n.pitch);
        if sel && st.drag.is_some_and(|d| matches!(d, RollDrag::Move { .. })) {
            s += d_beats;
            e += d_beats;
            pitch += d_pitch;
        }
        if let Some((ri, to)) = resize_to
            && ri == i
        {
            e = to;
        }
        let r = Rect::from_min_max(
            pos2(x_of(s), y_of(pitch) + 1.0),
            pos2(x_of(e).max(x_of(s) + 4.0), y_of(pitch) + st.row_h - 1.0),
        );
        if !r.intersects(area) {
            continue;
        }
        let fill = if n.low_confidence {
            p.tertiary.gamma_multiply(0.35 + 0.4 * n.confidence)
        } else {
            p.primary
        };
        painter.rect_filled(r, CornerRadius::same(3), fill);
        if n.low_confidence {
            dashed_rect(&painter, r, Stroke::new(1.2, p.on_surface));
        }
        if sel {
            painter.rect_stroke(
                r.expand(1.5),
                CornerRadius::same(4),
                Stroke::new(2.5, p.on_surface),
                StrokeKind::Outside,
            );
        }
        if let Some(pos) = pointer
            && r.contains(pos)
        {
            hit = Some((i, pos.x > r.right() - 7.0));
        }
    }
    if let Some(ph) = playhead {
        let x = x_of(ph);
        if x > area.left() && x < area.right() {
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, area.bottom())],
                Stroke::new(2.0, p.error),
            );
        }
    }

    // Cursor over a note's right edge: resize arrows.
    if let Some((_, true)) = hit
        && st.drag.is_none()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    // Input.
    let mods = ui.input(|i| i.modifiers);
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
        let middle = ui.input(|i| i.pointer.middle_down() || i.pointer.secondary_down());
        st.drag = Some(match hit {
            _ if middle => RollDrag::Pan {
                origin,
                beat0: st.beat0,
                top: st.pitch_top,
            },
            Some((i, true)) => RollDrag::Resize { idx: i },
            Some((i, false)) => {
                if !selected.contains(&i) {
                    out.push(RollAction::Select {
                        idx: Some(i),
                        add: mods.shift || mods.ctrl,
                    });
                }
                RollDrag::Move { origin }
            }
            None => RollDrag::Pan {
                origin,
                beat0: st.beat0,
                top: st.pitch_top,
            },
        });
    }
    if resp.drag_stopped() {
        match st.drag.take() {
            Some(RollDrag::Move { .. }) if d_beats != 0.0 || d_pitch != 0 => {
                out.push(RollAction::Move { d_beats, d_pitch })
            }
            Some(RollDrag::Resize { .. }) => {
                if let Some((idx, to)) = resize_to {
                    out.push(RollAction::Resize { idx, end_beat: to });
                }
            }
            _ => {}
        }
    }
    if resp.double_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && hit.is_none()
        && area.contains(pos)
    {
        let beat = if snap > 0.0 {
            (b_of(pos.x) / snap).floor() * snap
        } else {
            b_of(pos.x)
        };
        out.push(RollAction::Add {
            beat,
            pitch: pitch_of(pos.y),
        });
    } else if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        if pos.y < area.top() {
            out.push(RollAction::Seek(b_of(pos.x)));
        } else {
            out.push(RollAction::Select {
                idx: hit.map(|h| h.0),
                add: mods.shift || mods.ctrl,
            });
        }
    }
    // Wheel: scroll time; Shift+wheel: pitch; Ctrl+wheel: zoom.
    if resp.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        if zoom != 1.0 {
            st.ppb = (st.ppb * zoom).clamp(12.0, 400.0);
        } else if scroll != egui::Vec2::ZERO {
            if mods.shift {
                st.pitch_top = (st.pitch_top + (scroll.y / st.row_h).round() as i32).clamp(12, 127);
            } else {
                st.beat0 -= ((scroll.y + scroll.x) / st.ppb) as f64;
            }
        }
    }
    out
}
