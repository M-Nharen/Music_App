//! Figures out what you're listening on (wired vs Bluetooth, which codec)
//! by asking PipeWire through `pactl`, and can switch Bluetooth codecs.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputInfo {
    /// PipeWire sink name, e.g. `bluez_output.AA_BB_CC_DD_EE_FF.1`.
    pub sink: String,
    pub description: String,
    pub bluetooth: bool,
    /// Active Bluetooth codec, e.g. `sbc`, `aac`, `ldac`.
    pub codec: Option<String>,
    /// Card name used to switch codec, e.g. `bluez_card.AA_BB_CC_DD_EE_FF`.
    pub card: Option<String>,
    pub active_profile: Option<String>,
    /// A2DP profiles the card offers, e.g. `a2dp-sink-sbc`.
    pub profiles: Vec<String>,
}

impl OutputInfo {
    /// Rough delay to expect before measuring, in ms.
    pub fn typical_latency_ms(&self) -> f32 {
        if !self.bluetooth {
            return 10.0;
        }
        match self
            .codec
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some(c) if c.contains("lc3") => 40.0,
            Some(c) if c.contains("aptx_ll") => 40.0,
            Some(c) if c.contains("ldac") => 250.0,
            Some(c) if c.contains("aac") => 200.0,
            _ => 170.0,
        }
    }

    /// The profile that gives the lowest delay on this card, if it's not
    /// already active.
    pub fn lowest_latency_profile(&self) -> Option<&str> {
        let rank = |p: &str| {
            let p = p.to_ascii_lowercase();
            if p.contains("lc3") {
                0
            } else if p.contains("aptx_ll") {
                1
            } else if p.ends_with("-sbc") {
                2
            } else if p.contains("sbc_xq") {
                3
            } else if p.contains("aptx") {
                4
            } else if p.contains("aac") {
                5
            } else {
                9
            }
        };
        let best = self
            .profiles
            .iter()
            .filter(|p| p.starts_with("a2dp"))
            .min_by_key(|p| rank(p))?;
        (Some(best.as_str()) != self.active_profile.as_deref()).then_some(best.as_str())
    }
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Query the current default output. None if pactl isn't available.
pub fn query() -> Option<OutputInfo> {
    let sink = pactl(&["get-default-sink"])?.trim().to_string();
    // Without a session manager PipeWire has no default yet.
    if sink.is_empty() || sink.starts_with('@') {
        return None;
    }
    let sinks = pactl(&["list", "sinks"]).unwrap_or_default();
    let mut info = parse_sink(&sinks, &sink);
    info.sink = sink;
    if info.bluetooth {
        let cards = pactl(&["list", "cards"]).unwrap_or_default();
        let card_name = info.card.clone().or_else(|| card_from_sink(&info.sink));
        if let Some(card) = card_name {
            let (profiles, active) = parse_card_profiles(&cards, &card);
            info.profiles = profiles;
            info.active_profile = active;
            info.card = Some(card);
        }
    }
    Some(info)
}

