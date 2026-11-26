//! Part 4b: the looper bar on the Play tab, and virtual MIDI out settings.

use super::*;
use crate::audio::looper::{BEATS_PER_BAR, LoopCommand, LoopState, LoopStatus, MAX_LAYERS};
use crate::midiout::MidiOut;

/// Loop lengths offered (bars; 0 = free).
const LOOP_LENGTHS: [u32; 6] = [0, 1, 2, 4, 8, 16];

#[derive(Default)]
pub(super) struct LoopUi {
    /// First press of "clear" (it needs a second one within 2 s).
    clear_armed: Option<Instant>,
    /// The virtual MIDI port while it's open.
    pub midi: Option<MidiOut>,
    pub midi_error: Option<String>,
}

fn bars_name(b: u32) -> String {
    match b {
        0 => "Free".into(),
        1 => "1 bar".into(),
        n => format!("{n} bars"),
    }
}

impl QwertoneApp {
    pub(super) fn loop_status(&self) -> LoopStatus {
        self.engine
            .as_ref()
            .and_then(|e| e.stats.looper.lock().ok().map(|s| *s))
            .unwrap_or_default()
    }

    pub(super) fn loop_rec(&mut self) {
        let s = self.loop_status();
        let rec = Command::Loop(LoopCommand::Rec {
            count_in: self.cfg.count_in_beats.min(16),
            bars: self.cfg.loop_bars,
        });
        let msg = match (s.state, s.rec_layer) {
            (LoopState::Empty, _) => {
                let len = if self.cfg.loop_bars == 0 {
                    format!("press {} again at the end", self.keys_text(Action::Loop))
                } else {
                    bars_name(self.cfg.loop_bars)
                };
                match self.cfg.count_in_beats.min(16) {
                    0 => format!("Loop: recording the first layer ({len})"),
                    n => format!("Loop: {n} clicks, then play the first layer ({len})"),
                }
            }
            (LoopState::CountIn, _) => "Loop cancelled".into(),
            (LoopState::First, _) => "Loop closes at the nearest bar line".into(),
            (_, Some(i)) => format!("Layer {} kept", i + 1),
            _ if s.count as usize >= MAX_LAYERS => {
                self.notify(format!(
                    "The loop has {MAX_LAYERS} layers already: undo or clear one first"
                ));
                return;
            }
            _ => format!(
                "Overdub: layer {} is recording. Press {} again to keep it",
                s.count + 1,
                self.keys_text(Action::Loop)
            ),
        };
        self.send(rec);
        self.notify(msg);
    }

    pub(super) fn loop_play(&mut self) {
        let s = self.loop_status();
        match s.state {
            LoopState::Playing => {
                self.send(Command::Loop(LoopCommand::TogglePlay));
                self.notify("Loop stopped");
            }
            LoopState::Stopped => {
                self.send(Command::Loop(LoopCommand::TogglePlay));
                self.notify("Loop playing");
            }
            LoopState::CountIn | LoopState::First => {
                self.notify(format!(
                    "Still recording the first layer: press {} to close the loop",
                    self.keys_text(Action::Loop)
                ));
            }
            LoopState::Empty => self.notify(format!(
                "No loop yet: press {} to record one",
                self.keys_text(Action::Loop)
            )),
        }
    }

    pub(super) fn loop_undo(&mut self) {
        let s = self.loop_status();
        let msg = match (s.state, s.rec_layer) {
            (LoopState::Empty, _) => "Nothing to undo".to_string(),
            (LoopState::CountIn | LoopState::First, _) => "Loop cancelled".into(),
            (_, Some(i)) => format!("Layer {} thrown away", i + 1),
            _ if s.count <= 1 => "Loop cleared".into(),
            _ => format!("Layer {} removed", s.count),
        };
        self.send(Command::Loop(LoopCommand::Undo));
        self.notify(msg);
    }

