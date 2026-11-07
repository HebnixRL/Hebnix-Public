//! Page that turns on Rocket League's own Workshop menu on the
//! Epic build and fills it with maps from a local folder.
use super::lan_mutators::{self, SETTINGS};
use super::local_import::parse_workshop_vdf;
use super::steam_download::fetch_details;
use crate::messages::AppMsg;
use crate::patcher::workshop_menu::{self, WorkshopMap};
use crossbeam_channel::Sender;
use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// Metadata written beside maps loaded into Hebnix's workshop folder.
pub(super) const META: &str = "hebnix.json";

enum Job {
    Status(Result<bool, String>),
    Done(Result<String, String>),
}

#[derive(Default)]
pub struct InGameMenuState {
    folder: String,
    maps: Vec<WorkshopMap>,
    /// index of the map chosen in the list
    selected: usize,
    install: Option<PathBuf>,
    enabled: Option<Result<bool, String>>,
    status: String,
    pending: Option<Receiver<Job>>,
    lan: LanHost,
}

/// Hebnix's own flags, as URL options after the settings. The game ignores them.
const ALWAYS: &str = "?HbxAlways";
const AUTO: &str = "?HbxAuto";

const BOTS: [&str; 5] = ["BotsNone", "BotsEasy", "BotsMedium", "BotsHard", "BotsUnfair"];
/// Options after the game tags, as Rocket Plugin hosts with.
const LISTEN: &str = "?Playtest?NumPublicConnections=10?NumOpenPublicConnections=10?Lan?Listen";

/// LAN hosting from the Workshop menu. The host file (or its parked copy while
/// hosting is off) is the only copy of the settings and is parsed back on start.
/// Hosting is on when `always` is ticked, or `auto` is ticked and Hebnix is
/// connected to the multiplayer network.
#[derive(Default)]
struct LanHost {
    loaded: bool,
    always: bool,
    auto: bool,
    team_size: usize,
    bots: usize,
    /// chosen option of each game setting in `lan_mutators::SETTINGS`
    picks: Vec<usize>,
    /// tags no setting claims, comma separated, kept as written
    extra: String,
}

impl LanHost {
    fn load(&mut self, cooked: &Path) {
        self.loaded = true;
        let text = workshop_menu::read_host(cooked).unwrap_or_default();
        self.always = text.contains(ALWAYS);
        self.auto = text.contains(AUTO);
        let tags = text.split('?').next().unwrap_or_default();
        (self.team_size, self.bots) = (3, 0);
        let mut rest = Vec::new();
        for tag in tags.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            if let Some(n) = tag.strip_prefix("PlayerCount").and_then(|n| n.parse::<usize>().ok()) {
                self.team_size = (n / 2).clamp(1, 4);
            } else if let Some(i) = BOTS.iter().position(|b| *b == tag) {
                self.bots = i;
            } else {
                rest.push(tag);
            }
        }
        let (picks, extra) = lan_mutators::parse(rest.into_iter());
        self.picks = picks;
        self.extra = extra.join(",");
    }

    fn file_text(&self) -> String {
        let mut tags: Vec<String> = lan_mutators::tags(&self.picks)
            .map(str::to_string)
            .chain(self.extra.split(',').map(|t| t.trim().replace('?', "")).filter(|t| !t.is_empty()))
            .collect();
        tags.push(BOTS[self.bots].to_string());
        tags.push(format!("PlayerCount{}", self.team_size * 2));
        let flags = [(self.always, ALWAYS), (self.auto, AUTO)];
        let flags: String = flags.iter().filter(|(set, _)| *set).map(|(_, flag)| *flag).collect();
        format!("{}{LISTEN}{flags}", tags.join(","))
    }

    /// Write the settings and switch the host file on or off to match.
    fn sync(&self, cooked: &Path, connected: bool) -> Result<bool, String> {
        workshop_menu::store_host(cooked, &self.file_text(), self.always || (self.auto && connected))
    }
}

