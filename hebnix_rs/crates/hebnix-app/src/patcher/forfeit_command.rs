//! Forfeit command. Rocket League's forfeit button runs
//! GFxShell_TA.VoteToForfeit, which is
//!
//!     PRI_TA(Player.Actor.PlayerReplicationInfo).ServerVoteToForfeit();
//!
//! Hebnix gives the player controller an exec function with the same body so
//! `Forfeit` works from the console and from a key binding. There is no room
//! for a new function, so an exec function nothing binds
//! (PlayerController_TA.TargetSelectNext) is rewritten and renamed. Its
//! original payload and name go to Backups/ForfeitCommand to restore it.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
const DONOR: &str = "PlayerController_TA.TargetSelectNext";
const COMMAND: &str = "PlayerController_TA.Forfeit";
/// A function already called Forfeit, whose name table entry the command reuses.
const NAME_SOURCE: &str = "Team_TA.Forfeit";
const PRI_CLASS: &str = "PRI_TA";
const PRI_VARIABLE: &str = "Engine.Controller.PlayerReplicationInfo";
const VOTE: &str = "PRI_TA.ServerVoteToForfeit";
/// Return, Nothing, EndOfScript: how every function here ends.
const END: [u8; 3] = [0x04, 0x0b, 0x4c];
/// An export entry is class, super, outer, then the name.
const NAME_FIELD: usize = 12;

/// `PRI_TA(PlayerReplicationInfo).ServerVoteToForfeit()`, in the shape of
/// GFxShell_TA.VoteToForfeit: Context, no fail flag, a cast of the instance
/// variable, skip size of the call, no return property, the final-function call.
fn body(pri_class: i32, pri_variable: i32, vote: i32) -> Script {
    let mut call = Script::new();
    call.raw(&[0x1c]).object(vote).raw(&[0x16]);
    let mut out = Script::new();
    out.raw(&[0x5e, 0x19, 0x00, 0x2e]).object(pri_class).raw(&[0x01]).object(pri_variable);
    out.raw(&(call.mem_size() as u16).to_le_bytes());
    out.object(0).raw(&[0x00]).append(&call).raw(&END);
    out
}

fn object(upk: &UpkPackage, path: &str) -> Result<i32, String> {
    upk.find_object(path).ok_or_else(|| format!("{path} was not found"))
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

fn is_patched(upk: &UpkPackage) -> bool {
    upk.find_object(COMMAND).is_some()
}

/// Rewrite the donor into the command and give it the Forfeit name.
pub fn apply(upk: &mut UpkPackage) -> Result<(), String> {
    if is_patched(upk) {
        return Err(format!("{COMMAND} already exists"));
    }
    let script = body(object(upk, PRI_CLASS)?, object(upk, PRI_VARIABLE)?, object(upk, VOTE)?);
    let name = upk.exports[object(upk, NAME_SOURCE)? as usize - 1].object_name;
    let donor = object(upk, DONOR)? as usize - 1;
    replace_script(upk, DONOR, &script)?;
    rename(upk, donor, name.name_index, name.instance)
}

fn rename(upk: &mut UpkPackage, export: usize, name_index: usize, instance: i32) -> Result<(), String> {
    let mut field = (name_index as i32).to_le_bytes().to_vec();
    // The reader stores the file's instance number minus one.
    field.extend_from_slice(&(instance + 1).to_le_bytes());
    upk.patch(upk.exports[export].entry_offset + NAME_FIELD, &field)?;
    upk.exports[export].object_name.name_index = name_index;
    upk.exports[export].object_name.instance = instance;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
    /// The donor's export payload before the patch.
    original: Vec<u8>,
    name_index: usize,
    instance: i32,
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("ForfeitCommand").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

pub fn enabled(backups: &Path) -> bool {
    read_saved(backups).is_some()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the forfeit command.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads and
/// has (or lacks) the command.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.forfeit.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp).map(|saved| is_patched(&saved));
    if check != Ok(patched) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

/// Turn the command on.
pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk) {
        return Ok(());
    }
    let donor = object(&upk, DONOR)? as usize - 1;
    let name = upk.exports[donor].object_name;
    let saved = Saved {
        build: backup_guard::build_id(cooked)?,
        original: payload(&upk, donor),
        name_index: name.name_index,
        instance: name.instance,
    };
    apply(&mut upk)?;
    install(cooked, &live, &upk, true)?;
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec(&saved).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Turn the command off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk) {
        if saved.build != backup_guard::build_id(cooked)? {
            return Err("The forfeit backup belongs to another game build. Verify the game files.".into());
        }
        let command = object(&upk, COMMAND)? as usize - 1;
        upk.replace_export_payload(command, &saved.original)?;
        rename(&mut upk, command, saved.name_index, saved.instance)?;
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("ForfeitCommand"));
    Ok(())
}

