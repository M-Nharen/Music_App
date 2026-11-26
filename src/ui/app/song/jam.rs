//! Jam mode in the Song tab (Part 7): load the song's stems in the
//! background, play them as the backing instead of the MIDI parts, the
//! mixer (volume / mute / solo per stem, your part muted by default), and
//! keeping the audio thread's stem player in step (gains, pitch shift for
//! easy parts in another key). Speed, loops, wait mode and the falling
//! notes are Song mode's; the stems just follow the song clock.

use super::*;
use crate::audio::stems::{MAX_STEMS, StemSet};
use crate::song::jam;
use std::sync::atomic::AtomicU32;

/// A load in progress: result, progress (0–1000), folder.
type Loading = (Receiver<Result<StemSet, String>>, Arc<AtomicU32>, PathBuf);

pub(super) struct JamUi {
    /// The loaded stems (this Arc keeps them alive for the audio thread).
    set: Option<Arc<StemSet>>,
    /// Song folder the stems are from.
    dir: Option<PathBuf>,
    loading: Option<Loading>,
    error: Option<(PathBuf, String)>,
    /// Replaced sets, freed here once the audio thread has let go.
    retired: Vec<Arc<StemSet>>,
    /// Stem → muted, where you changed the default.
    overrides: HashMap<String, bool>,
    solo: Option<String>,
    sent_on: Option<bool>,
    sent_gains: Option<[f32; MAX_STEMS]>,
    sent_pitch: Option<i32>,
}

impl JamUi {
    pub fn new() -> JamUi {
        JamUi {
            set: None,
            dir: None,
            loading: None,
            error: None,
            retired: Vec::new(),
            overrides: HashMap::new(),
            solo: None,
            sent_on: None,
            sent_gains: None,
            sent_pitch: None,
        }
    }

    /// What you play changed: back to "your part is muted".
    pub fn parts_changed(&mut self) {
        self.overrides.clear();
        self.solo = None;
    }
}

impl QwertoneApp {
    /// The open song's folder, if it has stems.
    fn jam_song_dir(&self) -> Option<PathBuf> {
        let song = self.song_ui.song.as_ref()?;
        if song.stems.is_empty() {
            return None;
        }
        song.dir().map(|d| d.to_path_buf())
    }

    /// The stems are the backing right now.
    pub(super) fn jam_active(&self) -> bool {
        let j = &self.song_ui.jam;
        self.cfg.jam.on && j.set.is_some() && j.dir.is_some() && j.dir == self.jam_song_dir()
    }

    fn jam_mine(&self) -> Vec<String> {
        let su = &self.song_ui;
        match &su.song {
            Some(song) => jam::my_stems(song, &su.play_parts, su.play_chords),
            None => Vec::new(),
        }
    }

    fn jam_muted(&self) -> Vec<bool> {
        match &self.song_ui.song {
            Some(song) => jam::muted(song, &self.jam_mine(), &self.song_ui.jam.overrides),
            None => Vec::new(),
        }
    }

    fn jam_gains(&self) -> [f32; MAX_STEMS] {
        let Some(song) = &self.song_ui.song else {
            return [0.0; MAX_STEMS];
        };
        jam::gains(
            song,
            &self.jam_muted(),
            &self.cfg.jam.volumes,
            self.cfg.song.backing,
            self.song_ui.jam.solo.as_deref(),
        )
    }

    fn jam_unload(&mut self) {
        let j = &mut self.song_ui.jam;
        if let Some(old) = j.set.take() {
            j.retired.push(old);
            self.send(Command::Song(SongCommand::Stems(None)));
        }
        let j = &mut self.song_ui.jam;
        j.dir = None;
        j.sent_on = None;
        j.sent_gains = None;
        j.sent_pitch = None;
        j.parts_changed();
    }

