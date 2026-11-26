//! Colours. On an end4 (illogical-impulse) Hyprland setup the app reads the
//! Material You palette that end4 generates from your wallpaper, so it
//! matches your rice and follows wallpaper changes. Otherwise it uses a
//! built-in Material dark palette.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    pub tertiary: Color32,
    pub tertiary_container: Color32,
    pub surface: Color32,
    pub surface_container_low: Color32,
    pub surface_container: Color32,
    pub surface_container_high: Color32,
    pub surface_container_highest: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    pub error: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Default for Palette {
    /// Material 3 dark baseline with a violet seed.
    fn default() -> Self {
        Self {
            primary: hex(0xCFBCFF),
            on_primary: hex(0x381E72),
            primary_container: hex(0x4F378A),
            on_primary_container: hex(0xE9DDFF),
            secondary_container: hex(0x4A4458),
            on_secondary_container: hex(0xE8DEF8),
            tertiary: hex(0xEFB8C8),
            tertiary_container: hex(0x633B48),
            surface: hex(0x141218),
            surface_container_low: hex(0x1D1B20),
            surface_container: hex(0x211F26),
            surface_container_high: hex(0x2B2930),
            surface_container_highest: hex(0x36343B),
            on_surface: hex(0xE6E0E9),
            on_surface_variant: hex(0xCAC4D0),
            outline: hex(0x938F99),
            outline_variant: hex(0x49454F),
            error: hex(0xF2B8B5),
        }
    }
}

/// Built-in colour themes (Part 9): (config id, name, palette). Used when
/// "Match end4 colours" is off or end4 isn't installed.
pub fn builtin_themes() -> [(&'static str, &'static str, Palette); 5] {
    [
        ("violet", "Violet (dark)", Palette::default()),
        (
            "ocean",
            "Ocean (dark)",
            Palette {
                primary: hex(0x8FCDFF),
                on_primary: hex(0x00344F),
                primary_container: hex(0x004C70),
                on_primary_container: hex(0xCAE6FF),
                secondary_container: hex(0x384956),
                on_secondary_container: hex(0xD3E5F5),
                tertiary: hex(0xA6F0D8),
                tertiary_container: hex(0x2C5348),
                surface: hex(0x0F1418),
                surface_container_low: hex(0x171C20),
                surface_container: hex(0x1B2024),
                surface_container_high: hex(0x252B2F),
                surface_container_highest: hex(0x30363A),
                on_surface: hex(0xDEE3E8),
                on_surface_variant: hex(0xC1C7CE),
                outline: hex(0x8B9198),
                outline_variant: hex(0x41484D),
                error: hex(0xFFB4AB),
            },
        ),
        (
            "forest",
            "Forest (dark)",
            Palette {
                primary: hex(0xA4D397),
                on_primary: hex(0x10380B),
                primary_container: hex(0x275020),
                on_primary_container: hex(0xBFF0B1),
                secondary_container: hex(0x3B4B37),
                on_secondary_container: hex(0xD7E8CD),
                tertiary: hex(0xF0C48A),
                tertiary_container: hex(0x5E4318),
                surface: hex(0x111410),
                surface_container_low: hex(0x191D17),
                surface_container: hex(0x1D211B),
                surface_container_high: hex(0x272B25),
                surface_container_highest: hex(0x32362F),
                on_surface: hex(0xE1E4DA),
                on_surface_variant: hex(0xC3C8BB),
                outline: hex(0x8D9286),
                outline_variant: hex(0x43483F),
                error: hex(0xFFB4AB),
            },
        ),
        (
            "light",
            "Light",
            Palette {
                primary: hex(0x65558F),
                on_primary: hex(0xFFFFFF),
                primary_container: hex(0xE9DDFF),
                on_primary_container: hex(0x22005D),
                secondary_container: hex(0xE8DEF8),
                on_secondary_container: hex(0x1D192B),
                tertiary: hex(0x7E5260),
                tertiary_container: hex(0xFFD9E3),
                surface: hex(0xFEF7FF),
                surface_container_low: hex(0xF7F2FA),
                surface_container: hex(0xF3EDF7),
                surface_container_high: hex(0xECE6F0),
                surface_container_highest: hex(0xE6E0E9),
                on_surface: hex(0x1D1B20),
                on_surface_variant: hex(0x49454F),
                outline: hex(0x7A757F),
                outline_variant: hex(0xCAC4D0),
                error: hex(0xB3261E),
            },
        ),
        (
            "contrast",
            "High contrast",
            Palette {
                primary: hex(0xFFE066),
                on_primary: hex(0x000000),
                primary_container: hex(0x5C4A00),
                on_primary_container: hex(0xFFFFFF),
                secondary_container: hex(0x3A3A3A),
                on_secondary_container: hex(0xFFFFFF),
                tertiary: hex(0x7FE7FF),
                tertiary_container: hex(0x00505E),
                surface: hex(0x000000),
                surface_container_low: hex(0x0E0E0E),
                surface_container: hex(0x161616),
                surface_container_high: hex(0x242424),
                surface_container_highest: hex(0x333333),
                on_surface: hex(0xFFFFFF),
                on_surface_variant: hex(0xE6E6E6),
                outline: hex(0xBDBDBD),
                outline_variant: hex(0x6E6E6E),
                error: hex(0xFF8A80),
            },
        ),
    ]
}

