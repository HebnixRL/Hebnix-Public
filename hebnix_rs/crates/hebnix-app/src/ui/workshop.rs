//! workshop maps tab: browse the hebnix.com catalog, download + swap maps
//! over the rocket labs placeholders.

use crate::i18n::{t, t_args, t_n};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Sender;
use eframe::egui;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::messages::AppMsg;
use crate::multiplayer_lan::{
    HostSession, LocalInfo, MAP_SYNC_PORT, MAX_MAP_BYTES, MapFileProvider, MapProvider, PeerLink,
    PeerOffer, SlotMap, TailcatSidecarHandle, TransferProgress, ensure_sidecar_rule,
    fetch_map_file, is_local_map_id, redact, valid_map_id,
};
mod archive;
mod banner_image;
mod background_changer;
mod menu_map;
mod local_import;
mod steam_download;
use archive::ArchiveBrowser;
use background_changer::BackgroundChangerState;
use menu_map::MenuMapState;
use steam_download::SteamDownloader;

/// common Workshop multiplayer problems (VPNs/proxies, firewalls) and fixes
const MULTIPLAYER_HELP_URL: &str = "https://github.com/xplodingeggo/hebnix-linux/blob/feature/tsnet-multiplayer/docs/workshop-multiplayer-help.md"; // TEST ONLY: point at HebnixRL/Hebnix-Public main once merged
use local_import::{
    ImportWizard, LocalMap, is_local_entry, load_local_maps, record_received_map, remove_local_map,
};

const MULTIHOME_CHECK_MAX_ATTEMPTS: u8 = 30;
const MULTIPLAYER_SETTINGS_FILE: &str = "multiplayer_settings.json";
const MULTIHOME_CHECK_INTERVAL: Duration = Duration::from_secs(2);

// api.hebnix.com flakes on connect now and then, so retry transport failures a
// few times (real http errors bail immediately).
fn get_retry(url: &str, timeout: Duration) -> Result<ureq::Response, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match ureq::get(url).timeout(timeout).call() {
            Ok(r) => return Ok(r),
            Err(e @ ureq::Error::Status(..)) => return Err(e.to_string()),
            Err(e) => {
                last = e.to_string();
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(600 * (attempt + 1)));
                }
            }
        }
    }
    Err(last)
}

// ureq's .timeout() caps the whole request including reading the body, so a big
// map on a slow link died with "timed out reading response" even while bytes
// were still coming in. downloads only time out on connect and on a stalled read.
fn get_download(url: &str) -> Result<ureq::Response, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(30))
            .build();
        match agent.get(url).call() {
            Ok(r) => return Ok(r),
            Err(e @ ureq::Error::Status(..)) => return Err(e.to_string()),
            Err(e) => {
                last = e.to_string();
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(600 * (attempt + 1)));
                }
            }
        }
    }
    Err(last)
}

fn rocket_league_executable(rl_path: &str) -> Result<PathBuf, String> {
    let root = Path::new(rl_path);
    let candidates = [
        root.join("TAGame")
            .join("Binaries")
            .join("Win64")
            .join("RocketLeague.exe"),
        root.join("Binaries").join("Win64").join("RocketLeague.exe"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "Could not find RocketLeague.exe in the configured game folder.".to_string())
}

/// the name this player shows up as in everyone else's player list: the
/// Rocket League name from its launch log, else a stable random tag
fn multiplayer_player_name() -> String {
    hebnix_sdk::log::parse_launch_log(None, false, "INT")
        .session
        .username
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| {
            let token = multiplayer_client_token();
            format!("hebnix-{}", &token[..token.len().min(8)])
        })
}

fn multiplayer_client_token() -> String {
    use rand::RngCore;

    let path = crate::config::base_dir()
        .join("state")
        .join("multiplayer_player_token.txt");
    if let Ok(token) = std::fs::read_to_string(&path) {
        let token = token.trim();
        if token.len() == 64 && token.chars().all(|character| character.is_ascii_hexdigit()) {
            return token.to_string();
        }
    }
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &token);
    token
}

/// the -multihome value Rocket League was last launched with, if any.
/// Workshop multiplayer no longer uses it, and a game still started with it
/// (by an older Hebnix's Epic launch option, or by hand) can't see the
/// sidecar's loopback proxies.
fn rocket_league_multihome_address() -> Option<String> {
    let path = dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("My Games")
        .join("Rocket League")
        .join("TAGame")
        .join("Logs")
        .join("Launch.log");
    let log = std::fs::read_to_string(path).ok()?;
    log.lines().take(300).find_map(|line| {
        let line = line.to_ascii_lowercase();
        let start = line.find("-multihome=")? + "-multihome=".len();
        let address: String = line[start..]
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect();
        (!address.is_empty()).then_some(address)
    })
}

pub const WORKSHOP_PLUGIN_ID: &str = "workshop_map_loader";
pub const WORKSHOP_MODS_DIR_NAME: &str = "mods";
pub const REMOTE_FILES_BASE: &str = "https://hebnix.com";
pub const API_ENDPOINT: &str = "https://api.hebnix.com/maps";
pub const DOWNLOAD_ENDPOINT_BASE: &str = "https://api.hebnix.com/download/map/";

pub const TARGET_MAPS: [(&str, &str); 4] = [
    ("Utopia Retro", "Labs_Utopia_P.upk"),
    ("Underpass", "Labs_Underpass_P.upk"),
    ("Roadblock", "Labs_Octagon_B2B_02_P.upk"),
    ("Hourglass", "Labs_PillarGlass_P.upk"),
];

fn target_filename(target: &str) -> Option<&'static str> {
    TARGET_MAPS
        .iter()
        .find(|(name, _)| *name == target)
        .map(|(_, file)| *file)
}

// Map manager (shared with worker threads)

#[derive(Clone)]
pub struct MapManager {
    pub cache_dir: PathBuf,
    pub runtime_dir: PathBuf,
    active_maps: Arc<Mutex<serde_json::Map<String, Value>>>,
}

impl MapManager {
    pub fn new(base_dir: &Path) -> Self {
        let cache_dir = base_dir
            .join("plugins")
            .join("cache")
            .join(WORKSHOP_PLUGIN_ID);
        let runtime_dir = base_dir
            .join("plugins")
            .join("runtime")
            .join(WORKSHOP_PLUGIN_ID);
        let _ = std::fs::create_dir_all(&cache_dir);
        let _ = std::fs::create_dir_all(&runtime_dir);

        let active_maps = std::fs::read_to_string(runtime_dir.join("active_maps.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();

        Self {
            cache_dir,
            runtime_dir,
            active_maps: Arc::new(Mutex::new(active_maps)),
        }
    }

    fn save_active_maps(&self) {
        let maps = self.active_maps.lock().unwrap();
        if let Ok(text) = serde_json::to_string(&*maps) {
            let _ = std::fs::write(self.runtime_dir.join("active_maps.json"), text);
        }
    }

    fn install_state_path(rl_path: &str) -> PathBuf {
        Path::new(rl_path)
            .join("TAGame")
            .join("CookedPCConsole")
            .join(WORKSHOP_MODS_DIR_NAME)
            .join("workshop_maps.json")
    }

    fn save_install_state(&self, rl_path: &str) {
        let path = Self::install_state_path(rl_path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&*self.active_maps.lock().unwrap()) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Adopt the active-map state saved beside the mods for the attached
    /// Rocket League installation (Steam and Epic are independent).
    pub fn reload_install_state(&self, rl_path: &str) {
        let mut maps: serde_json::Map<String, Value> =
            std::fs::read_to_string(Self::install_state_path(rl_path))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
        let mods_dir = Path::new(rl_path)
            .join("TAGame")
            .join("CookedPCConsole")
            .join(WORKSHOP_MODS_DIR_NAME);
        maps.retain(|target, _| {
            target_filename(target).is_some_and(|filename| mods_dir.join(filename).is_file())
        });
        *self.active_maps.lock().unwrap() = maps;
        self.save_active_maps();
        self.save_install_state(rl_path);
    }

    pub fn active_maps(&self) -> serde_json::Map<String, Value> {
        self.active_maps.lock().unwrap().clone()
    }

    pub fn is_cached(&self, map_id: &str) -> bool {
        !map_id.is_empty() && self.cache_dir.join(format!("{map_id}.upk")).exists()
    }

    pub fn delete_from_cache(&self, map_id: &str) -> bool {
        if map_id.is_empty() {
            return false;
        }
        let target = self.cache_dir.join(format!("{map_id}.upk"));
        if target.exists() {
            std::fs::remove_file(&target).is_ok()
        } else {
            true
        }
    }

    pub fn get_active_targets_for_map(&self, map_id: &str) -> Vec<String> {
        self.active_maps
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, data)| id_of(data) == map_id)
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn download_map_file(&self, map_id: &str, local_path: &Path) -> Result<(), String> {
        let url = format!("{DOWNLOAD_ENDPOINT_BASE}{map_id}");
        let zip_path = local_path.with_extension("zip");
        let temp_extract_dir = local_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("temp_{map_id}"));

        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let resp = get_download(&url)?;
        let copied = std::fs::File::create(&zip_path)
            .map_err(|e| e.to_string())
            .and_then(|mut out| {
                std::io::copy(&mut resp.into_reader(), &mut out).map_err(|e| e.to_string())
            });
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&zip_path);
            return Err(e);
        }

        let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        archive
            .extract(&temp_extract_dir)
            .map_err(|e| e.to_string())?;

        // Find the first .upk/.udk in the extracted tree.
        let extracted_map = find_map_file(&temp_extract_dir);
        let result = match extracted_map {
            Some(found) => std::fs::rename(&found, local_path)
                .or_else(|_| {
                    std::fs::copy(&found, local_path)
                        .map(|_| ())
                        .and_then(|_| std::fs::remove_file(&found))
                })
                .map_err(|e| e.to_string()),
            None => Err(t("download-map-file-no-valid-upk-or-udk-map").to_string()),
        };

        let _ = std::fs::remove_file(&zip_path);
        let _ = std::fs::remove_dir_all(&temp_extract_dir);
        result
    }

    pub fn install_map(
        &self,
        map_data: &Value,
        target_name: &str,
        rl_path: &str,
    ) -> Result<(), String> {
        let map_id = id_of(map_data);
        let cached_map_path = self.cache_dir.join(format!("{map_id}.upk"));

        if !self.is_cached(&map_id) {
            self.download_map_file(&map_id, &cached_map_path)?;
        }

        let cooked_pc_dir = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        let mods_dir = cooked_pc_dir.join(WORKSHOP_MODS_DIR_NAME);
        std::fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

        let filename =
            target_filename(target_name).ok_or_else(|| "Invalid target map.".to_string())?;
        if target_name == "Hourglass" {
            let _ = std::fs::remove_file(mods_dir.join("Labs_Hourglass_P.upk"));
        }
        std::fs::copy(&cached_map_path, mods_dir.join(filename)).map_err(|e| e.to_string())?;

        self.active_maps
            .lock()
            .unwrap()
            .insert(target_name.to_string(), map_data.clone());
        self.save_active_maps();
        self.save_install_state(rl_path);
        Ok(())
    }

    /// Rocket League only notices mod files that existed when it started.
    /// Pre-seeding lets a peer later install a map into an otherwise unused
    /// slot, but creates four vanilla-map copies and can prompt the user when
    /// Rocket League opens. It is therefore opt-in at multiplayer launch.
    pub fn seed_empty_slots(&self, rl_path: &str) {
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        let mods_dir = cooked.join(WORKSHOP_MODS_DIR_NAME);
        if std::fs::create_dir_all(&mods_dir).is_err() {
            return;
        }
        for (_, filename) in TARGET_MAPS {
            let (vanilla, modded) = (cooked.join(filename), mods_dir.join(filename));
            if !modded.exists() && vanilla.is_file() {
                let _ = std::fs::copy(&vanilla, &modded);
            }
        }
    }

    pub fn unload_active_map(&self, target_name: &str, rl_path: &str) -> Result<(), String> {
        let filename =
            target_filename(target_name).ok_or_else(|| "Invalid target map.".to_string())?;
        let target_file = Path::new(rl_path)
            .join("TAGame")
            .join("CookedPCConsole")
            .join(WORKSHOP_MODS_DIR_NAME)
            .join(filename);
        if target_file.exists() {
            std::fs::remove_file(&target_file).map_err(|e| format!("Failed to unload map: {e}"))?;
        }
        if target_name == "Hourglass" {
            let _ = std::fs::remove_file(
                Path::new(rl_path)
                    .join("TAGame")
                    .join("CookedPCConsole")
                    .join(WORKSHOP_MODS_DIR_NAME)
                    .join("Labs_Hourglass_P.upk"),
            );
        }
        self.active_maps.lock().unwrap().remove(target_name);
        self.save_active_maps();
        self.save_install_state(rl_path);
        Ok(())
    }
}

