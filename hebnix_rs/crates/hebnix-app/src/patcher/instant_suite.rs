//! Instant Suite. When a match ends Rocket League shows the winner prompt
//! (GFX_WinnerMenu_SF, prompts.WinnerPrompt). As it announces the winner, the
//! patched movie reads a short-lived action through the native Shell bridge
//! after the Stats API's MatchEnded:
//!
//!   requeue   this.onQuickRequeue()            queue again, as the Play Again button does
//!   freeplay  this.handleFreeplayRequest()     go to freeplay, as the Free Play button does
//!   menu      Shell.ExitToMainMenu()           leave to the main menu
//!   workshop  Workshop.LoadWorkshopMap(path)   load the picked map from the in-game Workshop list
//!
//! With Hebnix closed nothing happens. Switching the patch on or off still needs
//! the game closed, since it rewrites the package.
use super::upk_package::{UpkPackage, strip};
use super::workshop_menu_ui::Movie;
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PACKAGE: &str = "GFX_WinnerMenu_SF.upk";

const MOVIE: Movie = Movie {
    package: PACKAGE,
    export: "GFX_WinnerMenu.WinnerMenu",
    patched: include_bytes!("../../assets/instant-suite/winner_menu_ui.swf"),
    vanilla: "28f28cd0d1f94f3d4926a381a903514b4b14d69b90ad73a1676cd1b19cf60069",
};

pub const MODES: [&str; 4] = ["requeue", "freeplay", "menu", "workshop"];
pub const DEFAULT_MODE: &str = "nothing";

/// What the movie is told: an action digit, 0 for nothing.
static CODE: AtomicU8 = AtomicU8::new(0);
static LAST_MATCH: Mutex<Option<String>> = Mutex::new(None);
const COMMAND: &str = "HbxInstant.txt";
const ENABLED: &str = "InstantSuite.enabled";
const ACTION_LIFETIME: Duration = Duration::from_secs(8);

fn permits_match(playlist: Option<i64>, private: bool, tournaments: bool) -> bool {
    match playlist {
        Some(0 | 6) => private,
        Some(22 | 34) => tournaments,
        Some(id) => id >= 0,
        None => false,
    }
}

fn command_path(cooked: &Path) -> PathBuf {
    cooked.parent().unwrap_or(cooked).join(COMMAND)
}

fn command_value(now: SystemTime, action: u8) -> Option<String> {
    (1..=4).contains(&action).then_some(())?;
    let expiry = now.checked_add(ACTION_LIFETIME)?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("{}{action}", expiry.as_millis()))
}

pub fn match_ended(cooked: &Path, guid: Option<&str>, private: bool, tournaments: bool) {
    let action = CODE.load(Ordering::Relaxed);
    if action == 0 || !permits_match(hebnix_sdk::log::current_match_playlist(), private, tournaments) {
        clear_match_end(cooked);
        return;
    }
    let Ok(mut last) = LAST_MATCH.lock() else { return };
    if guid.is_some() && last.as_deref() == guid {
        return;
    }
    if let Some(value) = command_value(SystemTime::now(), action) {
        if let Err(error) = super::upk_history::replace(&command_path(cooked), value.as_bytes()) {
            tracing::warn!("Instant Suite could not send the match action: {error}");
            return;
        }
        *last = guid.map(str::to_owned);
    }
}

pub fn clear_match_end(cooked: &Path) {
    let path = command_path(cooked);
    if path.is_file() {
        let _ = super::upk_history::replace(&path, b"0");
    }
}

pub fn match_destroyed(cooked: &Path) {
    clear_match_end(cooked);
    if let Ok(mut last) = LAST_MATCH.lock() {
        *last = None;
    }
}

pub fn suspend(cooked: &Path) {
    if CODE.swap(0, Ordering::Relaxed) != 0 {
        clear_match_end(cooked);
    }
}