/// A built-in theme by config id (unknown ids get the default).
pub fn builtin_palette(id: &str) -> Palette {
    builtin_themes()
        .into_iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, _, p)| p)
        .unwrap_or_default()
}

impl Palette {
    /// A light background (pick egui's light visuals underneath).
    pub fn is_light(&self) -> bool {
        let [r, g, b, _] = self.surface.to_array();
        0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32 > 128.0
    }
}

/// Where end4 writes its generated colours (newest first).
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(state) = dirs::state_dir() {
        v.push(state.join("quickshell/user/generated/colors.json"));
        v.push(state.join("ags/scss/_material.scss"));
    }
    if let Some(home) = dirs::home_dir() {
        v.push(home.join(".local/state/quickshell/user/generated/colors.json"));
        v.push(home.join(".local/state/ags/scss/_material.scss"));
    }
    v.dedup();
    v
}

/// The first end4 colour file that exists.
pub fn find_system_palette_file() -> Option<PathBuf> {
    candidate_paths().into_iter().find(|p| p.exists())
}

fn parse_hex(s: &str) -> Option<Color32> {
    let s = s
        .trim()
        .trim_matches(|c| c == '"' || c == ';' || c == ',')
        .trim();
    let s = s.strip_prefix('#')?;
    if s.len() < 6 {
        return None;
    }
    let v = u32::from_str_radix(&s[..6], 16).ok()?;
    Some(hex(v))
}

/// Accepts either end4's quickshell `colors.json` (`"primary": "#aabbcc"`)
/// or the older AGS `_material.scss` (`$primary: #aabbcc;`).
pub fn parse_palette(text: &str) -> Option<Palette> {
    let mut map = std::collections::HashMap::new();
    if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(text) {
        for (k, v) in obj {
            if let Some(c) = v.as_str().and_then(parse_hex) {
                map.insert(normalize(&k), c);
            }
        }
    } else {
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix('$') else {
                continue;
            };
            let Some((k, v)) = rest.split_once(':') else {
                continue;
            };
            if let Some(c) = parse_hex(v) {
                map.insert(normalize(k), c);
            }
        }
    }
    let primary = *map.get("primary")?;
    let surface = map
        .get("surface")
        .or_else(|| map.get("background"))
        .copied()?;
    let d = Palette::default();
    let get = |k: &str, fallback: Color32| map.get(k).copied().unwrap_or(fallback);
    Some(Palette {
        primary,
        on_primary: get("onprimary", d.on_primary),
        primary_container: get("primarycontainer", d.primary_container),
        on_primary_container: get("onprimarycontainer", d.on_primary_container),
        secondary_container: get("secondarycontainer", d.secondary_container),
        on_secondary_container: get("onsecondarycontainer", d.on_secondary_container),
        tertiary: get("tertiary", d.tertiary),
        tertiary_container: get("tertiarycontainer", d.tertiary_container),
        surface,
        surface_container_low: get("surfacecontainerlow", surface),
        surface_container: get("surfacecontainer", d.surface_container),
        surface_container_high: get("surfacecontainerhigh", d.surface_container_high),
        surface_container_highest: get("surfacecontainerhighest", d.surface_container_highest),
        on_surface: get("onsurface", get("onbackground", d.on_surface)),
        on_surface_variant: get("onsurfacevariant", d.on_surface_variant),
        outline: get("outline", d.outline),
        outline_variant: get("outlinevariant", d.outline_variant),
        error: get("error", d.error),
    })
}

