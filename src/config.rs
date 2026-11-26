//! Settings in `~/.config/qwertone/`. Everything is plain TOML so it can be
//! edited by hand; the app notices changes and reloads them.

use crate::audio::patterns::{self, Pattern};
use crate::audio::sequencer::ArpMode;
use crate::keymap::{BUILTIN_LAYOUTS, Keymap};
use crate::theory::Scale;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeviceProfile {
    /// Human name shown in the UI.
    pub label: String,
    /// Measured delay from "sound is produced" to "you hear it", in ms.
    pub latency_ms: f32,
    pub bluetooth: bool,
    /// Instrument to switch to when this device becomes active (index).
    pub instrument: Option<usize>,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            label: String::new(),
            latency_ms: 0.0,
            bluetooth: false,
            instrument: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Name of the keymap file in `keymaps/` (without `.toml`).
    pub keymap: String,
    pub instrument: usize,
    pub volume: f32,
    /// 0 = soft, 1 = normal, 2 = loud.
    pub velocity: usize,
    pub octave: i32,
    pub transpose: i32,
    /// Audio buffer size in frames. 64–128 is low latency; raise it if you
    /// hear crackles.
    pub buffer_frames: u32,
    /// SoundFont (.sf2) to use. Empty = search the usual places.
    pub soundfont: Option<PathBuf>,
    /// Use end4 / Material You colours when available.
    pub follow_system_theme: bool,
    /// When a Bluetooth device becomes active, switch to its saved
    /// instrument (pads/strings hide Bluetooth delay well).
    pub bluetooth_auto_instrument: bool,
    /// Keyed by the PipeWire sink name.
    pub devices: BTreeMap<String, DeviceProfile>,

    // ---- Chords + Melody (Part 2). The key is set by `transpose`.
    pub scale: Scale,
    /// Snap `note` keys to the scale so nothing sounds wrong.
    pub scale_lock: bool,
    /// Mark notes of the scale on key caps and the piano strip.
    pub show_scale: bool,
    /// Pick chord inversions that move the fewest notes.
    pub voice_leading: bool,
    /// Add the chord's root as a bass note.
    pub chord_bass: bool,
    /// How chords are played: "block", "strum", "arpeggio" or a pattern name.
    pub style: String,
    pub tempo: f32,
    /// Gap between strummed notes.
    pub strum_ms: f32,
    pub arp_mode: ArpMode,
    /// Arpeggio notes per beat: 2 = 8ths, 3 = triplets, 4 = 16ths.
    pub arp_rate: u32,
    pub arp_octaves: u32,
    /// Play the pattern's drums.
    pub drums: bool,
    /// Arpeggio / pattern keeps playing after you let go of the chord key.
    pub latch: bool,
    /// Instrument for chords (index into the instrument list); none = same
    /// as the melody.
    pub chord_instrument: Option<usize>,
    /// Bass sound (index into `music::BASS_SOUNDS`).
    pub bass_instrument: usize,
    /// Picture under the keyboard: "auto" (follows the instrument),
    /// "keyboard", "guitar", "strings", "flute", "drums" or "wave".
    pub visual: String,

    // ---- Learn mode (Part 3).
    pub learn: LearnProgress,

    // ---- Song mode + live listen (Part 5).
    pub song: SongSettings,

    // ---- Metronome + recording (Part 4a).
    /// Beats of clicks before a recording starts (0 = none).
    pub count_in_beats: u32,

    // ---- Looper + MIDI out (Part 4b).
    /// Length of a new loop in bars (0 = free: press the loop key again
    /// to close it at the nearest bar line).
    pub loop_bars: u32,
    /// Send what you play to the "Qwertone" virtual MIDI port.
    pub midi_out: bool,
    /// Send the song backing (Song mode) too.
    pub midi_out_song: bool,
    /// Keep playing the sound here while MIDI out is on.
    pub midi_out_local: bool,

    // ---- Jam mode (Part 7).
    pub jam: JamSettings,

    // ---- Lyrics (Part 10b).
    pub lyrics: LyricsSettings,
}