fn code(mode: &str) -> u8 {
    MODES.iter().position(|m| *m == mode).map_or(0, |i| i as u8 + 1)
}

const BRIDGE: &str = "GFxShell_TA.GetKeyForAction";
const BRIDGE_PATH: &str = r"..\..\TAGame\HbxInstant.txt";

pub(super) fn has_native_bridge(upk: &UpkPackage) -> bool {
    upk.find_object(BRIDGE)
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| upk.exports.get(index as usize))
        .is_some_and(|export| upk.image[export.serial_offset..export.serial_offset + export.serial_size]
            .windows(BRIDGE_PATH.len()).any(|bytes| bytes == BRIDGE_PATH.as_bytes()))
}

/// Shell.GetKeyForAction is a native-backed GFx method with a string return
/// slot. Handle one reserved action and run the original body for other keys.
fn edit_bridge(upk: &mut UpkPackage, enabled: bool) -> Result<bool, String> {
    use super::script_patch::{Script, replace_script};
    let object = |path| upk.find_object(path).ok_or_else(|| format!("Missing {path}"));
    let name = |value: &str| upk.names.iter().position(|name| name == value)
        .map(|index| index as i32).ok_or_else(|| format!("Missing {value}"));
    let index = usize::try_from(object(BRIDGE)?).ok().and_then(|index| index.checked_sub(1))
        .ok_or("Missing Shell key lookup export")?;
    let export = upk.exports.get(index).ok_or("Missing Shell key lookup export")?;
    let payload = &upk.image[export.serial_offset..export.serial_offset + export.serial_size];
    let mut body = Script::of_function(payload)?;
    let present = has_native_bridge(upk);
    if !present && !enabled {
        return Ok(false);
    }
    let slot = name("LoadFileToString").or_else(|_| name("TransformedLeftX"))?;
    let result = object("GFxShell_TA.GetKeyForAction.ReturnValue")?;
    let mut read = Script::new();
    read.raw(&[0x1b]).name(slot).raw(&[0x1f]).raw(BRIDGE_PATH.as_bytes())
        .raw(&[0, 0x2b]).object(result).raw(&[0x16]);
    let mut prefix = Script::new();
    // Action == 'GetSCT' (native Name equality). Return the file contents.
    prefix.raw(&[0x07, 0, 0, 0xfe, 0x2b]).object(object("GFxShell_TA.GetKeyForAction.Action")?)
        .raw(&[0x21]).name(name(super::title_patcher::abc::INSTANT_SUITE_ACTION)?).raw(&[0x16]);
    prefix.raw(&[0x5e, 0x12, 0, 0x20]).object(object("Core.FileSystem")?)
        .raw(&(read.mem_size() as u16).to_le_bytes()).object(0).raw(&[0]).append(&read);
    prefix.raw(&[0x04, 0x2b]).object(result);
    prefix.set_word(1, prefix.mem_size() as u16);
    if present {
        if !body.bytes().starts_with(prefix.bytes()) {
            return Err("Instant Suite's native bridge changed".into());
        }
        if enabled {
            return Ok(false);
        }
        body.cut(0, prefix.len(), prefix.refs());
        replace_script(upk, BRIDGE, &body)?;
        return Ok(true);
    }
    // GetKeyForAction has a straight-line body with no absolute jump targets.
    let tail = [&[0x16, 0x04, 0x3a][..], &result.to_le_bytes(), &[0x4c]].concat();
    if body.len() != 75 || body.refs() != 11 || !body.bytes().starts_with(&[0x04, 0x5e, 0x19, 0x01, 0x2e])
        || !body.bytes().ends_with(&tail)
    {
        return Err("Rocket League changed the Shell key lookup layout".into());
    }
    upk.rename_name(slot as usize, "LoadFileToString")?;
    replace_script(upk, BRIDGE, prefix.append(&body))?;
    Ok(true)
}

