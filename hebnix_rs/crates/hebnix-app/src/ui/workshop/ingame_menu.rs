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
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
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

const BOTS: [&str; 5] = [
    "BotsNone",
    "BotsEasy",
    "BotsMedium",
    "BotsHard",
    "BotsUnfair",
];
/// Options after the game tags, as Rocket Plugin hosts with.
const LISTEN: &str = "?Playtest?NumPublicConnections=10?NumOpenPublicConnections=10?Lan?Listen";

/// LAN hosting from the Workshop menu. The host file (or its parked copy while
/// hosting is off) is the only copy of the settings and is parsed back on start.
/// Hosting is on when `always` is ticked, or `auto` is ticked and Hebnix is
/// connected to the multiplayer network.
#[derive(Default)]
struct LanHost {
    loaded: bool,
    override_credentials: bool,
    name: String,
    password: String,
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
        let credentials = crate::patcher::workshop_host::read_lan_credentials(cooked);
        self.override_credentials = credentials.is_some();
        (self.name, self.password) = credentials.unwrap_or_default();
        let text = workshop_menu::read_host(cooked).unwrap_or_default();
        let tags = text.split('?').next().unwrap_or_default();
        (self.team_size, self.bots) = (3, 0);
        let mut rest = Vec::new();
        for tag in tags.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            if let Some(n) = tag
                .strip_prefix("PlayerCount")
                .and_then(|n| n.parse::<usize>().ok())
            {
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
        let mut tags: Vec<String> = lan_mutators::travel_tags(&self.picks)
            .map(str::to_string)
            .chain(
                self.extra
                    .split(',')
                    .map(|t| t.trim().replace('?', ""))
                    .filter(|t| !t.is_empty()),
            )
            .collect();
        tags.push(BOTS[self.bots].to_string());
        tags.push(format!("PlayerCount{}", self.team_size * 2));
        format!("{}{LISTEN}", tags.join(","))
    }

    /// Save the settings read by the in-game Host LAN launch path.
    fn sync(&self, cooked: &Path) -> Result<bool, String> {
        let heatseeker = lan_mutators::heatseeker(&self.picks);
        workshop_menu::store_host(cooked, &self.file_text(), false, heatseeker)
    }
}

/// Rewrite the host settings on a connection change, whichever tab is showing, which
/// also clears the always/auto hosting older builds saved.
pub fn sync_lan(cooked: &Path, _connected: bool) {
    if !cooked.is_dir() {
        return;
    }
    let mut lan = LanHost::default();
    lan.load(cooked);
    // the map list tells the menu whether a plain pick hosts
    if let Ok(true) = lan.sync(cooked) {
        let _ = refresh(cooked);
    }
}

/// Follow the game's Launch.log for the LAN browser's sync requests.
pub fn watch_ticks() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let log = dirs::document_dir()
                .map(|d| d.join("My Games/Rocket League/TAGame/Logs/Launch.log"));
            // ticks from an earlier game session were already applied
            let mut seen = log
                .as_ref()
                .and_then(|l| fs::metadata(l).ok())
                .map_or(0, |m| m.len());
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(log) = &log else { continue };
                let Ok(len) = fs::metadata(log).map(|m| m.len()) else {
                    continue;
                };
                // a new game launch starts a new log
                if len < seen {
                    seen = 0;
                }
                if len == seen {
                    continue;
                }
                let Ok(mut file) = fs::File::open(log) else {
                    continue;
                };
                if std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(seen)).is_err() {
                    continue;
                }
                let mut bytes = Vec::new();
                if file.read_to_end(&mut bytes).is_err() {
                    continue;
                }
                // a line the game is still writing waits for the next pass
                let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
                    continue;
                };
                seen += end as u64 + 1;
                let text = String::from_utf8_lossy(&bytes[..end]);
                queue_sync(&text);
                queue_downloads(&text);
            }
        });
    });
}

static SYNC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// the LAN browser's sync button logs owner|server|map of the selected row; the
/// map title is what is kept for the multiplayer code to pick up
fn queue_sync(text: &str) {
    let Some(row) = text
        .lines()
        .rev()
        .find_map(|line| line.split_once("HBXSYNC: [GFx] ").map(|(_, row)| row))
    else {
        return;
    };
    let title = row
        .trim()
        .rsplit('|')
        .next()
        .unwrap_or_default()
        .to_string();
    if let Ok(mut request) = SYNC.lock() {
        *request = Some(title);
    }
    crate::toast::report("slot-sync", "Syncing map slots...", None, false);
}

