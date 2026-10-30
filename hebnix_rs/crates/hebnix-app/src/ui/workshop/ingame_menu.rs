//! Experimental page that turns on Rocket League's own Workshop menu on the
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
/// Where Steam keeps Rocket League's workshop maps, under its app id.
const WORKSHOP_CONTENT: [&str; 4] = ["steamapps", "workshop", "content", "252950"];

enum Job {
    Status(Result<bool, String>),
    Done(Result<String, String>),
}

#[derive(Default)]
pub struct InGameMenuState {
    folder: String,
    maps: Vec<WorkshopMap>,
    install: Option<PathBuf>,
    enabled: Option<Result<bool, String>>,
    status: String,
    pending: Option<Receiver<Job>>,
    lan: LanHost,
}

/// Hebnix's own flags, as URL options after the settings. The game ignores them.
const ALWAYS: &str = "?HbxAlways";
const NO_AUTO: &str = "?HbxNoAuto";

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
        self.auto = !text.contains(NO_AUTO);
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
        let flags = [(self.always, ALWAYS), (!self.auto, NO_AUTO)];
        let flags: String = flags.iter().filter(|(set, _)| *set).map(|(_, flag)| *flag).collect();
        format!("{}{LISTEN}{flags}", tags.join(","))
    }

    /// Write the settings and switch the host file on or off to match.
    fn sync(&self, cooked: &Path, connected: bool) -> Result<(), String> {
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
    let _ = lan.sync(cooked, connected);
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
    // maps from Hebnix's folder already carry everything
    for map in maps.iter_mut().filter(|m| m.preview_url.is_empty()) {
        let Ok(details) = fetch_details(&map.id.to_string()) else { continue };
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

/// The picked folder's maps, then those in Hebnix's own workshop folder.
fn all_maps(folder: &Path, cooked: &Path) -> Vec<WorkshopMap> {
    let mut maps = scan(folder);
    let fallback = workshop_menu::maps_folder(cooked);
    if fallback.as_path() != folder {
        maps.extend(scan(&fallback));
    }
    maps
}

/// Rebuild the in-game list from the folder the menu was enabled with.
/// Returns Ok(None) when the menu is not enabled.
pub(super) fn refresh(cooked: &Path) -> Result<Option<usize>, String> {
    let Some(folder) = workshop_menu::enabled_folder(&cooked.join("Backups")) else {
        return Ok(None);
    };
    let mut maps = all_maps(&folder, cooked);
    from_steam(&mut maps, cooked);
    workshop_menu::write_list(cooked, &maps)?;
    Ok(Some(maps.len()))
}

/// The first Steam library that has Rocket League workshop maps.
fn steam_workshop_folder() -> Option<PathBuf> {
    let steam = PathBuf::from(r"C:\Program Files (x86)\Steam");
    let vdf = fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    let libraries = vdf
        .lines()
        .filter(|l| l.trim_start().starts_with("\"path\""))
        .filter_map(|l| l.split('"').nth(3))
        .map(|p| PathBuf::from(p.replace("\\\\", "\\")));
    std::iter::once(steam)
        .chain(libraries)
        .map(|library| WORKSHOP_CONTENT.iter().fold(library, |p, part| p.join(part)))
        .find(|folder| folder.is_dir())
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

    pub fn render(&mut self, ui: &mut egui::Ui, rl_path: &str, connected: bool, tx: &Sender<AppMsg>) {
        let ctx = ui.ctx().clone();
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        let backups = cooked.join("Backups");
        self.poll(&cooked, &ctx);
        if self.install.as_ref() != Some(&cooked) && self.pending.is_none() {
            self.install = Some(cooked.clone());
            self.enabled = None;
            let cooked = cooked.clone();
            self.spawn(&ctx, move || Job::Status(workshop_menu::status(&cooked)));
        }
        if self.folder.is_empty() {
            if let Some(folder) = steam_workshop_folder() {
                self.folder = folder.to_string_lossy().into_owned();
                self.maps = all_maps(&folder, &cooked);
            }
        }

        ui.heading("In-game Workshop menu (experimental)");
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
            ui.label("Map folder");
            ui.add(egui::TextEdit::singleline(&mut self.folder).desired_width(420.0));
            if ui.button("Browse…").clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.folder = folder.to_string_lossy().into_owned();
                    self.maps = all_maps(&folder, &cooked);
                }
            }
            if ui.button("Rescan").clicked() {
                self.maps = all_maps(Path::new(&self.folder), &cooked);
            }
        });
        ui.label(format!("{} maps found", self.maps.len()));
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for map in &self.maps {
                    ui.monospace(format!("{}  {}", map.title, map.path.display()));
                }
            });
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
            if (enable.clicked() || disable.clicked())
                && !crate::messages::block_item_action_if_game_running(tx)
            {
                self.status = "Patching TAGame.upk...".into();
                let (cooked, backups) = (cooked.clone(), backups.clone());
                if enable.clicked() {
                    let folder = PathBuf::from(&self.folder);
                    let mut maps = all_maps(&folder, &cooked);
                    self.spawn(&ctx, move || {
                        from_steam(&mut maps, &cooked);
                        Job::Done(workshop_menu::apply(&cooked, &backups, &folder, &maps))
                    });
                } else {
                    self.spawn(&ctx, move || Job::Done(workshop_menu::restore(&cooked, &backups)));
                }
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
            if let Err(e) = lan.sync(&cooked, connected) {
                self.status = e;
            }
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
        assert_eq!(host.file_text(), format!("10Minutes,Ball_Puck,Lan,BotsHard,PlayerCount4{}", super::LISTEN));
        // auto hosting follows the connection; the settings survive being parked
        host.sync(&cooked, false).unwrap();
        assert!(!crate::patcher::workshop_menu::host_path(&cooked).exists());
        host.sync(&cooked, true).unwrap();
        assert!(crate::patcher::workshop_menu::host_path(&cooked).exists());
        host.sync(&cooked, false).unwrap();
        let mut back = super::LanHost::default();
        back.load(&cooked);
        assert!(back.auto && !back.always && back.team_size == 2 && back.bots == 3 && back.extra == "Lan");
        assert_eq!(back.picks, host.picks);
        let off = super::LanHost { auto: false, always: true, ..Default::default() };
        off.sync(&cooked, false).unwrap();
        back.load(&cooked);
        assert!(!back.auto && back.always);
        assert!(crate::patcher::workshop_menu::host_path(&cooked).exists());
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

