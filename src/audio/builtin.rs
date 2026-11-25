//! A small built-in synth so the app makes sound even with no SoundFont
//! installed. Additive sine voices with an ADSR envelope; the patch is
//! picked from the General MIDI program number so the instrument list
//! still roughly means something.

use crate::music::midi_to_hz;
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug)]
struct Patch {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    /// Relative amplitudes of harmonics 1..=6.
    harmonics: [f32; 6],
    /// How fast upper harmonics fade (piano-like brightness decay). 0 = never.
    brightness_decay: f32,
    detune_cents: f32,
}

fn patch_for_program(program: u8) -> Patch {
    match program {
        // Pianos / keys: fast attack, decaying tone.
        0..=7 => Patch {
            attack: 0.002,
            decay: 1.6,
            sustain: 0.0,
            release: 0.25,
            harmonics: [1.0, 0.45, 0.22, 0.12, 0.06, 0.03],
            brightness_decay: 3.0,
            detune_cents: 2.0,
        },
        // Mallets / music box.
        8..=15 => Patch {
            attack: 0.001,
            decay: 1.0,
            sustain: 0.0,
            release: 0.3,
            harmonics: [1.0, 0.0, 0.0, 0.35, 0.0, 0.1],
            brightness_decay: 4.0,
            detune_cents: 0.0,
        },
        // Organs: instant on, full sustain.
        16..=23 => Patch {
            attack: 0.01,
            decay: 0.05,
            sustain: 0.9,
            release: 0.08,
            harmonics: [1.0, 0.7, 0.4, 0.5, 0.0, 0.25],
            brightness_decay: 0.0,
            detune_cents: 0.0,
        },
        // Guitars: plucky.
        24..=31 => Patch {
            attack: 0.003,
            decay: 0.9,
            sustain: 0.0,
            release: 0.2,
            harmonics: [1.0, 0.6, 0.35, 0.25, 0.12, 0.06],
            brightness_decay: 5.0,
            detune_cents: 1.0,
        },
        // Leads: bright, sustained.
        80..=87 => Patch {
            attack: 0.005,
            decay: 0.1,
            sustain: 0.8,
            release: 0.12,
            harmonics: [1.0, 0.5, 0.33, 0.25, 0.2, 0.16],
            brightness_decay: 0.0,
            detune_cents: 6.0,
        },
        // Strings, choir, pads, flute: slow soft attack (Bluetooth friendly).
        _ => Patch {
            attack: 0.12,
            decay: 0.4,
            sustain: 0.75,
            release: 0.6,
            harmonics: [1.0, 0.3, 0.15, 0.08, 0.04, 0.02],
            brightness_decay: 0.0,
            detune_cents: 8.0,
        },
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Clone, Copy)]
struct Voice {
    note: i32,
    freq: f32,
    amp: f32,
    phase: [f32; 2],
    env: f32,
    stage: Stage,
    age: f32,
    release_from: f32,
}

pub struct BuiltinSynth {
    sample_rate: f32,
    patch: Patch,
    voices: Vec<Voice>,
}

const MAX_VOICES: usize = 48;

impl BuiltinSynth {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate as f32,
            patch: patch_for_program(0),
            voices: Vec::with_capacity(MAX_VOICES),
        }
    }

    pub fn set_program(&mut self, program: u8) {
        self.patch = patch_for_program(program);
    }

    pub fn note_on(&mut self, note: i32, velocity: i32) {
        if self.voices.len() >= MAX_VOICES {
            // Steal the oldest voice.
            self.voices.remove(0);
        }
        let v = velocity.clamp(1, 127) as f32 / 127.0;
        self.voices.push(Voice {
            note,
            freq: midi_to_hz(note as f32),
            amp: v * v * 0.22,
            phase: [0.0; 2],
            env: 0.0,
            stage: Stage::Attack,
            age: 0.0,
            release_from: 0.0,
        });
    }

    pub fn note_off(&mut self, note: i32) {
        for v in self.voices.iter_mut() {
            if v.note == note && v.stage != Stage::Release && v.stage != Stage::Done {
                v.stage = Stage::Release;
                v.release_from = v.env;
            }
        }
    }

    pub fn all_off(&mut self) {
        self.voices.clear();
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let dt = 1.0 / self.sample_rate;
        let p = self.patch;
        let detune = 2f32.powf(p.detune_cents / 1200.0);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let mut sum = 0.0;
            for v in self.voices.iter_mut() {
                // Envelope.
                match v.stage {
                    Stage::Attack => {
                        v.env += dt / p.attack.max(1e-4);
                        if v.env >= 1.0 {
                            v.env = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        let target = p.sustain;
                        v.env -= (1.0 - target) * dt / p.decay.max(1e-4);
                        if v.env <= target {
                            v.env = target;
                            v.stage = if target <= 0.0005 {
                                Stage::Done
                            } else {
                                Stage::Sustain
                            };
                        }
                    }
                    Stage::Sustain => {}
                    Stage::Release => {
                        v.env -= v.release_from.max(0.05) * dt / p.release.max(1e-3);
                        if v.env <= 0.0 {
                            v.env = 0.0;
                            v.stage = Stage::Done;
                        }
                    }
                    Stage::Done => {}
                }
                v.age += dt;
                // Additive oscillator, two slightly detuned copies for width.
                let bright = if p.brightness_decay > 0.0 {
                    (-v.age * p.brightness_decay).exp()
                } else {
                    1.0
                };
                let mut s = [0.0f32; 2];
                for (osc, ph) in v.phase.iter().enumerate() {
                    for (h, a) in p.harmonics.iter().enumerate() {
                        if *a == 0.0 {
                            continue;
                        }
                        let hn = (h + 1) as f32;
                        let weight = if h == 0 { *a } else { a * bright };
                        s[osc] += weight * (ph * hn * TAU).sin();
                    }
                }
                let out = (s[0] + s[1]) * 0.5 * v.env * v.amp;
                sum += out;
                v.phase[0] = (v.phase[0] + v.freq * dt).fract();
                v.phase[1] = (v.phase[1] + v.freq * detune * dt).fract();
            }
            // Soft clip so big chords never crackle.
            let out = sum.tanh();
            *l = out;
            *r = out;
        }
        self.voices.retain(|v| v.stage != Stage::Done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_sound_and_stops() {
        let mut s = BuiltinSynth::new(48_000);
        s.note_on(60, 100);
        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        s.render(&mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.01));
        s.note_off(60);
        for _ in 0..20 {
            s.render(&mut l, &mut r);
        }
        assert_eq!(s.active_voices(), 0);
    }
}
