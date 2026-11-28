//! App config, stored under `%AppData%\Hebnix`. First run imports an old
//! config.ini (python version) if present so settings carry over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// default Rocket League install dir: the Epic path on Windows, the Steam
/// library path on Linux (`~/.steam/steam/steamapps/common/rocketleague`).
#[cfg(not(target_os = "linux"))]
pub fn default_rl_path() -> String {
    r"C:\Program Files\Epic Games\rocketleague".to_string()
}

#[cfg(target_os = "linux")]
pub fn default_rl_path() -> String {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".steam/steam/steamapps/common/rocketleague")
        .to_string_lossy()
        .into_owned()
}

#[cfg(not(target_os = "linux"))]
pub fn default_statsapi_path() -> String {
    r"C:\Program Files\Epic Games\rocketleague\TAGame\Config\DefaultStatsAPI.ini".to_string()
}

#[cfg(target_os = "linux")]
pub fn default_statsapi_path() -> String {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".steam/steam/steamapps/common/rocketleague/TAGame/Config/DefaultStatsAPI.ini")
        .to_string_lossy()
        .into_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowCfg {
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
}

impl Default for WindowCfg {
    fn default() -> Self {
        Self {
            width: 1250,
            height: 700,
            x: None,
            y: None,
        }
    }
}

/// What a Black paint on Infinium wheels becomes. BakkesMod applies a plain
/// #000000, which looks different from the in-game Black finish.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlackInfinium {
    #[default]
    Ask,
    Bakkes,
    Vanilla,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsCfg {
    pub black_infinium: BlackInfinium,
    #[serde(default)]
    pub owned_items_only: bool,
    pub hotkey: String,
    pub theme: String,
    pub legacy_ui: bool,
    /// Draw the light pages (Home, Console, About) with egui instead of web views.
    pub egui_light_pages: bool,
    /// UI language code ("en", "de", ...) or "auto" to follow the system
    pub language: String,
    /// Main tab selected when Hebnix starts.
    pub default_tab: String,
    pub navigation_memory: BTreeMap<String, usize>,
    /// One-time migration to the Home dashboard; later startup choices persist.
    #[serde(default)]
    pub home_default_migrated: bool,
    /// main window bg opacity (0.5-1.0)
    pub window_opacity: f32,
    pub start_in_tray: bool,
    /// Linux: the hotkey minimizes the window instead of only hiding it
    /// (for desktops where a hidden window still shows or blocks input)
    pub minimize_to_tray: bool,
    /// Linux: the per-desktop defaults (Minimize to Tray outside Hyprland) were applied once
    pub desktop_defaults_applied: bool,
    /// Linux: the one-time notice pointing at Settings > System when overlays
    /// or the hotkey misbehave (not shown on Hyprland) was dismissed
    pub desktop_notice_shown: bool,
    /// Linux: Rocket League's `My Games/Rocket League` folder inside its Wine
    /// prefix, when automatic detection picks the wrong install (empty = auto)
    pub rl_documents_path: String,
    pub close_to_tray: bool,
    pub rl_path: String,
    pub rl_path_confirmed: bool,
    pub statsapi_path: String,
    pub suppress_left_alerts: bool,
    pub suppress_fullscreen_warning: bool,
    pub suppress_statsapi_rate_warning: bool,
    pub allow_draw_on_hebnix_focus: bool,
    pub toast_position: crate::toast::ToastPos,
    pub restrict_hotkey_to_hebnix_or_rocket_league: bool,
    pub panel_enabled: bool,
    /// overlay font file stem from fonts/ (empty = bundled default)
    pub overlay_font: String,
    /// overlay regular / bold weights, only used by variable fonts
    pub overlay_font_weight: u32,
    pub overlay_font_bold_weight: u32,
    /// Controller side panel: show every page, not just the in-game ones (maps, plugins).
    pub panel_advanced: bool,
    /// Controller side panel: dock on the left edge instead of the right.
    pub panel_left: bool,
    /// Controller side panel width, as a share of the game window (0.2 to 0.5).
    pub panel_width: f32,
    /// Controller side panel: slide in like a drawer.
    pub panel_animate: bool,
    /// Relaunch elevated on start when requested by the user.
    pub run_as_admin: bool,
    /// Publish Hebnix/Rocket League activity to the local Discord client.
    pub discord_rich_presence: bool,
    /// Show presence only while Rocket League is running.
    pub discord_rocket_league_only: bool,
    /// Include the selected live match fields in Rich Presence.
    #[serde(alias = "discord_current_gamemode")]
    pub discord_game_state: bool,
    pub discord_show_score: bool,
    pub discord_show_map: bool,
    pub discord_show_gamemode: bool,
    pub discord_custom_message: String,
    /// let other Workshop multiplayer players download maps from this PC
    pub p2p_file_sharing: bool,
    /// how often the overlay and plugins update while a plugin is reading the
    /// screen (fps). lower saves CPU on slower PCs
    pub overlay_refresh_fps: u32,
    /// prefix console lines with the local time they were written
    pub console_timestamps: bool,
}