/// `surface_container_high`, `surfaceContainerHigh`, `m3surfaceContainerHigh`
/// all become `surfacecontainerhigh`.
fn normalize(k: &str) -> String {
    let k: String = k
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    k.strip_prefix("m3").map(str::to_string).unwrap_or(k)
}

pub fn load_system_palette() -> Option<(Palette, PathBuf)> {
    let path = find_system_palette_file()?;
    let text = std::fs::read_to_string(&path).ok()?;
    parse_palette(&text).map(|p| (p, path))
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.is_light() {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    let r = CornerRadius::same(14);
    v.override_text_color = Some(p.on_surface);
    v.panel_fill = p.surface;
    v.window_fill = p.surface_container;
    v.window_stroke = Stroke::new(1.0, p.outline_variant);
    v.window_corner_radius = CornerRadius::same(20);
    v.extreme_bg_color = p.surface_container_low;
    v.faint_bg_color = p.surface_container_low;
    v.code_bg_color = p.surface_container_high;
    v.hyperlink_color = p.primary;
    v.selection.bg_fill = p.primary_container;
    v.selection.stroke = Stroke::new(1.0, p.on_primary_container);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface_container;
    w.noninteractive.weak_bg_fill = p.surface_container;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.outline_variant);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.on_surface_variant);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.surface_container_high;
    w.inactive.weak_bg_fill = p.surface_container_high;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.on_surface);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = p.surface_container_highest;
    w.hovered.weak_bg_fill = p.surface_container_highest;
    w.hovered.bg_stroke = Stroke::new(1.0, p.outline);
    w.hovered.fg_stroke = Stroke::new(1.5, p.on_surface);
    w.hovered.corner_radius = r;

    w.active.bg_fill = p.primary_container;
    w.active.weak_bg_fill = p.primary_container;
    w.active.bg_stroke = Stroke::new(1.0, p.primary);
    w.active.fg_stroke = Stroke::new(2.0, p.on_primary_container);
    w.active.corner_radius = r;

    w.open = w.active;

    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = egui::Margin::same(16);
        for (style, font) in s.text_styles.iter_mut() {
            font.size = match style {
                egui::TextStyle::Heading => 24.0,
                egui::TextStyle::Body | egui::TextStyle::Button => 15.0,
                egui::TextStyle::Small => 12.0,
                egui::TextStyle::Monospace => 14.0,
                _ => font.size,
            };
        }
    });
}

/// `fc-match` a family; the font file, only if it really is that family
/// (fc-match always returns *something*).
fn fc_find(family: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{family}|%{file}", family])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (fam, file) = s.split_once('|')?;
    fam.to_lowercase()
        .contains(&family.to_lowercase())
        .then(|| PathBuf::from(file.trim()))
}

/// Scripts the lyrics can be in (Part 10b), as fallback fonts after the UI
/// font: (family, file name in Arch's `noto-fonts` / Debian's
/// `fonts-noto-core`). egui shapes them with harfrust, so conjuncts and
/// vowel signs come out right.
pub const SCRIPT_FONTS: [(&str, &str); 9] = [
    ("Noto Sans Tamil", "NotoSansTamil-Regular.ttf"),
    ("Noto Sans Telugu", "NotoSansTelugu-Regular.ttf"),
    ("Noto Sans Devanagari", "NotoSansDevanagari-Regular.ttf"),
    ("Noto Sans Kannada", "NotoSansKannada-Regular.ttf"),
    ("Noto Sans Malayalam", "NotoSansMalayalam-Regular.ttf"),
    ("Noto Sans Bengali", "NotoSansBengali-Regular.ttf"),
    ("Noto Sans Gujarati", "NotoSansGujarati-Regular.ttf"),
    ("Noto Sans Gurmukhi", "NotoSansGurmukhi-Regular.ttf"),
    ("Noto Sans Oriya", "NotoSansOriya-Regular.ttf"),
];