/// Lyrics on the Lyrics screen and in the falling notes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LyricsSettings {
    /// Show the original script (Tamil, Telugu, Devanagari …).
    pub native: bool,
    /// Show it in English letters (Tanglish, Tenglish, Hinglish …).
    pub romanized: bool,
    /// Show the English machine translation.
    pub english: bool,
    /// The current line inside the falling notes.
    pub overlay: bool,
    /// …at the top of the falling notes (false = just above the hit line).
    pub overlay_top: bool,
    /// …with the next line under it, smaller.
    pub overlay_next: bool,
    /// Lyrics nudge in ms (+ = lyrics earlier), on top of the Bluetooth
    /// delay compensation.
    pub offset_ms: f32,
    /// Follow what you hear instead of the falling notes. Only differs with
    /// "line up" on (the notes then run ahead of the sound by the output
    /// delay; singers usually want the lyrics with the voice they hear).
    pub follow_heard: bool,
    /// Text size on the Lyrics screen (1 = normal).
    pub size: f32,
}

impl Default for LyricsSettings {
    fn default() -> Self {
        Self {
            native: true,
            romanized: true,
            english: false,
            overlay: true,
            overlay_top: true,
            overlay_next: true,
            offset_ms: 0.0,
            follow_heard: true,
            size: 1.0,
        }
    }
}

/// Jam mode: the analyzer's stems as the backing track.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JamSettings {
    /// Play the real recording (stems) instead of the MIDI parts when a
    /// song has stems.
    pub on: bool,
    /// Volume per stem name ("vocals", "drums", …), 0–1.5; missing = 1.
    pub volumes: BTreeMap<String, f32>,
}

impl Default for JamSettings {
    fn default() -> Self {
        Self {
            on: true,
            volumes: BTreeMap::new(),
        }
    }
}

/// Song mode and live listen settings, plus best scores.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SongSettings {
    /// Stop at each note until you play it.
    pub wait: bool,
    /// Any key plays the next note.
    pub one_key: bool,
    /// A note in any octave counts.
    pub any_octave: bool,
    /// Switch to a layout made for the song when one is opened.
    pub song_layout: bool,
    pub metronome: bool,
    /// One bar of clicks before playing starts.
    pub count_in: bool,
    /// Speed up 5 % after each good loop pass, up to 100 %.
    pub auto_speed: bool,
    /// Volume of the song's other parts.
    pub backing: f32,
    /// Extra sync adjustment on top of the measured delay (ms, + = later).
    pub offset_ms: f32,
    /// Bluetooth delay compensation (Part 7): the falling notes run ahead
    /// of the sound by the output delay, so a key pressed on the line
    /// sounds together with the backing (same as delaying the backing and
    /// the notes by the delay). Off = the notes follow what you hear.
    pub line_up: bool,
    /// How many beats of falling notes to show.
    pub beats_visible: f32,
    /// Best accuracy (0–100) per song id.
    pub best: BTreeMap<String, u32>,
    /// Songs opened lately (paths), newest first.
    pub recent: Vec<PathBuf>,
    /// Live listen: input device id ("" = default), noise gate (dBFS),
    /// major/minor only.
    pub listen_input: String,
    pub listen_gate_db: f32,
    pub listen_basic: bool,
}

impl Default for SongSettings {
    fn default() -> Self {
        Self {
            wait: true,
            one_key: false,
            any_octave: true,
            song_layout: true,
            metronome: false,
            count_in: true,
            auto_speed: true,
            backing: 0.8,
            offset_ms: 0.0,
            line_up: true,
            beats_visible: 8.0,
            best: BTreeMap::new(),
            recent: Vec::new(),
            listen_input: String::new(),
            listen_gate_db: -50.0,
            listen_basic: false,
        }
    }
}

/// Lesson progress, ear-training records and the theory overlay switch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LearnProgress {
    /// Show the "what you're hearing" panel on the Play tab.
    pub explain: bool,
    /// Ids of finished lessons.
    pub done: BTreeSet<String>,
    /// Lesson that was open last, and its step.
    pub lesson: String,
    pub step: usize,
    /// Best first-try streak per ear game ("note", "interval", "quality", "chord").
    pub best: BTreeMap<String, u32>,
    /// Chosen level (0–2) per ear game.
    pub levels: BTreeMap<String, usize>,
}

