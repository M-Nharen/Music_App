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
        // Basses: plucky and round.
        32..=39 => Patch {
            attack: 0.003,
            decay: 1.4,
            sustain: 0.3,
            release: 0.1,
            harmonics: [1.0, 0.55, 0.25, 0.1, 0.05, 0.02],
            brightness_decay: 6.0,
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
    ch: u8,
    patch: Patch,
    /// Frequency ratio of the second, detuned oscillator.
    detune: f32,
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
    /// Patch per MIDI channel.
    patches: [Patch; 16],
    voices: Vec<Voice>,
    drums: DrumKit,
}

const MAX_VOICES: usize = 48;

impl BuiltinSynth {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate as f32,
            patches: [patch_for_program(0); 16],
            voices: Vec::with_capacity(MAX_VOICES),
            drums: DrumKit::new(sample_rate as f32),
        }
    }

    pub fn set_program(&mut self, ch: u8, program: u8) {
        self.patches[ch as usize & 15] = patch_for_program(program);
    }

    pub fn drum(&mut self, note: u8, velocity: u8) {
        self.drums.hit(note, velocity);
    }

    pub fn note_on(&mut self, ch: u8, note: i32, velocity: i32) {
        if self.voices.len() >= MAX_VOICES {
            // Steal the oldest voice.
            self.voices.remove(0);
        }
        let v = velocity.clamp(1, 127) as f32 / 127.0;
        self.voices.push(Voice {
            ch,
            patch: self.patches[ch as usize & 15],
            detune: 2f32.powf(self.patches[ch as usize & 15].detune_cents / 1200.0),
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

    pub fn note_off(&mut self, ch: u8, note: i32) {
        for v in self.voices.iter_mut() {
            if v.ch == ch && v.note == note && v.stage != Stage::Release && v.stage != Stage::Done {
                v.stage = Stage::Release;
                v.release_from = v.env;
            }
        }
    }

    pub fn all_off(&mut self) {
        self.voices.clear();
        self.drums.voices.clear();
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len() + self.drums.voices.len()
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let dt = 1.0 / self.sample_rate;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let mut sum = 0.0;
            for v in self.voices.iter_mut() {
                let p = v.patch;
                let detune = v.detune;
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
            sum += self.drums.next();
            // Soft clip so big chords never crackle.
            let out = sum.tanh();
            *l = out;
            *r = out;
        }
        self.voices.retain(|v| v.stage != Stage::Done);
        self.drums.voices.retain(|v| v.age < v.kit.length);
    }
}

/// Synthesised drum sounds for when there's no SoundFont: a pitched sine
/// (kick, toms) plus filtered noise (snare, hats, cymbals).
#[derive(Clone, Copy)]
struct DrumPatch {
    /// Tone start / end frequency and how fast it slides (per second).
    f0: f32,
    f1: f32,
    sweep: f32,
    /// Second tone (cowbell), 0 = none.
    f2: f32,
    tone: f32,
    tone_decay: f32,
    noise: f32,
    noise_decay: f32,
    /// High-pass amount for the noise, 0..1 (1 = only hiss).
    highpass: f32,
    length: f32,
}

#[allow(clippy::too_many_arguments)]
const fn dp(
    f0: f32,
    f1: f32,
    sweep: f32,
    tone: f32,
    tone_decay: f32,
    noise: f32,
    noise_decay: f32,
    highpass: f32,
) -> DrumPatch {
    let longest = if tone_decay > noise_decay {
        tone_decay
    } else {
        noise_decay
    };
    DrumPatch {
        f0,
        f1,
        sweep,
        f2: 0.0,
        tone,
        tone_decay,
        noise,
        noise_decay,
        highpass,
        length: longest * 7.0,
    }
}

fn drum_patch(note: u8) -> DrumPatch {
    match note {
        35 | 36 => dp(160.0, 48.0, 28.0, 1.0, 0.16, 0.05, 0.01, 0.0),
        38 | 40 => dp(220.0, 180.0, 20.0, 0.45, 0.06, 0.8, 0.09, 0.4),
        37 => dp(1700.0, 1600.0, 5.0, 0.4, 0.012, 0.5, 0.01, 0.6),
        39 => dp(0.0, 0.0, 0.0, 0.0, 0.01, 0.9, 0.07, 0.6),
        42 | 44 => dp(0.0, 0.0, 0.0, 0.0, 0.01, 0.5, 0.025, 0.95),
        46 => dp(0.0, 0.0, 0.0, 0.0, 0.01, 0.45, 0.18, 0.95),
        49 | 52 | 55 | 57 => dp(0.0, 0.0, 0.0, 0.0, 0.01, 0.4, 0.5, 0.9),
        51 | 59 => dp(3200.0, 3200.0, 0.0, 0.06, 0.4, 0.25, 0.3, 0.95),
        53 => dp(2600.0, 2600.0, 0.0, 0.25, 0.3, 0.1, 0.1, 0.95),
        41 | 43 => dp(130.0, 80.0, 12.0, 0.9, 0.2, 0.08, 0.03, 0.2),
        45 | 47 => dp(170.0, 110.0, 12.0, 0.9, 0.18, 0.08, 0.03, 0.2),
        48 | 50 => dp(220.0, 150.0, 12.0, 0.9, 0.16, 0.08, 0.03, 0.2),
        54 | 69 | 70 | 82 => dp(0.0, 0.0, 0.0, 0.0, 0.01, 0.35, 0.04, 0.9),
        56 => DrumPatch {
            f2: 845.0,
            ..dp(560.0, 560.0, 0.0, 0.5, 0.08, 0.0, 0.01, 0.0)
        },
        75..=77 => dp(2400.0, 2300.0, 5.0, 0.7, 0.025, 0.05, 0.005, 0.5),
        60..=64 => dp(420.0, 300.0, 20.0, 0.8, 0.1, 0.06, 0.01, 0.3),
        _ => dp(800.0, 600.0, 10.0, 0.3, 0.05, 0.4, 0.05, 0.6),
    }
}

#[derive(Clone, Copy)]
struct DrumVoice {
    kit: DrumPatch,
    amp: f32,
    age: f32,
    phase: f32,
    phase2: f32,
    lp: f32,
}

struct DrumKit {
    sample_rate: f32,
    voices: Vec<DrumVoice>,
    rng: u32,
}

impl DrumKit {
    fn new(sample_rate: f32) -> Self {
        DrumKit {
            sample_rate,
            voices: Vec::with_capacity(32),
            rng: 0x1234_5678,
        }
    }

    fn hit(&mut self, note: u8, velocity: u8) {
        if self.voices.len() >= 32 {
            self.voices.remove(0);
        }
        // A new hi-hat chokes the open one, like a real kit.
        if matches!(note, 42 | 44) {
            self.voices
                .retain(|v| v.kit.noise_decay != drum_patch(46).noise_decay);
        }
        let v = velocity.clamp(1, 127) as f32 / 127.0;
        self.voices.push(DrumVoice {
            kit: drum_patch(note),
            amp: v * v * 0.6,
            age: 0.0,
            phase: 0.0,
            phase2: 0.0,
            lp: 0.0,
        });
    }

    fn next(&mut self) -> f32 {
        if self.voices.is_empty() {
            return 0.0;
        }
        let dt = 1.0 / self.sample_rate;
        let mut sum = 0.0;
        for v in self.voices.iter_mut() {
            let k = v.kit;
            let mut s = 0.0;
            if k.tone > 0.0 {
                let f = k.f1 + (k.f0 - k.f1) * (-v.age * k.sweep).exp();
                v.phase = (v.phase + f * dt).fract();
                let mut t = (v.phase * TAU).sin();
                if k.f2 > 0.0 {
                    v.phase2 = (v.phase2 + k.f2 * dt).fract();
                    // Square-ish for the cowbell clank.
                    t = (t + (v.phase2 * TAU).sin()).signum() * 0.5;
                }
                s += t * k.tone * (-v.age / k.tone_decay).exp();
            }
            if k.noise > 0.0 {
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 17;
                self.rng ^= self.rng << 5;
                let white = (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
                v.lp += (white - v.lp) * 0.35;
                let shaped = k.highpass * (white - v.lp) + (1.0 - k.highpass) * v.lp;
                s += shaped * k.noise * (-v.age / k.noise_decay).exp();
            }
            v.age += dt;
            sum += s * v.amp;
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_sound_and_stops() {
        let mut s = BuiltinSynth::new(48_000);
        s.note_on(0, 60, 100);
        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        s.render(&mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.01));
        s.note_off(0, 60);
        for _ in 0..20 {
            s.render(&mut l, &mut r);
        }
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn drums_make_sound_and_end() {
        let mut s = BuiltinSynth::new(48_000);
        for n in [36, 38, 42, 46, 49, 56, 75] {
            s.drum(n, 110);
        }
        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        s.render(&mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.05));
        for _ in 0..80 {
            s.render(&mut l, &mut r);
        }
        assert_eq!(s.active_voices(), 0, "drum hits finish on their own");
    }

    #[test]
    fn channels_have_their_own_sound() {
        let mut s = BuiltinSynth::new(48_000);
        s.set_program(2, 33);
        s.note_on(2, 40, 100);
        s.note_on(0, 60, 100);
        s.note_off(0, 40); // wrong channel: bass keeps going
        assert!(
            s.voices
                .iter()
                .any(|v| v.ch == 2 && v.stage != Stage::Release)
        );
    }
}