fn install_bridge(cooked: &Path, enabled: bool) -> Result<bool, String> {
    let _guard = super::profile_pictures::PACKAGE_EDIT.lock().map_err(|error| error.to_string())?;
    let live = game_path::resolve(cooked, "TAGame.upk");
    if !enabled && !live.is_file() {
        return Ok(false);
    }
    let before = std::fs::read(&live).map_err(|error| error.to_string())?;
    let mut upk = UpkPackage::from_bytes(before.clone())?;
    if !edit_bridge(&mut upk, enabled)? {
        return Ok(false);
    }
    let after = upk.repacked_fast()?;
    if edit_bridge(&mut UpkPackage::from_bytes(after.clone())?, enabled)? {
        return Err("Staged Instant Suite bridge did not validate".into());
    }
    ensure_closed()?;
    super::upk_history::install_linked_from(cooked, &live, &before, &after, "Instant Suite bridge")?;
    Ok(true)
}

/// The byte-patched movie of earlier builds: its state and vanilla payload.
#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
    mode: String,
    export: usize,
}

fn folder(backups: &Path) -> PathBuf {
    backups.join("InstantSuite")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(folder(backups).join("state.json")).ok()?).ok()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before switching Instant Suite on or off.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage) -> Result<(), String> {
    let temp = cooked.join("GFX_WinnerMenu_SF.upk.instant.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp).map(|saved| {
        saved.exports.len() == upk.exports.len() && payload(&saved, upk_export(&saved)) == payload(upk, upk_export(upk))
    });
    if check != Ok(true) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = ensure_closed().and_then(|()| std::fs::rename(&temp, live)
        .map_err(|error| format!("Could not write {PACKAGE}: {error}")));
    let _ = std::fs::remove_file(&temp);
    copied
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

/// Index of the SwfMovie export, the same in both copies.
fn upk_export(upk: &UpkPackage) -> usize {
    (0..upk.exports.len())
        .find(|&i| {
            matches!(strip(&upk.class_of(&upk.exports[i])), "SwfMovie" | "GFxMovieInfo")
                && upk
                    .serialized_props(&upk.exports[i])
                    .is_ok_and(|(p, _)| p.iter().any(|p| p.name == "RawData"))
        })
        .unwrap_or(0)
}

/// Put the vanilla movie back over an earlier build's byte patch.
fn restore_legacy(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if saved.build != backup_guard::build_id(cooked)? {
        if MOVIE.movie(&upk).is_ok_and(|movie| super::workshop_menu_ui::digest(&movie) == MOVIE.vanilla) {
            return std::fs::remove_file(folder(backups).join("state.json")).map_err(|error| error.to_string());
        }
        return Err("The Instant Suite backup belongs to another game build. Verify the game files.".into());
    }
    let original = std::fs::read(folder(backups).join("original.bin")).map_err(|e| e.to_string())?;
    upk.replace_export_payload(saved.export, &original)?;
    install(cooked, &live, &upk)?;
    std::fs::remove_file(folder(backups).join("state.json")).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(folder(backups).join("original.bin"));
    let _ = std::fs::remove_dir(folder(backups));
    Ok(())
}

/// Whether Instant Suite is on, in this build's form or an earlier one's.
pub(super) fn status(cooked: &Path, backups: &Path) -> bool {
    read_saved(backups).is_some() || UpkPackage::load(&game_path::resolve(cooked, PACKAGE))
        .and_then(|upk| MOVIE.movie(&upk))
        .and_then(|movie| edit_hook(&movie, false))
        .is_ok_and(|(enabled, _)| enabled == Some(true))
}

pub(super) fn recorded_enabled(backups: &Path) -> bool {
    backups.join(ENABLED).is_file() || read_saved(backups).is_some()
}

