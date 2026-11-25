//! General MIDI drum sounds (channel 10). Names are used by the Drum layout,
//! the keymap file (`drum kick` or `drum 36`) and accompaniment patterns.

pub struct DrumSound {
    pub note: u8,
    /// Name used in files and patterns.
    pub code: &'static str,
    /// Short label for key caps.
    pub short: &'static str,
    pub name: &'static str,
}

const fn d(note: u8, code: &'static str, short: &'static str, name: &'static str) -> DrumSound {
    DrumSound {
        note,
        code,
        short,
        name,
    }
}

pub const DRUMS: &[DrumSound] = &[
    d(36, "kick", "Kick", "Kick drum"),
    d(35, "kick2", "Kick 2", "Deep kick"),
    d(38, "snare", "Snare", "Snare"),
    d(40, "snare2", "Snr 2", "Bright snare"),
    d(37, "rim", "Rim", "Side stick"),
    d(39, "clap", "Clap", "Hand clap"),
    d(42, "hat", "Hat", "Closed hi-hat"),
    d(44, "pedalhat", "Pedal", "Pedal hi-hat"),
    d(46, "openhat", "Open", "Open hi-hat"),
    d(41, "floortom", "Floor", "Floor tom"),
    d(45, "lowtom", "Tom L", "Low tom"),
    d(47, "midtom", "Tom M", "Mid tom"),
    d(50, "hitom", "Tom H", "High tom"),
    d(49, "crash", "Crash", "Crash cymbal"),
    d(57, "crash2", "Crash2", "Crash cymbal 2"),
    d(51, "ride", "Ride", "Ride cymbal"),
    d(53, "bell", "Bell", "Ride bell"),
    d(55, "splash", "Splash", "Splash cymbal"),
    d(52, "china", "China", "China cymbal"),
    d(54, "tamb", "Tamb", "Tambourine"),
    d(56, "cowbell", "Cowbl", "Cowbell"),
    d(70, "shaker", "Shake", "Shaker (maracas)"),
    d(75, "claves", "Clave", "Claves"),
    d(76, "block", "Block", "Wood block"),
    d(62, "conga", "Conga", "Conga (muted)"),
    d(63, "conga2", "Conga↑", "Conga (open)"),
    d(60, "bongo", "Bongo", "High bongo"),
    d(61, "bongo2", "Bongo↓", "Low bongo"),
];

pub fn by_note(note: u8) -> Option<&'static DrumSound> {
    DRUMS.iter().find(|d| d.note == note)
}

/// `kick`, `snare`, … or a GM note number.
pub fn parse(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    if let Ok(n) = s.parse::<u8>() {
        return (27..=87).contains(&n).then_some(n);
    }
    let s = match s.as_str() {
        "closedhat" | "hihat" | "hh" => "hat",
        "sidestick" => "rim",
        "tambourine" => "tamb",
        "ridebell" => "bell",
        "maracas" => "shaker",
        other => other,
    };
    DRUMS.iter().find(|d| d.code == s).map(|d| d.note)
}

/// Name for files: the code if known, else the number.
pub fn code(note: u8) -> String {
    by_note(note)
        .map(|d| d.code.to_string())
        .unwrap_or_else(|| note.to_string())
}

pub fn short(note: u8) -> String {
    by_note(note)
        .map(|d| d.short.to_string())
        .unwrap_or_else(|| format!("#{note}"))
}

pub fn name(note: u8) -> String {
    by_note(note)
        .map(|d| d.name.to_string())
        .unwrap_or_else(|| format!("Drum {note}"))
}