    pub(super) fn loop_clear(&mut self) {
        if self.loop_status().state == LoopState::Empty {
            self.notify("No loop to clear");
            return;
        }
        let now = Instant::now();
        match self.loop_ui.clear_armed {
            Some(at) if now.duration_since(at) < Duration::from_secs(2) => {
                self.loop_ui.clear_armed = None;
                self.send(Command::Loop(LoopCommand::Clear));
                self.notify("Loop cleared");
            }
            _ => {
                self.loop_ui.clear_armed = Some(now);
                self.notify(format!(
                    "Press {} again to clear every layer",
                    self.keys_text(Action::LoopClear)
                ));
            }
        }
    }

    pub(super) fn loop_mute(&mut self, n: u8) {
        let s = self.loop_status();
        let i = n.saturating_sub(1);
        if i >= s.count || s.state == LoopState::First {
            self.notify(format!("There's no layer {n} yet"));
            return;
        }
        let muted = !s.layers[i as usize].muted;
        self.send(Command::Loop(LoopCommand::Mute(i)));
        self.notify(format!(
            "Layer {n} {}",
            if muted { "muted" } else { "back on" }
        ));
    }

    /// Open / close the MIDI port to match the settings and tell the audio
    /// thread. Also called after the audio restarts.
    pub(super) fn apply_midi_out(&mut self) {
        if !self.cfg.midi_out {
            self.send(Command::MidiOut {
                tx: None,
                song: false,
                local: true,
            });
            self.loop_ui.midi = None;
            return;
        }
        if self.loop_ui.midi.is_none() {
            match MidiOut::start() {
                Ok(m) => {
                    self.loop_ui.midi_error = None;
                    self.loop_ui.midi = Some(m);
                }
                Err(e) => {
                    self.loop_ui.midi_error = Some(e.clone());
                    self.notify(format!("MIDI out: {e}"));
                    return;
                }
            }
        }
        let tx = self.loop_ui.midi.as_ref().map(MidiOut::sender);
        self.send(Command::MidiOut {
            tx,
            song: self.cfg.midi_out_song,
            local: self.cfg.midi_out_local,
        });
    }

