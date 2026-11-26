//! The Progress tab (Part 9): streak, today vs the goal, this week, a
//! calendar of the last few months, lessons / ear games / songs.

use super::*;
use crate::stats::{Day, DayStats, MONTHS, fmt_duration};

/// Weeks shown in the calendar.
const WEEKS: i32 = 20;

/// Mix two colours (t = 0 → a, 1 → b).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let [ar, ag, ab, _] = a.to_array();
    let [br, bg, bb, _] = b.to_array();
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(ar, br), f(ag, bg), f(ab, bb))
}

fn big_number(ui: &mut egui::Ui, p: &Palette, value: String, label: &str, sub: String) {
    ui.set_min_height(120.0);
    ui.label(RichText::new(label).color(p.on_surface_variant));
    ui.label(RichText::new(value).size(30.0).strong().color(p.primary));
    if !sub.is_empty() {
        ui.label(RichText::new(sub).small().color(p.on_surface_variant));
    }
}

fn day_tooltip(d: Day, s: &DayStats) -> String {
    if s.total_s() <= 0.0 && s.notes == 0 {
        return format!("{}: no practice", d.short_label());
    }
    let mut t = format!("{}: {}", d.short_label(), fmt_duration(s.total_s()));
    for (secs, what) in [
        (s.play_s, "free play"),
        (s.learn_s, "lessons & ear training"),
        (s.song_s, "songs"),
    ] {
        if secs >= 1.0 {
            t += &format!("\n  {} {what}", fmt_duration(secs));
        }
    }
    t += &format!("\n  {} notes & chords", s.notes);
    if s.lessons > 0 {
        t += &format!(", {} lessons", s.lessons);
    }
    if s.songs > 0 {
        t += &format!(", {} song runs", s.songs);
    }
    t
}

impl QwertoneApp {
    pub(super) fn progress_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let today = self.stats.today();
        let st = self.stats.streak();
        let td = self.stats.today_stats();
        let goal_s = self.cfg.practice.goal_minutes as f64 * 60.0;
        let week = today.week_start();
        let this_week = self.stats.stats.total_between(week, today);
        let last_week = self
            .stats
            .stats
            .total_between(week.plus(-7), week.plus(-1));
        let totals = self.stats.stats.totals();
        let days = self.stats.stats.days_practised();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(4, |cols| {
                card(&mut cols[0], &p, "", |ui| {
                    big_number(
                        ui,
                        &p,
                        match st.current {
                            1 => "1 day".into(),
                            n => format!("{n} days"),
                        },
                        "Streak",
                        if st.today_done || st.current == 0 {
                            format!("Best: {} days", st.best)
                        } else {
                            "Play a minute today to keep it".into()
                        },
                    );
                });
                card(&mut cols[1], &p, "", |ui| {
                    big_number(
                        ui,
                        &p,
                        fmt_duration(td.total_s()),
                        "Today",
                        if goal_s > 0.0 {
                            format!("Goal {}", fmt_duration(goal_s))
                        } else {
                            String::new()
                        },
                    );
                    if goal_s > 0.0 {
                        let f = (td.total_s() / goal_s).min(1.0) as f32;
                        ui.add(
                            egui::ProgressBar::new(f)
                                .desired_height(8.0)
                                .fill(p.primary)
                                .corner_radius(egui::CornerRadius::same(4)),
                        );
                        if f >= 1.0 {
                            ui.label(RichText::new("Goal reached ✓").color(p.tertiary));
                        }
                    }
                });
                card(&mut cols[2], &p, "", |ui| {
                    let change = if last_week > 0.0 {
                        let pct = (this_week / last_week - 1.0) * 100.0;
                        format!("{pct:+.0}% vs last week ({})", fmt_duration(last_week))
                    } else {
                        "Nothing last week".into()
                    };
                    big_number(ui, &p, fmt_duration(this_week), "This week", change);
                });
                card(&mut cols[3], &p, "", |ui| {
                    big_number(
                        ui,
                        &p,
                        fmt_duration(totals.total_s()),
                        "All time",
                        format!("{days} days · {} notes", totals.notes),
                    );
                });
            });
            ui.add_space(8.0);