/// Where a script font is: fontconfig first, then the usual folders.
pub fn find_script_font(family: &str, file: &str) -> Option<PathBuf> {
    if let Some(p) = fc_find(family) {
        return Some(p);
    }
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts/noto"),
        PathBuf::from("/usr/share/fonts/truetype/noto"),
        PathBuf::from("/usr/share/fonts/google-noto"),
        PathBuf::from("/usr/share/fonts/TTF"),
    ];
    if let Some(d) = dirs::data_dir() {
        dirs.push(d.join("fonts"));
    }
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".fonts"));
    }
    dirs.into_iter().map(|d| d.join(file)).find(|p| p.is_file())
}

/// Use end4's UI font if it's installed (Google Sans Flex / Rubik /
/// Gabarito), plus Noto Sans Tamil / Telugu / Devanagari & co as
/// fallbacks for lyrics. Returns the script fonts that are missing.
pub fn install_fonts(ctx: &egui::Context) -> Vec<&'static str> {
    let mut fonts = FontDefinitions::default();
    for family in ["Google Sans Flex", "Rubik", "Gabarito"] {
        let Some(file) = fc_find(family) else {
            continue;
        };
        if let Ok(bytes) = std::fs::read(&file) {
            fonts
                .font_data
                .insert(family.into(), Arc::new(FontData::from_owned(bytes)));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, family.into());
            break;
        }
    }
    let mut missing = Vec::new();
    for (family, file) in SCRIPT_FONTS {
        let bytes = find_script_font(family, file).and_then(|p| std::fs::read(p).ok());
        let Some(bytes) = bytes else {
            missing.push(family);
            continue;
        };
        fonts
            .font_data
            .insert(family.into(), Arc::new(FontData::from_owned(bytes)));
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push(family.into());
        }
    }
    ctx.set_fonts(fonts);
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quickshell_json() {
        let p = parse_palette(
            r##"{"primary": "#ffb4a8", "surface": "#1a1110", "on_surface": "#f1dfdc",
                 "surface_container_high": "#322826", "background": "#1a1110"}"##,
        )
        .unwrap();
        assert_eq!(p.primary, hex(0xffb4a8));
        assert_eq!(p.surface, hex(0x1a1110));
        assert_eq!(p.on_surface, hex(0xf1dfdc));
        assert_eq!(p.surface_container_high, hex(0x322826));
    }

    #[test]
    fn parses_ags_scss() {
        let p = parse_palette("$darkmode: true;\n$primary: #a8c7ff;\n$background: #111318;\n$onBackground: #e2e2e9;\n")
            .unwrap();
        assert_eq!(p.primary, hex(0xa8c7ff));
        assert_eq!(p.surface, hex(0x111318));
        assert_eq!(p.on_surface, hex(0xe2e2e9));
    }

    #[test]
    fn builtin_themes_are_distinct_and_readable() {
        let themes = builtin_themes();
        let ids: std::collections::HashSet<_> = themes.iter().map(|t| t.0).collect();
        assert_eq!(ids.len(), themes.len());
        assert_eq!(builtin_palette("nope"), Palette::default());
        assert!(builtin_palette("light").is_light());
        assert!(!builtin_palette("violet").is_light());
        // Text must stand out from the background in every theme.
        let lum = |c: Color32| {
            let [r, g, b, _] = c.to_array();
            0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
        };
        for (id, _, p) in themes {
            assert!((lum(p.on_surface) - lum(p.surface)).abs() > 120.0, "{id}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_palette("hello").is_none());
    }
}
