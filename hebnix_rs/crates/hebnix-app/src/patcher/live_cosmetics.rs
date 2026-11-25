//! File-backed cosmetic values. Package changes are owned, reversible transactions.
mod runtime;
mod lifecycle;
mod game_mods;
mod movies;
pub(crate) use lifecycle::{start, cleanup_after_exit};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}, sync::Mutex};

pub const FILE: &str = "HbxLiveCosmetics.ini";
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
static STATUS: Mutex<String> = Mutex::new(String::new());

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Alignment {
    pub camber: f32,
    pub vertical: f32,
    pub lateral: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Paint {
    pub enabled: bool,
    pub rgb: [u8; 3],
    pub rainbow: bool,
}
impl Default for Paint {
    fn default() -> Self { Self { enabled: false, rgb: [255; 3], rainbow: false } }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TeamColours { pub enabled: bool, pub blue: [u8; 3], pub orange: [u8; 3] }
impl Default for TeamColours {
    fn default() -> Self { Self { enabled: false, blue: [25, 115, 255], orange: [195, 100, 25] } }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub alignment: Alignment,
    pub primary: Paint,
    pub secondary: Paint,
    pub heatseeker: Paint,
    pub trim: Paint,
    pub wheels: Paint,
    pub scoreboard_mmr: bool,
    pub scoreboard_names: bool,
    pub stadium: TeamColours,
    pub hud: TeamColours,
    pub game_mods: std::collections::BTreeMap<String, bool>,
    pub bindings: std::collections::BTreeMap<String, String>,
    pub instant_mode: u8,
    pub instant_private: bool,
    pub instant_tournaments: bool,
}
impl Settings {
    fn validate(&self) -> Result<(), String> {
        for (value, limit) in [(self.alignment.camber, 30.), (self.alignment.vertical, 15.), (self.alignment.lateral, 15.)] {
            if !value.is_finite() || value.abs() > limit { return Err("Live alignment is outside its slider limits".into()); }
        }
        Ok(())
    }
    fn encode(&self) -> Result<String, String> {
        self.validate()?;
        let mut text = format!("\nHbxLive.version=1\nHbxLive.camber={}\nHbxLive.vertical={}\nHbxLive.lateral={}\n", self.alignment.camber, self.alignment.vertical, self.alignment.lateral);
        for (key, paint) in [("primary", &self.primary), ("secondary", &self.secondary), ("heatseeker", &self.heatseeker), ("trim", &self.trim), ("wheels", &self.wheels)] {
            text.push_str(&format!("HbxLive.{key}.enabled={}\n", u8::from(paint.enabled)));
            text.push_str(&format!("HbxLive.{key}.rainbow={}\n", u8::from(paint.rainbow)));
            for (channel, value) in ["r", "g", "b"].into_iter().zip(paint.rgb) {
                // Material parameters use linear RGB.
                let srgb = value as f32 / 255.;
                let value = if srgb <= 0.04045 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) };
                text.push_str(&format!("HbxLive.{key}.{channel}={value:.7}\n"));
            }
        }
        for (key, group) in [("stadium", &self.stadium), ("hud", &self.hud)] {
            text.push_str(&format!("HbxLive.{key}.enabled={}\n", u8::from(group.enabled)));
            for (team, rgb) in [("blue", group.blue), ("orange", group.orange)] {
                for (channel, value) in ["r", "g", "b"].into_iter().zip(rgb) {
                    let srgb = value as f32 / 255.;
                    let value = if srgb <= 0.04045 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) };
                    text.push_str(&format!("HbxLive.{key}.{team}.{channel}={value:.7}\n"));
                }
            }
        }
        for (id, enabled) in &self.game_mods {
            if !MOD_IDS.contains(&id.as_str()) { return Err("Unknown live Game Mod".into()); }
            text.push_str(&format!("HbxLive.mods.{id}={}\n", u8::from(*enabled)));
        }
        for (id, binding) in &self.bindings {
            if !valid_binding(id, binding) { return Err("Invalid live Game Mod binding".into()); }
            text.push_str(&format!("HbxLive.bind.{id}={binding}\n"));
        }
        text.push_str(&format!("HbxLive.instant.mode={}\nHbxLive.instant.private={}\nHbxLive.instant.tournaments={}\n", self.instant_mode.min(4), u8::from(self.instant_private), u8::from(self.instant_tournaments)));
        text.push_str(&format!("HbxLive.scoreboard.mmr={}\nHbxLive.scoreboard.names={}\nHbxLive.end=1\n", u8::from(self.scoreboard_mmr), u8::from(self.scoreboard_names)));
        Ok(text)
    }
}
fn preferences() -> PathBuf { crate::config::base_dir().join("live-cosmetics.json") }
fn values() -> Settings {
    let mut slot = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    slot.get_or_insert_with(|| fs::read(preferences()).ok().and_then(|b| serde_json::from_slice::<Settings>(&b).ok()).filter(|s| s.validate().is_ok()).unwrap_or_default()).clone()
}
pub fn snapshot() -> serde_json::Value {
    serde_json::json!({"settings": values(), "status": STATUS.lock().map(|s| s.clone()).unwrap_or_default(), "running": hebnix_sdk::process::is_rocket_league_running()})
}
pub fn status_snapshot() -> serde_json::Value {
    let detail = STATUS.lock().map(|s| s.clone()).unwrap_or_default();
    let installed = detail == "Live features ready";
    let failed = detail.starts_with("Error:");
    let close = !installed && !detail.starts_with("Checking installed") && hebnix_sdk::process::is_rocket_league_running();
    serde_json::json!({"label": if failed { "Error" } else if installed { "Installed" } else if close { "Close RL" } else { "Pending" },
        "colour": if failed { "danger" } else if installed { "success" } else if close { "danger" } else { "warning" }, "detail": detail})
}
fn status(value: impl Into<String>) { if let Ok(mut slot) = STATUS.lock() { *slot = value.into(); } }
pub fn unavailable(message: &str) { status(message); }
pub fn managed() -> bool { lifecycle::managed() }
const MOD_IDS: &[&str] = &["forfeit", "celebration", "bots-freeplay", "chat-export", "lan-commands", "chat-paste", "instant-suite"];
fn valid_binding(id: &str, binding: &str) -> bool {
    let Some((feature, kind)) = id.split_once('.') else { return false; };
    if !["forfeit", "celebration", "bots-freeplay"].contains(&feature) { return false; }
    match kind {
        "key" => super::forfeit_command::KEYS.contains(&binding),
        "pad" => binding.is_empty() || super::forfeit_command::PAD_BUTTONS.iter().any(|(name, _)| format!("XboxTypeS_{name}") == binding),
        _ => false,
    }
}
pub fn mod_enabled(id: &str) -> bool { values().game_mods.get(id).copied().unwrap_or(false) }
fn apply_game_options(s: &mut Settings, cfg: &crate::config::GameModsCfg) {
    for (id, key, pad, default) in [("forfeit", &cfg.forfeit_key, &cfg.forfeit_pad, super::forfeit_command::DEFAULT_KEY),
        ("celebration", &cfg.celebration_key, &cfg.celebration_pad, super::celebration::DEFAULT_KEY),
        ("bots-freeplay", &cfg.bots_freeplay_key, &cfg.bots_freeplay_pad, super::bots_freeplay::DEFAULT_KEY)] {
        s.bindings.insert(format!("{id}.key"), if key.is_empty() { default.to_owned() } else { key.clone() });
        s.bindings.insert(format!("{id}.pad"), if pad.is_empty() { String::new() } else { format!("XboxTypeS_{pad}") });
    }
    s.instant_mode = ["requeue", "freeplay", "menu", "workshop"].iter().position(|mode| *mode == cfg.instant_suite_mode).map_or(0, |i| i as u8 + 1);
    s.instant_private = cfg.instant_suite_private;
    s.instant_tournaments = cfg.instant_suite_tournaments;
}
pub fn sync_game_options(cooked: &Path, cfg: &crate::config::GameModsCfg) -> Result<(), String> {
    let mut s = values();
    let before = s.encode()?;
    apply_game_options(&mut s, cfg);
    if s.encode()? != before { update(cooked, s)?; }
    Ok(())
}
pub fn set_mod(cooked: &Path, id: &str, enabled: bool, binding: Option<(&str, &str)>) -> Result<(), String> {
    if !MOD_IDS.contains(&id) { return Err("Unknown Game Mod".into()); }
    let mut settings = values();
    settings.game_mods.insert(id.to_owned(), enabled);
    if let Some((key, pad)) = binding {
        settings.bindings.insert(format!("{id}.key"), key.to_owned());
        settings.bindings.insert(format!("{id}.pad"), if pad.is_empty() { String::new() } else { format!("XboxTypeS_{pad}") });
    }
    update(cooked, settings)
}
pub fn scoreboard_options() -> (bool, bool) { let s = values(); (s.scoreboard_mmr, s.scoreboard_names) }
pub fn set_scoreboard(game: &Path, mmr: bool, names: bool) -> Result<(), String> {
    let mut s = values(); s.scoreboard_mmr = mmr; s.scoreboard_names = names;
    update(&game.join("TAGame/CookedPCConsole"), s)
}
pub(super) fn flag(package: &super::upk_package::UpkPackage, key: &str) -> Result<super::script_patch::Script, String> { runtime::flag(package, key) }
pub fn has_native_bridge(package: &super::upk_package::UpkPackage) -> bool {
    let Some(index) = package.find_object("UISavedValues_TA.Get").filter(|i| *i > 0) else { return false; };
    let e = &package.exports[index as usize - 1];
    package.image[e.serial_offset..e.serial_offset + e.serial_size].windows(b"HbxLive.refresh".len()).any(|w| w == b"HbxLive.refresh")
}
pub fn update(cooked: &Path, mut settings: Settings) -> Result<(), String> {
    settings.validate()?;
    for value in [&mut settings.alignment.camber, &mut settings.alignment.vertical, &mut settings.alignment.lateral] { *value = (*value * 2.).round() / 2.; }
    let text = settings.encode()?;
    let parent = cooked.parent().ok_or("Invalid game directory")?;
    if !cooked.is_dir() { return Err("Select the Rocket League installation first".into()); }
    fs::create_dir_all(crate::config::base_dir()).map_err(|e| e.to_string())?;
    super::upk_history::replace(&preferences(), &serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?)?;
    super::upk_history::replace(&parent.join(FILE), text.as_bytes())?;
    *SETTINGS.lock().map_err(|e| e.to_string())? = Some(settings);
    Ok(())
}

