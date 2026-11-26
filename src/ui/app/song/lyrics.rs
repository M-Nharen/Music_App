//! Lyrics in the Song tab (Part 10b): the Spotify-style Lyrics screen, the
//! current line inside the falling notes, the "where these lyrics came
//! from" badge, finding lyrics with the analyzer, and the lyrics editor with
//! tap-along timing. The model is in `crate::song::lyrics`.
//!
//! Clocks: the falling-notes overlay uses the falling notes' own clock
//! (`song_ui.heard`, which already has the Bluetooth compensation of
//! `visual_delay_ms()`), plus the lyrics nudge. The Lyrics screen and the
//! editor can follow what you *hear* instead (`cfg.lyrics.follow_heard`,
//! only different with "line up" on, where the notes run ahead of the
//! sound). Taps are always stamped with what you hear.

use super::*;
use crate::song::lyrics::{Layer, Layers, Lyrics, LyricsEditor, SourceOf};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, FontId, Pos2, Rect, Sense, pos2};

/// Languages offered for "Wrong language?" and typed-in lyrics.
const LANGS: [(&str, &str); 12] = [
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("hi", "Hindi"),
    ("kn", "Kannada"),
    ("ml", "Malayalam"),
    ("bn", "Bengali"),
    ("mr", "Marathi"),
    ("pa", "Punjabi"),
    ("en", "English"),
    ("ta-Latn", "Tamil in English letters (Tanglish)"),
    ("te-Latn", "Telugu in English letters"),
    ("hi-Latn", "Hindi in English letters (Hinglish)"),
];

pub fn lang_name(code: &str) -> String {
    LANGS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| code.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Lrc,
}

pub(super) struct LyricsUi {
    pub doc: Option<Lyrics>,
    /// Song folder the lyrics are from.
    dir: Option<PathBuf>,
    error: Option<String>,
    job: Option<Job>,
    job_error: Option<(String, Vec<String>)>,
    pick: Option<(Pick, Receiver<Option<PathBuf>>)>,
    pub editor: Option<LyricsEditor>,
    confirm_discard: Option<Instant>,
    /// Lyrics screen scroll (pixels from the top of the first line).
    scroll: f32,
    /// Manual scrolling pauses following the song until then.
    paused_until: Option<Instant>,
    /// The line that was current last frame (to snap on jumps).
    last_line: Option<usize>,
    badge_open: bool,
    paste: String,
    paste_lang: String,
    wrong_lang: String,
    tap_flash: Option<Instant>,
    pub missing_fonts: Vec<&'static str>,
}

impl LyricsUi {
    pub fn new() -> LyricsUi {
        LyricsUi {
            doc: None,
            dir: None,
            error: None,
            job: None,
            job_error: None,
            pick: None,
            editor: None,
            confirm_discard: None,
            scroll: 0.0,
            paused_until: None,
            last_line: None,
            badge_open: false,
            paste: String::new(),
            paste_lang: "ta".into(),
            wrong_lang: "ta".into(),
            tap_flash: None,
            missing_fonts: Vec::new(),
        }
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

fn fmt_cs(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}.{:02}", cs / 6000, cs / 100 % 60, cs % 100)
}

/// Toggle labels for this song: (native, romanised, English).
fn layer_labels(doc: &Lyrics) -> (String, String, String) {
    let langs: Vec<&str> = if doc.languages.is_empty() {
        doc.language.iter().map(|s| s.as_str()).collect()
    } else {
        doc.languages.iter().map(|s| s.as_str()).collect()
    };
    let indic: Vec<&str> = langs
        .iter()
        .copied()
        .filter(|l| !l.starts_with("en") && !l.ends_with("-Latn") && *l != "und")
        .collect();
    let (native, roman) = match indic.as_slice() {
        ["ta"] => ("தமிழ்", "Tanglish"),
        ["te"] => ("తెలుగు", "Tenglish"),
        ["hi"] => ("हिन्दी", "Hinglish"),
        ["kn"] => ("ಕನ್ನಡ", "English letters"),
        ["ml"] => ("മലയാളം", "Manglish"),
        ["mr"] => ("मराठी", "English letters"),
        ["bn"] => ("বাংলা", "English letters"),
        _ => ("Original script", "English letters"),
    };
    let mt = doc
        .sources
        .translation
        .as_ref()
        .is_none_or(|s| s.machine_translation());
    let english = if mt {
        "English (machine translation)"
    } else {
        "English"
    };
    (native.into(), roman.into(), english.into())
}

impl QwertoneApp {
    pub(crate) fn set_missing_fonts(&mut self, m: Vec<&'static str>) {
        self.song_ui.lyrics.missing_fonts = m;
    }

    // ------------------------------------------------------------ clocks

    /// Song time for the lyrics in the falling notes: the notes' clock plus
    /// the lyrics nudge (+ = earlier).
    pub(super) fn lyrics_time_notes(&self) -> f64 {
        let su = &self.song_ui;
        su.heard + self.cfg.lyrics.offset_ms as f64 / 1000.0 * su.speed
    }

    /// Song time for the Lyrics screen / editor: what you hear (or the
    /// notes' clock), plus the nudge.
    pub(super) fn lyrics_time(&self) -> f64 {
        let su = &self.song_ui;
        let follow = if self.cfg.lyrics.follow_heard {
            self.heard_delay_ms()
        } else {
            self.visual_delay_ms()
        };
        let shift = (self.visual_delay_ms() - follow + self.cfg.lyrics.offset_ms) as f64 / 1000.0;
        su.heard + shift * su.speed
    }

    /// What you hear now, in song time (for taps).
    fn heard_now(&self) -> f64 {
        let su = &self.song_ui;
        su.heard + (self.visual_delay_ms() - self.heard_delay_ms()) as f64 / 1000.0 * su.speed
    }

    /// Jump the song so that lyric time `t` is now on screen.
    fn lyrics_seek(&mut self, t: f64) {
        let shift = self.lyrics_time() - self.song_ui.heard;
        self.song_seek(t - shift);
    }

    // ------------------------------------------------------------ loading

    /// Read `lyrics.json` for the open song (called when a song opens).
    pub(super) fn lyrics_load(&mut self) {
        let dir = self
            .song_ui
            .song
            .as_ref()
            .and_then(|s| s.dir())
            .map(|d| d.to_path_buf());
        let lu = &mut self.song_ui.lyrics;
        if lu.dir != dir {
            lu.editor = None;
            lu.badge_open = false;
            lu.job_error = None;
        }
        lu.dir = dir.clone();
        lu.doc = None;
        lu.error = None;
        lu.scroll = 0.0;
        lu.last_line = None;
        if let Some(d) = dir {
            match Lyrics::load(&d) {
                Ok(doc) => {
                    if let Some(l) = doc.as_ref().and_then(|d| d.language.clone()) {
                        lu.wrong_lang = l.clone();
                        lu.paste_lang = l;
                    }
                    lu.doc = doc;
                }
                Err(e) => lu.error = Some(e),
            }
        }
    }

    /// Every frame: the analyzer's lyrics run, the file picker.
    pub(super) fn lyrics_poll(&mut self, ctx: &egui::Context) -> bool {
        let mut busy = false;
        if let Some(job) = &mut self.song_ui.lyrics.job {
            busy = true;
            if job.poll() {
                ctx.request_repaint();
            }
            if let Some(res) = job.result.clone() {
                let log = job.log.clone();
                let status = job.lyrics_status.clone();
                self.song_ui.lyrics.job = None;
                match res {
                    Ok(dir) => {
                        if self.song_ui.lyrics.dir.as_ref() == Some(&dir) {
                            self.lyrics_load();
                        }
                        self.notify(match status.as_deref() {
                            Some("synced") => "Lyrics found, with timing.",
                            Some("plain") => {
                                "Lyrics found, but without timing: tap along to time them."
                            }
                            Some("instrumental") => "This song is instrumental (no lyrics).",
                            Some("not_found") => "No lyrics found for this song.",
                            _ => "Lyrics updated.",
                        });
                    }
                    Err(e) => self.song_ui.lyrics.job_error = Some((e, log)),
                }
            }
        }
        if let Some((what, rx)) = &self.song_ui.lyrics.pick
            && let Ok(res) = rx.try_recv()
        {
            let what = *what;
            self.song_ui.lyrics.pick = None;
            if let Some(p) = res {
                match what {
                    Pick::Lrc => self.lyrics_import(&p),
                }
            }
        }
        if let Some(at) = self.song_ui.lyrics.tap_flash
            && at.elapsed() < Duration::from_millis(400)
        {
            busy = true;
        }
        busy
    }