/// Put the command back after another feature rewrote TAGame.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk) {
        return Ok(());
    }
    ensure_closed()?;
    apply(&mut upk)?;
    install(cooked, &live, &upk, true)
}

/// Keys the page offers, by Rocket League's names.
pub const KEYS: [&str; 15] = [
    "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F11", "F12", "Home", "End", "Insert", "Delete", "PageUp",
];
pub const DEFAULT_KEY: &str = "F9";
/// The binding goes next to ReadyUp, a PlayerController_TA exec function bound
/// the same way, in every control preset that has it.
const NEIGHBOUR: &str = "Action=\"ReadyUp\"";
const ACTION: &str = "Action=\"Forfeit\"";

/// `ini` with the Forfeit binding set to `key`, or removed when `key` is None.
fn with_binding(ini: &str, key: Option<&str>) -> String {
    let nl = if ini.contains("
") { "
" } else { "
" };
    let mut out = Vec::new();
    let mut added = false;
    for line in ini.lines() {
        if line.trim_start().starts_with('[') {
            added = false;
        }
        if line.contains(ACTION) {
            continue;
        }
        out.push(line.to_string());
        if let Some(key) = key.filter(|_| !added && line.trim_start().starts_with("PCBindings") && line.contains(NEIGHBOUR)) {
            out.push(format!("PCBindings=( Action=\"Forfeit\", Key=\"{key}\" )"));
            added = true;
        }
    }
    let mut text = out.join(nl);
    if ini.ends_with('\n') {
        text.push_str(nl);
    }
    text
}

/// Bind the command to `key` in TAInput.ini, or unbind it with None.
pub fn set_binding(key: Option<&str>) -> Result<(), String> {
    let path = hebnix_sdk::utils::paths::documents_dir()
        .map(|d| d.join("My Games/Rocket League/TAGame/Config/TAInput.ini"))
        .ok_or("Could not find the Documents folder")?;
    let Ok(ini) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let updated = with_binding(&ini, key);
    if updated == ini {
        return Ok(());
    }
    std::fs::write(&path, updated).map_err(|e| format!("Could not write TAInput.ini: {e}"))
}

/// Page state: the job writing the package.
#[derive(Default)]
pub struct State {
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    status: String,
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Draw the toggle and key picker. Returns true when `key` changed.
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, key: &mut String) -> bool {
        use crate::i18n::t;
        use eframe::egui;
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.status = result.unwrap_or_else(|e| e);
                    self.job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            }
        }
        if key.is_empty() {
            *key = DEFAULT_KEY.into();
        }
        let on = enabled(backups);
        let mut want = on;
        let mut key_changed = false;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-forfeit")).on_hover_text(t("game-mod-forfeit-description"));
            ui.label(t("game-mod-forfeit-description"));
            ui.horizontal(|ui| {
                ui.label(t("game-mod-forfeit-key"));
                egui::ComboBox::from_id_salt("forfeit_key").selected_text(key.as_str()).show_ui(ui, |ui| {
                    for option in KEYS {
                        key_changed |= ui.selectable_value(key, option.to_string(), option).changed();
                    }
                });
            });
        });
        if want != on || (key_changed && on) {
            let (cooked, backups, key) = (cooked.to_path_buf(), backups.to_path_buf(), key.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if !want {
                    restore(&cooked, &backups).and_then(|_| set_binding(None)).map(|_| t("game-mod-forfeit-off"))
                } else {
                    enable(&cooked, &backups).and_then(|_| set_binding(Some(&key))).map(|_| t("game-mod-forfeit-on"))
                };
                let _ = tx.send(result);
            });
        }
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        }
        key_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body is the game's own statement with the player's PRI in place of
    /// Player.Actor.PlayerReplicationInfo: 5e 19 00 2e <cast> 01 <variable>
    /// then the skip, return property, size and the call.
    #[test]
    fn body_matches_the_games_own_call_shape() {
        let script = body(58203, -3726, 58051);
        let mut expected = vec![0x5e, 0x19, 0x00, 0x2e];
        expected.extend_from_slice(&58203i32.to_le_bytes());
        expected.push(0x01);
        expected.extend_from_slice(&(-3726i32).to_le_bytes());
        expected.extend_from_slice(&[0x0a, 0x00, 0, 0, 0, 0, 0x00, 0x1c]);
        expected.extend_from_slice(&58051i32.to_le_bytes());
        expected.extend_from_slice(&[0x16, 0x04, 0x0b, 0x4c]);
        assert_eq!(script.bytes(), &expected[..]);
    }

    #[test]
    fn binding_goes_after_each_presets_ready_up_and_can_be_removed() {
        let ini = "[A]
PCBindings=( Action=\"ReadyUp\", Key=\"Spacebar\" )
PCBindings=( Action=\"ReadyUp\", Key=\"X\" )
[B]
PCBindings=( Action=\"ReadyUp\", Key=\"Spacebar\" )
";
        let bound = with_binding(ini, Some("F9"));
        assert_eq!(bound.matches("Action=\"Forfeit\"").count(), 2);
        assert!(bound.contains("Spacebar\" )
PCBindings=( Action=\"Forfeit\", Key=\"F9\" )
"));
        assert_eq!(with_binding(&bound, Some("F8")).matches("Key=\"F8\"").count(), 2);
        assert_eq!(with_binding(&bound, None), ini);
    }

    /// Patch a copy of the real package, reload it and look for the command.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        apply(&mut upk).unwrap();
        let out = std::env::temp_dir().join("hebnix_forfeit_test.upk");
        upk.save_fast(&out).unwrap();
        let reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        let index = object(&reloaded, COMMAND).unwrap() as usize - 1;
        assert!(reloaded.find_object(DONOR).is_none());
        let script = Script::of_function(&payload(&reloaded, index)).unwrap();
        let vote = object(&reloaded, VOTE).unwrap();
        assert!(script.bytes().windows(4).any(|w| w == vote.to_le_bytes()));
    }

    /// Enable then restore on a temp copy of the real package, which must be
    /// the vanilla TAGame.upk, and check the donor comes back byte for byte.
    #[test]
    #[ignore]
    fn enable_restore() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let root = std::env::temp_dir().join("hebnix_forfeit_install");
        let _ = std::fs::remove_dir_all(&root);
        let cooked = root.join("TAGame").join("CookedPCConsole");
        let backups = cooked.join("Backups");
        std::fs::create_dir_all(&cooked).unwrap();
        std::fs::create_dir_all(root.join("TAGame/Binaries/Win64")).unwrap();
        std::fs::write(root.join("TAGame/Binaries/Win64/RocketLeague.exe"), b"exe").unwrap();
        std::fs::copy(&source, cooked.join("TAGame.upk")).unwrap();
        let load = || UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
        let vanilla = {
            let upk = load();
            payload(&upk, object(&upk, DONOR).unwrap() as usize - 1)
        };
        enable(&cooked, &backups).unwrap();
        assert!(enabled(&backups) && is_patched(&load()));
        enable(&cooked, &backups).unwrap();
        restore(&cooked, &backups).unwrap();
        let upk = load();
        assert!(!enabled(&backups) && !is_patched(&upk));
        assert_eq!(payload(&upk, object(&upk, DONOR).unwrap() as usize - 1), vanilla);
        let _ = std::fs::remove_dir_all(&root);
    }
}