/// the player list grows a search box past this many players
const PLAYER_SEARCH_THRESHOLD: usize = 6;
/// names listed in a "players differ" tooltip before it says "and N more"
const MAX_TOOLTIP_NAMES: usize = 10;
const OK_COLOUR: egui::Color32 = egui::Color32::LIGHT_GREEN;
const WARN_COLOUR: egui::Color32 = egui::Color32::from_rgb(0xf3, 0x9c, 0x12);

/// a player's maps in one line: "Map", "Map + 2 more" or "no map replaced"
fn offer_map_summary(maps: &[SlotMap]) -> String {
    let mut names = maps
        .iter()
        .filter(|map| target_filename(&map.slot).is_some())
        .map(|map| map.name.as_str());
    match (names.next(), names.count()) {
        (None, _) => "no map replaced".to_string(),
        (Some(first), 0) => first.to_string(),
        (Some(first), more) => format!("{first} + {more} more"),
    }
}

fn matches_filter(label: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty() || label.to_lowercase().contains(&filter.to_lowercase())
}

/// true if any of the player's maps isn't sitting in the same slot on your side
fn player_differs(offer: &PeerOffer, mine: &serde_json::Map<String, Value>) -> bool {
    offer
        .maps
        .iter()
        .filter(|map| target_filename(&map.slot).is_some())
        .any(|map| mine.get(&map.slot).is_none_or(|have| id_of(have) != map.id))
}

/// what's off about one of your slots for one player, None if they match
fn slot_problem(offer: &PeerOffer, slot: &str, id: &str) -> Option<String> {
    let who = offer.label();
    match offer.maps.iter().find(|m| m.slot == slot) {
        Some(theirs) if theirs.id == id => None,
        Some(theirs) => Some(format!("{who} has {} here", theirs.name)),
        None => Some(match offer.maps.iter().find(|m| m.id == id) {
            Some(elsewhere) => format!("{who} put it in {} instead", elsewhere.slot),
            None => format!("{who} hasn't replaced it"),
        }),
    }
}

/// one of your replaced slots, with a short "does everyone match" status
fn render_my_slot(ui: &mut egui::Ui, slot: &str, data: &Value, offers: &[PeerOffer]) {
    let id = id_of(data);
    let name = str_of(data, "name", slot);
    let problems: Vec<String> = offers
        .iter()
        .filter_map(|offer| slot_problem(offer, slot, &id))
        .collect();
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("  {slot}: {name}"));
        if offers.is_empty() {
            return;
        }
        if problems.is_empty() {
            ui.colored_label(OK_COLOUR, t("my-slot-everyone-has-it"));
            return;
        }
        let mut tip = problems
            .iter()
            .take(MAX_TOOLTIP_NAMES)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if problems.len() > MAX_TOOLTIP_NAMES {
            tip.push_str(&format!(
                "\nand {} more",
                problems.len() - MAX_TOOLTIP_NAMES
            ));
        }
        ui.colored_label(
            WARN_COLOUR,
            t_args(
                "my-slot-problems-of-offers-players-differ",
                &[
                    ("problems", (problems.len()).to_string().into()),
                    ("offers", (offers.len()).to_string().into()),
                ],
            ),
        )
        .on_hover_text(tip);
    });
}

/// "Name — Map", the map part coloured by whether you match them
fn player_header(
    ui: &egui::Ui,
    offer: &PeerOffer,
    mine: &serde_json::Map<String, Value>,
) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let visuals = ui.visuals();
    let map_colour = if offer_map_summary(&offer.maps) == "no map replaced" {
        visuals.weak_text_color()
    } else if player_differs(offer, mine) {
        WARN_COLOUR
    } else {
        OK_COLOUR
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &offer.label(),
        0.0,
        egui::TextFormat::simple(font.clone(), visuals.strong_text_color()),
    );
    job.append(
        " — ",
        0.0,
        egui::TextFormat::simple(font.clone(), visuals.weak_text_color()),
    );
    job.append(
        &offer_map_summary(&offer.maps),
        0.0,
        egui::TextFormat::simple(font, map_colour),
    );
    job
}

/// a player's maps once their row is expanded, with install options
fn render_player_maps(
    ui: &mut egui::Ui,
    offer: &PeerOffer,
    mine: &serde_json::Map<String, Value>,
    busy: bool,
    install: &mut Option<(SlotMap, IpAddr, bool)>,
) {
    let maps: Vec<&SlotMap> = offer
        .maps
        .iter()
        .filter(|map| target_filename(&map.slot).is_some())
        .collect();
    if maps.is_empty() {
        ui.small(t("player-maps-hasn-t-replaced-any-map"));
    }
    for map in maps {
        let have = mine.get(&map.slot);
        let same = have.is_some_and(|m| id_of(m) == map.id);
        let elsewhere = mine
            .iter()
            .find(|(slot, m)| **slot != map.slot && id_of(m) == map.id)
            .map(|(slot, _)| slot.clone());
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{}: {}", map.slot, map.name));
            if map.local {
                ui.small(t("player-maps-imported-not-on-the-workshop"));
            }
            if same {
                ui.colored_label(OK_COLOUR, t("player-maps-you-have-it"));
                return;
            }
            let note = match (&elsewhere, have) {
                (Some(slot), _) => {
                    format!("you put this map in {slot}, replace {} instead", map.slot)
                }
                (None, Some(m)) => format!(
                    "you have {} in {}",
                    str_of(m, "name", "another map"),
                    map.slot
                ),
                (None, None) => format!("you haven't replaced {}", map.slot),
            };
            ui.colored_label(WARN_COLOUR, note);
            let label = format!("Install to {}", map.slot);
            if map.local {
                // imported maps aren't on the cdn, only the peer has them
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    *install = Some((map.clone(), offer.ip, true));
                }
            } else {
                // cdn maps can come from either, so the peer is the way
                // around a slow or unreachable cdn
                ui.add_enabled_ui(!busy, |ui| {
                    ui.menu_button(format!("{label} v"), |ui| {
                        if ui.button(t("player-maps-from-the-cdn")).clicked() {
                            *install = Some((map.clone(), offer.ip, false));
                            ui.close();
                        }
                        if ui.button(t("player-maps-from-this-player")).clicked() {
                            *install = Some((map.clone(), offer.ip, true));
                            ui.close();
                        }
                    });
                });
            }
        });
    }
}

/// everyone else who's on, with how they're connected
fn render_players(ui: &mut egui::Ui, players: &[PeerLink]) {
    if players.is_empty() {
        ui.small(t("multiplayer-no-other-players-online"));
        return;
    }
    egui::CollapsingHeader::new(t_n("multiplayer-players-online", players.len() as i64))
        .id_salt("workshop_players_online")
        .default_open(players.len() <= 6)
        .show(ui, |ui| {
            for player in players {
                let link = if player.path == "unreachable" {
                    t("multiplayer-link-unreachable")
                } else {
                    let route = if player.path == "direct" {
                        t("multiplayer-route-direct")
                    } else {
                        t("multiplayer-route-relay")
                    };
                    let ping = player.latency_ms.map_or(0, |ms| ms.round() as i64);
                    t_args(
                        "multiplayer-link",
                        &[("route", route.into()), ("ping", ping.into())],
                    )
                };
                ui.small(format!("{}: {link}", player.name));
            }
        });
}

fn multiplayer_help_button(ui: &mut egui::Ui) {
    if ui
        .button(t("multiplayer-help-button-using-a-vpn-proxy-or-having"))
        .on_hover_text(t(
            "multiplayer-help-button-opens-the-workshop-multiplayer-help-page",
        ))
        .clicked()
    {
        ui.ctx()
            .open_url(egui::OpenUrl::new_tab(MULTIPLAYER_HELP_URL));
    }
}

fn find_map_file(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("upk") || ext.eq_ignore_ascii_case("udk") {
                return Some(path);
            }
        }
    }
    dirs.iter().find_map(|d| find_map_file(d))
}

pub fn id_of(map_data: &Value) -> String {
    match map_data.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => "0".to_string(),
    }
}

fn str_of<'a>(map_data: &'a Value, key: &str, default: &'a str) -> &'a str {
    map_data
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
}

/// remote url + the path to cache it under
pub fn banner_url_and_cache_rel(banner_path: &str) -> (String, String) {
    let normalized = banner_path.replace('\\', "/");
    let url = format!("{REMOTE_FILES_BASE}{normalized}");
    (url, normalized.trim_start_matches('/').to_string())
}

/// fetch banner_path cached under cache_dir
pub fn spawn_image_fetch(
    key: String,
    cache_dir: PathBuf,
    tx: Sender<AppMsg>,
    ctx: eframe::egui::Context,
    done: impl FnOnce(String, Vec<u8>) -> AppMsg + Send + 'static,
) {
    std::thread::spawn(move || {
        let (url, rel) = banner_url_and_cache_rel(&key);
        let local_path = cache_dir.join(rel);
        let bytes: Option<Vec<u8>> = if local_path.exists() {
            std::fs::read(&local_path).ok()
        } else {
            let result = get_retry(&url, Duration::from_secs(10))
                .ok()
                .and_then(|resp| {
                    let mut buf = Vec::new();
                    resp.into_reader().read_to_end(&mut buf).ok()?;
                    Some(buf)
                });
            if let Some(buf) = &result {
                if let Some(parent) = local_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&local_path, buf);
            }
            result
        };
        let bytes = bytes.and_then(|bytes| match banner_image::prepare(bytes) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                tracing::warn!(%key, %error, "Failed to decode banner image");
                None
            }
        });
        let _ = tx.send(done(key, bytes.unwrap_or_default()));
        ctx.request_repaint();
    });
}

// Tab state

pub enum ImageState {
    Loading,
    Ready(Arc<[u8]>),
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkshopView {
    Browse,
    Import,
    BackgroundChanger,
    MenuMap,
    Multiplayer,
}

struct MultiplayerState {
    wizard_started: bool,
    host_name: String,
    identity_update_in_flight: bool,
    identity_updated: bool,
    /// one relay session regardless of whether this peer ends up hosting or
    /// joining inside Rocket League's own UI - see hosting.rs
    relay: Option<HostSession>,
    status: String,
    setup_progress: Option<String>,
    saved_host: Option<SavedHost>,
    saved_room: Option<crate::multiplayer_lan::Room>,
    saved_host_checked: bool,
    saved_host_checking: bool,
    detected_target: Option<String>,
    detected_map: Option<String>,
    /// set while a peer's map is being downloaded + installed in the background
    map_install_busy: Arc<AtomicBool>,
    map_install_result: Arc<Mutex<String>>,
    /// progress of a map being downloaded from a peer
    map_transfer: Arc<TransferProgress>,
    /// maps received from a peer, waiting to be added to the catalog
    received_maps: Arc<Mutex<Vec<LocalMap>>>,
    /// a peer install waiting on the "files from other players" warning
    pending_peer_install: Option<(SlotMap, IpAddr, bool)>,
    /// the warning was accepted once this session, don't ask again
    peer_warning_accepted: bool,
    /// Back was pressed while the tailnet was still starting; the finished
    /// connection gets dropped instead of kept
    abandon_tailnet: bool,
    /// search text for the player list
    player_filter: String,
    /// route everything through relay servers so other players never see
    /// this machine's public address (saved in multiplayer_settings.json)
    relay_only: bool,
    /// Pre-create vanilla files in otherwise empty Workshop map slots before
    /// launching Rocket League. This is only needed when a peer might send a
    /// map for a slot that is not already present in mods.
    seed_other_map_slots: bool,

    // multiplayer sidecar state
    sidecar: Option<Arc<TailcatSidecarHandle>>,
    tailnet_requested: bool,
    /// the sidecar is up and announced to the directory
    tunnel_ready: bool,
    /// every other player and their ping, from the sidecar's latest status
    players: Vec<PeerLink>,

    rl_open: bool,
    /// Rocket League is running, and its launch log shows no -multihome
    /// (which would hide the sidecar's loopback proxies from it)
    launch_ready: bool,
    multihome_check_attempts: u8,
    multihome_check_in_flight: bool,
    /// true while Hebnix itself is closing/relaunching Rocket League, so the
    /// RL-monitor's "closed" transition doesn't arm the crash grace window
    /// for a restart Hebnix initiated on purpose
    launching_rocket_league: bool,