static DOWNLOADS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// the Workshop menu's downloader logs the catalogue id of each map picked
fn queue_downloads(text: &str) {
    let Ok(mut queue) = DOWNLOADS.lock() else {
        return;
    };
    for id in text
        .lines()
        .filter_map(|line| line.split_once("HBXDL: [GFx] ").map(|(_, id)| id.trim()))
    {
        if !id.is_empty() && !queue.iter().any(|queued| queued == id) {
            queue.push(id.to_string());
        }
    }
}

/// catalogue ids picked in the game's downloader since the last call
pub fn take_download_requests() -> Vec<String> {
    DOWNLOADS
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
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
        let field = |key: &str| {
            meta.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
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
            let title = pick(
                pick(field("title"), vdf_field(|v| &v.title)),
                stem.to_string(),
            );
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
        let response = ureq::get(&format!(
            "https://steamcommunity.com/profiles/{steam_id}/?xml=1"
        ))
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?;
        let mut body = String::new();
        response
            .into_reader()
            .take(256 * 1024)
            .read_to_string(&mut body)
            .ok()?;
        persona_from_xml(&body)
    })
}

fn persona_from_xml(xml: &str) -> Option<String> {
    let name = xml
        .split("<steamID><![CDATA[")
        .nth(1)?
        .split("]]>")
        .next()?;
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
    let ids: Vec<String> = maps
        .iter()
        .filter(|m| m.preview_url.is_empty())
        .map(|m| m.id.to_string())
        .collect();
    let mut fetched = Vec::with_capacity(ids.len());
    for batch in ids.chunks(8) {
        std::thread::scope(|scope| {
            let jobs: Vec<_> = batch
                .iter()
                .map(|id| scope.spawn(|| fetch_details(id)))
                .collect();
            fetched.extend(
                jobs.into_iter()
                    .map(|job| job.join().ok().and_then(Result::ok)),
            );
        });
    }
    let mut fetched = fetched.into_iter();
    for map in maps.iter_mut().filter(|m| m.preview_url.is_empty()) {
        let Some(Some(details)) = fetched.next() else {
            continue;
        };
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
            map.preview_url = format!(
                "{}?imw=512&imh=512&ima=fit&impolicy=Letterbox",
                details.preview_url
            );
        }
    }
    if let Ok(text) = serde_json::to_string(&authors) {
        let _ = fs::write(&cache, text);
    }
}

/// The maps in the game's mods folder.
pub(crate) fn all_maps(cooked: &Path) -> Vec<WorkshopMap> {
    scan(&workshop_menu::maps_folder(cooked))
}

