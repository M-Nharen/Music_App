//! Settings (Part 9): one header button for General, Keybinds, Audio &
//! latency and Keyboard test; the General page (colours, window size,
//! practice goal, where files live); and the small hooks that feed the
//! practice stats.

use super::*;
use crate::stats::{Activity, fmt_duration};

const PAGES: [(Tab, &str); 4] = [
    (Tab::General, "General"),
    (Tab::Keybinds, "Keybinds"),
    (Tab::Audio, "Audio & latency"),
    (Tab::KeyTest, "Keyboard test"),
];

/// Open a file or folder in the desktop's file manager / editor, without
/// leaving a zombie process behind.
pub(super) fn open_path(path: &std::path::Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        if let Ok(mut child) = std::process::Command::new("xdg-open").arg(&path).spawn() {
            let _ = child.wait();
        }
    });
}

impl QwertoneApp {
    /// The row of settings pages.
    pub(super) fn settings_nav(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        self.settings_page = self.tab;
        ui.horizontal(|ui| {
            for (tab, label) in PAGES {
                let sel = self.tab == tab;
                let btn = egui::Button::new(RichText::new(label).color(if sel {
                    p.on_secondary_container
                } else {
                    p.on_surface
                }))
                .fill(if sel {
                    p.secondary_container
                } else {
                    p.surface_container_high
                })
                .corner_radius(egui::CornerRadius::same(255));
                if ui.add(btn).clicked() && !sel {
                    self.tab = tab;
                    self.settings_page = tab;
                    self.capture_for = None;
                }
            }
        });
        ui.add_space(6.0);
    }

    /// Pick the palette (end4's if wanted and present, else the chosen
    /// built-in one) and the window scale.
    pub(super) fn apply_theme(&mut self, ctx: &egui::Context) {
        let builtin = theme::builtin_palette(&self.cfg.theme);
        let palette = if self.cfg.follow_system_theme {
            match theme::load_system_palette() {
                Some((p, path)) => {
                    if self.theme_file.as_ref() != Some(&path) {
                        self.theme_watch = Some(FileWatch::new(path.clone()));
                    }
                    self.theme_file = Some(path);
                    p
                }
                None => {
                    self.theme_file = None;
                    self.theme_watch = None;
                    builtin
                }
            }
        } else {
            builtin
        };
        self.palette = palette;
        theme::apply(ctx, &palette);
        ctx.set_zoom_factor(self.cfg.ui_scale.clamp(0.8, 1.6));
    }

    /// egui's own Ctrl + / Ctrl − zoom: remember it like the slider.
    pub(super) fn sync_zoom(&mut self, ctx: &egui::Context) {
        let z = (ctx.zoom_factor() * 100.0).round() / 100.0;
        if (z - self.cfg.ui_scale).abs() > 0.005 {
            self.cfg.ui_scale = z.clamp(0.8, 1.6);
            self.mark_dirty();
        }
    }

    /// A note, chord or drum hit you played: practice time + count.
    pub(super) fn count_note(&mut self) {
        let a = match self.tab {
            Tab::Learn => Activity::Learn,
            Tab::Song => Activity::Song,
            _ => Activity::Play,
        };
        self.stats.note(a, Instant::now());
    }