impl Default for LearnProgress {
    fn default() -> Self {
        Self {
            explain: true,
            done: BTreeSet::new(),
            lesson: String::new(),
            step: 0,
            best: BTreeMap::new(),
            levels: BTreeMap::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keymap: "piano".into(),
            instrument: 0,
            volume: 0.9,
            velocity: 1,
            octave: 0,
            transpose: 0,
            buffer_frames: 128,
            soundfont: None,
            follow_system_theme: true,
            bluetooth_auto_instrument: true,
            devices: BTreeMap::new(),
            scale: Scale::Major,
            scale_lock: false,
            show_scale: true,
            voice_leading: true,
            chord_bass: true,
            style: "block".into(),
            tempo: 100.0,
            strum_ms: 30.0,
            arp_mode: ArpMode::Up,
            arp_rate: 4,
            arp_octaves: 1,
            drums: true,
            latch: true,
            chord_instrument: None,
            bass_instrument: 0,
            visual: "auto".into(),
            learn: LearnProgress::default(),
            song: SongSettings::default(),
            count_in_beats: 4,
            loop_bars: 4,
            midi_out: false,
            midi_out_song: false,
            midi_out_local: true,
            jam: JamSettings::default(),
            lyrics: LyricsSettings::default(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qwertone")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn keymaps_dir() -> PathBuf {
    config_dir().join("keymaps")
}

pub fn keymap_path(name: &str) -> PathBuf {
    keymaps_dir().join(format!("{name}.toml"))
}

pub fn patterns_dir() -> PathBuf {
    config_dir().join("patterns")
}

impl Config {
    pub fn load() -> (Config, Option<String>) {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(c) => (c, None),
                Err(e) => (
                    Config::default(),
                    Some(format!("config.toml has an error, using defaults: {e}")),
                ),
            },
            Err(_) => {
                let c = Config::default();
                let _ = c.save();
                (c, None)
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(config_dir())?;
        let body = toml::to_string_pretty(self).unwrap_or_default();
        write_atomic(
            &config_path(),
            &format!("# Qwertone settings. Edit freely; the app reloads on change.\n\n{body}"),
        )
    }
}

/// Make sure the built-in keymaps exist on disk, then load the requested one.
pub fn load_keymap(name: &str) -> (Keymap, Vec<String>) {
    let _ = std::fs::create_dir_all(keymaps_dir());
    for (file, _) in BUILTIN_LAYOUTS {
        let path = keymap_path(file);
        if !path.exists()
            && let Some(km) = Keymap::builtin(file)
        {
            let _ = write_atomic(&path, &km.to_toml());
        }
    }
    let path = keymap_path(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => match Keymap::from_toml(&text) {
            Ok((mut km, w)) => {
                // Layouts saved by Part 1 predate the "next layout" key.
                let mut changed = !text.contains("layout+") && km.ensure_layout_key();
                // ... and the Part 4a metronome / record keys.
                changed |= !text.contains("metronome") && km.ensure_record_keys();
                // ... and the Part 4b looper keys.
                changed |= !text.contains("loop-play") && km.ensure_loop_keys();
                if changed {
                    let _ = write_atomic(&path, &km.to_toml());
                }
                (km, w)
            }
            Err(e) => (
                Keymap::piano(),
                vec![format!("{}: {e} (using Piano)", path.display())],
            ),
        },
        Err(_) => (
            Keymap::piano(),
            vec![format!("keymap `{name}` not found, using Piano")],
        ),
    }
}

pub fn save_keymap(file_name: &str, km: &Keymap) -> std::io::Result<()> {
    std::fs::create_dir_all(keymaps_dir())?;
    write_atomic(&keymap_path(file_name), &km.to_toml())
}

/// Names of every keymap file in the keymaps folder.
pub fn list_keymaps() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(keymaps_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? == "toml" {
                Some(p.file_stem()?.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    v.sort();
    v
}

/// Turn a display name into a safe file name.
pub fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "custom".into() } else { s }
}

/// Built-in patterns plus any `*.toml` in the patterns folder. A file with
/// the same name as a built-in replaces it. Returns problems as warnings.
pub fn load_patterns() -> (Vec<Pattern>, Vec<String>) {
    let dir = patterns_dir();
    if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let _ = write_atomic(&dir.join("shuffle.toml"), EXAMPLE_PATTERN);
    }
    let mut list = patterns::builtin();
    let mut warnings = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    for f in files {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match std::fs::read_to_string(&f)
            .map_err(|e| e.to_string())
            .and_then(|t| Pattern::parse(&t))
        {
            Ok(p) => match list
                .iter_mut()
                .find(|q| q.name.eq_ignore_ascii_case(&p.name))
            {
                Some(slot) => *slot = p,
                None => list.push(p),
            },
            Err(e) => warnings.push(format!("pattern {name}: {e}")),
        }
    }
    (list, warnings)
}

/// Written to the patterns folder on first run, as an example to copy.
const EXAMPLE_PATTERN: &str = r#"# An accompaniment pattern. Copy this file to make your own; Qwertone
# loads every .toml in this folder (Chords tab → Reload patterns).
#
# One line per part, one character per step:
#   .  rest        x  hit        X  loud hit      o  soft hit
#   -  hold the previous note one more step
#   bass:  x = root, 5 = fifth, 8 = root an octave up
#   chord: x = whole chord, 1-9 = one chord note (1 = lowest)
#   any other line is a drum: kick, snare, clap, rim, hat, openhat,
#   pedalhat, ride, crash, tamb, shaker, cowbell, lowtom, midtom, hitom …
# Shorter lines repeat. Spaces and | are ignored.

name = "Shuffle"
description = "Bluesy swung triplet groove. Try the Blues scale and 7th chords."
beats = 4
steps_per_beat = 3
[tracks]
bass  = "x-5 x-8 x-5 x-8"
chord = "x.x x.x x.x x.x"
kick  = "x.. ... x.. ..."
snare = "... x.. ... x.."
ride  = "x.x x.x x.x x.x"
"#;

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// Finds a General MIDI SoundFont in the places Arch packages put them.
pub fn find_soundfont(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured
        && p.exists()
    {
        return Some(p.to_path_buf());
    }
    let preferred = [
        "/usr/share/soundfonts/FluidR3_GM.sf2",
        "/usr/share/soundfonts/default.sf2",
        "/usr/share/soundfonts/GeneralUser-GS.sf2",
        "/usr/share/sounds/sf2/FluidR3_GM.sf2",
    ];
    for p in preferred {
        if Path::new(p).exists() {
            return Some(p.into());
        }
    }
    let mut dirs = vec![PathBuf::from("/usr/share/soundfonts")];
    if let Some(d) = dirs::data_dir() {
        dirs.insert(0, d.join("qwertone/soundfonts"));
    }
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sf2")))
                .collect();
            found.sort();
            if let Some(p) = found.into_iter().next() {
                return Some(p);
            }
        }
    }
    None
}