pub fn render(ui: &mut eframe::egui::Ui, cooked: &Path, alignment: bool) {
    let mut settings = values();
    let mut changed = false;
    ui.group(|ui| {
        ui.strong(if alignment { "Live Wheel Alignment · All local cars" } else { "Live Car Colours" });
        ui.label("Changes update automatically. The local file remains available while Hebnix is closed.");
        if alignment {
            changed |= ui.add(eframe::egui::Slider::new(&mut settings.alignment.camber, -30.0..=30.0).step_by(0.5).text("Camber")).changed();
            changed |= ui.add(eframe::egui::Slider::new(&mut settings.alignment.vertical, -15.0..=15.0).step_by(0.5).text("Up / Down")).changed();
            changed |= ui.add(eframe::egui::Slider::new(&mut settings.alignment.lateral, -15.0..=15.0).step_by(0.5).text("Track width")).changed();
            if ui.button("Reset alignment").clicked() { settings.alignment = Alignment::default(); changed = true; }
        } else {
            for (label, paint) in [("Primary paint", &mut settings.primary), ("Secondary paint", &mut settings.secondary), ("Heatseeker ball", &mut settings.heatseeker), ("Trim colour", &mut settings.trim), ("Wheel colour", &mut settings.wheels)] {
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut paint.enabled, label).changed();
                    ui.add_enabled_ui(!paint.rainbow, |ui| { changed |= ui.color_edit_button_srgb(&mut paint.rgb).changed(); });
                    changed |= ui.checkbox(&mut paint.rainbow, "Rainbow").changed();
                });
            }
        }

    });
    if changed { if let Err(e) = update(cooked, settings) { status(e); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_is_bounded_numeric_and_defaults_disable_overrides() {
        let text = Settings::default().encode().unwrap();
        assert!(text.len() < 4096);
        assert!(text.starts_with("\nHbxLive.version=1\n"));
        assert!(text.ends_with("HbxLive.end=1\n"));
        assert!(text.contains("HbxLive.primary.enabled=0\n"));
        for line in text.lines().filter(|l| !l.is_empty()) { assert!(line.split_once('=').unwrap().1.parse::<f32>().unwrap().is_finite()); }
        let mut invalid = Settings::default();
        invalid.alignment.camber = f32::NAN;
        assert!(invalid.encode().is_err());
        invalid.alignment.camber = 31.;
        assert!(invalid.encode().is_err());
    }
}