/// Switch LAN hosting for a connection change, whichever tab is showing.
pub fn sync_lan(cooked: &Path, connected: bool) {
    if !cooked.is_dir() {
        return;
    }
    let mut lan = LanHost::default();
    lan.load(cooked);
    // the map list tells the menu whether a plain pick hosts
    if let Ok(true) = lan.sync(cooked, connected) {
        let _ = refresh(cooked);
    }
}

/// Follow the game's Launch.log for the LAN browser's sync requests.
pub fn watch_ticks() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let log = dirs::document_dir().map(|d| d.join("My Games/Rocket League/TAGame/Logs/Launch.log"));
            // ticks from an earlier game session were already applied
            let mut seen = log.as_ref().and_then(|l| fs::metadata(l).ok()).map_or(0, |m| m.len());
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(log) = &log else { continue };
                let Ok(len) = fs::metadata(log).map(|m| m.len()) else { continue };
                // a new game launch starts a new log
                if len < seen {
                    seen = 0;
                }
                if len == seen {
                    continue;
                }
                let Ok(mut file) = fs::File::open(log) else { continue };
                if std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(seen)).is_err() {
                    continue;
                }
                let mut bytes = Vec::new();
                if file.read_to_end(&mut bytes).is_err() {
                    continue;
                }
                // a line the game is still writing waits for the next pass
                let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else { continue };
                seen += end as u64 + 1;
                let text = String::from_utf8_lossy(&bytes[..end]);
                queue_sync(&text);
            }
        });
    });
}

static SYNC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// the LAN browser's sync button logs owner|server|map of the selected row; the
/// map title is what is kept for the multiplayer code to pick up
fn queue_sync(text: &str) {
    let Some(row) = text.lines().rev().find_map(|line| line.split_once("HBXSYNC: [GFx] ").map(|(_, row)| row)) else {
        return;
    };
    let title = row.trim().rsplit('|').next().unwrap_or_default().to_string();
    if let Ok(mut request) = SYNC.lock() {
        *request = Some(title);
    }
    crate::toast::report("slot-sync", "Syncing map slots...", None, false);
}

/// the map title of a sync asked for in the game, once
pub fn take_sync_request() -> Option<String> {
    SYNC.lock().ok()?.take()
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// Map files in the folder and in each of its subfolders, the layout Steam
/// uses (one folder per workshop item, named by its id, with an optional .vdf).
fn scan(folder: &Path) -> Vec<WorkshopMap> {
    let mut dirs: Vec<PathBuf> = files(folder).into_iter().filter(|p| p.is_dir()).collect();
    dirs.insert(0, folder.to_path_buf());
    let mut maps = Vec::new();
    for dir in dirs {
        let files = files(&dir);
        let vdf = files
            .iter()
            .find(|p| extension(p) == "vdf")
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|text| parse_workshop_vdf(&text).ok());
        let meta: serde_json::Value = fs::read_to_string(dir.join(META))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let field = |key: &str| meta.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u64>().ok());
        for path in files.iter().cloned() {
            if !matches!(extension(&path).as_str(), "udk" | "umap") {
                continue;
            }
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let vdf_field = |get: fn(&super::local_import::VdfItem) -> &String| {
                vdf.as_ref().map(|v| get(v).clone()).unwrap_or_default()
            };
            let pick = |a: String, b: String| if a.is_empty() { b } else { a };
            let title = pick(pick(field("title"), vdf_field(|v| &v.title)), stem.to_string());
            // non-Steam folders get a stable id from their path
            let id = id.unwrap_or_else(|| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                path.hash(&mut hash);
                hash.finish() >> 11
            });
            maps.push(WorkshopMap {
                id,
                title,
                author: pick(field("author"), vdf_field(|v| &v.author)),
                description: pick(field("description"), vdf_field(|v| &v.description)),
                preview_url: field("preview_url"),
                path,
            });
        }
    }
    maps
}