    /// set when Rocket League closes with a session still active; if it
    /// doesn't come back before this deadline, the session is torn down for
    /// real (see CRASH_GRACE_WINDOW)
    shutdown_deadline: Option<std::time::Instant>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct SavedHost {
    pin: String,
    host_secret: String,
}

impl Default for MultiplayerState {
    fn default() -> Self {
        Self {
            wizard_started: false,
            host_name: "Hebnix Workshop".to_string(),
            identity_update_in_flight: false,
            identity_updated: false,
            relay: None,
            status: t("default-connect-then-host-or-join-inside").to_string(),
            setup_progress: None,
            saved_host: None,
            saved_room: None,
            saved_host_checked: false,
            saved_host_checking: false,
            detected_target: None,
            detected_map: None,
            map_install_busy: Arc::new(AtomicBool::new(false)),
            map_install_result: Arc::new(Mutex::new(String::new())),
            map_transfer: Arc::new(TransferProgress::default()),
            received_maps: Arc::new(Mutex::new(Vec::new())),
            pending_peer_install: None,
            peer_warning_accepted: false,
            player_filter: String::new(),
            abandon_tailnet: false,
            relay_only: false,
            seed_other_map_slots: false,
            sidecar: None,
            tailnet_requested: false,
            tunnel_ready: false,
            players: Vec::new(),
            rl_open: false,
            launch_ready: false,
            multihome_check_attempts: 0,
            multihome_check_in_flight: false,
            launching_rocket_league: false,
            shutdown_deadline: None,
        }
    }
}

pub struct WorkshopState {
    pub manager: MapManager,
    pub catalog: Vec<Value>,
    pub valid: Vec<usize>,
    pub page: usize,
    pub page_size: usize,
    pub search: String,
    pub view_downloaded: bool,
    pub target: String,
    pub images: HashMap<String, ImageState>,
    pub busy: HashSet<String>,
    pub catalog_status: String,
    pub fetched: bool,
    pub confirm_delete: Option<Value>,
    theme_selected_map: Option<String>,
    view: WorkshopView,
    background_changer: BackgroundChangerState,
    menu_map: MenuMapState,
    import_wizard: ImportWizard,
    steam_downloader: SteamDownloader,
    archive_browser: ArchiveBrowser,
    multiplayer: MultiplayerState,
    rl_launch: crate::config::RlLaunchCfg,
    /// the "P2P File Sharing" system setting. read by the map sync server
    /// each time a player asks for a file, so it applies straight away.
    share_files: Arc<AtomicBool>,
}

impl WorkshopState {
    pub fn new(base_dir: &Path) -> Self {
        let manager = MapManager::new(base_dir);
        let saved_host = std::fs::read(manager.runtime_dir.join("multiplayer_host.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let mut multiplayer = MultiplayerState::default();
        multiplayer.saved_host = saved_host;
        multiplayer.relay_only = std::fs::read(manager.runtime_dir.join(MULTIPLAYER_SETTINGS_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|settings| settings.get("relay_only").and_then(Value::as_bool))
            .unwrap_or(false);
        Self {
            manager,
            catalog: Vec::new(),
            valid: Vec::new(),
            page: 0,
            page_size: 12,
            search: String::new(),
            view_downloaded: false,
            target: TARGET_MAPS[0].0.to_string(),
            images: HashMap::new(),
            busy: HashSet::new(),
            catalog_status: t("catalog-loading-catalog").to_string(),
            fetched: false,
            confirm_delete: None,
            theme_selected_map: None,
            view: WorkshopView::Browse,
            background_changer: BackgroundChangerState::default(),
            menu_map: MenuMapState::default(),
            import_wizard: ImportWizard::default(),
            steam_downloader: SteamDownloader::default(),
            archive_browser: ArchiveBrowser::default(),
            multiplayer,
            rl_launch: crate::config::RlLaunchCfg::default(),
            share_files: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn finish_background_changer(&mut self, result: Result<String, String>) -> String {
        self.background_changer.finish(result)
    }

    pub fn finish_menu_map(&mut self, result: Result<String, String>) -> String {
        self.menu_map.finish(result)
    }

    pub fn update_background_changer_progress(&mut self, message: String) -> String {
        self.background_changer.progress(message)
    }

    /// installs a freshly fetched cdn catalog, keeping the maps the player
    /// imported themselves
    pub fn set_catalog(&mut self, items: Vec<Value>) {
        self.catalog = items;
        self.merge_local_maps();
        self.execute_search(true);
    }

    /// adds imported maps missing from the catalog (also what keeps them
    /// visible when the cdn can't be reached)
    pub fn merge_local_maps(&mut self) {
        let known: HashSet<String> = self.catalog.iter().map(id_of).collect();
        let missing: Vec<Value> = load_local_maps(&self.manager.runtime_dir)
            .iter()
            .filter(|map| !known.contains(&map.id))
            .map(LocalMap::to_catalog_entry)
            .collect();
        if !missing.is_empty() {
            self.catalog.extend(missing);
            self.execute_search(false);
        }
    }

    fn add_local_map(&mut self, map: LocalMap) {
        self.catalog.retain(|entry| id_of(entry) != map.id);
        self.catalog.push(map.to_catalog_entry());
        self.execute_search(false);
    }

    /// downloaded and imported maps, for the menu map picker. one directory
    /// listing instead of a stat per catalog entry.
    fn cached_maps(&self) -> Vec<menu_map::CustomMap> {
        let cached: HashSet<String> = std::fs::read_dir(&self.manager.cache_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        let mut maps: Vec<_> = self
            .catalog
            .iter()
            .filter_map(|map| {
                let id = id_of(map);
                let file = format!("{id}.upk");
                cached.contains(&file).then(|| menu_map::CustomMap {
                    name: str_of(map, "name", &id).to_string(),
                    path: self.manager.cache_dir.join(file),
                    id,
                })
            })
            .collect();
        maps.sort_by_key(|map| map.name.to_lowercase());
        maps
    }

    pub fn total_pages(&self) -> usize {
        self.valid.len().div_ceil(self.page_size).max(1)
    }

    /// kick off the async catalog fetch (once at startup)
    pub fn fetch_catalog(&mut self, tx: Sender<AppMsg>, ctx: eframe::egui::Context) {
        if self.fetched {
            return;
        }
        self.fetched = true;
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<Value>, String> {
                let resp = get_retry(API_ENDPOINT, Duration::from_secs(10))?;
                let data: Value = resp.into_json().map_err(|e| e.to_string())?;
                if let Value::Array(items) = data {
                    return Ok(items);
                }
                Ok(data
                    .get("items")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default())
            })();
            let _ = tx.send(AppMsg::WorkshopCatalog(result));
            ctx.request_repaint();
        });
    }

    pub fn execute_search(&mut self, reset_page: bool) {
        let query = self.search.to_lowercase().trim().to_string();
        self.valid.clear();
        for (i, m) in self.catalog.iter().enumerate() {
            let name = str_of(m, "name", "").to_lowercase();
            let author = str_of(m, "author", "").to_lowercase();
            let matches_query = name.contains(&query) || author.contains(&query);
            let matches_dl = !self.view_downloaded || self.manager.is_cached(&id_of(m));
            if matches_query && matches_dl {
                self.valid.push(i);
            }
        }
        if reset_page {
            self.page = 0;
        } else {
            self.page = self.page.min(self.total_pages() - 1);
        }
    }

    fn ensure_image(
        &mut self,
        banner_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &eframe::egui::Context,
    ) {
        if banner_path.is_empty() || self.images.contains_key(banner_path) {
            return;
        }
        self.images
            .insert(banner_path.to_string(), ImageState::Loading);

        spawn_image_fetch(
            banner_path.to_string(),
            self.manager.cache_dir.clone(),
            tx.clone(),
            ctx.clone(),
            |key, bytes| AppMsg::WorkshopImage { key, bytes },
        );
    }

    /// render the tab. rl_path and rl_launch come from the app config.
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        rl_path: &str,
        rl_launch: &crate::config::RlLaunchCfg,
        tx: &Sender<AppMsg>,
    ) {
        self.rl_launch = rl_launch.clone();
        let ctx = ui.ctx().clone();

        if crate::theme::presentation(&ctx).is_some_and(|p| p.layout.show_page_heading) {
            ui.heading(t("tab-workshop"));
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.view,
                WorkshopView::Browse,
                t("render-browse-maps"),
            );
            ui.selectable_value(&mut self.view, WorkshopView::Import, t("render-import-map"));
            ui.selectable_value(
                &mut self.view,
                WorkshopView::BackgroundChanger,
                t("render-background-changer"),
            );
            ui.selectable_value(&mut self.view, WorkshopView::MenuMap, t("menu-map-title"));
            ui.selectable_value(
                &mut self.view,
                WorkshopView::Multiplayer,
                t("render-multiplayer"),
            );
        });
        ui.separator();
        if self.view == WorkshopView::Import {
            let imported =
                self.import_wizard
                    .render(ui, &self.manager.cache_dir, &self.manager.runtime_dir);
            if let Some(map) = imported {
                self.add_local_map(map);
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            let downloaded = self.steam_downloader.render(
                ui,
                &self.manager.cache_dir,
                &self.manager.runtime_dir,
            );
            if let Some(map) = downloaded {
                self.add_local_map(map);
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            let from_archive =
                self.archive_browser
                    .render(ui, &self.manager.cache_dir, &self.manager.runtime_dir);
            if let Some(map) = from_archive {
                self.add_local_map(map);
            }
            return;
        }
        if self.view == WorkshopView::Multiplayer {
            self.render_multiplayer(ui, rl_path, tx, &ctx);
            return;
        }
        if self.view == WorkshopView::BackgroundChanger {
            self.background_changer.render(ui, rl_path, tx);
            return;
        }
        if self.view == WorkshopView::MenuMap {
            let custom = self.cached_maps();
            self.menu_map.render(ui, rl_path, &custom, tx);
            return;
        }

        // The shared browser controls leave the map card renderers intact.
        if crate::ui::browser::search(ui, &mut self.search, t("ball-name-or-author")) {
            self.execute_search(true);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut self.view_downloaded, t("render-view-downloaded"))
                .changed()
            {
                self.execute_search(true);
            }
            ui.menu_button("?", |ui| {
                ui.set_max_width(280.0);
                ui.label(t("render-can-t-find-the-map-you"));
                ui.label(t("render-no-hubcap-api-key-the-import"));
            })
            .response
            .on_hover_text(t("render-can-t-find-a-map"));

            ui.strong(t("render-map-to-replace"));
            let mut target_changed = false;
            egui::ComboBox::from_id_salt("target_map")
                .selected_text(self.target.clone())
                .show_ui(ui, |ui| {
                    for (name, _) in TARGET_MAPS {
                        if ui
                            .selectable_value(&mut self.target, name.to_string(), name)
                            .changed()
                        {
                            target_changed = true;
                        }
                    }
                });
            if target_changed {
                self.execute_search(false);
            }

            let restore_enabled = self.manager.active_maps().contains_key(&self.target);
            if ui
                .add_enabled(
                    restore_enabled,
                    egui::Button::new(t("ball-restore-original"))
                        .fill(egui::Color32::from_rgb(0xc0, 0x39, 0x2b)),
                )
                .clicked()
            {
                match self.manager.unload_active_map(&self.target, rl_path) {
                    Ok(()) => {
                        let _ = tx.send(AppMsg::Log(
                            "[Workshop] Original map restored successfully.".to_string(),
                        ));
                        self.execute_search(false);
                    }
                    Err(e) => {
                        let _ = tx.send(AppMsg::Log(format!("[Workshop] {e}")));
                    }
                }
            }
        });
        ui.add_space(4.0);
        let presentation = crate::theme::presentation(&ctx);
        let column_count = self.browse_columns(ui, presentation.as_deref());
        self.page_size = column_count * crate::ui::browser::ROWS;
        crate::ui::browser::pagination(ui, &mut self.page, self.valid.len(), self.page_size);

        // Card grid
        let start = self.page * self.page_size;
        let indices: Vec<usize> = self
            .valid
            .iter()
            .skip(start)
            .take(self.page_size)
            .copied()
            .collect();

        // Pre-fetch images for the visible page.
        for &i in &indices {
            let banner = str_of(&self.catalog[i], "banner_path", "").to_string();
            self.ensure_image(&banner, tx, &ctx);
        }

        let mut action: Option<(usize, CardAction)> = None;

        if let Some(p) = crate::theme::presentation(&ctx) {
            action = self.render_themed_library(ui, &indices, &p);
        } else {
            egui::ScrollArea::vertical()
                .id_salt("workshop_grid")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let grid_width = ui.available_width();
                    ui.set_width(grid_width);
                    for row in indices.chunks(column_count) {
                        ui.columns(column_count, |cols| {
                            for (col_idx, &map_idx) in row.iter().enumerate() {
                                let col = &mut cols[col_idx];
                                // A card must use its column's width. Otherwise a long
                                // button can grow this row, shifting later rows right.
                                let card_width = col.available_width();
                                col.set_width(card_width);
                                if let Some(act) = self.render_card(col, map_idx) {
                                    action = Some((map_idx, act));
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                    if indices.is_empty() {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(if self.catalog.is_empty() {
                                self.catalog_status.clone()
                            } else {
                                t("handle-messages-no-maps-found").to_string()
                            });
                        });
                    }
                });
        }

        if let Some((map_idx, act)) = action {
            self.handle_action(map_idx, act, rl_path, tx, &ctx);
        }

        // Delete-from-cache confirmation modal.
        if let Some(map_data) = self.confirm_delete.clone() {
            let name = str_of(&map_data, "name", "this map").to_string();
            let mut close = false;
            egui::Window::new(t("render-offboard-map")).id(egui::Id::new("render-offboard-map"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    if is_local_entry(&map_data) {
                        ui.label(t_args("render-remove-the-imported-map-name-its", &[("name", name.to_string().into())]));
                    } else {
                        ui.label(t_args("render-are-you-sure-you-want-to-2", &[("name", name.to_string().into())]));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(t("plugin-delete-prompt-yes")).clicked() {
                            let id = id_of(&map_data);
                            let ok = if is_local_entry(&map_data) {
                                let removed = remove_local_map(
                                    &self.manager.cache_dir,
                                    &self.manager.runtime_dir,
                                    &id,
                                )
                                .is_ok();
                                if removed {
                                    self.catalog.retain(|entry| id_of(entry) != id);
                                }
                                removed
                            } else {
                                self.manager.delete_from_cache(&id)
                            };
                            if !ok {
                                let _ = tx.send(AppMsg::Log(
                                    "[Workshop] Failed to delete file. Ensure the game is closed or the file isn't in use.".to_string(),
                                ));
                            }
                            self.execute_search(false);
                            close = true;
                        }
                        if ui.button(t("plugin-delete-prompt-no")).clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                self.confirm_delete = None;
            }
        }
    }

    /// One flow, not two: Rocket League's own UI is where someone picks
    /// Host or Join, not Hebnix's. Every peer runs the identical relay
    /// (confirmed by packet capture that joining broadcasts too, not just
    /// hosting - see hosting.rs), so there's nothing left for Hebnix to
    /// ask about up front. Connect, start Rocket League, the relay starts
    /// itself the moment the game's actually up - no separate button.
    fn render_multiplayer(
        &mut self,
        ui: &mut egui::Ui,
        rl_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &eframe::egui::Context,
    ) {
        if !self.multiplayer.wizard_started {
            ui.add_space(56.0);
            ui.vertical_centered(|ui| {
                crate::theme::page_heading(ui, t("multiplayer-workshop-multiplayer"));
                ui.label(t("multiplayer-connect-intro"));
                ui.add_space(12.0);
                if ui
                    .checkbox(
                        &mut self.multiplayer.relay_only,
                        t("multiplayer-hide-my-ip-from-other-players"),
                    )
                    .on_hover_text(t("multiplayer-relay-only-hover"))
                    .changed()
                {
                    self.save_multiplayer_settings();
                }
                ui.add_space(12.0);
                if ui
                    .add_sized([160.0, 38.0], egui::Button::new(t("multiplayer-connect")))
                    .clicked()
                {
                    self.multiplayer.wizard_started = true;
                    self.start_tailnet(tx, ctx);
                }
                ui.add_space(10.0);
                multiplayer_help_button(ui);
            });
            return;
        }
        let mut start_relay = false;
        let mut stop = false;
        let mut launch = false;
        let mut close_game = false;
        let mut install_peer_map: Option<(SlotMap, IpAddr, bool)> = None;
        let mut block_peer: Option<(IpAddr, String)> = None;
        let mut unblock_peer: Option<IpAddr> = None;
        let setup_in_progress = self.multiplayer.setup_progress.is_some();
        let tunnel_ready = self.multiplayer.tunnel_ready;
        let mut back = false;
        ui.horizontal(|ui| {
            if self.multiplayer.relay.is_none() && ui.button(t("multiplayer-back")).clicked() {
                back = true;
            }
            ui.strong(t("multiplayer-workshop-multiplayer"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                multiplayer_help_button(ui);
            });
        });
        if back {
            self.leave_multiplayer_setup();
            return;
        }
        ui.group(|ui| {
            ui.strong(t("multiplayer-setup"));
            if !tunnel_ready {
                ui.label(t("multiplayer-connecting-to-the-private-workshop-netwo"));
                return;
            }
            render_players(ui, &self.multiplayer.players);
            if !self.multiplayer.rl_open {
                ui.label(t("multiplayer-start-rocket-league-normally"));
                let response = ui.checkbox(
                    &mut self.multiplayer.seed_other_map_slots,
                    "Seed other map slots before launch",
                );
                response.on_hover_text("You'll get 4 prompts when opening Rocket League");
                if ui
                    .add_enabled(
                        !setup_in_progress,
                        egui::Button::new(t("action-start-rocket-league")),
                    )
                    .clicked()
                {
                    launch = true;
                }
            } else if !self.multiplayer.launch_ready {
                if self.waiting_for_multihome_check() {
                    ui.label(t("multiplayer-step-1-waiting-for-rocket-league"));
                    ui.small(format!("{} ", t("multiplayer-checking-launch-command")));
                } else {
                    ui.label(t("multiplayer-rocket-league-still-uses-multihome"));
                    if ui.button(t("action-close-rocket-league")).clicked() {
                        close_game = true;
                    }
                }
            } else if self.multiplayer.relay.is_none() {
                ui.label(t("multiplayer-starting-the-workshop-relay"));
                if !setup_in_progress {
                    start_relay = true;
                }
            } else {
                ui.label(t("multiplayer-ready-host-or-join-from-rocket"));
                ui.small(t("multiplayer-set-a-match-password"));
                if let Some(name) = &self.multiplayer.detected_map {
                    ui.small(t_args(
                        "multiplayer-detected-lan-match-on-map-name",
                        &[("name", name.to_string().into())],
                    ));
                }
            }
        });
        let received: Vec<LocalMap> = self
            .multiplayer
            .received_maps
            .lock()
            .map(|mut inbox| inbox.drain(..).collect())
            .unwrap_or_default();
        for map in received {
            self.add_local_map(map);
        }
        if self.multiplayer.relay.is_some() {
            self.render_maps_in_use(ui, &mut install_peer_map, &mut block_peer);
        }
        if let Some(session) = &self.multiplayer.relay {
            let blocked = session.blocked();
            if !blocked.is_empty() {
                ui.group(|ui| {
                    egui::CollapsingHeader::new(t_args(
                        "multiplayer-blocked-players-blocked",
                        &[("blocked", (blocked.len()).to_string().into())],
                    ))
                    .id_salt("workshop_blocked_players")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.small(t("multiplayer-blocks-last-until-you-disconnect"));
                        for (ip, label) in &blocked {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                if ui.small_button(t("multiplayer-unblock")).clicked() {
                                    unblock_peer = Some(*ip);
                                }
                            });
                        }
                    });
                });
            }
        }
        if self.multiplayer.relay.is_some() {
            ui.group(|ui| {
                ui.strong(t("multiplayer-connected"));
                if ui.button(t("multiplayer-disconnect")).clicked() {
                    stop = true;
                }
            });
        }
        ui.add_space(10.0);
        ui.label(&self.multiplayer.status);
        if let Some(progress) = &self.multiplayer.setup_progress {
            ui.add(egui::ProgressBar::new(0.5).animate(true).text(progress));
        }