    /// Every frame: load / drop stems, and send what changed. True while
    /// loading (keeps the progress moving).
    pub(super) fn jam_poll(&mut self, ctx: &egui::Context) -> bool {
        let dir = self.jam_song_dir();
        // Another song: let go of the old stems.
        if self.song_ui.jam.dir.is_some() && self.song_ui.jam.dir != dir {
            self.jam_unload();
        }
        if self
            .song_ui
            .jam
            .loading
            .as_ref()
            .is_some_and(|l| Some(&l.2) != dir.as_ref())
        {
            self.song_ui.jam.loading = None;
        }
        // Start loading.
        let j = &self.song_ui.jam;
        if self.cfg.jam.on
            && let Some(d) = &dir
            && j.set.is_none()
            && j.loading.is_none()
            && j.error.as_ref().is_none_or(|e| &e.0 != d)
            && let Some(song) = &self.song_ui.song
        {
            let files: Vec<(String, String)> = song
                .stems
                .iter()
                .map(|s| (s.name.clone(), s.file.clone()))
                .collect();
            let progress = Arc::new(AtomicU32::new(0));
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (p, d2, c) = (progress.clone(), d.clone(), ctx.clone());
            let spawned = std::thread::Builder::new()
                .name("qwertone-stems".into())
                .spawn(move || {
                    let _ = tx.send(StemSet::load(&d2, &files, &p));
                    c.request_repaint();
                });
            match spawned {
                Ok(_) => self.song_ui.jam.loading = Some((rx, progress, d.clone())),
                Err(e) => self.song_ui.jam.error = Some((d.clone(), e.to_string())),
            }
        }
        // Finished loading?
        let mut busy = self.song_ui.jam.loading.is_some();
        if let Some((rx, _, d)) = &self.song_ui.jam.loading
            && let Ok(res) = rx.try_recv()
        {
            let d = d.clone();
            self.song_ui.jam.loading = None;
            busy = false;
            match res {
                Ok(set) => {
                    let set = Arc::new(set);
                    if let Some(old) = self.song_ui.jam.set.replace(set.clone()) {
                        self.song_ui.jam.retired.push(old);
                    }
                    self.song_ui.jam.dir = Some(d);
                    self.song_ui.jam.sent_on = None;
                    self.song_ui.jam.sent_gains = None;
                    self.song_ui.jam.sent_pitch = None;
                    self.send(Command::Song(SongCommand::Stems(Some(set))));
                }
                Err(e) => {
                    self.notify(format!("Couldn't load the song's stems: {e}"));
                    self.song_ui.jam.error = Some((d, e));
                }
            }
        }
        // Free old stems once the audio thread has dropped its copy.
        self.song_ui
            .jam
            .retired
            .retain(|a| Arc::strong_count(a) > 1);
        // Keep the audio thread in step.
        let active = self.jam_active();
        if self.song_ui.jam.sent_on != Some(active) {
            self.song_ui.jam.sent_on = Some(active);
            self.send(Command::Song(SongCommand::StemsOn(active)));
            self.song_update_mute();
        }
        if self.song_ui.jam.set.is_some() {
            let g = self.jam_gains();
            if self.song_ui.jam.sent_gains != Some(g) {
                self.song_ui.jam.sent_gains = Some(g);
                self.send(Command::Song(SongCommand::StemGains(g)));
            }
            let pitch = self.song_transpose();
            if self.song_ui.jam.sent_pitch != Some(pitch) {
                self.song_ui.jam.sent_pitch = Some(pitch);
                self.send(Command::Song(SongCommand::StemPitch(pitch)));
            }
        }
        busy
    }

    /// The audio engine restarted: hand the stems over again.
    pub(super) fn jam_resend(&mut self) {
        let j = &mut self.song_ui.jam;
        j.sent_on = None;
        j.sent_gains = None;
        j.sent_pitch = None;
        if let Some(set) = j.set.clone() {
            self.send(Command::Song(SongCommand::Stems(Some(set))));
        }
    }

    // ---------------------------------------------------------------- UI