impl Default for SettingsCfg {
    fn default() -> Self {
        Self {
            owned_items_only: false,
            hotkey: "f2".to_string(),
            theme: "Dark".to_string(),
            legacy_ui: false,
            egui_light_pages: false,
            language: crate::i18n::AUTO.to_string(),
            default_tab: "Home".to_string(),
            navigation_memory: BTreeMap::new(),
            home_default_migrated: true,
            window_opacity: 0.96,
            start_in_tray: false,
            minimize_to_tray: false,
            desktop_defaults_applied: false,
            desktop_notice_shown: false,
            rl_documents_path: String::new(),
            close_to_tray: false,
            rl_path: default_rl_path(),
            rl_path_confirmed: false,
            statsapi_path: default_statsapi_path(),
            suppress_left_alerts: false,
            suppress_fullscreen_warning: false,
            suppress_statsapi_rate_warning: false,
            allow_draw_on_hebnix_focus: true,
            toast_position: Default::default(),
            restrict_hotkey_to_hebnix_or_rocket_league: true,
            overlay_font: String::new(),
            overlay_font_weight: 400,
            overlay_font_bold_weight: 700,
            panel_advanced: false,
            panel_enabled: true,
            panel_left: false,
            panel_width: 0.30,
            panel_animate: true,
            run_as_admin: false,
            black_infinium: BlackInfinium::Ask,
            discord_rich_presence: true,
            discord_rocket_league_only: false,
            discord_game_state: true,
            discord_show_score: true,
            discord_show_map: true,
            discord_show_gamemode: true,
            discord_custom_message: "Playing Rocket League".to_string(),
            p2p_file_sharing: true,
            overlay_refresh_fps: 30,
            console_timestamps: true,
        }
    }
}
/// How Rocket League actually gets launched/restarted - used by the
/// Restart Rocket League button and Workshop LAN's Host/Join. The default
/// preserves the original Steam-vs-Epic path detection for existing configs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RlLaunchMode {
    #[default]
    // Heroic is no longer supported; a Heroic-only setup falls back to
    // auto-detection.
    #[serde(alias = "heroic_direct")]
    Unconfigured,
    #[serde(alias = "steam_proton")]
    SteamNative,
    EpicDirect,
    /// A Steam non-Steam shortcut to any launcher (Slipstream, Heroic, a
    /// script), started with steam://rungameid.
    #[serde(alias = "steam_shortcut_to_heroic")]
    SteamShortcut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RlLaunchCfg {
    pub mode: RlLaunchMode,
    pub steam_id: String,
    /// The shortcut's target executable, picked by the setup scan. A
    /// Slipstream-style config.json next to it can say where the game is.
    pub shortcut_exe: String,
}