    fn lyrics_run(&mut self, extra: &[&str]) {
        let (Some(program), Some(dir)) = (
            self.song_ui
                .analyzer
                .clone()
                .or_else(analyze::find_analyzer),
            self.song_ui.lyrics.dir.clone(),
        ) else {
            self.notify(
                "The analyzer isn't installed (analyzer/setup.sh), so lyrics can't be looked up.",
            );
            return;
        };
        self.song_ui.lyrics.job_error = None;
        match Job::lyrics(&program, &dir, extra) {
            Ok(j) => self.song_ui.lyrics.job = Some(j),
            Err(e) => self.song_ui.lyrics.job_error = Some((e, Vec::new())),
        }
    }

    fn lyrics_import(&mut self, lrc: &std::path::Path) {
        let (Some(program), Some(dir)) = (
            self.song_ui
                .analyzer
                .clone()
                .or_else(analyze::find_analyzer),
            self.song_ui.lyrics.dir.clone(),
        ) else {
            // No analyzer: read the file here (no romanising / translation).
            match std::fs::read_to_string(lrc) {
                Ok(t) => {
                    let lang = self.song_ui.lyrics.paste_lang.clone();
                    self.lyrics_from_text(&t, &lang);
                }
                Err(e) => self.notify(format!("Couldn't read {}: {e}", lrc.display())),
            }
            return;
        };
        self.song_ui.lyrics.job_error = None;
        match Job::import_lrc(&program, lrc, &dir) {
            Ok(j) => self.song_ui.lyrics.job = Some(j),
            Err(e) => self.song_ui.lyrics.job_error = Some((e, Vec::new())),
        }
    }

    fn lyrics_pick_lrc(&mut self) {
        let (tx, rx) = unbounded();
        let start = dirs::audio_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        std::thread::spawn(move || {
            let try_run = |cmd: &str, args: &[&str]| -> Option<Option<PathBuf>> {
                let out = std::process::Command::new(cmd).args(args).output().ok()?;
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                Some((out.status.success() && !s.is_empty()).then(|| PathBuf::from(s)))
            };
            let start_s = format!("{}/", start.display());
            let res = try_run(
                "zenity",
                &[
                    "--file-selection",
                    "--title=Choose lyrics (.lrc or .txt)",
                    "--file-filter=Lyrics | *.lrc *.LRC *.txt",
                    "--file-filter=All files | *",
                    &format!("--filename={start_s}"),
                ],
            )
            .or_else(|| try_run("kdialog", &["--getopenfilename", &start_s, "*.lrc *.txt"]))
            .or_else(|| try_run("yad", &["--file", "--title=Choose lyrics"]));
            let _ = tx.send(res.flatten());
        });
        self.song_ui.lyrics.pick = Some((Pick::Lrc, rx));
    }

    /// Typed / pasted lyrics: open them in the editor, ready to tap along.
    /// An .lrc pasted in keeps its times.
    fn lyrics_from_text(&mut self, text: &str, lang: &str) {
        let dur = self
            .song_ui
            .song
            .as_ref()
            .map(|s| s.duration)
            .unwrap_or(0.0);
        let doc = parse_pasted(text, lang);
        let timed = doc.synced();
        let mut ed = LyricsEditor::new(doc, dur);
        ed.text_changed = true;
        ed.dirty = true;
        if !timed {
            ed.start_tapping(0);
        }
        self.song_ui.lyrics.editor = Some(ed);
        self.song_ui.view = SongView::LyricsEdit;
        self.song_ui.lyrics.paste.clear();
    }

    pub(super) fn lyrics_open_editor(&mut self, tap: bool) {
        let dur = self
            .song_ui
            .song
            .as_ref()
            .map(|s| s.duration)
            .unwrap_or(0.0);
        let Some(doc) = self.song_ui.lyrics.doc.clone() else {
            return;
        };
        if self.song_ui.lyrics.editor.is_none() {
            self.song_ui.lyrics.editor = Some(LyricsEditor::new(doc, dur));
        }
        let pos = self.lyrics_now().line.unwrap_or(0);
        if let Some(ed) = &mut self.song_ui.lyrics.editor {
            ed.selected = pos.min(ed.doc.lines.len().saturating_sub(1));
            if tap {
                let first = ed.doc.lines.iter().position(|l| l.start.is_none());
                ed.start_tapping(first.unwrap_or(0));
            }
        }
        self.song_ui.view = SongView::LyricsEdit;
    }

    fn lyrics_now(&self) -> crate::song::lyrics::Position {
        let t = self.lyrics_time();
        match &self.song_ui.lyrics.doc {
            Some(d) => d.position(t),
            None => crate::song::lyrics::Position {
                line: None,
                active: false,
                next: None,
            },
        }
    }

    fn lyrics_save(&mut self) {
        let (Some(dir), Some(song)) = (self.song_ui.lyrics.dir.clone(), self.song_ui.song.as_ref())
        else {
            self.notify("Lyrics can only be saved for analysed songs (a song folder).");
            return;
        };
        let title = song.title().to_string();
        let artist = song.metadata.artist.clone();
        let grid = self.song_ui.grid.clone();
        let Some(ed) = &mut self.song_ui.lyrics.editor else {
            return;
        };
        let doc = ed.finish(&grid);
        let rerun = ed.text_changed;
        match doc.save(&dir, &title, artist.as_deref()) {
            Ok(()) => {
                ed.dirty = false;
                ed.text_changed = false;
                self.song_ui.lyrics.doc = Some(doc.clone());
                // Keep editing the saved copy (with its sources and beats).
                let mut fresh = LyricsEditor::new(doc, ed.duration);
                fresh.selected = ed.selected;
                *ed = fresh;
                let analyzer = self
                    .song_ui
                    .analyzer
                    .clone()
                    .or_else(analyze::find_analyzer);
                if rerun && analyzer.is_some() {
                    self.notify(
                        "Saved. Making the English-letters and English lines for the new words…",
                    );
                    self.lyrics_run(&["--force"]);
                } else if rerun {
                    self.notify("Saved. Install the analyzer to get English letters / English for the edited lines.");
                } else {
                    self.notify("Lyrics saved.");
                }
            }
            Err(e) => self.notify(format!("Couldn't save the lyrics: {e}")),
        }
    }

    // ------------------------------------------------------------ settings

    /// Toggles for the three layers (only the ones this song has).
    fn lyrics_layer_toggles(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.song_ui.lyrics.doc else {
            return;
        };
        let have = doc.available();
        let (n, r, e) = layer_labels(doc);
        let mut changed = false;
        let l = &mut self.cfg.lyrics;
        if have.native {
            changed |= ui
                .selectable_label(l.native, n)
                .on_hover_text("The words in their own script")
                .clicked()
                .then(|| l.native = !l.native)
                .is_some();
        }
        if have.romanized {
            changed |= ui
                .selectable_label(l.romanized, r)
                .on_hover_text("The same words written in English letters, to read along")
                .clicked()
                .then(|| l.romanized = !l.romanized)
                .is_some();
        }
        if have.english {
            changed |= ui
                .selectable_label(l.english, e)
                .on_hover_text(
                    "What the words mean, translated by a computer: literal, sometimes wrong",
                )
                .clicked()
                .then(|| l.english = !l.english)
                .is_some();
        }
        if changed {
            self.mark_dirty();
        }
    }