pub fn set_profile(card: &str, profile: &str) -> Result<(), String> {
    let out = Command::new("pactl")
        .args(["set-card-profile", card, profile])
        .output()
        .map_err(|e| format!("couldn't run pactl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn card_from_sink(sink: &str) -> Option<String> {
    let rest = sink.strip_prefix("bluez_output.")?;
    let mac = rest.split('.').next()?;
    Some(format!("bluez_card.{mac}"))
}

fn prop_value(line: &str) -> Option<String> {
    let (_, v) = line.split_once('=')?;
    Some(v.trim().trim_matches('"').to_string())
}

fn parse_sink(text: &str, wanted: &str) -> OutputInfo {
    let mut info = OutputInfo::default();
    for block in text.split("\nSink #").chain(std::iter::once("")) {
        let is_it = block
            .lines()
            .any(|l| l.trim().strip_prefix("Name:").map(str::trim) == Some(wanted));
        if !is_it {
            continue;
        }
        for line in block.lines() {
            let t = line.trim();
            if let Some(d) = t.strip_prefix("Description:") {
                info.description = d.trim().to_string();
            } else if t.starts_with("device.bus =") || t.starts_with("device.api =") {
                if let Some(v) = prop_value(t)
                    && (v == "bluetooth" || v == "bluez5")
                {
                    info.bluetooth = true;
                }
            } else if t.starts_with("api.bluez5.codec =") {
                info.codec = prop_value(t);
                info.bluetooth = true;
            } else if t.starts_with("device.name =")
                && let Some(v) = prop_value(t)
                && v.starts_with("bluez_card.")
            {
                info.card = Some(v);
            }
        }
        break;
    }
    if wanted.starts_with("bluez_output.") {
        info.bluetooth = true;
    }
    info
}

fn parse_card_profiles(text: &str, card: &str) -> (Vec<String>, Option<String>) {
    for block in text.split("\nCard #") {
        if !block
            .lines()
            .any(|l| l.trim().strip_prefix("Name:").map(str::trim) == Some(card))
        {
            continue;
        }
        let mut profiles = Vec::new();
        let mut active = None;
        let mut in_profiles = false;
        for line in block.lines() {
            let t = line.trim();
            if t.starts_with("Profiles:") {
                in_profiles = true;
                continue;
            }
            if let Some(a) = t.strip_prefix("Active Profile:") {
                active = Some(a.trim().to_string());
                in_profiles = false;
                continue;
            }
            if in_profiles {
                // Profile lines are indented deeper than the section header.
                if let Some((name, rest)) = t.split_once(':') {
                    let usable = !rest.contains("available: no");
                    if usable && !name.contains(' ') {
                        profiles.push(name.to_string());
                    }
                } else {
                    in_profiles = false;
                }
            }
        }
        return (profiles, active);
    }
    (Vec::new(), None)
}

/// Polls the default output every couple of seconds on a background thread.
pub fn spawn_watcher(ctx: egui::Context) -> Arc<Mutex<Option<OutputInfo>>> {
    let shared = Arc::new(Mutex::new(None));
    let out = shared.clone();
    std::thread::Builder::new()
        .name("qwertone-devices".into())
        .spawn(move || {
            loop {
                let now = query();
                let changed = {
                    let mut g = out.lock().unwrap();
                    let changed = *g != now;
                    *g = now;
                    changed
                };
                if changed {
                    ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .ok();
    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = "Sink #50
	State: RUNNING
	Name: alsa_output.pci-0000_00_1f.3.analog-stereo
	Description: Built-in Audio Analog Stereo
	Properties:
		device.bus = \"pci\"

Sink #77
	State: RUNNING
	Name: bluez_output.AA_BB_CC_DD_EE_FF.1
	Description: WH-1000XM4
	Properties:
		api.bluez5.codec = \"ldac\"
		device.bus = \"bluetooth\"
		device.name = \"bluez_card.AA_BB_CC_DD_EE_FF\"
";

    const CARDS: &str = "Card #76
	Name: bluez_card.AA_BB_CC_DD_EE_FF
	Driver: module-bluez5-device.c
	Profiles:
		off: Off (sinks: 0, sources: 0, priority: 0, available: yes)
		a2dp-sink-sbc: High Fidelity Playback (A2DP Sink, codec SBC) (sinks: 1, sources: 0, priority: 18, available: yes)
		a2dp-sink-aac: High Fidelity Playback (A2DP Sink, codec AAC) (sinks: 1, sources: 0, priority: 19, available: yes)
		a2dp-sink: High Fidelity Playback (A2DP Sink, codec LDAC) (sinks: 1, sources: 0, priority: 20, available: yes)
		headset-head-unit: Headset Head Unit (HSP/HFP) (sinks: 1, sources: 1, priority: 1, available: yes)
	Active Profile: a2dp-sink
	Ports:
";

    #[test]
    fn parses_bluetooth_sink() {
        let i = parse_sink(SINKS, "bluez_output.AA_BB_CC_DD_EE_FF.1");
        assert!(i.bluetooth);
        assert_eq!(i.codec.as_deref(), Some("ldac"));
        assert_eq!(i.description, "WH-1000XM4");
        assert_eq!(i.card.as_deref(), Some("bluez_card.AA_BB_CC_DD_EE_FF"));
        let wired = parse_sink(SINKS, "alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert!(!wired.bluetooth);
        assert_eq!(wired.description, "Built-in Audio Analog Stereo");
    }

    #[test]
    fn parses_profiles_and_picks_sbc() {
        let (p, active) = parse_card_profiles(CARDS, "bluez_card.AA_BB_CC_DD_EE_FF");
        assert!(p.contains(&"a2dp-sink-sbc".to_string()));
        assert_eq!(active.as_deref(), Some("a2dp-sink"));
        let info = OutputInfo {
            bluetooth: true,
            profiles: p,
            active_profile: active,
            ..Default::default()
        };
        assert_eq!(info.lowest_latency_profile(), Some("a2dp-sink-sbc"));
    }

    #[test]
    fn card_name_from_sink() {
        assert_eq!(
            card_from_sink("bluez_output.AA_BB.1").as_deref(),
            Some("bluez_card.AA_BB")
        );
    }
}