    /// Looper bar on the Play tab, under the recorder.
    pub(super) fn looper_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let s = self.loop_status();
        if matches!(
            s.state,
            LoopState::CountIn | LoopState::First | LoopState::Playing
        ) {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
        card(ui, &p, "", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("🔁 Looper").strong());
                let recording = s.rec_layer.is_some() || s.state == LoopState::CountIn;
                let label = match (s.state, recording) {
                    (LoopState::Empty, _) => "●  Record loop",
                    (LoopState::CountIn, _) => "✕  Cancel",
                    (LoopState::First, _) => "⏹  Close loop",
                    (_, true) => "✔  Keep layer",
                    _ => "●  Overdub",
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
                        "Record a loop in time with the metronome, then play more layers on top ({})",
                        self.keys_text(Action::Loop)
                    ))
                    .clicked()
                {
                    self.loop_rec();
                }
                let has_loop = matches!(s.state, LoopState::Playing | LoopState::Stopped);
                if ui
                    .add_enabled(
                        has_loop,
                        egui::Button::new(if s.state == LoopState::Playing {
                            "■ Stop"
                        } else {
                            "▶ Play"
                        }),
                    )
                    .on_hover_text(format!(
                        "Play / stop the loop ({})",
                        self.keys_text(Action::LoopPlay)
                    ))
                    .clicked()
                {
                    self.loop_play();
                }
                if ui
                    .add_enabled(s.state != LoopState::Empty, egui::Button::new("↶ Undo"))
                    .on_hover_text(format!(
                        "Remove the last layer ({})",
                        self.keys_text(Action::LoopUndo)
                    ))
                    .clicked()
                {
                    self.loop_undo();
                }
                let armed = self
                    .loop_ui
                    .clear_armed
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                if ui
                    .add_enabled(
                        s.state != LoopState::Empty,
                        egui::Button::new(if armed { "Sure?" } else { "Clear" }),
                    )
                    .on_hover_text(format!(
                        "Remove every layer: press twice ({})",
                        self.keys_text(Action::LoopClear)
                    ))
                    .clicked()
                {
                    self.loop_clear();
                }
                ui.separator();
                ui.label("Length");
                let mut bars = self.cfg.loop_bars;
                ui.add_enabled_ui(s.state == LoopState::Empty, |ui| {
                    egui::ComboBox::from_id_salt("loop-bars")
                        .selected_text(bars_name(bars))
                        .show_ui(ui, |ui| {
                            for b in LOOP_LENGTHS {
                                ui.selectable_value(&mut bars, b, bars_name(b));
                            }
                        })
                        .response
                        .on_hover_text(
                            "How long a new loop is. Free: press the loop key again when you're \
                             done and it closes at the nearest bar line.",
                        );
                });
                if bars != self.cfg.loop_bars {
                    self.cfg.loop_bars = bars;
                    self.mark_dirty();
                }
                ui.label(
                    RichText::new(match s.state {
                        LoopState::Empty => format!(
                            "Count-in {} · {:.0} BPM",
                            self.cfg.count_in_beats,
                            self.cfg.tempo
                        ),
                        LoopState::CountIn => {
                            format!("Count-in… {:.0}", (-s.pos_beats).ceil().max(1.0))
                        }
                        LoopState::First => "● Recording the first layer".into(),
                        LoopState::Playing if s.rec_layer.is_some() => {
                            format!("● Overdubbing layer {}", s.rec_layer.unwrap_or(0) + 1)
                        }
                        LoopState::Playing => format!(
                            "Playing · {} bars",
                            (s.len_beats as f64 / BEATS_PER_BAR) as u32
                        ),
                        LoopState::Stopped => "Stopped".into(),
                    })
                    .color(if s.rec_layer.is_some() {
                        p.primary
                    } else {
                        p.on_surface_variant
                    }),
                );
            });
            if s.state != LoopState::Empty {
                ui.add_space(6.0);
                loop_timeline(ui, &p, &s);
                ui.add_space(4.0);
                self.layer_chips(ui, &s);
            }
        });
    }

    fn layer_chips(&mut self, ui: &mut egui::Ui, s: &LoopStatus) {
        let p = self.palette;
        let mut toggle = None;
        ui.horizontal_wrapped(|ui| {
            for i in 0..s.count as usize {
                let l = s.layers[i];
                let rec = s.rec_layer == Some(i as u8);
                let mut text = format!("{} · {} notes", i + 1, l.notes);
                if rec {
                    text = format!("● {text}");
                }
                if l.full {
                    text += " (full)";
                }
                let mut rt = RichText::new(text);
                if l.muted {
                    rt = rt.strikethrough().color(p.outline);
                } else if rec {
                    rt = rt.color(p.on_primary);
                }
                let fill = if rec {
                    p.primary
                } else if l.muted {
                    p.surface_container_high
                } else {
                    p.tertiary_container
                };
                let keys = self.keys_text(Action::LoopMute(i as u8 + 1));
                let hint = if keys == "(no key)" {
                    "Click to mute / unmute".to_string()
                } else {
                    format!("Click to mute / unmute ({keys})")
                };
                if ui
                    .add(
                        egui::Button::new(rt)
                            .fill(fill)
                            .corner_radius(egui::CornerRadius::same(255)),
                    )
                    .on_hover_text(hint)
                    .clicked()
                    && !rec
                {
                    toggle = Some(i as u8 + 1);
                }
            }
        });
        if let Some(n) = toggle {
            self.loop_mute(n);
        }
    }

    /// MIDI out card on the Audio tab.
    pub(super) fn midi_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        card(ui, &p, "Virtual MIDI out", |ui| {
            ui.label(
                "Sends everything you play (notes, chords, drums, the looper) to a MIDI port \
                 called “Qwertone”, so other apps can record it or play it with their own sounds. \
                 It's an ALSA sequencer port; PipeWire shows it through its MIDI bridge \
                 (qpwgraph, Helvum, JACK apps on pipewire-jack).",
            );
            ui.add_space(4.0);
            let mut changed = false;
            let mut on = self.cfg.midi_out;
            changed |= ui.checkbox(&mut on, "Send MIDI out").changed();
            ui.add_enabled_ui(on, |ui| {
                let mut local = self.cfg.midi_out_local;
                changed |= ui
                    .checkbox(&mut local, "Also play the sound here")
                    .on_hover_text(
                        "Untick to hear only the other app (the metronome still clicks here)",
                    )
                    .changed();
                let mut song = self.cfg.midi_out_song;
                changed |= ui
                    .checkbox(&mut song, "Send the song backing too (Song mode)")
                    .changed();
                self.cfg.midi_out_local = local;
                self.cfg.midi_out_song = song;
            });
            if changed {
                self.cfg.midi_out = on;
                self.mark_dirty();
                self.apply_midi_out();
            }
            match (&self.loop_ui.midi, &self.loop_ui.midi_error) {
                (Some(m), _) => {
                    ui.label(RichText::new(format!("Port open: {}", m.name)).color(p.primary));
                }
                (None, Some(e)) if self.cfg.midi_out => {
                    ui.label(RichText::new(e).color(p.error));
                }
                _ => {}
            }
            ui.label(
                RichText::new(
                    "Channels: 1 melody, 2 chords, 3 bass, 10 drums; loop layers use 16, 15, 14… \
                     Connect it with e.g. `aconnect Qwertone:0 <synth>` or in qpwgraph.",
                )
                .small()
                .color(p.on_surface_variant),
            );
        });
    }
}