    /// Nudge + "follow" (shared by the Lyrics screen, the editor and the
    /// Song-tab card).
    fn lyrics_sync_controls(&mut self, ui: &mut egui::Ui, follow: bool) {
        let p = self.palette;
        ui.horizontal_wrapped(|ui| {
            ui.label("Lyrics timing");
            let mut o = self.cfg.lyrics.offset_ms;
            let mut changed = false;
            if ui
                .small_button("− later")
                .on_hover_text("The words show up too early: move them later (50 ms)")
                .clicked()
            {
                o -= 50.0;
                changed = true;
            }
            changed |= ui
                .add(egui::DragValue::new(&mut o).range(-3000.0..=3000.0).speed(5.0).suffix(" ms"))
                .on_hover_text("+ = lyrics earlier, − = later. On top of the Bluetooth delay compensation.")
                .changed();
            if ui
                .small_button("+ earlier")
                .on_hover_text("The words show up too late: move them earlier (50 ms)")
                .clicked()
            {
                o += 50.0;
                changed = true;
            }
            if o != 0.0 && ui.small_button("reset").clicked() {
                o = 0.0;
                changed = true;
            }
            if changed {
                self.cfg.lyrics.offset_ms = o.clamp(-3000.0, 3000.0);
                self.mark_dirty();
            }
            let delay = self.output_delay_ms();
            if follow && self.cfg.song.line_up && delay > 1.0 {
                ui.separator();
                let mut fh = self.cfg.lyrics.follow_heard;
                if ui
                    .checkbox(&mut fh, "Follow what I hear")
                    .on_hover_text(format!(
                        "With “line up” on, the falling notes run {delay:.0} ms ahead of the sound. \
                         On: the lyrics follow the singing you hear. Off: they follow the falling notes."
                    ))
                    .changed()
                {
                    self.cfg.lyrics.follow_heard = fh;
                    self.mark_dirty();
                }
            } else if follow {
                ui.label(
                    RichText::new(format!("Bluetooth / output delay {delay:.0} ms is already taken into account"))
                        .small()
                        .color(p.outline),
                );
            }
        });
    }

    /// The "Lyrics" card under Sound & sync on the practice screen.
    pub(super) fn lyrics_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if self.song_ui.lyrics.doc.is_none() {
            ui.label(
                RichText::new("No lyrics yet: open ♪ Lyrics to find, import or type them.")
                    .small()
                    .color(p.outline),
            );
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let mut c = false;
            let l = &mut self.cfg.lyrics;
            c |= ui
                .checkbox(&mut l.overlay, "Show the current line in the falling notes")
                .changed();
            ui.add_enabled_ui(l.overlay, |ui| {
                c |= ui
                    .selectable_value(&mut l.overlay_top, true, "Top")
                    .changed();
                c |= ui
                    .selectable_value(&mut l.overlay_top, false, "Bottom")
                    .changed();
                c |= ui.checkbox(&mut l.overlay_next, "Next line too").changed();
            });
            if c {
                self.mark_dirty();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Show");
            self.lyrics_layer_toggles(ui);
        });
        self.lyrics_sync_controls(ui, false);
    }

    // ------------------------------------------------------------ overlay

    /// The current line (+ the next, smaller) inside the falling notes.
    /// Returns the height used at the top (so other overlays move down).
    pub(super) fn lyrics_overlay(&self, ui: &egui::Ui, area: Rect) -> f32 {
        let p = self.palette;
        let l = &self.cfg.lyrics;
        let Some(doc) = &self.song_ui.lyrics.doc else {
            return 0.0;
        };
        if !l.overlay || !doc.synced() || self.song_ui.view != SongView::Practice {
            return 0.0;
        }
        let t = self.lyrics_time_notes();
        let pos = doc.position(t);
        let want = Layers {
            native: l.native,
            romanized: l.romanized,
            english: l.english,
        };
        // Main line: the one being sung; in a gap, the next one (dimmed)
        // once it's close.
        let (main, upcoming) = match (pos.active, pos.line, pos.next) {
            (true, Some(i), n) => (Some(i), n),
            (false, _, Some(n)) => {
                let soon = doc.lines[n]
                    .start
                    .is_some_and(|s| s - t < 4.0 * self.song_ui.speed.max(0.25));
                (
                    soon.then_some(n),
                    if soon {
                        doc.lines[n + 1..]
                            .iter()
                            .position(|x| x.start.is_some() && !x.is_blank())
                            .map(|k| n + 1 + k)
                    } else {
                        None
                    },
                )
            }
            _ => (None, None),
        };
        let Some(main) = main else { return 0.0 };
        let painter = ui.painter_at(area);
        let width = (area.width() - 48.0).max(120.0);
        let line = &doc.lines[main];
        let texts = doc.texts(line, want);
        let sung = pos.active && pos.line == Some(main);
        let base = if sung {
            p.on_surface
        } else {
            p.on_surface_variant.gamma_multiply(0.7)
        };
        let mut galleys = Vec::new();
        if let Some((layer, text)) = texts.first() {
            let job = karaoke_job(
                doc,
                main,
                *layer,
                text,
                sung.then_some(t),
                22.0,
                base,
                p.primary,
                width,
            );
            galleys.push(painter.layout_job(job));
        }
        for (_, text) in texts.iter().skip(1) {
            galleys.push(
                painter.layout(
                    text.to_string(),
                    FontId::proportional(14.0),
                    p.on_surface_variant
                        .gamma_multiply(if sung { 0.9 } else { 0.6 }),
                    width,
                ),
            );
        }
        if l.overlay_next
            && let Some(n) = upcoming
        {
            let nt = doc.texts(&doc.lines[n], want);
            if let Some((_, s)) = nt.first() {
                galleys.push(painter.layout(
                    s.to_string(),
                    FontId::proportional(15.0),
                    p.outline,
                    width,
                ));
            }
        }
        let gap = 3.0;
        let h: f32 =
            galleys.iter().map(|g| g.size().y).sum::<f32>() + gap * (galleys.len() as f32 - 1.0);
        let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
        let pad = vec2(16.0, 8.0);
        let y0 = if l.overlay_top {
            area.top() + 8.0
        } else {
            // Just above the hit line (strip 34 + margin 8, see FallView).
            area.bottom() - 42.0 - 10.0 - h - pad.y * 2.0
        };
        let bg = Rect::from_min_size(
            pos2(area.center().x - w / 2.0 - pad.x, y0),
            vec2(w + pad.x * 2.0, h + pad.y * 2.0),
        );
        painter.rect_filled(
            bg,
            egui::CornerRadius::same(14),
            p.surface_container_highest.gamma_multiply(0.88),
        );
        let mut y = y0 + pad.y;
        for g in galleys {
            let x = area.center().x - g.size().x / 2.0;
            let gh = g.size().y;
            painter.galley(pos2(x, y), g, p.on_surface);
            y += gh + gap;
        }
        if l.overlay_top {
            bg.height() + 8.0
        } else {
            0.0
        }
    }

    // ------------------------------------------------------------ badge