    /// "🔥 4 days" in the header (click → Progress).
    pub(super) fn streak_chip(&mut self, ui: &mut egui::Ui) {
        if !self.cfg.practice.streak_chip {
            return;
        }
        let p = self.palette;
        let st = self.stats.streak();
        let today = self.stats.today_stats().total_s();
        let goal = self.cfg.practice.goal_minutes as f64 * 60.0;
        let text = match st.current {
            0 => "Start a streak today".to_string(),
            1 => "🔥 1 day".to_string(),
            n => format!("🔥 {n} days"),
        };
        let (fill, fg) = if st.today_done {
            (p.primary_container, p.on_primary_container)
        } else {
            (p.surface_container_high, p.on_surface_variant)
        };
        let btn = egui::Button::new(RichText::new(text).size(13.5).color(fg))
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(255));
        let hover = if st.today_done {
            format!(
                "Today: {} practised{}. Best streak: {} days.",
                fmt_duration(today),
                if goal > 0.0 && today >= goal {
                    " (goal reached)"
                } else {
                    ""
                },
                st.best
            )
        } else if st.current > 0 {
            "Play for a minute today to keep the streak going.".to_string()
        } else {
            "Play for a minute a day to build a streak.".to_string()
        };
        if ui.add(btn).on_hover_text(hover).clicked() {
            self.tab = Tab::Progress;
        }
    }

    pub(super) fn general_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(2, |cols| {
                card(&mut cols[0], &p, "Look", |ui| self.look_card(ui, &ctx));
                card(&mut cols[1], &p, "Practice goal", |ui| {
                    ui.label(
                        "Practice time counts while you're actually playing: the time between notes, \
                         chords and drum hits, as long as you don't stop for more than 30 seconds.",
                    );
                    ui.add_space(4.0);
                    let mut goal = self.cfg.practice.goal_minutes;
                    ui.horizontal(|ui| {
                        ui.label("Daily goal");
                        if ui
                            .add(egui::Slider::new(&mut goal, 0..=90).suffix(" min"))
                            .on_hover_text("0 = no goal")
                            .changed()
                        {
                            self.cfg.practice.goal_minutes = goal;
                            self.mark_dirty();
                        }
                    });
                    ui.label(
                        RichText::new(
                            "A day counts for your streak after 1 minute of playing, so a busy day \
                             doesn't break it. The goal is just for you.",
                        )
                        .small()
                        .color(p.on_surface_variant),
                    );
                    let mut chip_on = self.cfg.practice.streak_chip;
                    if ui
                        .checkbox(&mut chip_on, "Show the streak in the header")
                        .changed()
                    {
                        self.cfg.practice.streak_chip = chip_on;
                        self.mark_dirty();
                    }
                    if ui.button("Open Progress").clicked() {
                        self.tab = Tab::Progress;
                    }
                });
            });
            ui.add_space(8.0);
            card(ui, &p, "Where things are kept", |ui| self.files_card(ui));
            ui.add_space(8.0);
            card(ui, &p, "About", |ui| {
                ui.label(format!(
                    "Qwertone {} · MIT licence · the README explains every tab.",
                    env!("CARGO_PKG_VERSION")
                ));
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to("Project page", env!("CARGO_PKG_REPOSITORY"));
                    ui.label(
                        RichText::new(
                            "All settings live in config.toml and reload when the file changes, so \
                             you can also edit them by hand.",
                        )
                        .small()
                        .color(p.on_surface_variant),
                    );
                });
            });
        });
    }

    fn look_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.palette;
        let mut follow = self.cfg.follow_system_theme;
        if ui
            .checkbox(&mut follow, "Match end4 / wallpaper colours")
            .on_hover_text("Uses the Material You colours end4 (illogical-impulse) makes from your wallpaper, and follows wallpaper changes.")
            .changed()
        {
            self.cfg.follow_system_theme = follow;
            self.mark_dirty();
            self.apply_theme(ctx);
        }
        let end4_on = self.cfg.follow_system_theme && self.theme_file.is_some();
        ui.label(
            RichText::new(match &self.theme_file {
                Some(f) if end4_on => format!("Colours from {}", f.display()),
                _ if self.cfg.follow_system_theme => {
                    "end4's colour file wasn't found, so the theme below is used.".into()
                }
                _ => "Using the theme below.".into(),
            })
            .small()
            .color(p.on_surface_variant),
        );
        ui.add_space(4.0);
        ui.label(if end4_on {
            "Theme (used when end4 isn't running)"
        } else {
            "Theme"
        });
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for (id, name, pal) in theme::builtin_themes() {
                let sel = self.cfg.theme == id;
                let resp = egui::Frame::new()
                    .fill(pal.surface_container)
                    .stroke(egui::Stroke::new(
                        if sel { 2.5 } else { 1.0 },
                        if sel { p.primary } else { p.outline_variant },
                    ))
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for c in [pal.primary, pal.tertiary, pal.primary_container] {
                                let (r, _) =
                                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                                ui.painter().circle_filled(r.center(), 7.0, c);
                            }
                            ui.label(RichText::new(name).color(pal.on_surface));
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    picked = Some(id);
                }
            }
        });
        if let Some(id) = picked {
            self.cfg.theme = id.to_string();
            if end4_on {
                self.notify("Saved. Untick “Match end4” to see it now.");
            }
            self.mark_dirty();
            self.apply_theme(ctx);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Window size");
            let mut pct = (self.cfg.ui_scale * 100.0).round() as i32;
            let r = ui.add(
                egui::Slider::new(&mut pct, 80..=160)
                    .step_by(10.0)
                    .suffix(" %"),
            );
            // Apply when the drag ends, so the slider doesn't jump away
            // from the mouse while it grows.
            if r.changed() {
                self.cfg.ui_scale = pct as f32 / 100.0;
                self.mark_dirty();
            }
            if (r.drag_stopped() || (r.changed() && !r.dragged()))
                && (ctx.zoom_factor() - self.cfg.ui_scale).abs() > 0.005
            {
                ctx.set_zoom_factor(self.cfg.ui_scale);
            }
            if ui
                .add_enabled(self.cfg.ui_scale != 1.0, egui::Button::new("Reset"))
                .clicked()
            {
                self.cfg.ui_scale = 1.0;
                ctx.set_zoom_factor(1.0);
                self.mark_dirty();
            }
        });
        ui.label(
            RichText::new("Ctrl + / Ctrl − / Ctrl 0 work too (when your layout doesn't use Ctrl).")
                .small()
                .color(p.on_surface_variant),
        );
    }

    fn files_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut rows: Vec<(&str, PathBuf)> = vec![
            ("Settings (config.toml)", config::config_path()),
            ("Keyboard layouts", config::keymaps_dir()),
            ("Rhythm patterns", config::patterns_dir()),
            ("Analysed songs", crate::song::songs_dir()),
            ("Recordings (MIDI takes)", crate::record::takes_dir()),
        ];
        if let Some(sp) = self.stats.path() {
            rows.push(("Practice stats", sp.clone()));
        }
        rows.push(("Spotify cache", crate::spotify::cache::path()));
        egui::Grid::new("files")
            .num_columns(3)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for (what, path) in rows {
                    ui.label(what);
                    ui.add(
                        egui::Label::new(
                            RichText::new(path.display().to_string())
                                .monospace()
                                .color(p.on_surface_variant),
                        )
                        .wrap_mode(egui::TextWrapMode::Truncate),
                    );
                    // Open the file itself if it exists, else its folder.
                    let target = if path.exists() {
                        Some(path.clone())
                    } else {
                        path.parent().filter(|d| d.exists()).map(|d| d.to_path_buf())
                    };
                    if ui
                        .add_enabled(target.is_some(), egui::Button::new("Open").small())
                        .on_disabled_hover_text("Not created yet")
                        .clicked()
                        && let Some(t) = target
                    {
                        open_path(&t);
                    }
                    ui.end_row();
                }
            });
    }
}