/// Polls a file's modification time so hand edits can be picked up.
pub struct FileWatch {
    path: PathBuf,
    mtime: Option<SystemTime>,
}

impl FileWatch {
    pub fn new(path: PathBuf) -> Self {
        let mtime = mtime(&path);
        Self { path, mtime }
    }

    /// True once each time the file's mtime changes.
    pub fn changed(&mut self) -> bool {
        let now = mtime(&self.path);
        if now != self.mtime {
            self.mtime = now;
            return now.is_some();
        }
        false
    }

    /// Call after writing the file ourselves so we don't "reload" our own save.
    pub fn sync(&mut self) {
        self.mtime = mtime(&self.path);
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.mtime = mtime(&path);
        self.path = path;
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip() {
        let mut c = Config::default();
        c.devices.insert(
            "bluez_output.AA_BB.1".into(),
            DeviceProfile {
                label: "Buds".into(),
                latency_ms: 210.0,
                bluetooth: true,
                instrument: Some(7),
            },
        );
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, c);
        // Missing fields fall back to defaults.
        let partial: Config = toml::from_str("volume = 0.5").unwrap();
        assert_eq!(partial.buffer_frames, 128);
    }

    #[test]
    fn example_pattern_parses() {
        let p = Pattern::parse(EXAMPLE_PATTERN).unwrap();
        assert_eq!(p.steps.len(), 12);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My Layout!"), "my-layout");
        assert_eq!(slug("   "), "custom");
    }
}