fn edit_hook(movie: &[u8], enabled: bool) -> Result<(Option<bool>, Option<Vec<u8>>), String> {
    let current = std::cell::Cell::new(None);
    let edited = super::title_patcher::movie::rewrite(movie, &|data| {
        let Some((offset, was_enabled)) = super::title_patcher::abc::instant_suite_hook(data)? else {
            return Ok(None);
        };
        current.set(Some(was_enabled));
        if enabled && !super::title_patcher::abc::has_instant_suite_bridge(data) {
            return super::title_patcher::abc::instant_suite_bridge(data, true);
        }
        if was_enabled == enabled {
            return Ok(None);
        }
        let mut bytes = data.to_vec();
        bytes[offset] = if enabled { 0xd0 } else { 0x47 };
        Ok(Some((bytes, 1)))
    })?;
    Ok((current.get(), edited.map(|(movie, _)| movie)))
}

fn switch_existing_hook(cooked: &Path, enabled: bool) -> Result<bool, String> {
    let mut upk = UpkPackage::load(&game_path::resolve(cooked, PACKAGE))?;
    let (present, edited) = edit_hook(&MOVIE.movie(&upk)?, enabled)?;
    if let Some(movie) = edited {
        MOVIE.install_movie(cooked, &mut upk, &movie)?;
    }
    Ok(present.is_some())
}

/// Turn Instant Suite on.
pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    let _guard = super::title_patcher::UI_PATCH_LOCK.lock().map_err(|_| "UI patch lock unavailable")?;
    ensure_closed()?;
    backup_guard::check_install(cooked, backups)?;
    restore_legacy(cooked, backups)?;
    let mut live = UpkPackage::load(&game_path::resolve(cooked, PACKAGE))?;
    let current = MOVIE.movie(&live)?;
    let (installed, upgrade) = edit_hook(&current, true)?;
    let is_vanilla = super::workshop_menu_ui::digest(&current) == MOVIE.vanilla;
    let desired = if installed.is_none() && !is_vanilla {
        let original = MOVIE.recover_original(cooked, backups, &live)
            .ok_or("No matching original winner code is available to install the Instant Suite hook.")?;
        let edited = super::title_patcher::movie::install_instant_suite(&current, &original, MOVIE.patched)?
            .ok_or("The winner code block could not be located in the installed menu.")?;
        let (_, upgraded) = edit_hook(&edited, true)?;
        Some(upgraded.unwrap_or(edited))
    } else {
        upgrade
    };
    super::upk_history::replace(&command_path(cooked), b"0")?;
    let bridge_added = install_bridge(cooked, true)?;
    let result = (|| {
        if let Some(desired) = desired {
            MOVIE.install_movie(cooked, &mut live, &desired)?;
        } else if is_vanilla {
            MOVIE.apply(cooked, backups)?;
            switch_existing_hook(cooked, true)?;
        }
        super::upk_history::replace(&backups.join(ENABLED), b"on")
    })();
    if result.is_err() && bridge_added {
        if let Err(error) = install_bridge(cooked, false) {
            tracing::warn!("Instant Suite could not roll back the native bridge: {error}");
        }
    }
    result
}

