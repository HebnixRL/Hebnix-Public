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
    /// Zero means leave the game's original scale untouched.
    pub goal_explosion_scale: f32,
    pub primary: Paint,
    pub secondary: Paint,
    pub heatseeker: Paint,
    pub trim: Paint,
    pub wheels: Paint,
    pub boost: Paint,
    pub decal: Paint,
    pub antenna: Paint,
    pub topper: Paint,
    pub trail: Paint,
    pub goal_explosion: Paint,
    pub banner: Paint,
    pub avatar_border: Paint,
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
        if !self.goal_explosion_scale.is_finite() || (self.goal_explosion_scale != 0. && !(0.1..=10.).contains(&self.goal_explosion_scale)) {
            return Err("Goal explosion scale must be default or between 0.1 and 10".into());
        }
        Ok(())
    }
    fn encode(&self) -> Result<String, String> {
        self.validate()?;
        // UI camber/track-width directions oppose the game's wheel-bone axes.
        // Translate on write so existing installed hooks get the correction live.
        let mut text = format!("\nHbxLive.version=1\nHbxLive.camber={}\nHbxLive.vertical={}\nHbxLive.lateral={}\n", -self.alignment.camber, self.alignment.vertical, -self.alignment.lateral);
        text.push_str(&format!("HbxLive.goal-explosion.enabled={}\nHbxLive.goal-explosion.scale={}\n", u8::from(self.goal_explosion_scale != 0.), self.goal_explosion_scale));
        for (key, paint) in [("primary", &self.primary), ("secondary", &self.secondary), ("heatseeker", &self.heatseeker), ("trim", &self.trim), ("wheels", &self.wheels), ("boost", &self.boost), ("decal", &self.decal), ("antenna", &self.antenna), ("topper", &self.topper), ("trail", &self.trail), ("goal_explosion", &self.goal_explosion), ("banner", &self.banner), ("avatar_border", &self.avatar_border)] {
            text.push_str(&format!("HbxLive.{key}.enabled={}\n", u8::from(paint.enabled)));
            text.push_str(&format!("HbxLive.{key}.rainbow={}\n", u8::from(paint.rainbow && key != "goal_explosion")));
            for (channel, value) in ["r", "g", "b"].into_iter().zip(paint.rgb) {
                // Material parameters use linear RGB.
                let srgb = value as f32 / 255.;
                let value = if srgb <= 0.04045 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) };
                text.push_str(&format!("HbxLive.{key}.{channel}={value:.7}\n"));
            }
        }
        let item_paints = [&self.trim, &self.wheels, &self.boost, &self.decal, &self.antenna, &self.topper, &self.trail, &self.goal_explosion];
        text.push_str(&format!("HbxLive.items.enabled={}\n", u8::from(item_paints.iter().any(|paint| paint.enabled))));
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
        if text.len() > 8192 { return Err("Live cosmetic settings exceed the runtime file limit".into()); }
        Ok(text)
    }
}
fn preferences() -> PathBuf { crate::config::base_dir().join("live-cosmetics.json") }
fn values() -> Settings {
    let mut slot = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    slot.get_or_insert_with(|| fs::read(preferences()).ok().and_then(|b| serde_json::from_slice::<Settings>(&b).ok()).filter(|s| s.validate().is_ok()).unwrap_or_default()).clone()
}
pub fn snapshot() -> serde_json::Value {
    let running = hebnix_sdk::process::is_rocket_league_running();
    serde_json::json!({"settings": values(), "status": STATUS.lock().map(|s| s.clone()).unwrap_or_default(), "running": running,
        "runtime": if running { runtime_report() } else { serde_json::Value::Null }})
}
fn parse_runtime_report(text: &str) -> Option<serde_json::Value> {
    if text.len() > 8192 || !text.ends_with("HbxLiveRuntime.end=1\n") { return None; }
    let lines: std::collections::HashMap<_, _> = text.lines().filter_map(|line| line.split_once('=')).collect();
    let revision: u32 = lines.get("HbxLiveRuntime.version")?.parse().ok()?;
    let mut channels = serde_json::Map::new();
    for key in ["trim", "decal", "wheels", "boost", "antenna", "topper", "trail", "goal_explosion", "banner", "avatar_border"] {
        let boolean = |suffix| match lines.get(format!("{key}.{suffix}").as_str()).copied() {
            Some("1") => serde_json::Value::Bool(true), Some("0") => serde_json::Value::Bool(false), _ => serde_json::Value::Null,
        };
        channels.insert(key.into(), serde_json::json!({"found": boolean("found"), "applied": boolean("applied")}));
    }
    Some(serde_json::json!({"revision": revision, "current": revision == runtime::REVISION, "channels": channels}))
}
fn runtime_report() -> serde_json::Value {
    (|| {
        let path = lifecycle::runtime_report_path()?;
        let metadata = fs::metadata(&path).ok()?;
        if metadata.len() > 8192 || metadata.modified().ok()?.elapsed().ok()?.as_secs() > 4 { return None; }
        parse_runtime_report(&fs::read_to_string(path).ok()?.replace('\r', ""))
    })().unwrap_or(serde_json::Value::Null)
}
pub fn status_snapshot() -> serde_json::Value {
    let detail = STATUS.lock().map(|s| s.clone()).unwrap_or_default();
    let installed = detail == "Live features ready";
    let failed = detail.starts_with("Error:");
    let close = !installed && !detail.starts_with("Checking installed") && hebnix_sdk::process::is_rocket_league_running();
    serde_json::json!({"label": if failed { "Error" } else if installed { "Installed" } else if close { "Close RL" } else { "Installing" },
        "installing": !failed && !installed && !close,
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
            ui.strong("Car Colours");
            for (label, paint) in [("Primary paint", &mut settings.primary), ("Secondary paint", &mut settings.secondary), ("Body / trim", &mut settings.trim), ("Decal", &mut settings.decal), ("Wheels", &mut settings.wheels),
                // ("Antenna", &mut settings.antenna),
                ("Topper", &mut settings.topper), ("Boost", &mut settings.boost), ("Trail", &mut settings.trail), ("Goal explosion", &mut settings.goal_explosion), ("Player banner", &mut settings.banner), ("Avatar border", &mut settings.avatar_border), ("Heatseeker ball", &mut settings.heatseeker)] {
                if label == "Boost" { ui.separator(); ui.strong("Effects"); }
                if label == "Player banner" { ui.separator(); ui.strong("Player Colours"); }
                if label == "Heatseeker ball" { ui.separator(); ui.strong("Heatseeker Ball"); }
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut paint.enabled, label).changed();
                    ui.add_enabled_ui(!paint.rainbow, |ui| { changed |= ui.color_edit_button_srgb(&mut paint.rgb).changed(); });
                    // Rainbow only applies to an enabled paint, so ticking it enables the paint too.
                    if label == "Goal explosion" { paint.rainbow = false; }
                    else if ui.checkbox(&mut paint.rainbow, "Rainbow").changed() { paint.enabled |= paint.rainbow; changed = true; }
                });
            }
        }
        if !alignment {
            for (label, colours) in [("Stadium team colours", &mut settings.stadium), ("HUD team colours", &mut settings.hud)] {
                changed |= ui.checkbox(&mut colours.enabled, label).changed();
                if colours.enabled {
                    ui.horizontal(|ui| {
                        ui.label("Blue");
                        changed |= ui.color_edit_button_srgb(&mut colours.blue).changed();
                        ui.label("Orange");
                        changed |= ui.color_edit_button_srgb(&mut colours.orange).changed();
                    });
                }
            }
        }
    });
    if changed { if let Err(e) = update(cooked, settings) { status(e); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_report_distinguishes_missing_and_unsupported_channels() {
        assert!(parse_runtime_report("HbxLiveRuntime.version=11\nboost.found=1\n").is_none());
        let report = parse_runtime_report(&format!("HbxLiveRuntime.version={}\nboost.found=1\nboost.applied=1\ntrail.found=0\nHbxLiveRuntime.end=1\n", runtime::REVISION)).unwrap();
        assert_eq!(report["current"], true);
        assert_eq!(report["channels"]["boost"]["applied"], true);
        assert_eq!(report["channels"]["trail"]["found"], false);
        assert!(report["channels"]["decal"]["found"].is_null());
        let old = parse_runtime_report("HbxLiveRuntime.version=9\nHbxLiveRuntime.end=1\n").unwrap();
        assert_eq!(old["current"], false);
    }
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

pub fn render_goal_explosion(ui: &mut eframe::egui::Ui, cooked: &Path) {
    let mut settings = values();
    let mut enabled = settings.goal_explosion_scale != 0.;
    let mut changed = ui.checkbox(&mut enabled, "Custom goal explosion scale").changed();
    if changed { settings.goal_explosion_scale = if enabled { 1. } else { 0. }; }
    if enabled {
        changed |= ui.add(eframe::egui::Slider::new(&mut settings.goal_explosion_scale, 0.1..=10.).step_by(0.1).text("x")).changed();
    }
    ui.label("Default preserves the game's original scale. Changes apply to the next goal explosion.");
    if changed { if let Err(error) = update(cooked, settings) { status(error); } }
}

#[cfg(test)]
mod goal_scale_tests {
    use super::*;
    #[test]
    fn old_preferences_preserve_stock_scale() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.goal_explosion_scale, 0.);
        assert!(settings.encode().unwrap().contains("HbxLive.goal-explosion.enabled=0\n"));
    }
    #[test]
    fn boost_defaults_and_rainbow_round_trip() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.boost.enabled && !settings.boost.rainbow);
        assert!(settings.encode().unwrap().contains("HbxLive.boost.enabled=0\n"));
        settings.boost = Paint { enabled: true, rgb: [255, 0, 128], rainbow: true };
        let saved = serde_json::to_vec(&settings).unwrap();
        let loaded: Settings = serde_json::from_slice(&saved).unwrap();
        assert_eq!(loaded.boost.rgb, [255, 0, 128]);
        let encoded = loaded.encode().unwrap();
        assert!(encoded.contains("HbxLive.boost.enabled=1\n"));
        assert!(encoded.contains("HbxLive.boost.rainbow=1\n"));
        assert!(encoded.contains("HbxLive.boost.r=1.0000000\n"));
        assert!(encoded.contains("HbxLive.boost.g=0.0000000\n"));
    }
    #[test]
    fn equipped_item_colours_are_opt_in_and_fit_the_runtime_file() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        for paint in [&mut settings.trim, &mut settings.decal, &mut settings.wheels,
            &mut settings.boost, &mut settings.antenna, &mut settings.topper,
            &mut settings.trail, &mut settings.goal_explosion, &mut settings.banner, &mut settings.avatar_border] {
            assert!(!paint.enabled && !paint.rainbow);
            paint.enabled = true;
            paint.rainbow = true;
            paint.rgb = [35, 120, 200];
        }
        let saved = serde_json::to_vec(&settings).unwrap();
        let loaded: Settings = serde_json::from_slice(&saved).unwrap();
        let encoded = loaded.encode().unwrap();
        assert!(encoded.len() <= 8192);
        for key in ["trim", "decal", "wheels", "boost", "antenna", "topper", "trail", "goal_explosion", "banner", "avatar_border"] {
            assert!(encoded.contains(&format!("HbxLive.{key}.enabled=1\n")), "{key}");
            let rainbow = u8::from(key != "goal_explosion");
            assert!(encoded.contains(&format!("HbxLive.{key}.rainbow={rainbow}\n")), "{key}");
        }
        assert!(encoded.contains("HbxLive.items.enabled=1\n"));
    }
    #[test]
    fn scale_rejects_invalid_values() {
        let mut settings = Settings::default();
        for value in [f32::NAN, f32::INFINITY, -1., 0.05, 10.1] {
            settings.goal_explosion_scale = value;
            assert!(settings.validate().is_err());
        }
        settings.goal_explosion_scale = 1.;
        assert!(settings.encode().unwrap().contains("HbxLive.goal-explosion.enabled=1\n"));
    }
}