            ui.columns(2, |cols| {
                card(&mut cols[0], &p, "This week", |ui| self.week_bars(ui, week, today));
                card(&mut cols[1], &p, "Where the time went", |ui| {
                    let t = totals.total_s().max(1.0);
                    egui::Grid::new("split").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
                        for (label, secs, tip) in [
                            ("Free play", totals.play_s, "Play tab, and keys pressed on other tabs"),
                            ("Lessons & ear training", totals.learn_s, "Learn tab"),
                            ("Songs", totals.song_s, "Song tab, Jam and Spotify sync"),
                        ] {
                            ui.label(label).on_hover_text(tip);
                            ui.add(
                                egui::ProgressBar::new((secs / t) as f32)
                                    .desired_width(160.0)
                                    .desired_height(8.0)
                                    .fill(p.primary)
                                    .corner_radius(egui::CornerRadius::same(4)),
                            );
                            ui.label(fmt_duration(secs));
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    let lessons_total = crate::learn::lessons::all().count();
                    let lessons_done = crate::learn::lessons::all()
                        .filter(|l| self.cfg.learn.done.contains(l.id))
                        .count();
                    ui.label(format!(
                        "Lessons: {lessons_done} of {lessons_total} done · ear training: {} right answers · song runs: {}",
                        totals.ear, totals.songs
                    ));
                    if !self.cfg.learn.best.is_empty() {
                        let best = self.cfg.learn.best.values().max().copied().unwrap_or(0);
                        ui.label(
                            RichText::new(format!("Longest ear-training streak: {best} in a row"))
                                .color(p.on_surface_variant),
                        );
                    }
                });
            });
            ui.add_space(8.0);
            card(ui, &p, &format!("Last {WEEKS} weeks"), |ui| self.calendar(ui, today));
            ui.add_space(8.0);
            card(ui, &p, "Your songs", |ui| self.song_records(ui));
            ui.add_space(8.0);
            card(ui, &p, "", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(
                            "Stats stay on this computer. Change the daily goal in Settings → General.",
                        )
                        .color(p.on_surface_variant),
                    );
                    let armed = self
                        .confirm_reset_stats
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(4));
                    let btn = egui::Button::new(
                        RichText::new(if armed { "Really reset?" } else { "Reset stats" })
                            .color(if armed { p.surface } else { p.error }),
                    )
                    .fill(if armed { p.error } else { Color32::TRANSPARENT });
                    if ui.add(btn).clicked() {
                        if armed {
                            self.stats.reset();
                            let _ = self.stats.maybe_save(true);
                            self.confirm_reset_stats = None;
                            self.notify("Practice stats reset");
                        } else {
                            self.confirm_reset_stats = Some(Instant::now());
                        }
                    }
                    if armed {
                        ui.ctx().request_repaint_after(Duration::from_millis(500));
                    }
                });
            });
        });
    }

    /// Minutes per day, Monday to Sunday, with the goal as a line.
    fn week_bars(&self, ui: &mut egui::Ui, week: Day, today: Day) {
        let p = self.palette;
        let goal_s = self.cfg.practice.goal_minutes as f64 * 60.0;
        let vals: Vec<(Day, DayStats)> = (0..7)
            .map(|i| (week.plus(i), self.stats.stats.day(week.plus(i))))
            .collect();
        let max = vals
            .iter()
            .map(|(_, s)| s.total_s())
            .fold(goal_s.max(60.0), f64::max);
        let w = ui.available_width();
        let h = 130.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h + 22.0), egui::Sense::hover());
        let plot = egui::Rect::from_min_size(rect.min, egui::vec2(w, h));
        let painter = ui.painter_at(rect);
        painter.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            egui::Stroke::new(1.0, p.outline_variant),
        );
        let slot = w / 7.0;
        const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        for (i, (d, s)) in vals.iter().enumerate() {
            let x0 = plot.left() + slot * i as f32;
            let bw = (slot * 0.5).min(36.0);
            let cx = x0 + slot / 2.0;
            let frac = (s.total_s() / max) as f32;
            if frac > 0.0 {
                let top = plot.bottom() - (h - 6.0) * frac.max(0.03);
                let bar = egui::Rect::from_min_max(
                    egui::pos2(cx - bw / 2.0, top),
                    egui::pos2(cx + bw / 2.0, plot.bottom()),
                );
                let color = if *d > today { p.outline_variant } else { p.primary };
                painter.rect_filled(
                    bar,
                    egui::CornerRadius {
                        nw: 4,
                        ne: 4,
                        sw: 0,
                        se: 0,
                    },
                    color,
                );
            }
            let label_color = if *d == today {
                p.on_surface
            } else {
                p.on_surface_variant
            };
            painter.text(
                egui::pos2(cx, plot.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                WD[i],
                egui::FontId::proportional(12.0),
                label_color,
            );
            // Hover target: the whole column.
            let col = egui::Rect::from_min_max(
                egui::pos2(x0, plot.top()),
                egui::pos2(x0 + slot, rect.bottom()),
            );
            ui.interact(col, ui.scope_id().with(("wk", i)), egui::Sense::hover())
                .on_hover_text(day_tooltip(*d, s));
        }
        if goal_s > 0.0 {
            let y = plot.bottom() - (h - 6.0) * (goal_s / max) as f32;
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                egui::Stroke::new(1.0, p.tertiary),
            );
            painter.text(
                egui::pos2(plot.right(), y - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("goal {}", fmt_duration(goal_s)),
                egui::FontId::proportional(11.0),
                p.on_surface_variant,
            );
        }
    }

    /// GitHub-style calendar: a column per week, Monday at the top. One
    /// hue, darker / brighter = more practice.
    fn calendar(&self, ui: &mut egui::Ui, today: Day) {
        let p = self.palette;
        let start = today.week_start().plus(-7 * (WEEKS - 1));
        let goal_s = (self.cfg.practice.goal_minutes as f64 * 60.0).max(600.0);
        let gap = 3.0;
        let left = 34.0;
        let cell = ((ui.available_width() - left) / WEEKS as f32 - gap).clamp(12.0, 24.0);
        let top = 18.0;
        let w = left + WEEKS as f32 * (cell + gap);
        let h = top + 7.0 * (cell + gap);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        for (row, name) in [(0, "Mon"), (2, "Wed"), (4, "Fri"), (6, "Sun")] {
            painter.text(
                egui::pos2(rect.left(), rect.top() + top + row as f32 * (cell + gap) + cell / 2.0),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(11.0),
                p.on_surface_variant,
            );
        }
        let mut last_month = 0;
        for wk in 0..WEEKS {
            let monday = start.plus(wk * 7);
            let (_, m, _) = monday.ymd();
            let x = rect.left() + left + wk as f32 * (cell + gap);
            if m != last_month {
                painter.text(
                    egui::pos2(x, rect.top()),
                    egui::Align2::LEFT_TOP,
                    MONTHS[m as usize - 1],
                    egui::FontId::proportional(11.0),
                    p.on_surface_variant,
                );
                last_month = m;
            }
            for dow in 0..7 {
                let d = monday.plus(dow);
                if d > today {
                    continue;
                }
                let s = self.stats.stats.day(d);
                let t = s.total_s();
                let fill = if t <= 0.0 {
                    p.surface_container_high
                } else {
                    // Four steps: a little, half the goal, the goal, double.
                    let step = if t < goal_s * 0.5 {
                        0.35
                    } else if t < goal_s {
                        0.55
                    } else if t < goal_s * 2.0 {
                        0.8
                    } else {
                        1.0
                    };
                    mix(p.surface_container_high, p.primary, step)
                };
                let r = egui::Rect::from_min_size(
                    egui::pos2(x, rect.top() + top + dow as f32 * (cell + gap)),
                    egui::vec2(cell, cell),
                );
                painter.rect_filled(r, egui::CornerRadius::same(3), fill);
                if d == today {
                    painter.rect_stroke(
                        r,
                        egui::CornerRadius::same(3),
                        egui::Stroke::new(1.5, p.on_surface),
                        egui::StrokeKind::Outside,
                    );
                }
                ui.interact(r.expand(gap / 2.0), ui.scope_id().with(("cal", d.0)), egui::Sense::hover())
                    .on_hover_text(day_tooltip(d, &s));
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Less").small().color(p.on_surface_variant));
            for step in [0.0, 0.35, 0.55, 0.8, 1.0] {
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    r,
                    egui::CornerRadius::same(3),
                    mix(p.surface_container_high, p.primary, step),
                );
            }
            ui.label(RichText::new("More (the strongest colour = twice your daily goal or more)").small().color(p.on_surface_variant));
        });
    }

    fn song_records(&self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut songs: Vec<_> = self.stats.stats.songs.values().collect();
        if songs.is_empty() {
            ui.label(
                RichText::new(
                    "Play a song to the end in the Song tab and it shows up here with your best score.",
                )
                .color(p.on_surface_variant),
            );
            return;
        }
        songs.sort_by(|a, b| b.last_day.cmp(&a.last_day).then(b.plays.cmp(&a.plays)));
        egui::Grid::new("song-records")
            .striped(true)
            .num_columns(4)
            .spacing([24.0, 6.0])
            .show(ui, |ui| {
                for h in ["Song", "Runs", "Best", "Last"] {
                    ui.label(RichText::new(h).small().color(p.on_surface_variant));
                }
                ui.end_row();
                for r in songs.iter().take(12) {
                    ui.label(if r.title.is_empty() { "(untitled)" } else { &r.title });
                    ui.label(r.plays.to_string());
                    ui.label(format!("{}%", r.best));
                    let when = Day::parse(&r.last_day)
                        .map(|d| d.short_label())
                        .unwrap_or_default();
                    ui.label(format!("{}% · {when}", r.last));
                    ui.end_row();
                }
            });
    }
}