        if launch {
            self.launch_multiplayer(rl_path, tx, ctx);
        }

        if let Some(session) = &self.multiplayer.relay {
            if let Some((ip, label)) = block_peer {
                session.block(ip, label);
            }
            if let Some(ip) = unblock_peer {
                session.unblock(ip);
            }
        }

        if let Some(request) = install_peer_map {
            if request.2 && !self.multiplayer.peer_warning_accepted {
                self.multiplayer.pending_peer_install = Some(request);
            } else {
                let (map, peer, from_peer) = request;
                self.install_peer_map(map, peer, from_peer, rl_path, ctx);
            }
        }
        self.render_peer_install_warning(rl_path, ctx);

        if close_game {
            self.multiplayer.status =
                t("multiplayer-closing-rocket-league-start-it-again").to_string();
            std::thread::spawn(|| {
                let _ = crate::winutil::kill_rocket_league();
            });
        }

        if stop {
            self.disconnect_multiplayer(tx);
        }
        if start_relay {
            self.start_relay(tx, ctx);
        }
    }

    /// Back before the session is running: stop the sidecar and remove the
    /// firewall rules instead of leaving them until Hebnix closes.
    fn leave_multiplayer_setup(&mut self) {
        self.multiplayer.wizard_started = false;
        self.multiplayer.pending_peer_install = None;
        self.multiplayer.setup_progress = None;
        self.multiplayer.tunnel_ready = false;
        self.multiplayer.players.clear();
        self.multiplayer.launch_ready = false;
        self.multiplayer.launching_rocket_league = false;
        self.multiplayer.multihome_check_attempts = 0;
        self.multiplayer.multihome_check_in_flight = false;
        self.multiplayer.status = t("default-connect-then-host-or-join-inside").to_string();
        let sidecar = self.multiplayer.sidecar.take();
        if sidecar.is_none() && self.multiplayer.tailnet_requested {
            // still starting up in the background, so there's nothing to drop
            // yet: throw it away the moment it reports in (Connect again
            // before then just keeps it, see start_tailnet)
            self.multiplayer.abandon_tailnet = true;
        } else {
            self.multiplayer.tailnet_requested = false;
        }
        // dropping the handle waits for the sidecar to exit, and the cleanup
        // runs external commands, so none of it on the ui thread
        std::thread::Builder::new()
            .name("workshop-leave".into())
            .spawn(move || {
                drop(sidecar);
                let _ = crate::multiplayer_lan::cleanup_system_state();
            })
            .ok();
    }

    fn disconnect_multiplayer(&mut self, tx: &Sender<AppMsg>) {
        let was_connected = self.multiplayer.tunnel_ready;
        let cleanup_error = self
            .multiplayer
            .relay
            .take()
            .and_then(|mut session| session.stop().err());
        // dropping the last handle stops the sidecar
        self.multiplayer.sidecar = None;
        let _ = crate::multiplayer_lan::cleanup_system_state();
        self.multiplayer.tailnet_requested = false;
        self.multiplayer.wizard_started = false;
        self.multiplayer.tunnel_ready = false;
        self.multiplayer.players.clear();
        self.multiplayer.launch_ready = false;
        self.multiplayer.multihome_check_attempts = 0;
        self.multiplayer.multihome_check_in_flight = false;
        self.multiplayer.shutdown_deadline = None;
        self.multiplayer.setup_progress = None;
        self.multiplayer.status = match cleanup_error {
            Some(error) => format!(
                "Disconnected, but cleanup failed: {}",
                redact(&error.to_string())
            ),
            None => t("multiplayer-disconnected").to_string(),
        };
        if was_connected {
            let _ = tx.send(AppMsg::Log("[Multiplayer] Disconnected".to_string()));
        }
    }