/// A strip showing the loop: bars, beats, where it is now.
fn loop_timeline(ui: &mut egui::Ui, p: &Palette, s: &LoopStatus) {
    let width = ui.available_width().min(900.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, p.surface_container_high);
    // While the first layer records the length isn't known: show the bars
    // so far (or up to the target).
    let len = if s.len_beats > 0.0 {
        s.len_beats
    } else {
        let bar = BEATS_PER_BAR as f32;
        let so_far = (s.pos_beats.max(0.0) / bar).floor() * bar + bar;
        s.target_beats.max(so_far).max(bar)
    };
    let x = |beat: f32| rect.left() + rect.width() * (beat / len).clamp(0.0, 1.0);
    let pos = s.pos_beats.max(0.0);
    if s.state != LoopState::Stopped {
        let fill = if s.rec_layer.is_some() {
            p.primary
        } else {
            p.tertiary
        };
        painter.rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(x(pos), rect.max.y)),
            6.0,
            fill.gamma_multiply(0.45),
        );
    }
    let beats = len.round() as u32;
    // Long loops: bar lines only (every 4th bar past 32 bars).
    let bpb = BEATS_PER_BAR as u32;
    let step = if beats <= 64 {
        1
    } else if beats <= 128 {
        bpb
    } else {
        4 * bpb
    };
    for b in (step..beats).step_by(step as usize) {
        let bar_line = b % bpb == 0;
        let h = if bar_line { 0.0 } else { rect.height() * 0.55 };
        painter.line_segment(
            [
                egui::pos2(x(b as f32), rect.top() + h),
                egui::pos2(x(b as f32), rect.bottom()),
            ],
            egui::Stroke::new(if bar_line { 1.5 } else { 1.0 }, p.outline),
        );
    }
    if s.state != LoopState::Stopped && s.state != LoopState::CountIn {
        painter.line_segment(
            [
                egui::pos2(x(pos), rect.top()),
                egui::pos2(x(pos), rect.bottom()),
            ],
            egui::Stroke::new(2.0, p.on_surface),
        );
    }
}
