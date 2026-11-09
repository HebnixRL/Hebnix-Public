//! glyph fallbacks for languages egui's built-in fonts can't draw (CJK, Thai,
//! Arabic/Hebrew glyphs, Devanagari). windows uses fonts that ship with
//! windows; linux asks fontconfig for an installed font covering the language.
//! if none is found the text just shows boxes like before.
//!
//! note: egui has no RTL shaping, so Arabic/Hebrew would draw unjoined and
//! left-to-right even with a font. not shipped for that reason.

use std::path::PathBuf;

use eframe::egui;

/// languages the built-in fonts already cover need no fallback
#[cfg(target_os = "linux")]
fn needs_fallback(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    let primary = lower.split('-').next().unwrap_or("");
    matches!(
        primary,
        "zh" | "ja" | "ko" | "ar" | "fa" | "ur" | "he" | "th" | "hi" | "mr" | "ne"
    )
}

/// the font file fontconfig picks for `code` (`fc-match :lang=<code>`)
#[cfg(target_os = "linux")]
fn fontconfig_match(code: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", &format!(":lang={code}")])
        .output()
        .ok()?;
    let file = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !file.is_empty()).then(|| PathBuf::from(file))
}

#[cfg(target_os = "linux")]
fn fallback_bytes(code: &str) -> Option<Vec<u8>> {
    if !needs_fallback(code) {
        return None;
    }
    let bytes = fontconfig_match(code).and_then(|path| std::fs::read(path).ok());
    if bytes.is_none() {
        tracing::warn!("i18n: no system font found for `{code}`, glyphs may be missing");
    }
    bytes
}

#[cfg(not(target_os = "linux"))]
fn candidates(code: &str) -> &'static [&'static str] {
    let lower = code.to_ascii_lowercase();
    let primary = lower.split('-').next().unwrap_or("");
    match primary {
        "zh" if lower.contains("hant") || lower.ends_with("-tw") || lower.ends_with("-hk") => {
            &["msjh.ttc", "msyh.ttc", "simsun.ttc"]
        }
        "zh" => &["msyh.ttc", "msjh.ttc", "simsun.ttc"],
        "ja" => &["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"],
        "ko" => &["malgun.ttf", "batang.ttc"],
        "ar" | "fa" | "ur" | "he" => &["segoeui.ttf", "arial.ttf"],
        "th" => &["leelawui.ttf", "tahoma.ttf"],
        "hi" | "mr" | "ne" => &["Nirmala.ttc", "mangal.ttf"],
        _ => &[],
    }
}

#[cfg(not(target_os = "linux"))]
fn fonts_dir() -> PathBuf {
    let windir = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    windir.join("Fonts")
}

#[cfg(not(target_os = "linux"))]
fn fallback_bytes(code: &str) -> Option<Vec<u8>> {
    let dir = fonts_dir();
    candidates(code)
        .iter()
        .find_map(|file| std::fs::read(dir.join(file)).ok())
}

/// appends a fallback font for the active language behind whatever fonts are
/// already set, so a theme font stays first.
pub fn add_fallbacks(fonts: &mut egui::FontDefinitions) {
    let code = super::current();
    let Some(bytes) = fallback_bytes(&code) else {
        return;
    };

    let name = format!("i18n-fallback-{code}");
    fonts
        .font_data
        .insert(name.clone(), egui::FontData::from_owned(bytes).into());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(name.clone());
    }
}