    /// Starts the multiplayer sidecar, which connects to everyone else who's
    /// on. This happens before Rocket League is touched at all; RL can be
    /// open already, since nothing about its launch changes.
    fn start_tailnet(&mut self, tx: &Sender<AppMsg>, ctx: &eframe::egui::Context) {
        if self.multiplayer.tailnet_requested {
            // came back before an abandoned startup finished: keep it
            self.multiplayer.abandon_tailnet = false;
            return;
        }
        self.multiplayer.tailnet_requested = true;
        self.multiplayer.status =
            t("start-tailnet-setting-up-the-private-workshop-network").to_string();
        let relay_only = self.multiplayer.relay_only;
        let tx = tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Arc<TailcatSidecarHandle>, String> {
                let base_dir = crate::config::base_dir();
                crate::multiplayer_assets::ensure_present(&base_dir).map_err(|error| {
                    format!("could not extract multiplayer components: {error}")
                })?;
                // an older Hebnix may have left -multihome in Epic's launch
                // options, which would hide the sidecar from Rocket League
                let _ = crate::winutil::clear_rocket_league_multihome();
                let sidecar = base_dir.join("multiplayer-lan").join("hebnix-tc.exe");
                if crate::spoofer::is_admin() {
                    // without admin Windows asks once instead; relays work
                    // either way
                    if let Err(error) = ensure_sidecar_rule(&sidecar) {
                        let _ = tx.send(AppMsg::Log(format!(
                            "[Multiplayer] Could not add the firewall rule: {error}"
                        )));
                    }
                }
                TailcatSidecarHandle::spawn(&multiplayer_player_name(), relay_only, tx.clone())
                    .map(Arc::new)
            })();
            let _ = tx.send(AppMsg::WorkshopTailnetStarted { result });
            repaint.request_repaint();
        });
    }

    /// turns serving map files to other players on or off (system setting)
    pub fn set_share_files(&self, enabled: bool) {
        self.share_files.store(enabled, Ordering::Relaxed);
    }

    pub fn finish_tailnet_started(
        &mut self,
        result: Result<Arc<TailcatSidecarHandle>, String>,
        tx: &Sender<AppMsg>,
        ctx: &eframe::egui::Context,
    ) {
        if !self.multiplayer.tailnet_requested {
            return;
        }
        match result {
            Ok(sidecar) if self.multiplayer.abandon_tailnet => {
                // the player left the screen while this was starting
                self.multiplayer.abandon_tailnet = false;
                self.multiplayer.tailnet_requested = false;
                std::thread::Builder::new()
                    .name("workshop-leave".into())
                    .spawn(move || {
                        drop(sidecar);
                        let _ = crate::multiplayer_lan::cleanup_system_state();
                    })
                    .ok();
            }
            Ok(sidecar) => {
                self.multiplayer.sidecar = Some(sidecar);
                self.multiplayer.tunnel_ready = true;
                self.multiplayer.status =
                    t("set-tailnet-ip-ready-on-the-private-workshop-network").to_string();
                let _ = tx.send(AppMsg::Log("[Multiplayer] Connected".to_string()));
                self.refresh_launch_status(tx, ctx);
            }
            Err(_) if self.multiplayer.abandon_tailnet => {
                self.multiplayer.abandon_tailnet = false;
                self.multiplayer.tailnet_requested = false;
            }
            Err(error) => {
                self.multiplayer.tailnet_requested = false;
                self.multiplayer.wizard_started = false;
                self.multiplayer.status = t_args(
                    "multiplayer-could-not-start",
                    &[("error", redact(&error).into())],
                );
            }
        }
    }

    pub fn multiplayer_players(&mut self, players: Vec<PeerLink>) {
        if self.multiplayer.tunnel_ready {
            self.multiplayer.players = players;
        }
    }

    /// the sidecar exited on its own (crashed, or was killed)
    pub fn multiplayer_sidecar_exited(&mut self, tx: &Sender<AppMsg>) {
        if self.multiplayer.tunnel_ready {
            self.disconnect_multiplayer(tx);
            self.multiplayer.status = t("multiplayer-helper-stopped").to_string();
        }
    }

    fn launch_multiplayer(
        &mut self,
        rl_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &eframe::egui::Context,
    ) {
        if !self.multiplayer.tunnel_ready {
            self.multiplayer.status =
                t("launch-multiplayer-the-workshop-network-is-not-ready").to_string();
            return;
        }
        self.multiplayer.launching_rocket_league = true;
        self.multiplayer.setup_progress = Some(t("multiplayer-starting-rocket-league").to_string());
        let rl_path = rl_path.to_string();
        let rl_launch = self.rl_launch.clone();
        let seed_other_map_slots = self.multiplayer.seed_other_map_slots;
        let manager = self.manager.clone();
        let tx = tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            if seed_other_map_slots {
                manager.seed_empty_slots(&rl_path);
            }
            let result = crate::winutil::start_rocket_league(&rl_launch, Path::new(&rl_path))
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMsg::WorkshopMultiplayerLaunched { result });
            repaint.request_repaint();
        });
    }

    pub fn set_multiplayer_progress(&mut self, status: String) {
        self.multiplayer.setup_progress = Some(status);
    }

    pub fn finish_multiplayer_launch(&mut self, result: Result<(), String>) {
        self.multiplayer.setup_progress = None;
        match result {
            Ok(()) => {
                self.multiplayer.status = t("multiplayer-rocket-league-is-starting").to_string();
            }
            Err(error) => {
                self.multiplayer.launching_rocket_league = false;
                self.multiplayer.status = format!("Could not start Rocket League: {error}");
            }
        }
    }

    /// starts the session (map sync, see hosting.rs) once the sidecar is up
    /// and Rocket League is running. Triggered automatically, not by a
    /// separate button.
    fn start_relay(&mut self, tx: &Sender<AppMsg>, ctx: &eframe::egui::Context) {
        let Some(sidecar) = self.multiplayer.sidecar.clone() else {
            self.multiplayer.status =
                t("start-relay-the-workshop-network-is-not-ready").to_string();
            return;
        };
        self.multiplayer.setup_progress =
            Some(t("start-relay-starting-the-workshop-lan-relay").to_string());
        let manager = self.manager.clone();
        // the in-game name comes from Rocket League's launch log and doesn't
        // change mid-session, so it's only looked up until one is found
        let player_name = Arc::new(Mutex::new(String::new()));
        let map_provider: MapProvider = Arc::new(move || {
            let mut name = player_name.lock().unwrap();
            if name.is_empty() {
                *name = hebnix_sdk::log::parse_launch_log(None, false, "INT")
                    .session
                    .username
                    .unwrap_or_default();
            }
            LocalInfo {
                player_name: name.clone(),
                maps: manager
                    .active_maps()
                    .iter()
                    .filter_map(|(slot, data)| {
                        let id = id_of(data);
                        valid_map_id(&id).then(|| SlotMap {
                            slot: slot.clone(),
                            name: str_of(data, "name", slot).to_string(),
                            local: is_local_map_id(&id),
                            author: str_of(data, "author", "").to_string(),
                            description: str_of(data, "short_description", "").to_string(),
                            id,
                        })
                    })
                    .collect(),
            }
        });
        // peers can fetch any map in the cache, imported or from the cdn,
        // unless P2P file sharing is turned off in the system settings
        let cache_dir = self.manager.cache_dir.clone();
        let share_files = self.share_files.clone();
        let map_files: MapFileProvider = Arc::new(move |id| {
            if !share_files.load(Ordering::Relaxed) || !valid_map_id(id) {
                return None;
            }
            let path = cache_dir.join(format!("{id}.upk"));
            path.is_file().then_some(path)
        });
        let tx = tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = HostSession::start(sidecar, map_provider, map_files, tx.clone());
            let _ = tx.send(AppMsg::WorkshopRelayStarted { result });
            repaint.request_repaint();
        });
    }

    /// who replaced which in-game map with what, next to what you have in
    /// the same slot. every player has to replace the same in-game map, so
    /// this flags a peer's map that sits in a different slot on your side.
    /// players are a collapsed list so it stays usable with lots of them.
    fn render_maps_in_use(
        &mut self,
        ui: &mut egui::Ui,
        install: &mut Option<(SlotMap, IpAddr, bool)>,
        block: &mut Option<(IpAddr, String)>,
    ) {
        let Some(session) = &self.multiplayer.relay else {
            return;
        };
        let mine = self.manager.active_maps();
        let mut offers = session.peer_offers();
        offers.sort_by_key(|offer| offer.label().to_lowercase());
        let busy = self.multiplayer.map_install_busy.load(Ordering::Relaxed);
        ui.group(|ui| {
            ui.strong(t("maps-in-use-maps-in-use"));
            ui.small(t("maps-in-use-everyone-has-to-replace-the-same"));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(t("maps-in-use-you")).strong());
            if mine.is_empty() {
                ui.small(format!("  {}", t("maps-none-replaced-yet")));
            }
            for (slot, data) in &mine {
                render_my_slot(ui, slot, data, &offers);
            }

            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(t_args(
                    "maps-in-use-players-offers",
                    &[("offers", (offers.len()).to_string().into())],
                ))
                .strong(),
            );
            if offers.is_empty() {
                ui.small(t("maps-in-use-no-other-players-found-yet"));
            } else {
                ui.small(t("maps-in-use-maps-installed-from-another-player-come"));
            }
            if offers.len() > PLAYER_SEARCH_THRESHOLD {
                ui.horizontal(|ui| {
                    ui.label(t("spoofer-search"));
                    ui.text_edit_singleline(&mut self.multiplayer.player_filter);
                });
            } else {
                self.multiplayer.player_filter.clear();
            }
            let filter = self.multiplayer.player_filter.clone();
            egui::ScrollArea::vertical()
                .id_salt("workshop_players")
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let mut shown = 0;
                    for offer in offers
                        .iter()
                        .filter(|o| matches_filter(&o.label(), &filter))
                    {
                        shown += 1;
                        egui::CollapsingHeader::new(player_header(ui, offer, &mine))
                            .id_salt(("workshop_player", offer.ip))
                            .default_open(false)
                            .show(ui, |ui| {
                                render_player_maps(ui, offer, &mine, busy, install);
                                ui.add_space(4.0);
                                if ui
                                    .button(t("maps-in-use-block-player"))
                                    .on_hover_text(t("maps-in-use-hide-this-player-s-maps-and"))
                                    .clicked()
                                {
                                    *block = Some((offer.ip, offer.label()));
                                }
                            });
                    }
                    if shown == 0 && !offers.is_empty() {
                        ui.small(t("maps-in-use-no-players-match-that-search"));
                    }
                });

            let transfer = &self.multiplayer.map_transfer;
            let total = transfer.total.load(Ordering::Relaxed);
            if busy && total > 0 {
                let done = transfer.done.load(Ordering::Relaxed);
                ui.add(
                    egui::ProgressBar::new(done as f32 / total as f32).text(format!(
                        "Downloading from a player: {} / {} MB",
                        done / 1_000_000,
                        total / 1_000_000
                    )),
                );
            } else if busy {
                ui.small(t("maps-in-use-installing-the-map"));
            } else if let Ok(result) = self.multiplayer.map_install_result.lock() {
                if !result.is_empty() {
                    ui.small(result.as_str());
                }
            }
        });
    }

    /// asks once per session before installing a map straight from another
    /// player's pc
    fn render_peer_install_warning(&mut self, rl_path: &str, ctx: &eframe::egui::Context) {
        let Some((map, _, _)) = &self.multiplayer.pending_peer_install else {
            return;
        };
        let name = map.name.clone();
        let mut choice = None;
        egui::Window::new(t("peer-install-warning-install-from-another-player"))
            .id(egui::Id::new(
                "peer-install-warning-install-from-another-player",
            ))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t_args(
                    "peer-install-warning-name-will-be-downloaded-straight-from",
                    &[("name", name.to_string().into())],
                ));
                ui.label(t("peer-install-warning-hebnix-only-checks-that-the-file"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t("hebnix-install-install")).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(t("spawner-enable-prompt-cancel")).clicked() {
                        choice = Some(false);
                    }
                });
            });
        match choice {
            Some(true) => {
                self.multiplayer.peer_warning_accepted = true;
                if let Some((map, peer, from_peer)) = self.multiplayer.pending_peer_install.take() {
                    self.install_peer_map(map, peer, from_peer, rl_path, ctx);
                }
            }
            Some(false) => self.multiplayer.pending_peer_install = None,
            None => {}
        }
    }

    fn save_multiplayer_settings(&self) {
        let settings = serde_json::json!({ "relay_only": self.multiplayer.relay_only });
        let _ = std::fs::write(
            self.manager.runtime_dir.join(MULTIPLAYER_SETTINGS_FILE),
            settings.to_string(),
        );
    }

    /// installs a map a peer has into the same slot. cdn maps are downloaded
    /// from the cdn unless `from_peer` is set; maps the peer imported
    /// themselves (not on the cdn) are always fetched from that peer and
    /// checked against their id first. the id was validated when it came off
    /// the wire (see map_sync.rs).
    fn install_peer_map(
        &mut self,
        map: SlotMap,
        peer: IpAddr,
        from_peer: bool,
        rl_path: &str,
        ctx: &eframe::egui::Context,
    ) {
        if self
            .multiplayer
            .map_install_busy
            .swap(true, Ordering::Relaxed)
        {
            return;
        }
        let map_data = self
            .catalog
            .iter()
            .find(|entry| id_of(entry) == map.id)
            .cloned()
            .unwrap_or_else(|| {
                serde_json::json!({
                    "id": map.id,
                    "name": map.name,
                    "author": if map.author.is_empty() { "Unknown" } else { map.author.as_str() },
                    "short_description": map.description,
                    "local": map.local,
                })
            });
        let manager = self.manager.clone();
        let busy = self.multiplayer.map_install_busy.clone();
        let result = self.multiplayer.map_install_result.clone();
        let transfer = self.multiplayer.map_transfer.clone();
        let received = self.multiplayer.received_maps.clone();
        transfer.done.store(0, Ordering::Relaxed);
        transfer.total.store(0, Ordering::Relaxed);
        let rl_path = rl_path.to_string();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let outcome = (|| {
                if (map.local || from_peer) && !manager.is_cached(&map.id) {
                    fetch_map_file(
                        peer,
                        MAP_SYNC_PORT,
                        &map.id,
                        &manager.cache_dir.join(format!("{}.upk", map.id)),
                        MAX_MAP_BYTES,
                        &transfer,
                    )?;
                    if map.local {
                        let entry = LocalMap {
                            id: map.id.clone(),
                            name: map.name.clone(),
                            author: if map.author.is_empty() {
                                "Unknown".to_string()
                            } else {
                                map.author.clone()
                            },
                            description: map.description.clone(),
                            banner_path: String::new(),
                        };
                        record_received_map(&manager.runtime_dir, &entry)?;
                        if let Ok(mut inbox) = received.lock() {
                            inbox.push(entry);
                        }
                    }
                }
                manager.install_map(&map_data, &map.slot, &rl_path)
            })();
            let text = match outcome {
                Ok(()) => format!("Installed {} in {}.", map.name, map.slot),
                Err(error) => format!("Could not install {}: {error}", map.name),
            };
            if let Ok(mut slot) = result.lock() {
                *slot = text;
            }
            busy.store(false, Ordering::Relaxed);
            repaint.request_repaint();
        });
    }

    pub fn refresh_launch_status(&mut self, tx: &Sender<AppMsg>, ctx: &eframe::egui::Context) {
        if !self.multiplayer.wizard_started
            || self.multiplayer.multihome_check_in_flight
            || !self.multiplayer.tunnel_ready
        {
            return;
        }
        self.multiplayer.multihome_check_in_flight = true;
        let tx = tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let rl_open = hebnix_sdk::process::is_rocket_league_running();
            if rl_open {
                std::thread::sleep(MULTIHOME_CHECK_INTERVAL);
            }
            let rl_open = hebnix_sdk::process::is_rocket_league_running();
            // Launch.log is rewritten as RL starts, so a stale -multihome
            // from the last launch clears itself on a later retry
            let launch_ready = rl_open && rocket_league_multihome_address().is_none();
            let _ = tx.send(AppMsg::WorkshopLaunchCheck {
                rl_open,
                launch_ready,
            });
            repaint.request_repaint();
        });
    }

    pub fn finish_launch_check(&mut self, rl_open: bool, launch_ready: bool) {
        self.multiplayer.rl_open = rl_open;
        self.multiplayer.launch_ready = launch_ready;
        self.multiplayer.multihome_check_in_flight = false;
        if !rl_open || launch_ready {
            self.multiplayer.multihome_check_attempts = 0;
        } else {
            self.multiplayer.multihome_check_attempts =
                self.multiplayer.multihome_check_attempts.saturating_add(1);
        }
    }

    pub fn retry_multihome_check(&self) -> bool {
        self.multiplayer.wizard_started
            && self.multiplayer.rl_open
            && !self.multiplayer.launch_ready
            && !self.multiplayer.multihome_check_in_flight
            && self.multiplayer.multihome_check_attempts < MULTIHOME_CHECK_MAX_ATTEMPTS
    }

    fn waiting_for_multihome_check(&self) -> bool {
        self.multiplayer.rl_open
            && !self.multiplayer.launch_ready
            && self.multiplayer.multihome_check_attempts < MULTIHOME_CHECK_MAX_ATTEMPTS
    }

    pub fn update_workshop_map_from_stats(&mut self, arena: &str, _tx: &Sender<AppMsg>) {
        if !self.multiplayer.wizard_started || arena.trim().is_empty() {
            return;
        }
        // informational only now (shown once the relay's up) - starting the
        // relay itself no longer needs to know the map, host or guest
        let arena = arena.trim_end_matches(".upk");
        if let Some((target, map)) = self.manager.active_maps().into_iter().find(|(target, _)| {
            target_filename(target)
                .map(|name| name.trim_end_matches(".upk").eq_ignore_ascii_case(arena))
                .unwrap_or_else(|| target.trim_end_matches(".upk").eq_ignore_ascii_case(arena))
        }) {
            self.multiplayer.detected_map = Some(str_of(&map, "name", &target).to_string());
            self.multiplayer.detected_target = Some(target);
        }
    }

    pub fn finish_relay_started(&mut self, result: Result<HostSession, String>) {
        self.multiplayer.setup_progress = None;
        match result {
            Ok(session) => {
                self.multiplayer.identity_updated = false;
                self.multiplayer.identity_update_in_flight = false;
                self.multiplayer.status =
                    t("finish-relay-started-relaying-host-or-join-from-rocket").to_string();
                self.multiplayer.relay = Some(session);
            }
            Err(error) => {
                self.multiplayer.status = format!("Could not start the relay: {}", redact(&error))
            }
        }
    }

    pub fn finish_player_update(&mut self, result: Result<(), String>) {
        self.multiplayer.identity_update_in_flight = false;
        match result {
            Ok(()) => self.multiplayer.identity_updated = true,
            Err(error) => {
                self.multiplayer.status = format!("Could not update player details: {error}")
            }
        }
    }

    pub fn finish_host_session_check(
        &mut self,
        result: Result<crate::multiplayer_lan::Room, String>,
    ) {
        self.multiplayer.saved_host_checking = false;
        self.multiplayer.saved_host_checked = true;
        match result {
            Ok(room) => {
                self.multiplayer.saved_room = Some(room.clone());
                self.multiplayer.status = format!(
                    "Previous hosting session {} is alive for {}.",
                    room.pin, room.map.name
                );
            }
            Err(_) => {
                self.multiplayer.saved_host = None;
                self.multiplayer.saved_room = None;
                self.clear_host_state();
            }
        }
    }

    fn save_host_state(&self) {
        if let Some(saved) = &self.multiplayer.saved_host {
            if let Ok(bytes) = serde_json::to_vec(saved) {
                let _ = std::fs::write(
                    self.manager.runtime_dir.join("multiplayer_host.json"),
                    bytes,
                );
            }
        }
    }

    fn clear_host_state(&mut self) {
        self.multiplayer.saved_host = None;
        self.multiplayer.saved_room = None;
        let _ = std::fs::remove_file(self.manager.runtime_dir.join("multiplayer_host.json"));
    }

    fn map_hash(&self, map_id: &str) -> Result<String, String> {
        let path = self.manager.cache_dir.join(format!("{map_id}.upk"));
        let mut file = std::fs::File::open(&path)
            .map_err(|error| format!("Could not open the downloaded map: {error}"))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("Could not read the downloaded map: {error}"))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    fn render_card(&mut self, ui: &mut egui::Ui, map_idx: usize) -> Option<CardAction> {
        let map_data = &self.catalog[map_idx];
        let map_id = id_of(map_data);
        let mut name = str_of(map_data, "name", "Unknown").to_string();
        // full name + description show when hovering the title
        let hover = match str_of(map_data, "short_description", "") {
            "" => name.clone(),
            description => format!("{name}\n\n{description}"),
        };
        if name.chars().count() > 28 {
            name = format!("{}...", name.chars().take(25).collect::<String>());
        }
        let author = str_of(map_data, "author", "Unknown").to_string();
        let banner = str_of(map_data, "banner_path", "").to_string();

        let active_targets = self.manager.get_active_targets_for_map(&map_id);
        let is_active_on_current = active_targets.contains(&self.target);
        let is_cached = self.manager.is_cached(&map_id);
        let is_busy = self.busy.contains(&map_id);

        let mut result = None;

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(230.0);
            ui.vertical_centered(|ui| {
                // Image
                let img_size = egui::vec2(160.0, 90.0);
                match self.images.get(&banner) {
                    Some(ImageState::Ready(bytes)) => {
                        ui.add(
                            egui::Image::from_bytes(
                                format!("bytes://workshop/{banner}"),
                                bytes.clone(),
                            )
                            .fit_to_exact_size(img_size),
                        );
                    }
                    Some(ImageState::Failed) => {
                        ui.add_sized(img_size, egui::Label::new(t("card-failed-to-load")));
                    }
                    _ => {
                        if banner.is_empty() {
                            ui.add_sized(img_size, egui::Label::new(t("card-no-image-available")));
                        } else {
                            ui.add_sized(img_size, egui::Label::new(t("card-loading-image")));
                        }
                    }
                }

                ui.strong(name).on_hover_text(hover);
                ui.label(
                    egui::RichText::new(t_args(
                        "hebnix-install-by-author",
                        &[("author", author.to_string().into())],
                    ))
                    .italics()
                    .size(11.0)
                    .color(egui::Color32::GRAY),
                );

                if !active_targets.is_empty() {
                    let active_text = format!("Active on: {}", active_targets.join(", "));
                    let mut font = egui::TextStyle::Body.resolve(ui.style());
                    font.size = 12.0;
                    let text_width = ui.painter()
                        .layout_no_wrap(active_text.clone(), font, ui.visuals().text_color())
                        .size().x;
                    let row_width = (13.0 + ui.spacing().item_spacing.x + text_width)
                        .min(ui.available_width());
                    ui.allocate_ui_with_layout(
                        egui::vec2(row_width, ui.spacing().interact_size.y),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(13.0, 13.0), egui::Sense::hover(),
                            );
                            let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(68, 190, 100));
                            let origin = rect.left_top();
                            ui.painter().line_segment(
                                [
                                    origin + egui::vec2(1.0, 7.0),
                                    origin + egui::vec2(5.0, 11.0),
                                ],
                                stroke,
                            );
                            ui.painter().line_segment(
                                [
                                    origin + egui::vec2(5.0, 11.0),
                                    origin + egui::vec2(12.0, 2.0),
                                ],
                                stroke,
                            );
                            ui.label(egui::RichText::new(active_text).size(12.0));
                        },
                    );
                } else {
                    let status = if is_local_entry(map_data) {
                        "📁 Imported".to_string()
                    } else if is_cached {
                        "📦 Cached".to_string()
                    } else {
                        "☁ Cloud".to_string()
                    };
                    ui.label(egui::RichText::new(status).size(12.0));
                }
                ui.add_space(4.0);

                let (btn_text, btn_color) = if is_busy {
                    (t("tab-working").to_string(), None)
                } else if is_active_on_current {
                    (
                        format!("Unload {}", self.target),
                        Some(egui::Color32::from_rgb(0xc0, 0x39, 0x2b)),
                    )
                } else if is_cached {
                    (format!("Load to {}", self.target), None)
                } else {
                    (format!("Download for {}", self.target), None)
                };

                ui.horizontal(|ui| {
                    let mut button = egui::Button::new(btn_text);
                    if let Some(color) = btn_color {
                        button = button.fill(color);
                    }
                    let show_delete = is_cached && active_targets.is_empty() && !is_busy;
                    let btn_width = if show_delete {
                        ui.available_width() - 28.0 - ui.spacing().item_spacing.x
                    } else {
                        ui.available_width()
                    }
                    .max(0.0);
                    if ui
                        .add_enabled(!is_busy, button.min_size(egui::vec2(btn_width, 24.0)))
                        .clicked()
                    {
                        result = Some(if is_active_on_current {
                            CardAction::Unload
                        } else {
                            CardAction::InstallOrDownload
                        });
                    }
                    if show_delete
                        && ui
                            .add(
                                egui::Button::new("🗑")
                                    .fill(egui::Color32::from_rgb(0xc0, 0x39, 0x2b))
                                    .min_size(egui::vec2(28.0, 24.0)),
                            )
                            .clicked()
                    {
                        result = Some(CardAction::DeleteCache);
                    }
                });
            });
        });

        result
    }

    fn handle_action(
        &mut self,
        map_idx: usize,
        action: CardAction,
        rl_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &eframe::egui::Context,
    ) {
        let map_data = self.catalog[map_idx].clone();
        let map_id = id_of(&map_data);

        match action {
            CardAction::Unload => match self.manager.unload_active_map(&self.target, rl_path) {
                Ok(()) => self.execute_search(false),
                Err(e) => {
                    let _ = tx.send(AppMsg::Log(format!("[Workshop] {e}")));
                }
            },
            CardAction::DeleteCache => {
                self.confirm_delete = Some(map_data);
            }
            CardAction::InstallOrDownload => {
                self.busy.insert(map_id.clone());
                let manager = self.manager.clone();
                let target = self.target.clone();
                let rl_path = rl_path.to_string();
                let tx = tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = manager.install_map(&map_data, &target, &rl_path);
                    let msg = match &result {
                        Ok(()) => format!(
                            "[Workshop] Installed '{}' to {target}.",
                            str_of(&map_data, "name", "map")
                        ),
                        Err(e) => format!("[Workshop] Map Install Error: {e}"),
                    };
                    let _ = tx.send(AppMsg::WorkshopOpDone { message: msg });
                    ctx.request_repaint();
                });
            }
        }
    }

    /// called when a WorkshopOpDone message arrives
    pub fn finish_op(&mut self) {
        self.busy.clear();
        self.execute_search(false);
    }

    /// Rocket League closed. Rather than tearing the session down
    /// immediately (which would kick everyone out of the room over an
    /// ordinary crash or restart), this arms a grace window; see
    /// tick_shutdown_grace for the part that actually tears things down.
    pub fn shutdown_multiplayer(&mut self) {
        if self.multiplayer.launching_rocket_league {
            // this is Hebnix's own close-then-relaunch, not a real exit
            return;
        }
        if self.multiplayer.relay.is_none() || self.multiplayer.shutdown_deadline.is_some() {
            return;
        }
        self.multiplayer.shutdown_deadline =
            Some(std::time::Instant::now() + crate::multiplayer_lan::CRASH_GRACE_WINDOW);
        self.multiplayer.status =
            t("shutdown-multiplayer-rocket-league-closed-keeping-the-session").to_string();
    }

    /// call periodically (piggybacked on the existing RL-status poll) to
    /// resolve the crash grace window one way or the other
    pub fn tick_shutdown_grace(&mut self) {
        let Some(deadline) = self.multiplayer.shutdown_deadline else {
            return;
        };
        if hebnix_sdk::process::is_rocket_league_running() {
            self.multiplayer.shutdown_deadline = None;
            self.multiplayer.status =
                t("tick-shutdown-grace-rocket-league-reconnected").to_string();
            return;
        }
        if std::time::Instant::now() < deadline {
            return;
        }
        self.multiplayer.shutdown_deadline = None;
        let relay = self.multiplayer.relay.take();
        self.multiplayer.status =
            t("tick-shutdown-grace-workshop-multiplayer-stopped-because-roc").to_string();

        // Stopping a session can wait for a network heartbeat, while firewall
        // cleanup launches external commands. This runs from the egui
        // message handler, so doing any of that here freezes the whole app.
        std::thread::Builder::new()
            .name("workshop-shutdown".into())
            .spawn(move || {
                if let Some(mut session) = relay {
                    let _ = session.stop();
                }
                let _ = crate::winutil::clear_rocket_league_multihome();
                let _ = crate::multiplayer_lan::cleanup_system_state();
            })
            .ok();
    }

    pub fn rocket_league_reopened(&mut self) {
        self.multiplayer.launching_rocket_league = false;
        self.multiplayer.shutdown_deadline = None;
    }

    pub fn suspend_multiplayer(&mut self) {
        if let Some(session) = self.multiplayer.relay.as_mut() {
            session.suspend();
        }
        self.multiplayer.relay = None;
        self.multiplayer.sidecar = None;
        if !hebnix_sdk::process::is_rocket_league_running() {
            let _ = crate::winutil::clear_rocket_league_multihome();
            let _ = crate::multiplayer_lan::cleanup_system_state();
        }
    }
}

