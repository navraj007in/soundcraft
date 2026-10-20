//! Fonts: IBM Plex (bundled, SIL OFL 1.1) first, then egui's bundled defaults and the operating system's
//! own UI font (loaded at runtime when present, never redistributed) as fallbacks for missing glyphs,
//! and a Hangul fallback after every family so Korean names draw. The "medium", "bold" (Plex Sans
//! SemiBold) and "mono-medium" families are always defined.

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// (path, ttc index) candidates for the regular UI text, by platform.
const REGULAR: &[(&str, u32)] = &[
    ("/System/Library/Fonts/HelveticaNeue.ttc", 0),
    ("/System/Library/Fonts/Helvetica.ttc", 0),
    ("C:\\Windows\\Fonts\\segoeui.ttf", 0),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf", 0),
    ("/usr/local/share/fonts/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", 0),
    ("/usr/share/fonts/liberation-sans-fonts/LiberationSans-Regular.ttf", 0),
];
/// Hangul fallback, appended after every family so Korean session, track and clip names draw
/// whatever the interface language. The Noto Sans CJK collection's face 1 is its Korean face.
const HANGUL: &[(&str, u32)] = &[
    ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0),
    ("/System/Library/Fonts/Supplemental/AppleGothic.ttf", 0),
    ("C:\\Windows\\Fonts\\malgun.ttf", 0),
    ("C:\\Windows\\Fonts\\gulim.ttc", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 1),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 1),
    ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc", 1),
    ("/usr/share/fonts/truetype/nanum/NanumGothic.ttf", 0),
    ("/usr/share/fonts/naver-nanum/NanumGothic.ttf", 0),
];
/// Font files larger than this are skipped: egui keeps the whole file in memory.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FONT_BYTES: u64 = 64 << 20;

/// Asks fontconfig for the desktop's font matching `pattern`. Only TrueType/OpenType results are
/// used: egui cannot read Type 1 or bitmap fonts, which `fc-match` may return on minimal systems.
/// With `lang`, the match must support that language (`fc-match` always returns its best match,
/// which is a Latin face when no Korean font is installed).
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn load_from_fontconfig(pattern: &str, lang: Option<&str>) -> Option<FontData> {
    let output = std::process::Command::new("fc-match").args(["--format=%{file}\\n%{index}\\n%{lang}\\n", pattern]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let mut lines = stdout.lines();
    let path = lines.next()?.trim();
    // The upper 16 bits carry a variable font's named instance; the face index is the lower 16.
    let index = lines.next()?.trim().parse::<u32>().ok()? & 0xFFFF;
    if let Some(lang) = lang
        && !lines.next().unwrap_or_default().split('|').any(|l| l.trim() == lang)
    {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > MAX_FONT_BYTES {
        return None;
    }
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
fn load(cands: &[(&str, u32)], pattern: &str, lang: Option<&str>) -> Option<FontData> {
    if std::env::var_os("SOUNDCRAFT_NO_SYSTEM_FONTS").is_some() {
        return None;
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if let Some(font) = load_from_fontconfig(pattern, lang) {
        return Some(font);
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let _ = (pattern, lang);
    for (p, idx) in cands {
        if std::fs::metadata(p).is_ok_and(|m| m.len() > MAX_FONT_BYTES) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(p) {
            let mut fd = FontData::from_owned(bytes);
            fd.index = *idx;
            return Some(fd);
        }
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn load(_: &[(&str, u32)], _: &str, _: Option<&str>) -> Option<FontData> {
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
    if let Some(reg) = load(REGULAR, "sans-serif", None) {
        fonts.font_data.insert("system-regular".into(), Arc::new(reg));
        fallback.push("system-regular".into());
    }
    let with = |first: &str, rest: &[String]| -> Vec<String> { std::iter::once(first.to_string()).chain(rest.iter().cloned()).collect() };
    fonts.families.insert(FontFamily::Proportional, with("plex-sans", &fallback));
    fonts.families.insert(FontFamily::Monospace, with("plex-mono", &monospace));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), with("plex-sans-medium", &fallback));
    fonts.families.insert(FontFamily::Name(BOLD.into()), with("plex-sans-semibold", &fallback));
    fonts.families.insert(FontFamily::Name(MONO_MEDIUM.into()), with("plex-mono-medium", &monospace));
    // Korean names draw in every family, whatever the interface language.
    if let Some(h) = load(HANGUL, "sans-serif:lang=ko", Some("ko")) {
        fonts.font_data.insert("system-hangul".into(), Arc::new(h));
        for family in fonts.families.values_mut() {
            family.push("system-hangul".into());
        }
    }
    fonts
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hangul_candidates_cover_every_desktop_platform() {
        for prefix in ["/System/Library/Fonts/", "C:\\Windows\\Fonts\\", "/usr/share/fonts/"] {
            assert!(HANGUL.iter().any(|(p, _)| p.starts_with(prefix)), "no Hangul candidate under {prefix}");
        }
        // Face 0 of the Noto Sans CJK collection is Japanese; the Korean face is 1.
        assert!(HANGUL.iter().filter(|(p, _)| p.ends_with("NotoSansCJK-Regular.ttc")).all(|(_, i)| *i == 1));
    }

    /// Korean names draw in every family, not as boxes; skipped when the system has no Korean font.
    #[test]
    fn every_family_draws_hangul_with_a_system_korean_font() {
        let defs = definitions();
        if !defs.font_data.contains_key("system-hangul") {
            eprintln!("skipping: no Korean system font");
            return;
        }
        let families: Vec<FontFamily> = defs.families.keys().cloned().collect();
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        for family in families {
            let font = egui::FontId::new(13.0, family.clone());
            assert!(ctx.fonts_mut(|f| f.has_glyphs(&font, "가한국어세션")), "{family:?} has no Hangul");
        }
    }
}