    pub(super) fn song_jam(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(song) = &self.song_ui.song else {
            return;
        };
        if song.dir().is_none() {
            return; // a MIDI file: nothing to jam with
        }
        if song.stems.is_empty() {
            ui.label(
                RichText::new(
                    "Jam mode: this song has no stems (separated vocals, drums, bass…). Analyse the recording again \
                     with stem separation on to play along with the real band.",
                )
                .small()
                .color(p.outline),
            );
            return;
        }
        let stems: Vec<String> = song.stems.iter().map(|s| s.name.clone()).collect();
        egui::CollapsingHeader::new(RichText::new("Jam with the recording").strong())
            .id_salt("song-jam")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut on = self.cfg.jam.on;
                    if ui
                        .checkbox(&mut on, "Play the real recording")
                        .on_hover_text(
                            "Use the song's separated parts (stems) as the backing instead of the MIDI notes. \
                             The part you play is muted, so you take its place.",
                        )
                        .changed()
                    {
                        self.cfg.jam.on = on;
                        self.mark_dirty();
                    }
                    let j = &self.song_ui.jam;
                    if let Some((_, prog, _)) = &j.loading {
                        let f = prog.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0;
                        ui.add(
                            egui::ProgressBar::new(f)
                                .desired_width(160.0)
                                .text(format!("loading stems {:.0}%", f * 100.0)),
                        );
                    } else if let Some((_, e)) = &j.error {
                        ui.label(RichText::new(format!("Couldn't load the stems: {e}")).color(p.error));
                        if ui.small_button("Try again").clicked() {
                            self.song_ui.jam.error = None;
                        }
                    } else if let Some(set) = &j.set
                        && self.cfg.jam.on
                    {
                        chip(
                            ui,
                            p.primary_container,
                            p.on_primary_container,
                            format!("{} stems · {:.0} MB", set.stems.len(), set.bytes() as f64 / 1e6),
                        );
                    }
                });
                if !self.cfg.jam.on {
                    ui.label(
                        RichText::new("Off: you hear the MIDI parts (Sound & sync → Hear).")
                            .small()
                            .color(p.outline),
                    );
                    return;
                }
                self.jam_mixer(ui, &stems);
                self.jam_status(ui);
            });
    }

    fn jam_mixer(&mut self, ui: &mut egui::Ui, stems: &[String]) {
        let p = self.palette;
        let mine = self.jam_mine();
        let muted = self.jam_muted();
        let mut gains_changed = false;
        ui.horizontal_wrapped(|ui| {
            for (i, name) in stems.iter().enumerate().take(MAX_STEMS) {
                let is_mine = mine.contains(name);
                let is_muted = muted.get(i).copied().unwrap_or(false);
                let solo = self.song_ui.jam.solo.as_deref() == Some(name.as_str());
                let silent = is_muted && !solo || self.song_ui.jam.solo.is_some() && !solo;
                egui::Frame::new()
                    .fill(if silent { p.surface } else { p.surface_container_low })
                    .stroke(egui::Stroke::new(1.0, if silent { p.surface_container } else { p.outline_variant }))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let c = if silent { p.outline } else { p.on_surface };
                                ui.label(RichText::new(jam::stem_label(name)).strong().color(c));
                                if is_mine {
                                    chip(ui, p.tertiary_container, p.on_surface, "your part");
                                }
                            });
                            ui.horizontal(|ui| {
                                let label = if is_muted { "🔇 Muted" } else { "🔊 On" };
                                let tip = if is_mine {
                                    "Muted because you play this part. Turn it on to hear the original as a guide."
                                } else {
                                    "Mute or unmute this part"
                                };
                                if ui.selectable_label(!is_muted, label).on_hover_text(tip).clicked() {
                                    let now = !is_muted;
                                    let j = &mut self.song_ui.jam;
                                    if now == is_mine {
                                        j.overrides.remove(name);
                                    } else {
                                        j.overrides.insert(name.clone(), now);
                                    }
                                    gains_changed = true;
                                }
                                if ui
                                    .selectable_label(solo, "Solo")
                                    .on_hover_text("Hear only this part (click again to hear everything)")
                                    .clicked()
                                {
                                    self.song_ui.jam.solo = if solo { None } else { Some(name.clone()) };
                                    gains_changed = true;
                                }
                            });
                            let mut v = self.cfg.jam.volumes.get(name).copied().unwrap_or(1.0) * 100.0;
                            if ui
                                .add(
                                    egui::Slider::new(&mut v, 0.0..=150.0)
                                        .suffix("%")
                                        .integer()
                                        .step_by(5.0),
                                )
                                .on_hover_text("Volume of this part")
                                .changed()
                            {
                                if (v - 100.0).abs() < 0.5 {
                                    self.cfg.jam.volumes.remove(name);
                                } else {
                                    self.cfg.jam.volumes.insert(name.clone(), v / 100.0);
                                }
                                self.mark_dirty();
                                gains_changed = true;
                            }
                        });
                    });
            }
        });
        if gains_changed {
            // Sent by jam_poll on the next frame; repaint so that's now.
            ui.ctx().request_repaint();
        }
    }

    fn jam_status(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut lines = Vec::new();
        let speed = self.song_ui.speed;
        if (speed - 1.0).abs() > 1e-6 {
            lines.push(format!(
                "Speed {:.0}%: the recording is time-stretched, so it keeps its pitch.",
                speed * 100.0
            ));
        }
        let t = self.song_transpose();
        if t != 0 {
            lines.push(format!(
                "Your easy part is in an easier key, so the recording is pitch-shifted {t:+} semitone{} to match (drums stay as they are).",
                if t.abs() == 1 { "" } else { "s" }
            ));
        }
        if self.cfg.song.wait && !self.cfg.song.one_key {
            lines.push(
                "Wait mode: while the song waits for you, the band holds its last sound and fades."
                    .into(),
            );
        }
        for l in lines {
            ui.label(RichText::new(l).small().color(p.on_surface_variant));
        }
        self.line_up_toggle(ui);
    }

    /// The Bluetooth delay compensation switch (also in Sound & sync).
    pub(super) fn line_up_toggle(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let delay = self.output_delay_ms();
        ui.horizontal_wrapped(|ui| {
            let mut on = self.cfg.song.line_up;
            if ui
                .checkbox(&mut on, format!("Line up with my output delay ({delay:.0} ms)"))
                .on_hover_text(
                    "Bluetooth earbuds play everything late, your notes too. With this on, the backing and the \
                     falling notes are shifted by that delay, so a key pressed when its note reaches the line \
                     sounds together with the band. Off: the notes follow what you hear instead.",
                )
                .changed()
            {
                self.cfg.song.line_up = on;
                self.mark_dirty();
            }
            let text = if on {
                "press on the line: your note and the band arrive together"
            } else {
                "notes follow what you hear; your presses sound a little late"
            };
            ui.label(RichText::new(text).small().color(p.outline));
        });
    }
}