    /// The source chip; click toggles the details.
    fn lyrics_badge(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(doc) = &self.song_ui.lyrics.doc else {
            return;
        };
        let cur = self.lyrics_now().line;
        let words = doc.source(cur, SourceOf::Words);
        let timing = doc.source(cur, SourceOf::Timing);
        let mut text = words
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "unknown source".into());
        if let (Some(w), Some(t)) = (words, timing)
            && w.name != t.name
        {
            text.push_str(&format!(" · timing: {}", t.name));
        }
        let auto = doc.auto_generated.words || doc.auto_generated.timing;
        let (fill, fg) = if auto {
            text = format!("⚠ {text}");
            (p.tertiary_container, p.on_surface)
        } else {
            (p.surface_container_high, p.on_surface)
        };
        let r = ui
            .add(
                egui::Button::new(RichText::new(format!("ⓘ {text}")).size(13.0).color(fg))
                    .fill(fill)
                    .corner_radius(egui::CornerRadius::same(255)),
            )
            .on_hover_text("Where these lyrics came from (click for details)");
        if r.clicked() {
            self.song_ui.lyrics.badge_open = !self.song_ui.lyrics.badge_open;
        }
    }

    fn lyrics_badge_details(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(doc) = self.song_ui.lyrics.doc.clone() else {
            return;
        };
        let cur = self.lyrics_now().line;
        let mut fix = None;
        let mut redo_lang = None;
        let mut forget = false;
        egui::Frame::new()
            .fill(p.surface_container)
            .corner_radius(egui::CornerRadius::same(16))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Where these lyrics came from").strong().color(p.primary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            self.song_ui.lyrics.badge_open = false;
                        }
                    });
                });
                let rows = [
                    ("Words", SourceOf::Words),
                    ("Timing", SourceOf::Timing),
                    ("English letters", SourceOf::Transliteration),
                    ("English", SourceOf::Translation),
                ];
                egui::Grid::new("lyrics-sources").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (label, what) in rows {
                        let Some(s) = doc.source(None, what) else { continue };
                        ui.label(RichText::new(label).color(p.on_surface_variant));
                        ui.vertical(|ui| source_row(ui, &p, s));
                        ui.end_row();
                    }
                    if let Some(id) = doc.extra.get("song").and_then(|s| s.get("identified_by")).filter(|v| !v.is_null()) {
                        ui.label(RichText::new("Song").color(p.on_surface_variant));
                        let name = id.get("name").and_then(|v| v.as_str()).unwrap_or("AcoustID");
                        let t = id.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        let a = id.get("artist").and_then(|v| v.as_str()).unwrap_or("");
                        ui.label(format!("recognised by {name} as “{t}” by {a}"));
                        ui.end_row();
                    }
                });
                // This line's own sources (user edits).
                if let Some(i) = cur
                    && let Some(ls) = doc.lines.get(i).and_then(|l| l.sources.as_ref())
                {
                    for (label, s) in [("words", &ls.words), ("timing", &ls.timing)] {
                        if let Some(s) = s {
                            ui.label(
                                RichText::new(format!("This line's {label}: {}{}", s.name, s.fetched_date().map(|d| format!(", {d}")).unwrap_or_default()))
                                    .small()
                                    .color(p.tertiary),
                            );
                        }
                    }
                }
                if doc.auto_generated.words || doc.auto_generated.timing {
                    ui.add_space(4.0);
                    let what = if doc.auto_generated.words {
                        "The words were written down by a speech model from the singing"
                    } else {
                        "The timing was worked out by a speech model from the singing"
                    };
                    ui.label(RichText::new(format!("⚠ Auto-generated: {what}. It may be wrong.")).color(p.tertiary));
                    if doc.lines.iter().any(|l| l.confidence.is_some_and(|c| c < 0.5)) {
                        ui.label(
                            RichText::new("Greyed lines are the ones it was least sure about.")
                                .small()
                                .color(p.outline),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("✎ Fix in the editor").clicked() {
                            fix = Some(false);
                        }
                        if ui.button("Tap along to retime").clicked() {
                            fix = Some(true);
                        }
                    });
                }
                if doc.sources.words.as_ref().is_some_and(|s| s.is_user())
                    && self.song_ui.lyrics.job.is_none()
                    && ui
                        .button("Forget my edits (look the lyrics up again)")
                        .on_hover_text("Deletes lyrics/imported.lrc saved by the editor and runs the lyrics step again")
                        .clicked()
                {
                    forget = true;
                }
                if doc.lines.iter().any(|l| l.translation.is_some()) {
                    ui.label(
                        RichText::new("The English is machine translation of song lyrics: literal, and sometimes wrong.")
                            .small()
                            .color(p.outline),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let lang = doc.language.clone().unwrap_or_else(|| "und".into());
                    let mut s = format!("Language: {}", lang_name(&lang));
                    let others: Vec<String> = doc.languages.iter().filter(|l| **l != lang).map(|l| lang_name(l)).collect();
                    if !others.is_empty() {
                        s.push_str(&format!(" (also {})", others.join(", ")));
                    }
                    if let Some(src) = &doc.language_source {
                        let how = match src.as_str() {
                            "user" => "you chose it",
                            "whisper" => "heard in the singing",
                            "text" => "from the script",
                            "tag" => "from the lyrics file",
                            _ => "",
                        };
                        if !how.is_empty() {
                            s.push_str(&format!(" · {how}"));
                        }
                    }
                    ui.label(s);
                    ui.label(RichText::new("Wrong language?").color(p.on_surface_variant));
                    let cur_l = self.song_ui.lyrics.wrong_lang.clone();
                    egui::ComboBox::from_id_salt("lyrics-lang")
                        .selected_text(lang_name(&cur_l))
                        .show_ui(ui, |ui| {
                            for (c, n) in LANGS {
                                ui.selectable_value(&mut self.song_ui.lyrics.wrong_lang, c.to_string(), n);
                            }
                        });
                    let busy = self.song_ui.lyrics.job.is_some();
                    if ui
                        .add_enabled(!busy, egui::Button::new("Redo lyrics"))
                        .on_hover_text("Run the lyrics step again in this language (qwertone-analyze lyrics --language … --force)")
                        .clicked()
                    {
                        redo_lang = Some(self.song_ui.lyrics.wrong_lang.clone());
                    }
                });
            });
        if let Some(tap) = fix {
            self.lyrics_open_editor(tap);
        }
        if forget {
            if let Some(d) = self.song_ui.lyrics.dir.clone() {
                Lyrics::forget_edit(&d);
            }
            self.lyrics_run(&["--force"]);
        }
        if let Some(l) = redo_lang {
            let base = l.trim_end_matches("-Latn").to_string();
            self.lyrics_run(&["--language", &base, "--force"]);
        }
    }

    // ------------------------------------------------------------ screen

    /// Play / pause, time, timeline, speed: the parts of the transport the
    /// Lyrics screen and the editor need.
    fn lyrics_transport(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("⏮")
                .on_hover_text("Back to the start (F6)")
                .clicked()
            {
                self.song_restart();
            }
            let playing = self.song_playing();
            let (label, fill, fg) = if playing {
                ("⏸  Pause", p.primary, p.on_primary)
            } else {
                ("▶  Play", p.secondary_container, p.on_secondary_container)
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).color(fg).strong())
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(255)),
                )
                .on_hover_text("F5")
                .clicked()
            {
                self.song_play_pause();
            }
            let dur = self
                .song_ui
                .song
                .as_ref()
                .map(|s| s.duration)
                .unwrap_or(0.0);
            ui.label(
                RichText::new(format!(
                    "{} / {}",
                    fmt_time(self.lyrics_time()),
                    fmt_time(dur)
                ))
                .monospace(),
            );
            ui.separator();
            ui.label("Speed");
            let mut s = self.song_ui.speed * 100.0;
            if ui
                .add(
                    egui::Slider::new(&mut s, 25.0..=150.0)
                        .suffix("%")
                        .integer()
                        .step_by(5.0),
                )
                .on_hover_text("F7 slower · F8 faster")
                .changed()
            {
                self.song_set_speed(s / 100.0);
            }
            for v in [50, 75, 100] {
                if ui.small_button(format!("{v}%")).clicked() {
                    self.song_set_speed(v as f64 / 100.0);
                }
            }
        });
        let (dur, sections) = {
            let song = self.song_ui.song.as_ref().unwrap();
            let secs: Vec<(f64, f64, String)> = song
                .sections
                .iter()
                .map(|s| (s.start, s.end, s.title()))
                .collect();
            (song.duration, secs)
        };
        // Lines as little marks on the timeline.
        let marks: Vec<(f64, Color32)> = self
            .song_ui
            .lyrics
            .doc
            .as_ref()
            .map(|d| {
                d.lines
                    .iter()
                    .filter_map(|l| l.start)
                    .map(|s| (s, p.outline.gamma_multiply(0.6)))
                    .collect()
            })
            .unwrap_or_default();
        let tl = song_view::timeline(
            ui,
            &p,
            dur,
            self.song_ui.heard,
            &sections,
            self.song_ui.looping,
            &marks,
        );
        if let Some(t) = tl.seek {
            self.song_seek(t);
        }
        if let Some((a, b)) = tl.selected {
            let (a, b) = self.bar_span(a, b);
            self.song_set_loop(Some((a, b)));
        }
    }

    /// Leave the Lyrics screen / editor for the practice view.
    fn lyrics_back_to_practice(&mut self) {
        self.song_ui.view = SongView::Practice;
        // Wait mode was off here: pick up the notes from where we are.
        let t = self.song_ui.heard;
        self.song_restart_practice(t);
    }

    /// The Spotify-style Lyrics screen.
    pub(super) fn lyrics_screen(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal_wrapped(|ui| {
            if ui.button("← Practice").clicked() {
                self.lyrics_back_to_practice();
            }
            let name = self
                .song_ui
                .song
                .as_ref()
                .map(|s| s.display_name())
                .unwrap_or_default();
            ui.label(RichText::new(name).size(19.0).strong().color(p.primary));
            ui.add_space(6.0);
            self.lyrics_layer_toggles(ui);
            ui.add_space(6.0);
            self.lyrics_badge(ui);
            if self
                .song_ui
                .lyrics
                .doc
                .as_ref()
                .is_some_and(|d| !d.lines.is_empty())
                && ui.button("✎ Edit lyrics").clicked()
            {
                self.lyrics_open_editor(false);
            }
            ui.label("Size");
            let mut s = self.cfg.lyrics.size;
            if ui
                .add(egui::Slider::new(&mut s, 0.6..=1.6).show_value(false))
                .changed()
            {
                self.cfg.lyrics.size = s;
                self.mark_dirty();
            }
        });
        if self.song_ui.lyrics.badge_open {
            self.lyrics_badge_details(ui);
        }
        if !self.song_ui.lyrics.missing_fonts.is_empty()
            && let Some(doc) = &self.song_ui.lyrics.doc
            && doc.lines.iter().any(|l| !l.is_latin())
        {
            ui.label(
                RichText::new(format!(
                    "Some script fonts aren't installed ({}), so some letters may show as boxes: install `noto-fonts`.",
                    self.song_ui.lyrics.missing_fonts.join(", ")
                ))
                .small()
                .color(p.error),
            );
        }
        ui.add_space(4.0);
        self.lyrics_transport(ui);
        self.lyrics_sync_controls(ui, true);
        ui.add_space(6.0);
        let h = ui.available_height().max(200.0);
        if self
            .song_ui
            .lyrics
            .doc
            .as_ref()
            .is_some_and(|d| !d.lines.is_empty())
        {
            self.lyrics_panel(ui, h);
        } else {
            self.lyrics_empty(ui);
        }
    }

    /// No lyrics (yet): find, import, type.
    fn lyrics_empty(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let doc_c = self.song_ui.lyrics.doc.clone();
        let doc = doc_c.as_ref();
        let has_dir = self.song_ui.lyrics.dir.is_some();
        let analyzer = self.song_ui.analyzer.is_some() || analyze::find_analyzer().is_some();
        egui::Frame::new()
            .fill(p.surface_container_low)
            .corner_radius(egui::CornerRadius::same(20))
            .inner_margin(egui::Margin::same(24))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let (title, text) = match doc {
                    _ if !has_dir => (
                        "Lyrics need an analysed song",
                        "This is a plain MIDI file. Analyse the recording (Songs → Analyze) to get a song folder; lyrics are looked up for it.".to_string(),
                    ),
                    Some(d) if d.instrumental() => (
                        "This song is instrumental",
                        "LRCLIB says it has no words. If it does, paste or import them below.".into(),
                    ),
                    Some(_) => (
                        "No lyrics found",
                        "Nothing in the song's files, on LRCLIB, or (if the models are installed) in the singing. You can import an .lrc or paste the words and tap along.".into(),
                    ),
                    None => (
                        "No lyrics yet",
                        "The lyrics step hasn't run for this song. “Find lyrics” looks in the song's files and on LRCLIB, and can write them down from the singing (that takes a few minutes on a CPU).".into(),
                    ),
                };
                ui.label(RichText::new(title).size(22.0).strong().color(p.primary));
                ui.label(RichText::new(text).color(p.on_surface_variant));
                if let Some(e) = &self.song_ui.lyrics.error {
                    ui.label(RichText::new(e).color(p.error));
                }
                ui.add_space(10.0);
                if !has_dir {
                    return;
                }
                if let Some(job) = &self.song_ui.lyrics.job {
                    let mut cancel = false;
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("{} ({:.0} s)", job.message, job.started.elapsed().as_secs_f32()));
                        cancel = ui.button("Cancel").clicked();
                    });
                    if cancel && let Some(j) = &mut self.song_ui.lyrics.job {
                        j.cancel();
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(analyzer, egui::Button::new(RichText::new("🔍 Find lyrics").strong()))
                            .on_disabled_hover_text("Needs the analyzer (analyzer/setup.sh)")
                            .clicked()
                        {
                            let force: &[&str] = if doc.is_some() { &["--force", "--refresh"] } else { &[] };
                            self.lyrics_run(force);
                        }
                        if ui
                            .add_enabled(analyzer, egui::Button::new("Quick (no models)"))
                            .on_hover_text("Only the song's files and LRCLIB: a few seconds")
                            .clicked()
                        {
                            self.lyrics_run(&["--light", "--force"]);
                        }
                        if ui.button("Import .lrc / .txt…").clicked() {
                            self.lyrics_pick_lrc();
                        }
                    });
                }
                if let Some((e, log)) = &self.song_ui.lyrics.job_error {
                    ui.label(RichText::new(format!("The lyrics step failed: {e}")).color(p.error));
                    for l in log.iter().rev().take(4).rev() {
                        ui.label(RichText::new(l).small().monospace().color(p.outline));
                    }
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Or paste the words (one line per line; an .lrc keeps its times)").strong());
                ui.horizontal(|ui| {
                    ui.label("Language");
                    let cur = self.song_ui.lyrics.paste_lang.clone();
                    egui::ComboBox::from_id_salt("paste-lang")
                        .selected_text(lang_name(&cur))
                        .show_ui(ui, |ui| {
                            for (c, n) in LANGS {
                                ui.selectable_value(&mut self.song_ui.lyrics.paste_lang, c.to_string(), n);
                            }
                        });
                });
                ui.add(
                    egui::TextEdit::multiline(&mut self.song_ui.lyrics.paste)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .hint_text("காலை தென்றல் வந்தது…"),
                );
                let ok = !self.song_ui.lyrics.paste.trim().is_empty();
                if ui
                    .add_enabled(ok, egui::Button::new("Use these words → tap along"))
                    .clicked()
                {
                    let t = self.song_ui.lyrics.paste.clone();
                    let l = self.song_ui.lyrics.paste_lang.clone();
                    self.lyrics_from_text(&t, &l);
                }
            });
    }

    /// The big scrolling lyrics.
    fn lyrics_panel(&mut self, ui: &mut egui::Ui, height: f32) {
        let p = self.palette;
        let t = self.lyrics_time();
        let scale = self.cfg.lyrics.size.clamp(0.5, 2.0);
        let want = Layers {
            native: self.cfg.lyrics.native,
            romanized: self.cfg.lyrics.romanized,
            english: self.cfg.lyrics.english,
        };
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::CornerRadius::same(20), p.surface_container_low);
        let doc = self.song_ui.lyrics.doc.as_ref().unwrap();
        let synced = doc.synced();
        let pos = doc.position(t);
        let job_msg = self.song_ui.lyrics.job.as_ref().map(|j| j.message.clone());

        let pad_x = 28.0;
        let wrap = (width - pad_x * 2.0).max(100.0);
        let big = 30.0 * scale;
        let cur_size = 36.0 * scale;
        let small = 17.0 * scale;
        let gap = 20.0 * scale;
        // Lay out every line: (top offset, height, galleys).
        struct Block {
            y: f32,
            h: f32,
            galleys: Vec<Arc<egui::Galley>>,
            start: Option<f64>,
            doubtful: bool,
        }
        let mut blocks: Vec<Block> = Vec::with_capacity(doc.lines.len());
        let mut y = 0.0;
        for (i, line) in doc.lines.iter().enumerate() {
            if line.is_blank() {
                // An empty timed line: a short pause marker.
                let g = painter.layout_no_wrap("♪".into(), FontId::proportional(small), p.outline);
                let h = g.size().y;
                blocks.push(Block {
                    y,
                    h,
                    galleys: vec![g],
                    start: None,
                    doubtful: false,
                });
                y += h + gap;
                continue;
            }
            let is_cur = synced && pos.line == Some(i);
            let active = is_cur && pos.active;
            let past = synced && pos.line.is_some_and(|c| i < c || (i == c && !pos.active));
            let dist = match pos.line {
                Some(c) if i > c => i - c,
                None if synced => i + 1,
                _ => 0,
            };
            let doubtful = line.confidence.is_some_and(|c| c < 0.5);
            let mut color = if !synced || active {
                p.on_surface
            } else if past {
                p.on_surface_variant.gamma_multiply(0.42)
            } else if dist == 1 || (pos.line.is_none() && i == 0) {
                p.on_surface.gamma_multiply(0.62)
            } else {
                p.on_surface.gamma_multiply(0.38)
            };
            if doubtful {
                color = color.gamma_multiply(0.7);
            }
            let texts = doc.texts(line, want);
            let mut galleys = Vec::new();
            for (k, (layer, text)) in texts.iter().enumerate() {
                if k == 0 {
                    let size = if active { cur_size } else { big };
                    let job = karaoke_job(
                        doc,
                        i,
                        *layer,
                        text,
                        active.then_some(t),
                        size,
                        color,
                        p.primary,
                        wrap,
                    );
                    galleys.push(painter.layout_job(job));
                } else {
                    let c = if *layer == Layer::English {
                        color.gamma_multiply(0.75)
                    } else {
                        color.gamma_multiply(0.85)
                    };
                    galleys.push(painter.layout(
                        text.to_string(),
                        FontId::proportional(small),
                        c,
                        wrap,
                    ));
                }
            }
            let h = galleys.iter().map(|g| g.size().y).sum::<f32>()
                + 4.0 * (galleys.len() as f32 - 1.0);
            blocks.push(Block {
                y,
                h,
                galleys,
                start: line.start,
                doubtful,
            });
            y += h + gap;
        }
        let total = y;

        // Scrolling: follow the current line (40 % down), unless you scrolled.
        let lu = &mut self.song_ui.lyrics;
        let now = Instant::now();
        let hovered = resp.hovered() || ui.rect_contains_pointer(rect);
        let wheel = if hovered {
            ui.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        if wheel != 0.0 {
            lu.scroll -= wheel;
            lu.paused_until = Some(now + Duration::from_secs(4));
            ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
        }
        let following = lu.paused_until.is_none_or(|u| now >= u);
        let anchor = height * 0.4;
        let focus = if synced {
            match (pos.line, pos.next) {
                (Some(c), _) => Some(c),
                (None, Some(_)) => Some(0),
                _ => None,
            }
        } else {
            None
        };
        let mut moving = false;
        if following && let Some(c) = focus {
            let b = &blocks[c];
            let target = b.y + b.h / 2.0 - anchor;
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let jump = lu.last_line.is_some_and(|l| l.abs_diff(c) > 3);
            if jump {
                lu.scroll = target;
            } else {
                let k = 1.0 - (-dt * 9.0).exp();
                lu.scroll += (target - lu.scroll) * k;
            }
            moving = (target - lu.scroll).abs() > 0.5;
            lu.last_line = Some(c);
        }
        let max_scroll = (total - height * 0.5).max(0.0);
        lu.scroll = lu.scroll.clamp(-anchor, max_scroll);
        let scroll = lu.scroll;

        // Draw.
        let top = rect.top() + 12.0 - scroll;
        let mut clicked: Option<f64> = None;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        for b in &blocks {
            let y0 = top + b.y;
            if y0 > rect.bottom() || y0 + b.h < rect.top() {
                continue;
            }
            let row = Rect::from_min_size(
                pos2(rect.left() + 8.0, y0 - 6.0),
                vec2(width - 16.0, b.h + 12.0),
            );
            if synced
                && b.start.is_some()
                && pointer.is_some_and(|pt| row.contains(pt) && rect.contains(pt))
            {
                painter.rect_filled(
                    row,
                    egui::CornerRadius::same(12),
                    p.surface_container.gamma_multiply(0.9),
                );
                if resp.clicked() {
                    clicked = b.start;
                }
            }
            let mut yy = y0;
            for g in &b.galleys {
                let gh = g.size().y;
                painter.galley(pos2(rect.left() + pad_x, yy), g.clone(), p.on_surface);
                yy += gh + 4.0;
            }
            if b.doubtful {
                painter.text(
                    pos2(rect.left() + 10.0, y0 + 4.0),
                    Align2::LEFT_TOP,
                    "?",
                    FontId::proportional(13.0),
                    p.tertiary,
                );
            }
        }
        // Fades at the top and bottom edges.
        fade(&painter, rect, p.surface_container_low);
        // Banners.
        let mut chip_y = rect.top() + 10.0;
        if !synced {
            let msg = "These lyrics have no timing yet. ✎ Edit lyrics → Tap along to time them.";
            banner(&painter, pos2(rect.center().x, chip_y), msg, &p);
            chip_y += 34.0;
        }
        if let Some(m) = job_msg {
            banner(
                &painter,
                pos2(rect.center().x, chip_y),
                &format!("Updating lyrics: {m}"),
                &p,
            );
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let paused = !following && synced;
        if paused {
            let b = Rect::from_center_size(
                pos2(rect.center().x, rect.bottom() - 28.0),
                vec2(220.0, 32.0),
            );
            let r = ui.interact(b, ui.scope_id().with("lyrics-back"), Sense::click());
            painter.rect_filled(
                b,
                egui::CornerRadius::same(255),
                if r.hovered() {
                    p.primary
                } else {
                    p.secondary_container
                },
            );
            painter.text(
                b.center(),
                Align2::CENTER_CENTER,
                "↓ Back to the current line",
                FontId::proportional(14.0),
                if r.hovered() {
                    p.on_primary
                } else {
                    p.on_secondary_container
                },
            );
            if r.clicked() {
                self.song_ui.lyrics.paused_until = None;
            }
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        if moving || self.song_playing() {
            ui.ctx().request_repaint();
        }
        if let Some(s) = clicked {
            self.song_ui.lyrics.paused_until = None;
            self.lyrics_seek(s);
        }
    }

    // ------------------------------------------------------------ editor

    /// Keys in the lyrics editor. Returns true if used.
    pub(super) fn lyrics_editor_key(
        &mut self,
        k: KeyId,
        repeat: bool,
        mods: egui::Modifiers,
    ) -> bool {
        let KeyId::Key(key) = k else {
            return false;
        };
        let heard = self.heard_now();
        let playing = self.song_playing();
        let Some(ed) = &mut self.song_ui.lyrics.editor else {
            return false;
        };
        let n = ed.doc.lines.len();
        let step = if mods.shift { 0.01 } else { 0.05 };
        match key {
            Key::Space if !repeat => {
                if ed.tapping.is_some() {
                    if !playing {
                        self.song_play_pause();
                        return true;
                    }
                    if ed.tap(heard).is_some() {
                        self.song_ui.lyrics.tap_flash = Some(Instant::now());
                    } else {
                        self.notify("All lines are timed. Save, or keep fixing.");
                    }
                } else {
                    self.song_play_pause();
                }
            }
            Key::Backspace if !repeat && ed.tapping.is_some() => ed.untap(),
            Key::ArrowUp => ed.selected = ed.selected.saturating_sub(1),
            Key::ArrowDown => ed.selected = (ed.selected + 1).min(n.saturating_sub(1)),
            Key::ArrowLeft => {
                let i = ed.selected;
                ed.nudge(i, -step);
            }
            Key::ArrowRight => {
                let i = ed.selected;
                ed.nudge(i, step);
            }
            Key::Enter if !repeat => {
                if let Some(s) = ed.doc.lines.get(ed.selected).and_then(|l| l.start) {
                    self.lyrics_seek(s - 1.0);
                    if !self.song_playing() {
                        self.song_play_pause();
                    }
                }
            }
            Key::Z if mods.ctrl && mods.shift => ed.redo(),
            Key::Z if mods.ctrl => ed.undo(),
            Key::Y if mods.ctrl => ed.redo(),
            Key::S if mods.ctrl && !repeat => self.lyrics_save(),
            _ => return false,
        }
        true
    }

    pub(super) fn lyrics_editor(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if self.song_ui.lyrics.editor.is_none() {
            self.song_ui.view = SongView::Lyrics;
            return;
        }
        let (dirty, can_undo, can_redo, tapping) = {
            let ed = self.song_ui.lyrics.editor.as_ref().unwrap();
            (ed.dirty, ed.can_undo(), ed.can_redo(), ed.tapping)
        };
        ui.horizontal_wrapped(|ui| {
            let armed = self
                .song_ui
                .lyrics
                .confirm_discard
                .is_some_and(|t| t.elapsed() < Duration::from_secs(3));
            let back = if dirty && armed {
                "Discard changes?"
            } else {
                "← Lyrics"
            };
            if ui.button(back).clicked() {
                if dirty && !armed {
                    self.song_ui.lyrics.confirm_discard = Some(Instant::now());
                } else {
                    self.song_ui.lyrics.editor = None;
                    self.song_ui.lyrics.confirm_discard = None;
                    self.song_ui.view = SongView::Lyrics;
                    return;
                }
            }
            ui.label(
                RichText::new("Edit lyrics")
                    .size(19.0)
                    .strong()
                    .color(p.primary),
            );
            let save = egui::Button::new(RichText::new("💾 Save").color(p.on_primary).strong())
                .fill(p.primary)
                .corner_radius(egui::CornerRadius::same(255));
            if ui
                .add_enabled(dirty, save)
                .on_hover_text("Ctrl+S")
                .clicked()
            {
                self.lyrics_save();
            }
            let ed = self.song_ui.lyrics.editor.as_mut().unwrap();
            if ui
                .add_enabled(can_undo, egui::Button::new("↶ Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                ed.undo();
            }
            if ui
                .add_enabled(can_redo, egui::Button::new("↷ Redo"))
                .on_hover_text("Ctrl+Shift+Z")
                .clicked()
            {
                ed.redo();
            }
            ui.separator();
            let label = if tapping.is_some() {
                "■ Stop tapping"
            } else {
                "👆 Tap along"
            };
            if ui
                .selectable_label(tapping.is_some(), label)
                .on_hover_text("Play the song and press Space as each line starts")
                .clicked()
            {
                if tapping.is_some() {
                    ed.stop_tapping();
                } else {
                    let from = ed.selected;
                    ed.start_tapping(from);
                }
            }
            if ui
                .small_button("Clear all times")
                .on_hover_text("Start tapping from scratch")
                .clicked()
            {
                ed.clear_times();
                ed.start_tapping(0);
            }
            ui.separator();
            ui.label("Whole song");
            if ui
                .small_button("−0.1 s")
                .on_hover_text("Every line 0.1 s earlier")
                .clicked()
            {
                ed.shift_all(-0.1);
            }
            if ui
                .small_button("+0.1 s")
                .on_hover_text("Every line 0.1 s later")
                .clicked()
            {
                ed.shift_all(0.1);
            }
        });
        if self.song_ui.view != SongView::LyricsEdit {
            return;
        }
        self.lyrics_transport(ui);
        self.lyrics_sync_controls(ui, true);
        let help = if tapping.is_some() {
            "Tap along: press Space when each line starts (Space also starts the song). Backspace = undo the last tap. Slow the song down if it's hard to keep up."
        } else {
            "Click a time to select the line. ↑/↓ choose a line, ←/→ move it 50 ms (Shift: 10 ms), Enter plays from just before it, Space plays / pauses. Edit the words in the boxes."
        };
        ui.label(RichText::new(help).small().color(p.on_surface_variant));
        if self.song_ui.lyrics.dir.is_some() {
            ui.label(
                RichText::new("Saving writes lyrics.json and lyrics/imported.lrc in the song folder, so a later “Find lyrics” keeps your version.")
                    .small()
                    .color(p.outline),
            );
        }
        ui.add_space(4.0);

        let t = self.lyrics_time();
        let heard = self.heard_now();
        let flash = self
            .song_ui
            .lyrics
            .tap_flash
            .map(|a| 1.0 - a.elapsed().as_secs_f32() / 0.4)
            .filter(|a| *a > 0.0);
        let mut seek = None;
        let mut set_now: Option<usize> = None;
        let ed = self.song_ui.lyrics.editor.as_mut().unwrap();
        let cur = ed.doc.position(t);
        let mut select = None;
        let mut text_change: Option<(usize, String)> = None;
        let mut nudge: Option<(usize, f64)> = None;
        let mut remove = None;
        let mut insert = None;
        egui::ScrollArea::vertical()
            .id_salt("lyrics-edit")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..ed.doc.lines.len() {
                    let line = &ed.doc.lines[i];
                    let is_cur = cur.line == Some(i) && cur.active;
                    let is_sel = ed.selected == i;
                    let is_tap = ed.tapping == Some(i);
                    let fill = if is_tap {
                        p.tertiary_container
                            .gamma_multiply(0.6 + 0.4 * flash.unwrap_or(0.0))
                    } else if is_sel {
                        p.secondary_container
                    } else if is_cur {
                        p.surface_container_high
                    } else {
                        p.surface_container_low
                    };
                    egui::Frame::new()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let mark = if is_tap {
                                    "▶"
                                } else if is_cur {
                                    "♪"
                                } else {
                                    " "
                                };
                                ui.label(RichText::new(mark).monospace().color(p.primary));
                                let time =
                                    line.start.map(fmt_cs).unwrap_or_else(|| "–:––.––".into());
                                let tc = if ed.timed.get(i).copied().unwrap_or(false) {
                                    p.tertiary
                                } else {
                                    p.on_surface
                                };
                                let r = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(time).monospace().color(tc),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Select; double-click to play from here");
                                if r.double_clicked() {
                                    if let Some(s) = line.start {
                                        seek = Some(s - 1.0);
                                    }
                                } else if r.clicked() {
                                    select = Some(i);
                                }
                                if ui
                                    .small_button("−")
                                    .on_hover_text("50 ms earlier")
                                    .clicked()
                                {
                                    nudge = Some((i, -0.05));
                                }
                                if ui.small_button("+").on_hover_text("50 ms later").clicked() {
                                    nudge = Some((i, 0.05));
                                }
                                if ui
                                    .small_button("⏱")
                                    .on_hover_text("This line starts now (what you hear)")
                                    .clicked()
                                {
                                    set_now = Some(i);
                                }
                                let mut s = line.text.clone();
                                let w = (ui.available_width() - 190.0).max(120.0);
                                let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(w));
                                if r.changed() {
                                    text_change = Some((i, s));
                                }
                                if r.gained_focus() {
                                    select = Some(i);
                                }
                                if let Some(l) = &line.language {
                                    ui.label(RichText::new(l).small().color(p.outline));
                                }
                                if line.confidence.is_some_and(|c| c < 0.5) {
                                    ui.label(RichText::new("?").color(p.tertiary))
                                        .on_hover_text("The model wasn't sure about this line");
                                }
                                if ui
                                    .small_button("＋")
                                    .on_hover_text("Add a line after this one")
                                    .clicked()
                                {
                                    insert = Some(i);
                                }
                                if ui
                                    .small_button("🗑")
                                    .on_hover_text("Delete this line")
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                            });
                            if let Some(r) = line.romanized.as_ref().filter(|r| **r != line.text) {
                                ui.label(
                                    RichText::new(format!("      {r}"))
                                        .small()
                                        .color(p.on_surface_variant),
                                );
                            }
                            if let Some(tr) = &line.translation {
                                ui.label(
                                    RichText::new(format!("      = {tr}"))
                                        .small()
                                        .color(p.outline),
                                );
                            }
                        });
                }
                if ed.doc.lines.is_empty() && ui.button("＋ Add a line").clicked() {
                    insert = Some(usize::MAX);
                }
                ui.add_space(40.0);
            });
        if let Some(i) = select {
            ed.selected = i;
        }
        if let Some((i, s)) = text_change {
            ed.set_text(i, &s);
        }
        if let Some((i, d)) = nudge {
            ed.nudge(i, d);
            ed.selected = i;
        }
        if let Some(i) = set_now {
            ed.set_start(i, heard);
        }
        if let Some(i) = insert {
            ed.insert_after(if i == usize::MAX { None } else { Some(i) });
        }
        if let Some(i) = remove {
            ed.remove(i);
        }
        if let Some(s) = seek {
            self.lyrics_seek(s);
            if !self.song_playing() {
                self.song_play_pause();
            }
        }
        if self.song_playing() {
            ui.ctx().request_repaint();
        }
    }
}