/// List each ticked map by its LAN slot copy. Hosting needs no patch, only the copy.
fn slot_for_lan(cooked: &Path, maps: &[WorkshopMap]) {
    let report = crate::patcher::workshop_slots::assign_host(cooked, maps);
    if report.skipped > 0 {
        crate::toast::report(
            "slot-full",
            &format!(
                "{} maps have no LAN slot (128 max), untick some to host them",
                report.skipped
            ),
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
    let mine: Vec<_> = maps
        .iter()
        .filter(|m| m.path.starts_with(&folder))
        .collect();
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
        return Err(
            "LAN host settings are missing, open the in-game Workshop menu settings once".into(),
        );
    }
    let host = workshop_menu::host_path(cooked);
    let text = format!(
        "{}?{}",
        slot.display(),
        host.file_name().unwrap_or_default().to_string_lossy()
    );
    let run = crate::patcher::workshop_host::run_file(cooked);
    std::fs::write(&run, text).map_err(|e| format!("Could not write {}: {e}", run.display()))
}

/// Rebuild the list for an active Workshop loader.
/// Returns Ok(None) when the menu is not enabled.
pub(crate) fn refresh(cooked: &Path) -> Result<Option<usize>, String> {
    if !workshop_menu::status(cooked)? {
        return Ok(None);
    }
    let mut maps = all_maps(cooked);
    from_steam(&mut maps, cooked);
    slot_for_lan(cooked, &maps);
    workshop_menu::write_list(cooked, &maps)?;
    Ok(Some(maps.len()))
}

fn change_downloader(cooked: &Path, enabled: bool) -> Result<String, String> {
    if hebnix_sdk::process::is_rocket_league_running() || !workshop_menu::status(cooked)? {
        return Err(
            "Enable the Workshop menu and close Rocket League before changing the downloader."
                .into(),
        );
    }
    let previous = workshop_menu::downloader_on(cooked);
    workshop_menu::set_downloader(cooked, enabled)?;
    if let Err(error) =
        workshop_menu::update_downloader(cooked).and_then(|()| refresh(cooked).map(|_| ()))
    {
        workshop_menu::set_downloader(cooked, previous)?;
        workshop_menu::update_downloader(cooked)?;
        return Err(error);
    }
    Ok(if enabled {
        "Map downloader enabled."
    } else {
        "Map downloader disabled."
    }
    .into())
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

    pub(super) fn tick(&mut self, cooked: &Path, ctx: &egui::Context) {
        let previous = self.install.clone();
        self.poll(previous.as_deref().unwrap_or(cooked), ctx);
        if self.install.as_deref() != Some(cooked) && self.pending.is_none() {
            self.install = Some(cooked.to_owned());
            self.enabled = None;
            let cooked = cooked.to_owned();
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
    pub(super) fn start(
        &mut self,
        ctx: &egui::Context,
        cooked: &Path,
        enable: bool,
        tx: &Sender<AppMsg>,
    ) {
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
            self.spawn(ctx, move || {
                Job::Done(workshop_menu::restore(&cooked, &backups))
            });
        }
    }

    /// `lan_only` drops the map list and the menu patch controls, leaving the LAN hosting settings.
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        rl_path: &str,
        _connected: bool,
        lan_only: bool,
        tx: &Sender<AppMsg>,
    ) {
        let ctx = ui.ctx().clone();
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        self.tick(&cooked, &ctx);
        if self.folder.is_empty() {
            self.folder = workshop_menu::maps_folder(&cooked)
                .to_string_lossy()
                .into_owned();
            self.maps = all_maps(&cooked);
        }

        if !self.lan.loaded {
            self.lan.load(&cooked);
        }
        let mut changed = false;
        ui.columns(2, |columns| {
            let [left, right] = columns else { unreachable!() };
            egui::ScrollArea::vertical().id_salt("workshop_menu_controls").show(left, |ui| {
                if !lan_only {
                    crate::i18n::layout::bordered_panel(ui, |ui| {
                        ui.heading("In-game Workshop menu");
                        ui.label("Adds Workshop to Rocket League’s offline Play menu. Close Rocket League before changing it.");
                        let busy = self.busy();
                        let mut menu_on = self.is_enabled().unwrap_or(false);
                        if ui.add_enabled(!busy && self.is_enabled().is_some(), egui::Checkbox::new(&mut menu_on, "Enable in-game menu")).changed() {
                            self.start(&ctx, &cooked, menu_on, tx);
                        }
                        if ui.add_enabled(!busy && self.is_enabled() == Some(true), egui::Button::new("Update Workshop patch"))
                            .on_hover_text("Close Rocket League before updating the in-game menu patch.")
                            .clicked() && !crate::messages::block_item_action_if_game_running(tx) {
                            let cooked = cooked.clone();
                            self.spawn(&ctx, move || Job::Done(workshop_menu::upgrade_movie(&cooked)
                                .map(|()| "Workshop patch updated. Start Rocket League to use it.".into())));
                        }
                        if let Some(Err(error)) = &self.enabled {
                            ui.label(error);
                        }
                        if busy {
                            ui.spinner();
                        }
                        if !self.status.is_empty() {
                            ui.label(&self.status);
                        }
                    });
                    ui.add_space(8.0);
                    crate::i18n::layout::bordered_panel(ui, |ui| {
                        ui.heading("Map downloader");
                        let mut downloader = workshop_menu::downloader_on(&cooked);
                        if ui.add_enabled(
                            self.is_enabled() == Some(true) && !self.busy(),
                            egui::Checkbox::new(&mut downloader, "Enable map downloader"),
                        ).changed() && !crate::messages::block_item_action_if_game_running(tx) {
                            let cooked = cooked.clone();
                            self.spawn(&ctx, move || Job::Done(change_downloader(&cooked, downloader)));
                        }
                        ui.weak("With the downloader enabled, press L3 in the Workshop menu to browse maps.");
                    });
                    ui.add_space(8.0);
                    crate::i18n::layout::bordered_panel(ui, |ui| {
                        ui.heading("Local maps");
                        ui.label(&self.folder);
                        if ui.button("Rescan").clicked() {
                            self.maps = all_maps(&cooked);
                        }
                        ui.label(format!("{} maps found.", self.maps.len()));
                    });
                    ui.add_space(8.0);
                }
                crate::i18n::layout::bordered_panel(ui, |ui| {
                    ui.heading("LAN match");
                    ui.label("Used when you host a map from the in-game Workshop menu.");
                    let lan = &mut self.lan;
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
                    for line in crate::patcher::workshop_lan_trial::offer_status(&cooked) {
                        ui.label(format!("Peer map: {line}"));
                    }
                });
                ui.add_space(8.0);
                crate::i18n::layout::bordered_panel(ui, |ui| {
                    ui.heading("LAN match access");
                    let lan = &mut self.lan;
                    ui.checkbox(&mut lan.override_credentials, "Use custom LAN credentials");
                    ui.add_enabled_ui(lan.override_credentials, |ui| {
                        ui.label("Match name");
                        ui.add(egui::TextEdit::singleline(&mut lan.name).char_limit(32).hint_text("Game default"));
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut lan.password).char_limit(32).password(true).hint_text("No password"));
                    });
                    if ui.button("Save LAN match access").clicked() {
                        let credentials = lan.override_credentials.then_some((lan.name.trim(), lan.password.as_str()));
                        let result = crate::patcher::workshop_host::store_lan_credentials(&cooked, credentials);
                        self.status = result.map(|()| "LAN match access saved. Host a new match to refresh the server listing.".into())
                            .unwrap_or_else(|error| error);
                    }
                });
            });
            crate::i18n::layout::bordered_panel(right, |ui| {
                ui.heading("LAN Mutators");
                egui::ScrollArea::vertical().id_salt("workshop_game_settings").show(ui, |ui| {
                    let lan = &mut self.lan;
                ui.weak("These settings are used when hosting a LAN match. Heatseeker needs two Soccar goals.");
                if lan_only && !self.status.is_empty() {
                    ui.label(&self.status);
                }
                changed |= lan_mutators::render_picks(ui, "workshop_lan_settings", &mut lan.picks, |_| true);
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
        });
        let lan = &mut self.lan;
        if changed {
            match lan.sync(&cooked) {
                Err(e) => self.status = e,
                Ok(true) => drop(refresh(&cooked)),
                Ok(false) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::persona_from_xml;

    #[test]
    fn lan_host_file_round_trips() {
        let cooked = std::env::temp_dir()
            .join("hebnix_lan_host_test")
            .join("TAGame")
            .join("CookedPCConsole");
        std::fs::create_dir_all(&cooked).unwrap();
        let (picks, extra) = super::lan_mutators::parse("10Minutes,Ball_Puck,Lan".split(','));
        let host = super::LanHost {
            team_size: 2,
            bots: 3,
            picks,
            extra: extra.join(","),
            ..Default::default()
        };
        assert_eq!(
            host.file_text(),
            format!(
                "10Minutes,Ball_Puck,Lan,BotsHard,PlayerCount4{}",
                super::LISTEN
            )
        );
        // the settings are kept for the Host LAN button, and a plain pick never hosts
        crate::patcher::workshop_menu::store_solo(&cooked, Some("Freeplay,LowGravity"), false).unwrap();
        host.sync(&cooked).unwrap();
        assert!(!crate::patcher::workshop_menu::host_on(&cooked));
        assert_eq!(crate::patcher::workshop_menu::read_solo(&cooked).as_deref(), Some("Freeplay,LowGravity"));
        crate::patcher::workshop_menu::store_solo_preferences(&cooked, false, "Freeplay,LowGravity", false).unwrap();
        assert!(crate::patcher::workshop_menu::read_solo(&cooked).is_none());
        assert_eq!(
            crate::patcher::workshop_menu::read_solo_preferences(&cooked),
            (false, "Freeplay,LowGravity".into())
        );
        assert_eq!(crate::patcher::workshop_menu::read_host(&cooked), Some(host.file_text()));
        crate::patcher::workshop_menu::store_solo_preferences(&cooked, true, "Freeplay,LowGravity", false).unwrap();
        assert_eq!(
            crate::patcher::workshop_menu::read_solo_preferences(&cooked),
            (true, "Freeplay,LowGravity".into())
        );
        let backend = crate::patcher::workshop_host::store_lan_credentials;
        backend(&cooked, Some(("Practice night", "goal-42"))).unwrap();
        assert_eq!(
            crate::patcher::workshop_host::read_lan_credentials(&cooked),
            Some(("Practice night".into(), "goal-42".into()))
        );
        backend(&cooked, None).unwrap();
        assert!(crate::patcher::workshop_host::read_lan_credentials(&cooked).is_none());
        let mut back = super::LanHost::default();
        back.load(&cooked);
        assert!(back.team_size == 2 && back.bots == 3 && back.extra == "Lan");
        assert_eq!(back.picks, host.picks);
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