enum CardAction {
    InstallOrDownload,
    Unload,
    DeleteCache,
}

// Every tile reserves the same slots, including the optional cache action.
struct ThemedTileMetrics {
    size: egui::Vec2,
    padding: f32,
    image_size: egui::Vec2,
    title_height: f32,
    small_height: f32,
    action_height: f32,
    delete_height: f32,
    footer_height: f32,
}
impl ThemedTileMetrics {
    fn new(ui: &egui::Ui, card: &crate::theme::MapCardStyle, width: f32, target: &str, details: bool) -> Self {
        let gap = ui.spacing().item_spacing.y;
        // Reserve the selected border even on unselected cards.
        let padding = card.padding + 2.0;
        let inner_width = (width - padding * 2.0).max(1.0);
        let image_width = if card.horizontal { 100.0_f32.min(inner_width * 0.4) } else { inner_width };
        let body_width = if card.horizontal { (inner_width - image_width - ui.spacing().item_spacing.x).max(1.0) } else { inner_width };
        let image_height = if card.horizontal { card.image_height.min(140.0) } else { card.image_height };
        let title_height = (ui.text_style_height(&egui::TextStyle::Button) + ui.spacing().button_padding.y * 2.0 + 2.0)
            .max(ui.spacing().interact_size.y);
        let small_height = ui.text_style_height(&egui::TextStyle::Small);
        let button_font = egui::TextStyle::Button.resolve(ui.style());
        let text_width = (body_width - ui.spacing().button_padding.x * 2.0 - 2.0).max(1.0);
        let labels = if details { vec!["Details".to_owned()] } else {
            vec![t("tab-working"), format!("Unload {target}"), format!("Load to {target}"), format!("Download for {target}")]
        };
        let text_height = labels.into_iter().map(|label|ui.fonts_mut(|fonts|
            fonts.layout(label,button_font.clone(),egui::Color32::WHITE,text_width).size().y
        )).fold(0.0_f32,f32::max);
        let action_height = 28.0_f32.max(ui.spacing().interact_size.y)
            .max(text_height + ui.spacing().button_padding.y * 2.0 + 2.0);
        let delete_height = (small_height + ui.spacing().button_padding.y * 2.0 + 2.0).max(ui.spacing().interact_size.y);
        let footer_height = if details { action_height } else { small_height + gap + action_height + gap + delete_height };
        let body_height = title_height + gap + small_height + gap + footer_height;
        let min_height = if card.horizontal { image_height.max(body_height) } else { image_height + gap + body_height };
        Self { size: egui::vec2(width,card.height.max(min_height + padding * 2.0)), padding,
            image_size: egui::vec2(image_width,image_height), title_height, small_height,
            action_height, delete_height, footer_height }
    }
}
fn centered_image_uv(source: egui::Vec2, viewport: egui::Vec2) -> egui::Rect {
    let source_ratio = source.x / source.y.max(1.0);
    let view_ratio = viewport.x / viewport.y.max(1.0);
    let fraction = if source_ratio > view_ratio { egui::vec2(view_ratio/source_ratio,1.0) }
        else { egui::vec2(1.0,source_ratio/view_ratio) };
    egui::Rect::from_center_size(egui::pos2(0.5,0.5),fraction)
}