/// A line's main text as a layout job, with the words already sung in
/// `hi` (the current word fills in as it's sung). `t` = None: no highlight.
#[allow(clippy::too_many_arguments)]
fn karaoke_job(
    doc: &Lyrics,
    i: usize,
    layer: Layer,
    text: &str,
    t: Option<f64>,
    size: f32,
    base: Color32,
    hi: Color32,
    wrap: f32,
) -> LayoutJob {
    let font = FontId::proportional(size);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let line = &doc.lines[i];
    let words = line.timed_words().filter(|_| layer != Layer::English);
    let fmt = |c: Color32| TextFormat {
        font_id: font.clone(),
        color: c,
        ..Default::default()
    };
    match (t, words) {
        (Some(t), Some(words)) => {
            let at = doc.word_at(i, t);
            for (k, w) in words.iter().enumerate() {
                let s = match layer {
                    Layer::Romanized => w.romanized.as_deref().unwrap_or(&w.text),
                    _ => w.text.as_str(),
                };
                let c = match at {
                    Some((cur, _)) if k < cur => hi,
                    Some((cur, f)) if k == cur => lerp(base, hi, 0.35 + 0.65 * f),
                    _ => base,
                };
                if k > 0 {
                    job.append(" ", 0.0, fmt(c));
                }
                job.append(s, 0.0, fmt(c));
            }
        }
        (Some(_), None) => job.append(text, 0.0, fmt(lerp(base, hi, 0.5))),
        (None, _) => job.append(text, 0.0, fmt(base)),
    }
    job
}