/// Turn Instant Suite off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let _guard = super::title_patcher::UI_PATCH_LOCK.lock().map_err(|_| "UI patch lock unavailable")?;
    ensure_closed()?;
    backup_guard::check_install(cooked, backups)?;
    restore_legacy(cooked, backups)?;
    clear_match_end(cooked);
    if status(cooked, backups) {
        if MOVIE.status(cooked) && MOVIE.recover_original(
            cooked, backups, &UpkPackage::load(&game_path::resolve(cooked, PACKAGE))?,
        ).is_some() {
            MOVIE.restore(cooked, backups)?;
        } else {
            switch_existing_hook(cooked, false)?;
        }
    }
    install_bridge(cooked, false)?;
    let _ = std::fs::remove_file(command_path(cooked));
    match std::fs::remove_file(backups.join(ENABLED)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Item recovery archives the backup folder, but Instant Suite remains active.
pub(super) fn retain_recovery_backup(cooked: &Path, archive: &Path) -> Result<(), String> {
    let name = format!("{PACKAGE}.bak");
    let backups = cooked.join("Backups");
    let source = archive.join(&name);
    let destination = backups.join(&name);
    if !source.is_file() || !status(cooked, &backups)
        || MOVIE.clean_movie(&destination).is_some() || MOVIE.clean_movie(&source).is_none()
    {
        return Ok(());
    }
    let bytes = std::fs::read(&source).map_err(|error| error.to_string())?;
    super::upk_history::replace(&destination, &bytes)?;
    super::upk_history::replace(&backups.join(MOVIE.marker()), backup_guard::build_id(cooked)?.as_bytes())
}

pub(super) fn recover_after_update(cooked: &Path, archive: &Path, wanted: bool) -> Result<(), String> {
    let backups = cooked.join("Backups");
    retain_recovery_backup(cooked, archive)?;
    if wanted || recorded_enabled(archive) || status(cooked, &backups) {
        enable(cooked, &backups)?;
    }
    Ok(())
}

/// Page state: whether the patch is in, and the job writing the package.
#[derive(Default)]
pub struct State {
    job: Option<std::sync::mpsc::Receiver<Result<bool, String>>>,
    status: String,
    on: Option<bool>,
    /// An installed hook to validate once Rocket League is closed.
    needs_upgrade: bool,
    install: Option<PathBuf>,
    winner_stamp: Option<(u64, SystemTime)>,
    next_check: Option<std::time::Instant>,
    /// the in-game Workshop menu's state and its maps, read when the Workshop action is shown
    maps: Option<(bool, Vec<super::workshop_menu::WorkshopMap>)>,
    flagged: Option<String>,
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn spawn(&mut self, job: impl FnOnce() -> Result<bool, String> + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    /// Every frame: keep what the movie is told and the Workshop list's "n" flag
    /// in step with the settings.
    pub fn tick(&mut self, cooked: &Path, backups: &Path, mode: &str, map: &str) {
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(Ok(on)) => {
                    self.on = Some(on);
                    self.job = None;
                }
                Ok(Err(error)) => {
                    self.status = error;
                    self.job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.busy() && self.install.as_deref() != Some(cooked) {
            if let Some(old) = &self.install {
                clear_match_end(old);
            }
            *self = Self { install: Some(cooked.to_path_buf()), ..Default::default() };
        }
        let now = std::time::Instant::now();
        if !self.busy() && self.next_check.is_none_or(|check| now >= check) {
            self.next_check = Some(now + Duration::from_secs(2));
            let stamp = std::fs::metadata(game_path::resolve(cooked, PACKAGE)).ok()
                .and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
            if self.winner_stamp != stamp {
                self.winner_stamp = stamp;
                self.on = None;
                self.needs_upgrade = false;
            }
        }
        if self.on.is_none() && !self.busy() && self.winner_stamp.is_some() {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            self.needs_upgrade = true;
            self.spawn(move || Ok(status(&cooked, &backups)));
        }
        if self.needs_upgrade && self.on == Some(true) && !self.busy()
            && !hebnix_sdk::process::is_rocket_league_running()
        {
            self.needs_upgrade = false;
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            self.spawn(move || {
                if !backups.join(ENABLED).is_file() && super::game_update::Recovery::load(&cooked)?.is_none() {
                    enable(&cooked, &backups)?;
                }
                Ok(true)
            });
        }
        let on = self.on == Some(true) && self.install.as_deref() == Some(cooked) && !self.busy();
        let action = if on && (mode != "workshop" || Path::new(map).is_file()) {
            code(mode)
        } else {
            0
        };
        if CODE.swap(action, Ordering::Relaxed) != action {
            clear_match_end(cooked);
        }
        let flag = if action == 4 { map } else { "" };
        if self.flagged.as_deref() != Some(flag) {
            let refresh = !flag.is_empty() || self.flagged.as_deref().is_some_and(|old| !old.is_empty());
            self.flagged = Some(flag.to_string());
            super::workshop_menu::set_next(Path::new(flag));
            if cooked.is_dir() && refresh {
                let cooked = cooked.to_path_buf();
                std::thread::spawn(move || {
                    if let Err(error) = crate::ui::workshop::ingame_menu::refresh(&cooked) {
                        tracing::warn!("Instant Suite could not refresh the Workshop list: {error}");
                    }
                });
            }
        }
    }

    /// Draw the toggle, the action picker and, for a Workshop map, the map picker.
    /// Returns true when a setting changed.
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, settings: &mut crate::config::GameModsCfg) -> bool {
        use crate::i18n::t;
        use eframe::egui;
        let mode_setting = &mut settings.instant_suite_mode;
        let map_setting = &mut settings.instant_suite_map;
        if mode_setting != DEFAULT_MODE && !MODES.contains(&mode_setting.as_str()) {
            *mode_setting = DEFAULT_MODE.into();
        }
        let on = self.on == Some(true);
        let mut want = on;
        let mut changed = false;
        ui.add_enabled_ui(!self.busy() && self.on.is_some(), |ui| {
            ui.checkbox(&mut want, t("game-mod-instant-suite")).on_hover_text(t("game-mod-instant-suite-description"));
        });
        ui.separator();
        ui.label(t("game-mod-instant-suite-description"));
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut settings.instant_suite_private, t("game-mod-instant-suite-private")).changed();
            changed |= ui.checkbox(&mut settings.instant_suite_tournaments, t("game-mod-instant-suite-tournaments")).changed();
        });
        ui.horizontal(|ui| {
            ui.label(t("game-mod-instant-suite-mode"));
            egui::ComboBox::from_id_salt("instant_suite_mode")
                .selected_text(t(&format!("game-mod-instant-suite-mode-{mode_setting}")))
                .show_ui(ui, |ui| {
                    for option in std::iter::once(DEFAULT_MODE).chain(MODES) {
                        changed |= ui
                            .selectable_value(mode_setting, option.to_string(), t(&format!("game-mod-instant-suite-mode-{option}")))
                            .changed();
                    }
                });
        });
        if mode_setting == "workshop" {
            let (menu_on, maps) = self.maps.get_or_insert_with(|| {
                (super::workshop_menu::status(cooked).unwrap_or(false), crate::ui::workshop::ingame_menu::all_maps(cooked))
            });
            if !*menu_on {
                ui.colored_label(ui.visuals().warn_fg_color, t("game-mod-instant-suite-needs-menu"));
            }
            let mut reload = false;
            let picked = maps.iter().find(|m| m.path == Path::new(map_setting.as_str())).map(|m| m.title.clone());
            ui.horizontal(|ui| {
                ui.label(t("game-mod-instant-suite-map"));
                egui::ComboBox::from_id_salt("instant_suite_map")
                    .selected_text(picked.unwrap_or_else(|| t("game-mod-instant-suite-pick-map")))
                    .show_ui(ui, |ui| {
                        for map in maps.iter() {
                            let path = map.path.to_string_lossy().into_owned();
                            changed |= ui.selectable_value(map_setting, path, &map.title).changed();
                        }
                    });
                reload = ui.small_button(t("game-mod-instant-suite-reload-maps")).clicked();
            });
            if reload {
                self.maps = None;
            }
        }
        if want != on {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            self.status.clear();
            self.needs_upgrade = false;
            self.spawn(move || if want { enable(&cooked, &backups).map(|()| true) } else { restore(&cooked, &backups).map(|()| false) });
        }
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_expires_and_encodes_only_valid_actions() {
        let now = UNIX_EPOCH + Duration::from_millis(1_800_000_000_000);
        for action in 1..=4 {
            let value: u64 = command_value(now, action).unwrap().parse().unwrap();
            assert_eq!(value / 10, 1_800_000_008_000);
            assert_eq!(value % 10, u64::from(action));
            assert_eq!(value as f64 as u64, value);
        }
        assert_eq!(command_value(now, 0), None);
    }

    #[test]
    fn restricted_playlists_require_their_own_opt_in() {
        assert!(!permits_match(None, true, true));
        assert!(!permits_match(Some(6), false, true));
        assert!(permits_match(Some(6), true, false));
        for id in [22, 34] {
            assert!(!permits_match(Some(id), true, false));
            assert!(permits_match(Some(id), false, true));
        }
        assert!(permits_match(Some(13), false, false));
        assert!(!permits_match(Some(0), false, true));
        assert!(permits_match(Some(0), true, false));
    }

    #[test]
    fn every_mode_has_its_own_action_digit() {
        let codes: Vec<u8> = MODES.iter().map(|m| code(m)).collect();
        assert_eq!(codes, [1, 2, 3, 4]);
        assert_eq!(code("nothing"), 0);
        assert_eq!(DEFAULT_MODE, "nothing");
        assert_eq!(crate::config::GameModsCfg::default().instant_suite_mode, "nothing");
    }

    /// Switch Instant Suite on and off over a copy of the real package. Run with
    /// HEBNIX_WINNER_UPK set to a vanilla GFX_WinnerMenu_SF.upk.
    #[test]
    #[ignore]
    fn apply_and_restore_on_a_copy() {
        let source = std::env::var("HEBNIX_WINNER_UPK").unwrap();
        let root = std::env::temp_dir().join(format!("hebnix-winner-{:016x}", rand::random::<u64>()));
        let dir = root.join("TAGame").join("CookedPCConsole");
        let backups = dir.join("Backups");
        std::fs::create_dir_all(&backups).unwrap();
        // backups are tied to the game build, read from the exe
        std::fs::create_dir_all(root.join("Binaries").join("Win64")).unwrap();
        std::fs::write(root.join("Binaries").join("Win64").join("RocketLeague.exe"), b"build").unwrap();
        std::fs::copy(&source, dir.join(PACKAGE)).unwrap();
        if let Some(tagame) = std::env::var_os("HEBNIX_INSTANT_TAGAME_UPK") {
            std::fs::copy(tagame, dir.join("TAGame.upk")).unwrap();
            let original = UpkPackage::load(&dir.join("TAGame.upk")).unwrap();
            let index = original.find_object(BRIDGE).unwrap() as usize - 1;
            let expected = payload(&original, index);
            drop(original);
            enable(&dir, &backups).unwrap();
            assert!(status(&dir, &backups));
            assert!(has_native_bridge(&UpkPackage::load(&dir.join("TAGame.upk")).unwrap()));
            restore(&dir, &backups).unwrap();
            assert!(!status(&dir, &backups));
            let restored = UpkPackage::load(&dir.join("TAGame.upk")).unwrap();
            assert!(payload(&restored, index) == expected);
        } else {
            MOVIE.apply(&dir, &backups).unwrap();
            assert!(status(&dir, &backups));
            MOVIE.restore(&dir, &backups).unwrap();
            assert!(!status(&dir, &backups));
            assert!(MOVIE.clean_movie(&dir.join(PACKAGE)).is_some());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn embedded_movie_is_not_the_vanilla_one() {
        assert_eq!(&MOVIE.patched[..3], b"FWS");
        assert!(MOVIE.patched.windows(6).any(|w| w == b"hbxAsk"));
        let (_, upgraded) = edit_hook(MOVIE.patched, true).unwrap();
        let upgraded = upgraded.unwrap();
        assert_eq!(edit_hook(&upgraded, true).unwrap(), (Some(true), None));
        let (_, disabled) = edit_hook(&upgraded, false).unwrap();
        assert_eq!(edit_hook(&disabled.unwrap(), false).unwrap(), (Some(false), None));
    }
}
