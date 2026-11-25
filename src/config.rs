//! Settings in `~/.config/qwertone/`. Everything is plain TOML so it can be
//! edited by hand; the app notices changes and reloads them.

use crate::keymap::Keymap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    let piano = keymap_path("piano");
    if !piano.exists() {
        let _ = write_atomic(&piano, &Keymap::piano().to_toml());
    }
    let path = keymap_path(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => match Keymap::from_toml(&text) {
            Ok((km, w)) => (km, w),
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
    fn slugs() {
        assert_eq!(slug("My Layout!"), "my-layout");
        assert_eq!(slug("   "), "custom");
    }
}