fn source_row(ui: &mut egui::Ui, p: &Palette, s: &crate::song::lyrics::Source) {
    ui.horizontal_wrapped(|ui| {
        match &s.url {
            Some(u) if !u.is_empty() => {
                ui.hyperlink_to(&s.name, u);
            }
            _ => {
                ui.label(RichText::new(&s.name).strong());
            }
        }
        if let Some(d) = s.fetched_date() {
            ui.label(RichText::new(d).small().color(p.outline));
        }
        if s.machine_translation() {
            chip(
                ui,
                p.tertiary_container,
                p.on_surface,
                "machine translation",
            );
        } else if s.auto_generated() {
            chip(ui, p.tertiary_container, p.on_surface, "auto-generated");
        }
        if s.is_user() {
            chip(ui, p.primary_container, p.on_primary_container, "your edit");
        }
    });
    if let Some(f) = &s.file {
        ui.label(
            RichText::new(f)
                .small()
                .monospace()
                .color(p.on_surface_variant),
        );
    }
    let mut extra = Vec::new();
    for k in ["model", "library", "style"] {
        if let Some(v) = s.get(k).filter(|v| !v.eq_ignore_ascii_case(&s.name)) {
            extra.push(v);
        }
    }
    if let Some(m) = s.extra.get("matched").and_then(|v| v.as_f64()) {
        extra.push(format!("{:.0} % of the words heard", m * 100.0));
    }
    if let Some(b) = s.based_on() {
        extra.push(format!("based on {}", b.name));
    }
    if !extra.is_empty() {
        ui.label(RichText::new(extra.join(" · ")).small().color(p.outline));
    }
}