/// A profile's display name, from its public xml page (no api key needed).
/// Lookups now and then fail for a moment, so retry with growing waits.
fn persona(steam_id: &str) -> Option<String> {
    [0, 1, 2, 4].into_iter().find_map(|wait| {
        std::thread::sleep(Duration::from_secs(wait));
        let response = ureq::get(&format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1"))
            .timeout(Duration::from_secs(15))
            .call()
            .ok()?;
        let mut body = String::new();
        response.into_reader().take(256 * 1024).read_to_string(&mut body).ok()?;
        persona_from_xml(&body)
    })
}

fn persona_from_xml(xml: &str) -> Option<String> {
    let name = xml.split("<steamID><![CDATA[").nth(1)?.split("]]>").next()?;
    (!name.trim().is_empty()).then(|| name.to_string())
}

/// Names found earlier, by SteamID64, so a failed lookup keeps the name.
const AUTHORS: &str = "WorkshopMenu.authors.json";

/// Fill in what the local files lack from the Steam workshop: the author
/// always, the title and description only when there was no vdf, and the
/// preview image the game downloads itself.
fn from_steam(maps: &mut [WorkshopMap], cooked: &Path) {
    let cache = cooked.join("Backups").join(AUTHORS);
    let mut authors: std::collections::HashMap<String, String> = fs::read_to_string(&cache)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    // maps from Hebnix's folder already carry everything. one request each, so a big
    // library goes eight at a time rather than one after the other
    let ids: Vec<String> = maps.iter().filter(|m| m.preview_url.is_empty()).map(|m| m.id.to_string()).collect();
    let mut fetched = Vec::with_capacity(ids.len());
    for batch in ids.chunks(8) {
        std::thread::scope(|scope| {
            let jobs: Vec<_> = batch.iter().map(|id| scope.spawn(|| fetch_details(id))).collect();
            fetched.extend(jobs.into_iter().map(|job| job.join().ok().and_then(Result::ok)));
        });
    }
    let mut fetched = fetched.into_iter();
    for map in maps.iter_mut().filter(|m| m.preview_url.is_empty()) {
        let Some(Some(details)) = fetched.next() else { continue };
        if map.author.is_empty() {
            match persona(&details.creator) {
                Some(name) => {
                    authors.insert(details.creator.clone(), name.clone());
                    map.author = name;
                }
                None => map.author = authors.get(&details.creator).cloned().unwrap_or_default(),
            }
        }
        if map.path.file_stem().is_some_and(|s| *s == *map.title) {
            map.title = details.title;
        }
        if map.description.is_empty() {
            map.description = details.description;
        }
        // the game cannot decode some full-size uploads (a 1 MB png failed);
        // Steam's image CDN resizes and re-encodes to jpeg on request
        if !details.preview_url.is_empty() {
            map.preview_url = format!("{}?imw=512&imh=512&ima=fit&impolicy=Letterbox", details.preview_url);
        }
    }
    if let Ok(text) = serde_json::to_string(&authors) {
        let _ = fs::write(&cache, text);
    }
}

/// The maps in the game's mods folder.
fn all_maps(cooked: &Path) -> Vec<WorkshopMap> {
    scan(&workshop_menu::maps_folder(cooked))
}

/// List each ticked map by its LAN slot copy. Hosting needs no patch, only the copy.
fn slot_for_lan(cooked: &Path, maps: &[WorkshopMap]) {
    let report = crate::patcher::workshop_slots::assign_host(cooked, maps);
    if report.skipped > 0 {
        crate::toast::report(
            "slot-full",
            &format!("{} maps have no LAN slot (128 max), untick some to host them", report.skipped),
            None,
            true,
        );
    }
}

/// Make the maps in one workshop-folder directory joinable over LAN: copy (or
/// hardlink) them into their slots and refresh the in-game list if it is on.
pub(super) fn host_lan(cooked: &Path, dir: &str) -> Result<(), String> {
    let folder = workshop_menu::maps_folder(cooked).join(dir);
    let maps = all_maps(cooked);
    let mine: Vec<_> = maps.iter().filter(|m| m.path.starts_with(&folder)).collect();
    if mine.is_empty() {
        return Err("the map is not in the Workshop folder".into());
    }
    for map in &mine {
        crate::patcher::workshop_slots::set_ticked(cooked, &map.path, true)?;
    }
    slot_for_lan(cooked, &maps);
    refresh(cooked)?;
    // the path a right-click in the game's list would load: the slot copy, with the host settings named last
    let slot = mine
        .iter()
        .find_map(|m| crate::patcher::workshop_slots::filled(cooked, &m.path))
        .ok_or("the map is not in a LAN slot yet (all 128 are used, or the game holds its slot)")?;
    if workshop_menu::read_host(cooked).is_none() {
        return Err("LAN host settings are missing, open the in-game Workshop menu settings once".into());
    }
    let host = workshop_menu::host_path(cooked);
    let text = format!("{}?{}", slot.display(), host.file_name().unwrap_or_default().to_string_lossy());
    let run = crate::patcher::workshop_host::run_file(cooked);
    std::fs::write(&run, text).map_err(|e| format!("Could not write {}: {e}", run.display()))
}

/// Rebuild the in-game list from the folder the menu was enabled with.
/// Returns Ok(None) when the menu is not enabled.
pub(super) fn refresh(cooked: &Path) -> Result<Option<usize>, String> {
    if workshop_menu::enabled_folder(&cooked.join("Backups")).is_none() {
        return Ok(None);
    }
    let mut maps = all_maps(cooked);
    from_steam(&mut maps, cooked);
    slot_for_lan(cooked, &maps);
    workshop_menu::write_list(cooked, &maps)?;
    Ok(Some(maps.len()))
}

impl InGameMenuState {
    fn spawn(&mut self, ctx: &egui::Context, job: impl FnOnce() -> Job + Send + 'static) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    fn poll(&mut self, cooked: &Path, ctx: &egui::Context) {
        let Some(rx) = &self.pending else { return };
        let Ok(job) = rx.try_recv() else { return };
        self.pending = None;
        match job {
            Job::Status(enabled) => self.enabled = Some(enabled),
            Job::Done(result) => {
                self.status = result.unwrap_or_else(|e| e);
                let cooked = cooked.to_path_buf();
                self.spawn(ctx, move || Job::Status(workshop_menu::status(&cooked)));
            }
        }
    }

    /// Collect a finished job and check the game's menu state once per install.
    pub(super) fn tick(&mut self, ctx: &egui::Context, cooked: &Path) {
        self.poll(cooked, ctx);
        if self.install.as_deref() != Some(cooked) && self.pending.is_none() {
            self.install = Some(cooked.to_path_buf());
            self.enabled = None;
            let cooked = cooked.to_path_buf();
            self.spawn(ctx, move || Job::Status(workshop_menu::status(&cooked)));
        }
    }

    /// Whether the menu is enabled, None while that is still being checked.
    pub(super) fn is_enabled(&self) -> Option<bool> {
        match &self.enabled {
            Some(Ok(on)) => Some(*on),
            _ => None,
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Patch the game to show the menu (or take it out again) and list the maps.
    pub(super) fn start(&mut self, ctx: &egui::Context, cooked: &Path, enable: bool, tx: &Sender<AppMsg>) {
        if self.busy() || crate::messages::block_item_action_if_game_running(tx) {
            return;
        }
        self.status = "Patching TAGame.upk...".into();
        let (cooked, backups) = (cooked.to_path_buf(), cooked.join("Backups"));
        if enable {
            let folder = workshop_menu::maps_folder(&cooked);
            let mut maps = all_maps(&cooked);
            self.spawn(ctx, move || {
                from_steam(&mut maps, &cooked);
                slot_for_lan(&cooked, &maps);
                Job::Done(workshop_menu::apply(&cooked, &backups, &folder, &maps))
            });
        } else {
            self.spawn(ctx, move || Job::Done(workshop_menu::restore(&cooked, &backups)));
        }
    }

    /// `lan_only` drops the map list and the menu patch controls, leaving the LAN hosting settings.
    pub fn render(&mut self, ui: &mut egui::Ui, rl_path: &str, connected: bool, lan_only: bool, tx: &Sender<AppMsg>) {
        let ctx = ui.ctx().clone();
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        self.tick(&ctx, &cooked);
        if self.folder.is_empty() {
            self.folder = workshop_menu::maps_folder(&cooked).to_string_lossy().into_owned();
            self.maps = all_maps(&cooked);
        }

        if !lan_only {
        ui.heading("In-game Workshop menu");
        ui.label(
            "Shows Rocket League's own Workshop button on the Epic version (Play, then the \
             offline modes) and lists the maps from a local folder, such as a Steam \
             library's workshop folder. Close Rocket League before changing it.",
        );
        ui.add_space(6.0);
        ui.label(match &self.enabled {
            None => "Status: checking TAGame.upk...".to_string(),
            Some(Ok(true)) => "Status: enabled".to_string(),
            Some(Ok(false)) => "Status: not enabled".to_string(),
            Some(Err(e)) => format!("Status: {e}"),
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(format!("Maps are kept in {}", self.folder));
            if ui.button("Rescan").clicked() {
                self.maps = all_maps(&cooked);
            }
        });
        ui.label(format!("{} maps found.", self.maps.len()));
        let mut toggled = None;
        if !self.maps.is_empty() {
            let ticked = crate::patcher::workshop_slots::ticked(&cooked);
            let is_on = |map: &WorkshopMap| ticked.contains(&map.path.to_string_lossy().into_owned());
            let name = |map: &WorkshopMap| format!("{}{}", if is_on(map) { "[LAN] " } else { "" }, map.title);
            self.selected = self.selected.min(self.maps.len() - 1);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("ingame_menu_maps")
                    .selected_text(name(&self.maps[self.selected]))
                    .show_ui(ui, |ui| {
                        for (index, map) in self.maps.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, index, name(map));
                        }
                    });
                let map = &self.maps[self.selected];
                let on = is_on(map);
                let text = if on { "Stop hosting over LAN" } else { "Host over LAN" };
                if ui.button(text).clicked() {
                    toggled = Some((map.path.clone(), !on));
                }
            });
        }
            if let Some((path, on)) = toggled {
                let cooked = cooked.clone();
                self.spawn(&ctx, move || {
                    let done = crate::patcher::workshop_slots::set_ticked(&cooked, &path, on).and_then(|()| refresh(&cooked));
                    Job::Done(done.map(|_| if on { "Map ticked for LAN.".into() } else { "Map unticked for LAN.".into() }))
                });
            }
        ui.add_space(6.0);
        let busy = self.pending.is_some();
        ui.horizontal(|ui| {
            let enable = ui.add_enabled(
                !busy && !self.maps.is_empty(),
                egui::Button::new("Enable / update map list"),
            );
            let disable = ui.add_enabled(
                !busy && matches!(self.enabled, Some(Ok(true))),
                egui::Button::new("Disable"),
            );
            if enable.clicked() {
                self.start(&ctx, &cooked, true, tx);
            } else if disable.clicked() {
                self.start(&ctx, &cooked, false, tx);
            }
            if busy {
                ui.spinner();
            }
        });
        if !self.status.is_empty() {
            ui.label(&self.status);
        }

        if !self.lan.loaded {
            self.lan.load(&cooked);
        }
        ui.add_space(10.0);
        ui.strong("Host LAN matches from the Workshop menu");
        ui.label(
            "When on, picking a map in the in-game Workshop menu hosts a LAN match on it \
             with these settings instead of freeplay. Players join from Rocket League's LAN \
             browser and need the same map. Works while the game runs.",
        );
        let lan = &mut self.lan;
        let mut changed = ui.checkbox(&mut lan.always, "Always host a LAN match when I pick a map").changed();
        changed |= ui
            .checkbox(&mut lan.auto, "Host a LAN match while connected to Hebnix multiplayer")
            .changed();
        ui.add_enabled_ui(lan.always || lan.auto, |ui| {
            ui.horizontal(|ui| {
                ui.label("Team size");
                egui::ComboBox::from_id_salt("workshop_lan_team_size")
                    .selected_text(format!("{0}v{0}", lan.team_size))
                    .show_ui(ui, |ui| {
                        for n in 1..=4 {
                            changed |= ui.selectable_value(&mut lan.team_size, n, format!("{n}v{n}")).changed();
                        }
                    });
                ui.label("Bots");
                egui::ComboBox::from_id_salt("workshop_lan_bots")
                    .selected_text(BOTS[lan.bots].trim_start_matches("Bots"))
                    .show_ui(ui, |ui| {
                        for (i, bots) in BOTS.iter().enumerate() {
                            changed |= ui.selectable_value(&mut lan.bots, i, bots.trim_start_matches("Bots")).changed();
                        }
                    });
            });
            ui.collapsing("Game settings", |ui| {
                egui::Grid::new("workshop_lan_settings").num_columns(2).show(ui, |ui| {
                    for (i, setting) in SETTINGS.iter().enumerate() {
                        ui.label(setting.label);
                        egui::ComboBox::from_id_salt(("workshop_lan_setting", i))
                            .selected_text(setting.options[lan.picks[i]].0)
                            .show_ui(ui, |ui| {
                                for (j, (label, _)) in setting.options.iter().enumerate() {
                                    changed |= ui.selectable_value(&mut lan.picks[i], j, *label).changed();
                                }
                            });
                        ui.end_row();
                    }
                });
                if !lan.extra.is_empty() {
                    ui.label(format!("Other tags kept: {}", lan.extra));
                }
                if ui.button("Reset to defaults").clicked() {
                    lan.picks = vec![0; SETTINGS.len()];
                    lan.extra.clear();
                    changed = true;
                }
            });
        });
        if changed {
            match lan.sync(&cooked, connected) {
                Err(e) => self.status = e,
                Ok(true) => drop(refresh(&cooked)),
                Ok(false) => {}
            }
        }
        for line in crate::patcher::workshop_lan_trial::offer_status(&cooked) {
            ui.label(format!("Peer map: {line}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::persona_from_xml;

    #[test]
    fn lan_host_file_round_trips() {
        let cooked = std::env::temp_dir().join("hebnix_lan_host_test").join("TAGame").join("CookedPCConsole");
        std::fs::create_dir_all(&cooked).unwrap();
        let (picks, extra) = super::lan_mutators::parse("10Minutes,Ball_Puck,Lan".split(','));
        let host = super::LanHost { team_size: 2, bots: 3, picks, extra: extra.join(","), auto: true, ..Default::default() };
        assert_eq!(host.file_text(), format!("10Minutes,Ball_Puck,Lan,BotsHard,PlayerCount4{}{}", super::LISTEN, super::AUTO));
        // auto hosting follows the connection; the settings survive being parked
        host.sync(&cooked, false).unwrap();
        assert!(!crate::patcher::workshop_menu::host_on(&cooked));
        host.sync(&cooked, true).unwrap();
        assert!(crate::patcher::workshop_menu::host_on(&cooked));
        host.sync(&cooked, false).unwrap();
        let mut back = super::LanHost::default();
        back.load(&cooked);
        assert!(back.auto && !back.always && back.team_size == 2 && back.bots == 3 && back.extra == "Lan");
        assert_eq!(back.picks, host.picks);
        let off = super::LanHost { auto: false, always: true, ..Default::default() };
        off.sync(&cooked, false).unwrap();
        back.load(&cooked);
        assert!(!back.auto && back.always);
        assert!(crate::patcher::workshop_menu::host_on(&cooked));
        crate::patcher::workshop_menu::set_host(&cooked, None).unwrap();
        let _ = std::fs::remove_dir_all(cooked.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn persona_is_read_from_profile_xml() {
        let xml = "<profile><steamID64>1</steamID64><steamID><![CDATA[dmc]]></steamID></profile>";
        assert_eq!(persona_from_xml(xml).as_deref(), Some("dmc"));
        assert_eq!(persona_from_xml("<profile></profile>"), None);
    }
}