impl WorkshopState {
    fn browse_columns(
        &self,
        ui: &egui::Ui,
        presentation: Option<&crate::theme::Presentation>,
    ) -> usize {
        let mut width = ui.available_width() - ui.spacing().scroll.allocated_width();
        let minimum = if let Some(p) = presentation {
            if p.pages.workshop.layout == crate::theme::WorkshopPresentation::LibraryWithDetails
                && ui.available_width() >= 880.0
            {
                width -= p
                    .pages
                    .workshop
                    .details_width
                    .min(ui.available_width() * 0.36);
            }
            p.components.map_card.min_width.max(224.0)
        } else {
            224.0
        };
        crate::ui::browser::columns_for_width(width, ui.spacing().item_spacing.x, minimum)
    }

    fn themed_image(&self, ui: &mut egui::Ui, map_idx: usize, size: egui::Vec2) -> egui::Response {
        let banner = str_of(&self.catalog[map_idx], "banner_path", "");
        let (rect,response) = ui.allocate_exact_size(size,egui::Sense::click());
        ui.painter().rect_filled(rect,4,ui.visuals().extreme_bg_color);
        match self.images.get(banner) {
            Some(ImageState::Ready(bytes)) => {
                let image = egui::Image::from_bytes(format!("bytes://workshop/{banner}"),bytes.clone()).corner_radius(4);
                let uv = image.load_for_size(ui.ctx(),size).ok().and_then(|poll|poll.size())
                    .map(|source|centered_image_uv(source,size))
                    .unwrap_or(egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)));
                image.uv(uv).paint_at(ui,rect);
            }
            _ => { ui.painter().with_clip_rect(ui.clip_rect().intersect(rect)).text(rect.center(),egui::Align2::CENTER_CENTER,
                t("card-no-image-available"),egui::TextStyle::Small.resolve(ui.style()),ui.visuals().weak_text_color()); }
        }
        response
    }
    fn render_themed_library(
        &mut self,
        ui: &mut egui::Ui,
        indices: &[usize],
        p: &crate::theme::Presentation,
    ) -> Option<(usize, CardAction)> {
        use crate::theme::{WorkshopPresentation, color, surface_color};
        if indices.is_empty() {
            ui.label(if self.catalog.is_empty() {
                self.catalog_status.clone()
            } else {
                t("handle-messages-no-maps-found")
            });
            return None;
        }
        let selected = indices
            .iter()
            .copied()
            .find(|&i| self.theme_selected_map.as_deref() == Some(&id_of(&self.catalog[i])))
            .unwrap_or(indices[0]);
        self.theme_selected_map = Some(id_of(&self.catalog[selected]));
        let with_details = p.pages.workshop.layout == WorkshopPresentation::LibraryWithDetails
            && ui.available_width() >= 880.0;
        let column_count = self.browse_columns(ui, Some(p));
        let mut action = None;
        if with_details {
            let width = p
                .pages
                .workshop
                .details_width
                .min(ui.available_width() * 0.36);
            egui::Panel::right("workshop_theme_details")
                .resizable(false)
                .exact_size(width)
                .frame(
                    egui::Frame::new()
                        .fill(surface_color(ui, &p.components.map_card.surface))
                        .inner_margin(12),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("workshop_theme_details_scroll")
                        .show(ui, |ui| {
                            self.themed_image(
                                ui,
                                selected,
                                egui::vec2(
                                    ui.available_width(),
                                    p.components.map_card.image_height * 1.5,
                                ),
                            );
                            ui.heading(str_of(&self.catalog[selected], "name", "Unknown"));
                            ui.label(
                                egui::RichText::new(str_of(
                                    &self.catalog[selected],
                                    "author",
                                    "Unknown",
                                ))
                                .weak(),
                            );
                            ui.separator();
                            ui.label(str_of(&self.catalog[selected], "short_description", ""));
                            ui.add_space(12.0);
                            ui.label(format!("{}: {}", t("render-map-to-replace"), self.target));
                            if let Some(a) = self.themed_map_actions(ui, selected, None) {
                                action = Some((selected, a));
                            }
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("workshop_grid")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let card = &p.components.map_card;
                        let columns = column_count;
                        let width = ((ui.available_width()
                            - ui.spacing().item_spacing.x * (columns - 1) as f32)
                            / columns as f32)
                            .max(1.0);
                        let metrics =
                            ThemedTileMetrics::new(ui, card, width, &self.target, with_details);
                        for row in indices.chunks(columns) {
                            ui.columns(columns, |cols| {
                                for (col, &idx) in cols.iter_mut().zip(row) {
                                    let id = id_of(&self.catalog[idx]);
                                    let is_selected =
                                        self.theme_selected_map.as_deref() == Some(&id);
                                    let (rect, _) =
                                        col.allocate_exact_size(metrics.size, egui::Sense::hover());
                                    col.painter().rect_filled(
                                        rect,
                                        card.corner_radius,
                                        surface_color(col, &card.surface),
                                    );
                                    col.painter().rect_stroke(
                                        rect,
                                        card.corner_radius,
                                        egui::Stroke::new(
                                            if is_selected { 2.0 } else { 1.0 },
                                            color(if is_selected {
                                                &card.selected_border
                                            } else {
                                                &card.border
                                            }),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                    let inner = rect.shrink(metrics.padding);
                                    let mut tile = col.new_child(
                                        egui::UiBuilder::new()
                                            .id_salt(("map_tile", id.clone()))
                                            .max_rect(inner)
                                            .layout(egui::Layout::top_down(egui::Align::Min)),
                                    );
                                    tile.set_clip_rect(tile.clip_rect().intersect(inner));
                                    let image_rect =
                                        egui::Rect::from_min_size(inner.min, metrics.image_size);
                                    let mut image_ui = tile.new_child(
                                        egui::UiBuilder::new()
                                            .id_salt("image")
                                            .max_rect(image_rect),
                                    );
                                    if self
                                        .themed_image(&mut image_ui, idx, metrics.image_size)
                                        .clicked()
                                    {
                                        self.theme_selected_map = Some(id.clone());
                                    }
                                    let body_min = if card.horizontal {
                                        inner.min
                                            + egui::vec2(
                                                metrics.image_size.x
                                                    + tile.spacing().item_spacing.x,
                                                0.0,
                                            )
                                    } else {
                                        inner.min
                                            + egui::vec2(
                                                0.0,
                                                metrics.image_size.y
                                                    + tile.spacing().item_spacing.y,
                                            )
                                    };
                                    let mut body = tile.new_child(
                                        egui::UiBuilder::new()
                                            .id_salt("body")
                                            .max_rect(egui::Rect::from_min_max(body_min, inner.max))
                                            .layout(egui::Layout::top_down(egui::Align::Min)),
                                    );
                                    if let Some(a) = self.themed_card_body(
                                        &mut body,
                                        idx,
                                        with_details,
                                        &metrics,
                                    ) {
                                        action = Some((idx, a));
                                    }
                                }
                            });
                            ui.add_space(8.0);
                        }
                    });
            });
        action
    }
    fn themed_card_body(&mut self, ui: &mut egui::Ui, idx: usize, with_details: bool, metrics: &ThemedTileMetrics) -> Option<CardAction> {
        let name = str_of(&self.catalog[idx],"name","Unknown").to_string();
        let width = ui.available_width();
        if ui.add_sized([width,metrics.title_height],egui::Button::new(egui::RichText::new(&name).strong())
            .frame(false).truncate()).on_hover_text(&name).clicked() {
            self.theme_selected_map = Some(id_of(&self.catalog[idx]));
        }
        let author = str_of(&self.catalog[idx],"author","Unknown");
        ui.add_sized([width,metrics.small_height],egui::Label::new(egui::RichText::new(author).small().weak()).truncate())
            .on_hover_text(author);
        let rect = egui::Rect::from_min_max(egui::pos2(ui.max_rect().left(),ui.max_rect().bottom()-metrics.footer_height),ui.max_rect().max);
        let mut footer = ui.new_child(egui::UiBuilder::new().id_salt("actions").max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)));
        if !with_details { self.themed_map_actions(&mut footer,idx,Some(metrics)) } else {
            if footer.add_sized([width,metrics.action_height],egui::Button::new("Details")).clicked() {
                self.theme_selected_map = Some(id_of(&self.catalog[idx]));
            }
            None
        }
    }
    fn themed_map_actions(&mut self, ui: &mut egui::Ui, idx: usize, metrics: Option<&ThemedTileMetrics>) -> Option<CardAction> {
        let id = id_of(&self.catalog[idx]);
        let active = self.manager.get_active_targets_for_map(&id).contains(&self.target);
        let cached = self.manager.is_cached(&id);
        let busy = self.busy.contains(&id);
        let label = if busy { t("tab-working") } else if active { format!("Unload {}",self.target) }
            else if cached { format!("Load to {}",self.target) } else { format!("Download for {}",self.target) };
        let mut result = None;
        let status = if active {"Active"} else if cached {"Downloaded"} else {"Not downloaded"};
        if let Some(m) = metrics {
            ui.add_sized([ui.available_width(), m.small_height],
                egui::Label::new(egui::RichText::new(status).small().weak()).truncate())
                .on_hover_text(status);
        } else {
            ui.label(egui::RichText::new(status).small().weak());
        }
        let button = egui::Button::new(&label).wrap().min_size(egui::vec2(0.0,28.0));
        let response = if let Some(m) = metrics { ui.add_enabled_ui(!busy,|ui| ui.add_sized([ui.available_width(),m.action_height],button)).inner }
            else { ui.add_enabled(!busy,button) };
        if response.on_hover_text(&label).clicked() {
            result = Some(if active {CardAction::Unload} else {CardAction::InstallOrDownload});
        }
        if cached && !active && !busy {
            let clicked = if let Some(m)=metrics { ui.add_sized([ui.available_width(),m.delete_height],egui::Button::new(egui::RichText::new("Delete cached map").small()).truncate()).clicked() }
                else { ui.small_button("Delete cached map").clicked() };
            if clicked { result = Some(CardAction::DeleteCache); }
        } else if let Some(m)=metrics { ui.allocate_space(egui::vec2(ui.available_width(),m.delete_height)); }
        result
    }
}