fn banner(painter: &egui::Painter, center_top: Pos2, text: &str, p: &Palette) {
    let g = painter.layout_no_wrap(text.to_string(), FontId::proportional(13.5), p.on_surface);
    let r = Rect::from_center_size(
        pos2(center_top.x, center_top.y + g.size().y / 2.0 + 6.0),
        g.size() + vec2(28.0, 12.0),
    );
    painter.rect_filled(
        r,
        egui::CornerRadius::same(255),
        p.surface_container_highest,
    );
    painter.galley(r.min + vec2(14.0, 6.0), g, p.on_surface);
}

/// Soft fades at the top and bottom of the lyrics panel.
fn fade(painter: &egui::Painter, rect: Rect, bg: Color32) {
    let steps = 12;
    let h = 46.0;
    for k in 0..steps {
        let a = 1.0 - k as f32 / steps as f32;
        let c = bg.gamma_multiply(a * a);
        let y = k as f32 * h / steps as f32;
        let top = Rect::from_min_size(
            pos2(rect.left(), rect.top() + y),
            vec2(rect.width(), h / steps as f32 + 0.5),
        );
        let bot = Rect::from_min_size(
            pos2(rect.left(), rect.bottom() - y - h / steps as f32),
            vec2(rect.width(), h / steps as f32 + 0.5),
        );
        painter.rect_filled(top, 0.0, c);
        painter.rect_filled(bot, 0.0, c);
    }
}

