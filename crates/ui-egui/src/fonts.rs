//! Fonts: IBM Plex (bundled, SIL OFL 1.1) first, then egui's bundled defaults and the operating system's
//! own UI font (loaded at runtime when present, never redistributed) as fallbacks for missing glyphs.
//! The "medium", "bold" (Plex Sans SemiBold) and "mono-medium" families are always defined.

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// (path, ttc index) candidates for regular and bold UI text, by platform.
const REGULAR: &[(&str, u32)] = &[
    ("/System/Library/Fonts/HelveticaNeue.ttc", 0),
    ("/System/Library/Fonts/Helvetica.ttc", 0),
    ("C:\\Windows\\Fonts\\segoeui.ttf", 0),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/dejavu/DejaVuSans.ttf", 0),
    ("/usr/local/share/fonts/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", 0),
];

/// Asks fontconfig for the desktop's UI font. Only TrueType/OpenType results are used: egui cannot
/// read Type 1 or bitmap fonts, which `fc-match` may return on minimal systems.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn load_from_fontconfig(bold: bool) -> Option<FontData> {
    let pattern = if bold { "sans-serif:style=Bold" } else { "sans-serif" };
    let output = std::process::Command::new("fc-match").args(["--format=%{file}\\n%{index}\\n", pattern]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let mut lines = stdout.lines();
    let path = lines.next()?.trim();
    // The upper 16 bits carry a variable font's named instance; the face index is the lower 16.
    let index = lines.next()?.trim().parse::<u32>().ok()? & 0xFFFF;
    let bytes = std::fs::read(path).ok()?;
    let sfnt = matches!(bytes.get(..4), Some(b"\0\x01\0\0" | b"OTTO" | b"true" | b"ttcf"));
    if !sfnt {
        return None;
    }
    let mut font = FontData::from_owned(bytes);
    font.index = index;
    Some(font)
}

#[cfg(not(target_arch = "wasm32"))]
fn load(cands: &[(&str, u32)], bold: bool) -> Option<FontData> {
    if std::env::var_os("SOUNDCRAFT_NO_SYSTEM_FONTS").is_some() {
        return None;
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if let Some(font) = load_from_fontconfig(bold) {
        return Some(font);
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let _ = bold;
    for (p, idx) in cands {
        if let Ok(bytes) = std::fs::read(p) {
            let mut fd = FontData::from_owned(bytes);
            fd.index = *idx;
            return Some(fd);
        }
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn load(_: &[(&str, u32)], _: bool) -> Option<FontData> {
    None
}

/// IBM Plex (SIL OFL 1.1), bundled unmodified; see `assets/fonts/ibm-plex/`.
const PLEX_SANS: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf");
const PLEX_SANS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Medium.ttf");
const PLEX_SANS_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-SemiBold.ttf");
const PLEX_MONO: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-Regular.ttf");
const PLEX_MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-Medium.ttf");

/// The family names the UI draws with (see `theme::{regular, medium, bold, mono, mono_medium}`).
pub const MEDIUM: &str = "medium";
pub const BOLD: &str = "bold";
pub const MONO_MEDIUM: &str = "mono-medium";

/// Plex is first in every family. egui's own fonts and the operating system's UI font follow as
/// fallbacks for the glyphs Plex does not have (emoji, symbols, scripts it does not cover).
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("plex-sans", PLEX_SANS),
        ("plex-sans-medium", PLEX_SANS_MEDIUM),
        ("plex-sans-semibold", PLEX_SANS_SEMIBOLD),
        ("plex-mono", PLEX_MONO),
        ("plex-mono-medium", PLEX_MONO_MEDIUM),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let proportional: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let monospace: Vec<String> = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    // An operating-system font, when there is one, goes after the bundled and egui fonts.
    let mut fallback = proportional.clone();
    if let Some(reg) = load(REGULAR, false) {
        fonts.font_data.insert("system-regular".into(), Arc::new(reg));
        fallback.push("system-regular".into());
    }
    let with = |first: &str, rest: &[String]| -> Vec<String> { std::iter::once(first.to_string()).chain(rest.iter().cloned()).collect() };
    fonts.families.insert(FontFamily::Proportional, with("plex-sans", &fallback));
    fonts.families.insert(FontFamily::Monospace, with("plex-mono", &monospace));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), with("plex-sans-medium", &fallback));
    fonts.families.insert(FontFamily::Name(BOLD.into()), with("plex-sans-semibold", &fallback));
    fonts.families.insert(FontFamily::Name(MONO_MEDIUM.into()), with("plex-mono-medium", &monospace));
    fonts
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}
