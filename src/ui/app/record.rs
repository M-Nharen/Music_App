//! Part 4a: metronome, tap tempo and recording to a MIDI file.

use super::*;
use crate::record::{Take, takes_dir};

/// UI side of the metronome and recorder.
#[derive(Default)]
pub(super) struct RecordUi {
    pub metronome: bool,
    pub tap: crate::record::TapTempo,
    /// Some while recording.
    pub take: Option<Take>,
    /// When the count-in ends (for the "3, 2, 1" display).
    pub live_at: Option<Instant>,
    pub started: Option<Instant>,
    pub last_saved: Option<PathBuf>,
}

impl QwertoneApp {
    pub(super) fn toggle_metronome(&mut self) {
        self.rec.metronome = !self.rec.metronome;
        self.send(Command::Click(self.rec.metronome));
        self.notify(if self.rec.metronome {
            format!("Metronome on · {:.0} BPM", self.cfg.tempo)
        } else {
            "Metronome off".into()
        });
    }

    pub(super) fn tap_tempo(&mut self) {
        match self.rec.tap.tap(Instant::now()) {
            Some(bpm) => {
                self.set_tempo(bpm);
                self.notify(format!("Tempo {bpm:.0} (keep tapping)"));
            }
            None => self.notify("Tap again in time…"),
        }
    }

    pub(super) fn toggle_record(&mut self) {
        if self.rec.take.is_none() {
            let beats = self.cfg.count_in_beats.min(16);
            self.drain_recording();
            self.rec.take = Some(Take::default());
            let now = Instant::now();
            let lead = Duration::from_millis(50)
                + Duration::from_secs_f32(beats as f32 * 60.0 / self.cfg.tempo.max(1.0));
            self.rec.started = Some(now);
            self.rec.live_at = (beats > 0).then(|| now + lead);
            self.send(Command::Record {
                on: true,
                count_in: beats,
            });
            self.notify(if beats > 0 {
                format!("Count-in: {beats} clicks, then play")
            } else {
                "Recording".into()
            });
            return;
        }
        self.send(Command::Record {
            on: false,
            count_in: 0,
        });
        self.drain_recording();
        let take = self.rec.take.take().unwrap_or_default();
        self.rec.live_at = None;
        if take.note_count() == 0 {
            self.notify("Nothing recorded");
            return;
        }
        let sr = self
            .engine
            .as_ref()
            .map_or(48_000, |e| e.stats.sample_rate.load(Ordering::Relaxed));
        let bytes = take.to_midi(sr, self.cfg.tempo);
        match crate::record::save(&takes_dir(), &bytes) {
            Ok(p) => {
                self.notify(format!("Saved {}", p.display()));
                self.rec.last_saved = Some(p);
            }
            Err(e) => self.notify(format!("Couldn't save the recording: {e}")),
        }
    }

    /// Move what the audio thread recorded into the take. Returns true while
    /// recording (so the timer keeps redrawing).
    pub(super) fn drain_recording(&mut self) -> bool {
        let Some(engine) = &self.engine else {
            return false;
        };
        for ev in engine.recorded.try_iter() {
            if let Some(t) = &mut self.rec.take {
                t.events.push(ev);
            }
        }
        self.rec.take.is_some()
    }

    /// Compact bar on the Play tab.
    pub(super) fn record_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        card(ui, &p, "", |ui| {
            ui.horizontal_wrapped(|ui| {
                let m = self.rec.metronome;
                if ui
                    .selectable_label(m, "🕪 Metronome")
                    .on_hover_text(format!(
                        "Click on every beat ({})",
                        self.keys_text(Action::Metronome)
                    ))
                    .clicked()
                {
                    self.toggle_metronome();
                }
                let mut t = self.cfg.tempo;
                if ui
                    .add(
                        egui::DragValue::new(&mut t)
                            .range(40.0..=240.0)
                            .suffix(" BPM"),
                    )
                    .on_hover_text(format!(
                        "Tempo: drag, or {} / {}",
                        self.keys_text(Action::TempoDown),
                        self.keys_text(Action::TempoUp)
                    ))
                    .changed()
                {
                    self.set_tempo(t);
                }
                if ui
                    .button("Tap")
                    .on_hover_text(format!(
                        "Tap in time to set the tempo ({})",
                        self.keys_text(Action::TapTempo)
                    ))
                    .clicked()
                {
                    self.tap_tempo();
                }
                ui.separator();
                let recording = self.rec.take.is_some();
                let label = if recording {
                    "■  Stop + save"
                } else {
                    "●  Record"
                };
                let btn = egui::Button::new(RichText::new(label).color(if recording {
                    p.on_primary
                } else {
                    p.on_secondary_container
                }))
                .fill(if recording {
                    p.primary
                } else {
                    p.secondary_container
                })
                .corner_radius(egui::CornerRadius::same(255));
                if ui
                    .add(btn)
                    .on_hover_text(format!(
                        "Records notes, chords and drums you play as a MIDI file in {} ({})",
                        takes_dir().display(),
                        self.keys_text(Action::Record)
                    ))
                    .clicked()
                {
                    self.toggle_record();
                }
                if recording {
                    let now = Instant::now();
                    match self.rec.live_at {
                        Some(at) if at > now => {
                            let beat = 60.0 / self.cfg.tempo.max(1.0);
                            let left = ((at - now).as_secs_f32() / beat).ceil();
                            ui.label(RichText::new(format!("Count-in… {left:.0}")).strong());
                        }
                        _ => {
                            let from = self.rec.live_at.or(self.rec.started).unwrap_or(now);
                            let s = now.saturating_duration_since(from).as_secs();
                            let n = self.rec.take.as_ref().map_or(0, Take::note_count);
                            ui.label(
                                RichText::new(format!("● {}:{:02} · {n} notes", s / 60, s % 60))
                                    .color(p.primary),
                            );
                        }
                    }
                } else {
                    ui.label("Count-in");
                    let mut b = self.cfg.count_in_beats;
                    if ui
                        .add(egui::DragValue::new(&mut b).range(0..=16).suffix(" beats"))
                        .changed()
                    {
                        self.cfg.count_in_beats = b;
                        self.mark_dirty();
                    }
                    if let Some(path) = &self.rec.last_saved
                        && ui
                            .small_button("Open folder")
                            .on_hover_text(path.display().to_string())
                            .clicked()
                    {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(takes_dir())
                            .spawn();
                    }
                }
            });
        });
    }
}