#[cfg(test)]
mod theme_layout_tests {
    use super::*;
    #[test]
    fn maps_browser_keeps_map_tiles_and_pages_in_three_rows() {
        let base =
            std::env::temp_dir().join(format!("hebnix-maps-browser-{}", rand::random::<u64>()));
        let themes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        for theme in ["Dark", "Ocean Studio Dark"] {
            for width in [760.0, 1250.0, 1530.0] {
                let ctx = egui::Context::default();
                crate::theme::apply_theme(&ctx, &themes, &themes, theme).unwrap();
                let mut workshop = WorkshopState::new(&base);
                workshop.fetched = true;
                workshop.catalog = (0..40).map(|i| serde_json::json!({"id":format!("map-{i}"),"name":format!("Map {i}"),"author":"Map author","banner_path":""})).collect();
                workshop.execute_search(true);
                let (tx, _rx) = crossbeam_channel::unbounded();
                let mut output = egui::FullOutput::default();
                for _ in 0..3 {
                    output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 1000.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                workshop.render(
                                    ui,
                                    "",
                                    &crate::config::RlLaunchCfg::default(),
                                    &tx,
                                );
                                assert!(
                                    ui.min_rect().right() <= width + 1.0,
                                    "Map content overflows at {theme}/{width}"
                                );
                            });
                        },
                    );
                }
                assert!(
                    workshop.page_size.is_multiple_of(3) && (3..=18).contains(&workshop.page_size)
                );
                fn text(shape: &egui::Shape, needle: &str) -> bool {
                    match shape {
                        egui::Shape::Text(t) => t.galley.text().contains(needle),
                        egui::Shape::Vec(v) => v.iter().any(|s| text(s, needle)),
                        _ => false,
                    }
                }
                for content in ["Map 0", "Map author", "Download for", "(40 items)"] {
                    assert!(
                        output.shapes.iter().any(|s| text(&s.shape, content)),
                        "Map tile content missing: {content} ({theme}/{width})"
                    );
                }
                workshop.search = "absent".into();
                workshop.execute_search(true);
                assert!(workshop.valid.is_empty());
                assert_eq!(workshop.page, 0);
            }
        }
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let resolved = base.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp) && resolved != temp);
        std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn theme_shells_and_workshop_render_headlessly_at_supported_sizes() {
        let base = std::env::temp_dir().join(format!("hebnix-theme-layout-{}", std::process::id()));
        let mut workshop = WorkshopState::new(&base);
        workshop.catalog = (0..6).map(|i| serde_json::json!({"id":format!("test-{i}"),"name":format!("Sample map {i}"),"author":"Example","banner_path":""})).collect();
        let indices: Vec<usize> = (0..6).collect();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        for name in ["Carbon Studio", "Ocean Workspace", "Crimson Arena"] {
            for size in [egui::vec2(1250.0, 700.0), egui::vec2(1536.0, 900.0)] {
                let ctx = egui::Context::default();
                crate::theme::apply_theme(&ctx, &dir, &dir, name).unwrap();
                let p = crate::theme::presentation(&ctx).unwrap();
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                };
                let output = ctx.run_ui(input, |ui| {
                    crate::theme::show_shell(ui, &p, |ui, part| match part {
                        crate::theme::ShellPart::Navigation(vertical) => {
                            if vertical {
                                ui.label("Hebnix");
                            } else {
                                ui.horizontal(|ui| {
                                    ui.label("Hebnix");
                                });
                            }
                        }
                        crate::theme::ShellPart::Content => {
                            assert!(workshop.render_themed_library(ui, &indices, &p).is_none());
                        }
                    });
                });
                assert!(!output.shapes.is_empty());
            }
        }
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let resolved = base.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp) && resolved != temp);
        std::fs::remove_dir_all(resolved).unwrap();
    }

    fn all_shapes<'a>(shape: &'a egui::Shape, result: &mut Vec<&'a egui::Shape>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    all_shapes(shape, result)
                }
            }
            _ => result.push(shape),
        }
    }
    #[test]
    fn themed_tiles_keep_fixed_rectangles_across_images_states_and_columns() {
        let base = std::env::temp_dir().join(format!("hebnix-fixed-tiles-{}", std::process::id()));
        let mut workshop = WorkshopState::new(&base);
        workshop.target = "Utopia Retro".into();
        workshop.catalog=(0..8).map(|i|serde_json::json!({
            "id":format!("tile-{i}"),"name":if i==1 {"A deliberately very long map title that must not grow its tile".to_string()} else {format!("Tile {i}")},
            "author":if i==2 {"A creator name that is far too long to fit".to_string()} else {format!("Creator {i}")},
            "banner_path":format!("banner-{i}")
        })).collect();
        for (i, (width, height)) in [
            (0, (512, 128)),
            (1, (128, 256)),
            (2, (256, 256)),
            (3, (320, 180)),
        ] {
            let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                width,
                height,
                image::Rgba([80, 120, 160, 255]),
            ));
            let mut encoded = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut encoded, image::ImageFormat::Png)
                .unwrap();
            workshop.images.insert(
                format!("banner-{i}"),
                ImageState::Ready(encoded.into_inner().into()),
            );
        }
        workshop
            .images
            .insert("banner-4".into(), ImageState::Loading);
        workshop
            .images
            .insert("banner-5".into(), ImageState::Failed);
        for id in ["tile-0", "tile-2", "tile-6"] {
            std::fs::write(
                workshop.manager.cache_dir.join(format!("{id}.upk")),
                b"test",
            )
            .unwrap();
        }
        workshop
            .manager
            .active_maps
            .lock()
            .unwrap()
            .insert("Utopia Retro".into(), serde_json::json!({"id":"tile-2"}));
        workshop.busy.insert("tile-3".into());
        let indices: Vec<usize> = (0..8).collect();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        for (columns, width, font, horizontal) in [
            (4, 1250.0, 14.0, false),
            (3, 1250.0, 14.0, false),
            (4, 760.0, 22.0, false),
            (4, 360.0, 22.0, false),
            (3, 1250.0, 22.0, true),
        ] {
            let ctx = egui::Context::default();
            egui_extras::install_image_loaders(&ctx);
            crate::theme::apply_theme(&ctx, &dir, &dir, "Ocean Studio Dark").unwrap();
            ctx.style_mut_of(ctx.theme(), |style| {
                style
                    .text_styles
                    .insert(egui::TextStyle::Button, egui::FontId::proportional(font));
                if font > 14.0 {
                    style.spacing.button_padding = egui::vec2(24.0, 16.0);
                    style
                        .text_styles
                        .insert(egui::TextStyle::Small, egui::FontId::proportional(18.0));
                }
            });
            let mut p = (*crate::theme::presentation(&ctx).unwrap()).clone();
            p.pages.workshop.columns = columns;
            p.components.map_card.horizontal = horizontal;
            let mut output = egui::FullOutput::default();
            for _ in 0..4 {
                output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 5000.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        assert!(workshop.render_themed_library(ui, &indices, &p).is_none());
                    },
                );
            }
            let mut shapes = Vec::new();
            for clipped in &output.shapes {
                all_shapes(&clipped.shape, &mut shapes);
            }
            let tiles: Vec<egui::Rect> = shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == crate::theme::color(&p.components.map_card.surface) =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                tiles.len(),
                8,
                "Missing tiles at {width}/{font}/{horizontal}"
            );
            for rect in &tiles {
                assert!(
                    (rect.width() - tiles[0].width()).abs() < 0.1
                        && (rect.height() - tiles[0].height()).abs() < 0.1,
                    "Nonuniform tile {rect:?} vs {:?}",
                    tiles[0]
                );
                assert!(rect.width() <= width && rect.left() >= 0.0 && rect.right() <= width);
            }
            let actual_columns = tiles
                .iter()
                .take_while(|r| (r.top() - tiles[0].top()).abs() < 1.0)
                .count();
            assert_eq!(
                actual_columns,
                crate::ui::browser::columns_for_width(
                    width,
                    10.0,
                    p.components.map_card.min_width.max(224.0)
                )
            );
            for (i, rect) in tiles.iter().enumerate() {
                for other in tiles.iter().skip(i + 1) {
                    assert!(!rect.intersects(*other), "Tiles overlap");
                }
                let texts: Vec<_> = shapes
                    .iter()
                    .filter_map(|shape| match shape {
                        egui::Shape::Text(text) if rect.contains(text.pos) => Some(text),
                        _ => None,
                    })
                    .collect();
                let action = texts
                    .iter()
                    .find(|text| {
                        text.galley.text().starts_with("Load to ")
                            || text.galley.text().starts_with("Unload ")
                            || text.galley.text().starts_with("Download for ")
                            || text.galley.text() == t("tab-working")
                    })
                    .expect("Tile primary action missing");
                assert!(
                    rect.contains_rect(egui::Rect::from_min_size(action.pos, action.galley.size())),
                    "Action overflows its tile: {:?}",
                    action.galley.text()
                );
                for other in tiles.iter().filter(|r| (r.top() - rect.top()).abs() < 1.0) {
                    let same_row = shapes
                        .iter()
                        .filter_map(|shape| match shape {
                            egui::Shape::Text(text) if other.contains(text.pos) => Some(text),
                            _ => None,
                        })
                        .find(|text| {
                            text.galley.text().starts_with("Load to ")
                                || text.galley.text().starts_with("Unload ")
                                || text.galley.text().starts_with("Download for ")
                                || text.galley.text() == t("tab-working")
                        })
                        .unwrap();
                    assert!(
                        (action.pos.y + action.galley.size().y * 0.5
                            - same_row.pos.y
                            - same_row.galley.size().y * 0.5)
                            .abs()
                            < 1.0,
                        "Primary buttons are not aligned"
                    );
                }
            }
            let images: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect),
                    _ => None,
                })
                .collect();
            assert_eq!(images.len(), 4);
            // Painting snaps the same logical viewport to physical pixels;
            // fractional column widths can differ by one raster pixel.
            for image in &images {
                assert!(
                    (image.rect.width() - images[0].rect.width()).abs() <= 1.01
                        && (image.rect.height() - images[0].rect.height()).abs() <= 1.01
                );
            }
            let uv = images[1].brush.as_ref().unwrap().uv;
            assert!(
                uv.height() < 1.0,
                "Portrait thumbnail must crop within its fixed viewport"
            );
        }
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let resolved = base.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp) && resolved != temp);
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