/// Pasted text: an .lrc (with times) or plain lines.
fn parse_pasted(text: &str, lang: &str) -> Lyrics {
    let mut doc = Lyrics::from_text("", Some(lang));
    let mut timed = false;
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            if let Some(t) = parse_lrc_time(tag) {
                times.push(t);
                rest = r[end + 1..].trim_start();
            } else {
                // [ti:…] and other tags.
                rest = "";
                break;
            }
        }
        // Drop enhanced-LRC word stamps.
        let mut clean = String::new();
        let mut s = rest;
        while let Some(a) = s.find('<') {
            match s[a..].find('>') {
                Some(b) if parse_lrc_time(&s[a + 1..a + b]).is_some() => {
                    clean.push_str(&s[..a]);
                    s = &s[a + b + 1..];
                }
                _ => {
                    clean.push_str(&s[..=a]);
                    s = &s[a + 1..];
                }
            }
        }
        clean.push_str(s);
        let text = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        if times.is_empty() {
            times.push(f64::NAN);
        }
        for t in times {
            let mut l = crate::song::lyrics::Line::new(&text);
            l.language = Some(lang.to_string());
            if lang.ends_with("-Latn") {
                l.romanized = Some(text.clone());
            }
            if t.is_finite() {
                l.start = Some(t);
                timed = true;
            }
            doc.lines.push(l);
        }
    }
    if timed {
        doc.lines.sort_by(|a, b| {
            a.start
                .unwrap_or(f64::MAX)
                .total_cmp(&b.start.unwrap_or(f64::MAX))
        });
        let starts: Vec<Option<f64>> = doc.lines.iter().map(|l| l.start).collect();
        for (k, l) in doc.lines.iter_mut().enumerate() {
            if l.start.is_some() {
                l.end = starts[k + 1..].iter().flatten().next().copied();
            }
        }
    }
    doc.status = if doc.lines.is_empty() {
        "not_found".into()
    } else if timed {
        "synced".into()
    } else {
        "plain".into()
    };
    doc
}

fn parse_lrc_time(s: &str) -> Option<f64> {
    let (m, sec) = s.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let sec: f64 = sec.trim().replace(':', ".").parse().ok()?;
    (m >= 0.0 && (0.0..60.0).contains(&sec)).then_some(m * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_lrc_and_plain() {
        let d = parse_pasted(
            "[ti:x]\n[00:01.50]ஒன்று\n[00:03.00][00:10.00]இரண்டு <00:03.50>மூன்று\n\n",
            "ta",
        );
        assert_eq!(d.lines.len(), 3);
        assert_eq!(d.lines[0].start, Some(1.5));
        assert_eq!(d.lines[1].text, "இரண்டு மூன்று");
        assert_eq!(d.lines[2].start, Some(10.0));
        assert_eq!(d.lines[0].end, Some(3.0));
        assert_eq!(d.status, "synced");
        let d = parse_pasted("nee enakku\n\nnaan unakku\n", "ta-Latn");
        assert_eq!(d.lines.len(), 2);
        assert_eq!(d.status, "plain");
        assert_eq!(d.lines[1].romanized.as_deref(), Some("naan unakku"));
        assert_eq!(parse_lrc_time("01:02.5"), Some(62.5));
        assert_eq!(parse_lrc_time("ar:someone"), None);
    }

    #[test]
    fn toggle_labels() {
        let d = Lyrics::from_json(r#"{"language": "te", "languages": ["te", "en"], "lines": []}"#)
            .unwrap();
        let (n, r, e) = layer_labels(&d);
        assert_eq!((n.as_str(), r.as_str()), ("తెలుగు", "Tenglish"));
        assert!(e.contains("machine"));
        let d = Lyrics::from_json(r#"{"language": "hi", "languages": ["hi", "ta"], "lines": []}"#)
            .unwrap();
        assert_eq!(layer_labels(&d).0, "Original script");
    }
}