impl Default for RlLaunchCfg {
    fn default() -> Self {
        Self {
            mode: RlLaunchMode::Unconfigured,
            steam_id: "252950".to_string(),
            shortcut_exe: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchSource {
    #[default]
    Catalog,
    Custom,
}

/// experimental animation speed patch, see patcher/speed_patch.rs
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeedPatchCfg {
    /// show a speed picker on the Items page decal and swap rows
    pub items_page: bool,
}

/// Game Mods page settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GameModsCfg {
    /// Rocket League key name the forfeit command is bound to.
    pub forfeit_key: String,
    /// Rocket League key name the celebration command is bound to.
    pub celebration_key: String,
    /// Rocket League key name the bots in freeplay command is bound to.
    pub bots_freeplay_key: String,
    /// Controller buttons (XboxTypeS_ names without the prefix) for the same commands, empty for none.
    pub forfeit_pad: String,
    pub celebration_pad: String,
    pub bots_freeplay_pad: String,
    /// What Instant Suite does when a match ends: requeue, freeplay, menu or workshop.
    pub instant_suite_mode: String,
    /// The map Instant Suite's workshop action loads, by path.
    pub instant_suite_map: String,
    pub instant_suite_private: bool,
    pub instant_suite_tournaments: bool,
}

impl Default for GameModsCfg {
    fn default() -> Self {
        Self {
            forfeit_key: String::new(), celebration_key: String::new(), bots_freeplay_key: String::new(),
            forfeit_pad: String::new(), celebration_pad: String::new(), bots_freeplay_pad: String::new(),
            instant_suite_mode: "nothing".into(), instant_suite_map: String::new(),
            instant_suite_private: false, instant_suite_tournaments: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayUploadCfg {
    #[serde(default = "legacy_replay_upload_enabled")]
    pub upload_enabled: bool,
    pub save_locally: bool,
    /// Empty uses the Steam/Epic replay directory detected for this installation.
    pub save_directory: String,
    pub provider: String,
    pub api_key: String,
    #[serde(default = "default_replay_naming_template")]
    pub naming_template: String,
    pub visibility: String,
    pub group_id: String,
    pub debug_logging: bool,
}

// Older replay settings had an explicit Start button and implied uploading.
fn legacy_replay_upload_enabled() -> bool {
    true
}

fn default_replay_naming_template() -> String {
    "Hebnix - {gamemode} - {date} {time24}".to_string()
}

impl ReplayUploadCfg {
    pub fn enabled(&self) -> bool {
        self.upload_enabled || self.save_locally
    }
}

impl Default for ReplayUploadCfg {
    fn default() -> Self {
        Self {
            upload_enabled: false,
            save_locally: false,
            save_directory: String::new(),
            provider: "ballchasing.com".to_string(),
            api_key: String::new(),
            naming_template: default_replay_naming_template(),
            visibility: "private".to_string(),
            group_id: String::new(),
            debug_logging: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PatcherCfg {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_ball: Option<String>,
    pub active_boost: Option<String>,
    pub active_decals: std::collections::HashMap<String, String>,
    pub active_cars: std::collections::HashMap<String, String>,
    pub active_anthems: std::collections::HashMap<String, String>,
    pub ball_source: PatchSource,
    pub boost_source: PatchSource,
    pub decal_source: PatchSource,
    pub decal_only_owned: bool,
    pub car_source: PatchSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionButtonAction {
    StartRocketLeague,
    RestartRocketLeague,
    CloseRocketLeague,
    OpenHebnixFolder,
    OpenPluginsFolder,
    ReloadPlugins,
    FixEpicConnection,
}

impl ActionButtonAction {
    pub const CLOSED: [Self; 5] = [
        Self::StartRocketLeague,
        Self::OpenHebnixFolder,
        Self::OpenPluginsFolder,
        Self::ReloadPlugins,
        Self::FixEpicConnection,
    ];

    pub const OPEN: [Self; 5] = [
        Self::RestartRocketLeague,
        Self::CloseRocketLeague,
        Self::OpenHebnixFolder,
        Self::OpenPluginsFolder,
        Self::ReloadPlugins,
    ];

    pub fn label(self) -> String {
        use crate::i18n::t;
        match self {
            Self::StartRocketLeague => t("action-start-rocket-league"),
            Self::RestartRocketLeague => t("action-restart-rocket-league"),
            Self::CloseRocketLeague => t("action-close-rocket-league"),
            Self::OpenHebnixFolder => t("action-open-hebnix-folder"),
            Self::OpenPluginsFolder => t("action-open-plugins-folder"),
            Self::ReloadPlugins => t("action-reload-plugins"),
            Self::FixEpicConnection => t("action-fix-epic-connection"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionButtonEntry {
    pub action: ActionButtonAction,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionButtonCfg {
    pub show_in_header: bool,
    pub rocket_league_closed: Vec<ActionButtonEntry>,
    pub rocket_league_open: Vec<ActionButtonEntry>,
}

impl Default for ActionButtonCfg {
    fn default() -> Self {
        Self {
            show_in_header: false,
            rocket_league_closed: ActionButtonAction::CLOSED
                .into_iter()
                .map(|action| ActionButtonEntry {
                    enabled: action == ActionButtonAction::StartRocketLeague,
                    action,
                })
                .collect(),
            rocket_league_open: ActionButtonAction::OPEN
                .into_iter()
                .map(|action| ActionButtonEntry {
                    enabled: action == ActionButtonAction::RestartRocketLeague,
                    action,
                })
                .collect(),
        }
    }
}

impl ActionButtonCfg {
    fn normalize(&mut self) {
        normalize_action_entries(
            &mut self.rocket_league_closed,
            &ActionButtonAction::CLOSED,
            ActionButtonAction::StartRocketLeague,
        );
        normalize_action_entries(
            &mut self.rocket_league_open,
            &ActionButtonAction::OPEN,
            ActionButtonAction::RestartRocketLeague,
        );
    }
}

fn normalize_action_entries(
    entries: &mut Vec<ActionButtonEntry>,
    allowed: &[ActionButtonAction],
    default_action: ActionButtonAction,
) {
    let mut normalized = Vec::with_capacity(allowed.len());
    for entry in entries.drain(..) {
        if allowed.contains(&entry.action)
            && !normalized
                .iter()
                .any(|existing: &ActionButtonEntry| existing.action == entry.action)
        {
            normalized.push(entry);
        }
    }
    for &action in allowed {
        if !normalized.iter().any(|entry| entry.action == action) {
            normalized.push(ActionButtonEntry {
                action,
                enabled: false,
            });
        }
    }
    if !normalized.iter().any(|entry| entry.enabled) {
        if let Some(entry) = normalized
            .iter_mut()
            .find(|entry| entry.action == default_action)
        {
            entry.enabled = true;
        }
    }
    *entries = normalized;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreboardMmrSource {
    #[default]
    Playlist,
    Gamemode,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoreboardCfg {
    pub ignore_self: bool,
    pub mmr_source: ScoreboardMmrSource,
    pub fixed_playlist: Option<i64>,
}

impl Default for ScoreboardCfg {
    fn default() -> Self {
        Self {
            ignore_self: true,
            mmr_source: ScoreboardMmrSource::Playlist,
            fixed_playlist: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Last web notification explicitly dismissed by the user.
    pub notification_id: u64,
    pub window: WindowCfg,
    pub settings: SettingsCfg,
    pub rl_launch: RlLaunchCfg,
    pub patcher: PatcherCfg,
    pub speed_patch: SpeedPatchCfg,
    pub game_mods: GameModsCfg,
    pub replay_upload: ReplayUploadCfg,
    pub scoreboard: ScoreboardCfg,
    pub action_button: ActionButtonCfg,
    /// enabled state keyed by plugin slug
    pub plugins: BTreeMap<String, bool>,
    /// overlay stacking, bottom first. slugs not listed go on top in load order.
    pub overlay_order: Vec<String>,
}

impl Config {
    /// load config.toml, else import an old config.ini, else defaults
    pub fn load(base_dir: &Path) -> Self {
        let toml_path = base_dir.join("config.toml");
        if let Ok(text) = std::fs::read_to_string(&toml_path) {
            match toml::from_str::<Config>(&text) {
                Ok(mut cfg) => {
                    cfg.action_button.normalize();
                    return cfg;
                }
                Err(e) => {
                    // Keep the unreadable file: it may be from a newer Hebnix, and saving
                    // the defaults below would otherwise wipe every setting in it.
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs());
                    let kept = base_dir.join(format!("config.toml.invalid-{stamp}"));
                    match std::fs::rename(&toml_path, &kept) {
                        Ok(()) => tracing::warn!(
                            "config.toml is invalid ({e}); moved it to {} and using defaults",
                            kept.display()
                        ),
                        Err(rename_err) => {
                            // could not set it aside, so leave it untouched and don't save
                            tracing::warn!(
                                "config.toml is invalid ({e}) and could not be moved aside ({rename_err}); using defaults without saving"
                            );
                            return Config::default();
                        }
                    }
                }
            }
        }

        let ini_path = base_dir.join("config.ini");
        if ini_path.exists() {
            if let Some(cfg) = Self::import_ini(&ini_path) {
                tracing::info!("Imported legacy config.ini");
                let _ = cfg.save(base_dir);
                return cfg;
            }
        }

        let cfg = Config::default();
        let _ = cfg.save(base_dir);
        cfg
    }

    /// Navigation must never rewrite patcher manifests on the UI thread.
    pub(crate) fn save_navigation_memory(&self, base_dir:&Path) {
        use std::sync::{OnceLock,mpsc};
        type Job=(PathBuf,BTreeMap<String,usize>);
        static WRITER:OnceLock<mpsc::Sender<Job>>=OnceLock::new();
        let writer=WRITER.get_or_init(|| {
            let(tx,rx)=mpsc::channel::<Job>();
            std::thread::Builder::new().name("navigation-save".into()).spawn(move||{
                while let Ok(mut job)=rx.recv() {
                    while let Ok(next)=rx.try_recv(){job=next;}
                    let result=(||->std::io::Result<()> {
                        let bytes=serde_json::to_vec(&job.1).map_err(std::io::Error::other)?;
                        let path=job.0.join("navigation-memory.json");
                        let temporary=job.0.join("navigation-memory.tmp");
                        std::fs::write(&temporary,bytes)?;
                        std::fs::rename(temporary,path)
                    })();
                    if let Err(error)=result {tracing::warn!(%error,"could not save navigation memory");}
                }
            }).expect("could not start navigation settings writer");
            tx
        });
        let _=writer.send((base_dir.to_owned(),self.settings.navigation_memory.clone()));
    }

    pub(crate) fn restore_navigation_memory(&mut self,base_dir:&Path) {
        let path=base_dir.join("navigation-memory.json");
        // A normal settings save also includes navigation; prefer it when newer.
        let newer=std::fs::metadata(&path).and_then(|m|m.modified()).ok()
            .zip(std::fs::metadata(base_dir.join("config.toml")).and_then(|m|m.modified()).ok())
            .is_none_or(|(navigation,config)|navigation>config);
        if newer && let Ok(bytes)=std::fs::read(path)
            && let Ok(memory)=serde_json::from_slice::<BTreeMap<String,usize>>(&bytes) {
            self.settings.navigation_memory=memory;
        }
    }

    pub fn save(&self, base_dir: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let path = base_dir.join("config.toml");
        let tmp = base_dir.join("config.toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;

        #[cfg(not(feature = "lite"))]
        // keep active patch state with the game installation
        let game_root = Path::new(&self.settings.rl_path);
        #[cfg(not(feature = "lite"))]
        if game_root.is_dir() {
            let marker = serde_json::json!({
                "game_path": game_root.to_string_lossy(),
                "patcher": &self.patcher,
            });
            let _ = std::fs::write(
                game_root.join("patcher.json"),
                serde_json::to_vec_pretty(&marker).unwrap_or_default(),
            );
        }
        Ok(())
    }

    /// one-time import of the old python config.ini
    fn import_ini(path: &Path) -> Option<Config> {
        let ini = ini::Ini::load_from_file(path).ok()?;
        let mut cfg = Config::default();

        if let Some(win) = ini.section(Some("Window")) {
            if let Some(w) = win.get("width").and_then(|v| v.parse().ok()) {
                cfg.window.width = w;
            }
            if let Some(h) = win.get("height").and_then(|v| v.parse().ok()) {
                cfg.window.height = h;
            }
        }
        if let Some(settings) = ini.section(Some("Settings")) {
            if let Some(v) = settings.get("hotkey") {
                cfg.settings.hotkey = v.to_string();
            }
            if let Some(v) = settings.get("theme") {
                cfg.settings.theme = v.to_string();
            }
            if let Some(v) = settings.get("start_in_tray") {
                cfg.settings.start_in_tray = parse_ini_bool(v, false);
            }
            if let Some(v) = settings.get("rl_path") {
                cfg.settings.rl_path = v.to_string();
            }
            if let Some(v) = settings.get("statsapi_path") {
                cfg.settings.statsapi_path = v.to_string();
            }
        }
        if let Some(plugins) = ini.section(Some("Plugins")) {
            for (name, val) in plugins.iter() {
                cfg.plugins
                    .insert(name.to_string(), parse_ini_bool(val, false));
            }
        }
        Some(cfg)
    }
}

fn parse_ini_bool(v: &str, default: bool) -> bool {
    match v.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => true,
        "false" | "0" | "no" | "off" => false,
        _ => default,
    }
}

/// App root dir: `%AppData%\Hebnix`, or `HEBNIX_BASE_DIR` for dev runs.
pub fn base_dir() -> PathBuf {
    hebnix_sdk::utils::paths::base_dir()
}

#[cfg(test)]
mod rl_launch_tests {
    use super::*;

    /// Configs saved with the removed Heroic modes still load.
    #[test]
    fn old_heroic_modes_still_load() {
        let shortcut: RlLaunchCfg = toml::from_str(
            "mode = \"steam_shortcut_to_heroic\"\nsteam_id = \"123\"\nheroic_binary = \"/opt/Heroic/heroic\"\nheroic_app_name = \"Sugar\"\nheroic_runner = \"legendary\"\n",
        )
        .unwrap();
        assert_eq!(shortcut.mode, RlLaunchMode::SteamShortcut);
        assert_eq!(shortcut.steam_id, "123");
        let direct: RlLaunchCfg = toml::from_str("mode = \"heroic_direct\"\n").unwrap();
        assert_eq!(direct.mode, RlLaunchMode::Unconfigured);
        let saved = toml::to_string(&shortcut).unwrap();
        assert!(saved.contains("mode = \"steam_shortcut\""), "{saved}");
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;

    /// An unreadable config.toml is moved aside, not overwritten with defaults.
    #[test]
    fn invalid_config_is_kept() {
        let dir = std::env::temp_dir().join(format!("hebnix-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bad = "[rl_launch]\nmode = \"from_the_future\"\n";
        std::fs::write(dir.join("config.toml"), bad).unwrap();

        let _ = Config::load(&dir);

        let kept: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("config.toml.invalid-"))
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(kept[0].path()).unwrap(), bad);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
