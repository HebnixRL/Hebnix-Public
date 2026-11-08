//! Forfeit command. Rocket League's forfeit button runs
//! GFxShell_TA.VoteToForfeit, which is
//!
//!     PRI_TA(Player.Actor.PlayerReplicationInfo).ServerVoteToForfeit();
//!
//! Hebnix runs a copy of that statement as a block of the dispatcher
//! (dispatcher.rs) and adds a key action for it (actions.rs), so a key bound to
//! that action starts the vote. Builds before the dispatcher rewrote
//! PlayerController_TA.ShowControllerApplet instead (and, before that,
//! TargetSelectNext); their payload is in Backups/ForfeitCommand and is put back.
use super::script_patch::Script;
use super::upk_package::UpkPackage;
use super::{actions, backup_guard, dispatcher, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
/// The function builds before the dispatcher rewrote.
const LEGACY_DONOR: &str = "PlayerController_TA.ShowControllerApplet";
/// Where builds before the key action fix put the command, renamed.
const LEGACY_COMMAND: &str = "PlayerController_TA.Forfeit";
const PRI_CLASS: &str = "PRI_TA";
const PRI_VARIABLE: &str = "Engine.Controller.PlayerReplicationInfo";
const VOTE: &str = "PRI_TA.ServerVoteToForfeit";
const LOG_MARKER: &str = "HBXFORFEIT";
/// The key action slot (see actions.rs), also the dispatcher block id.
const SLOT: u32 = 1;
/// An export entry is class, super, outer, then the name.
const NAME_FIELD: usize = 12;

/// `PRI_TA(PlayerReplicationInfo).ServerVoteToForfeit()`, in the shape of
/// GFxShell_TA.VoteToForfeit: Context, no fail flag, a cast of the instance
/// variable, skip size of the call, no return property, the final-function call.
fn body(pri_class: i32, pri_variable: i32, vote: i32) -> Script {
    let mut call = Script::new();
    call.raw(&[0x1c]).object(vote).raw(&[0x16]);
    let mut out = Script::new();
    // LogInternal("HBXFORFEIT"), a ScriptLog line that shows the command ran.
    out.raw(&[0xe7, 0x1f]).raw(LOG_MARKER.as_bytes()).raw(&[0x00, 0x48, 0x48, 0x16]);
    out.raw(&[0x5e, 0x19, 0x00, 0x2e]).object(pri_class).raw(&[0x01]).object(pri_variable);
    out.raw(&(call.mem_size() as u16).to_le_bytes());
    out.object(0).raw(&[0x00]).append(&call);
    out
}

fn object(upk: &UpkPackage, path: &str) -> Result<i32, String> {
    upk.find_object(path).ok_or_else(|| format!("{path} was not found"))
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

/// The old donor's payload carries the log marker only once it is rewritten.
fn legacy_donor_rewritten(upk: &UpkPackage) -> bool {
    upk.find_object(LEGACY_DONOR).is_some_and(|index| {
        let marker = LOG_MARKER.as_bytes();
        payload(upk, index as usize - 1).windows(marker.len()).any(|w| w == marker)
    })
}

/// The command is whole once its block and key action exist and no old
/// rewrite is left.
fn is_patched(upk: &UpkPackage) -> bool {
    dispatcher::has(upk, SLOT as u8) && actions::has(upk, SLOT) && !legacy_donor_rewritten(upk)
}

/// Add the dispatcher block and the key action that runs it.
pub fn apply(upk: &mut UpkPackage, backups: &Path, build: &str) -> Result<(), String> {
    let script = body(object(upk, PRI_CLASS)?, object(upk, PRI_VARIABLE)?, object(upk, VOTE)?);
    dispatcher::install(upk, backups, build, SLOT as u8, &script, &[])?;
    actions::remove(upk, SLOT)?;
    actions::add(upk, SLOT, &format!("{} {SLOT}", dispatcher::COMMAND))
}

/// The Action name TAInput.ini binds the key to.
pub fn action() -> String {
    actions::action_name(SLOT)
}

/// Undo what earlier builds did: the command lived in TargetSelectNext,
/// renamed Forfeit, and after that in ShowControllerApplet. Put the saved
/// payload (and name) back.
fn undo_legacy(upk: &mut UpkPackage, saved: &Saved) -> Result<(), String> {
    if let Some(index) = upk.find_object(LEGACY_COMMAND) {
        let index = index as usize - 1;
        upk.replace_export_payload(index, &saved.original)?;
        rename(upk, index, saved.name_index, saved.instance)?;
    }
    if legacy_donor_rewritten(upk) {
        let donor = object(upk, LEGACY_DONOR)? as usize - 1;
        upk.replace_export_payload(donor, &saved.original)?;
    }
    Ok(())
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
    /// What an earlier build rewrote, before its patch. Empty from the
    /// dispatcher on, which keeps its own backup.
    original: Vec<u8>,
    /// Only set by builds before the key action fix, which renamed the donor.
    #[serde(default)]
    name_index: usize,
    #[serde(default)]
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
    if let Some(old) = read_saved(backups) {
        undo_legacy(&mut upk, &old)?;
    }
    if is_patched(&upk) {
        return Ok(());
    }
    let build = backup_guard::build_id(cooked)?;
    apply(&mut upk, backups, &build)?;
    install(cooked, &live, &upk, true)?;
    let saved = Saved { build, original: Vec::new(), name_index: 0, instance: 0 };
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
    let touched = upk.find_object(LEGACY_COMMAND).is_some()
        || legacy_donor_rewritten(&upk)
        || actions::has(&upk, SLOT)
        || dispatcher::has(&upk, SLOT as u8);
    if touched {
        let build = backup_guard::build_id(cooked)?;
        if saved.build != build {
            return Err("The forfeit backup belongs to another game build. Verify the game files.".into());
        }
        undo_legacy(&mut upk, &saved)?;
        actions::remove(&mut upk, SLOT)?;
        dispatcher::remove(&mut upk, backups, &build, SLOT as u8)?;
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
    if let Some(saved) = read_saved(backups) {
        undo_legacy(&mut upk, &saved)?;
    }
    apply(&mut upk, backups, &backup_guard::build_id(cooked)?)?;
    install(cooked, &live, &upk, true)
}

/// Keys the page offers, by Rocket League's names.
pub const KEYS: [&str; 41] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F11", "F12", "Home", "End", "Insert", "Delete", "PageUp",
];
pub const DEFAULT_KEY: &str = "J";
/// The binding goes next to ReadyUp, a PlayerController_TA exec function bound
/// the same way, in every control preset that has it.
const NEIGHBOUR: &str = "Action=\"ReadyUp\"";
/// Actions earlier builds bound: the donor's own action, then one the game has no command for.
pub const LEGACY: [&str; 2] = ["ShowControllerApplet", "Forfeit"];

/// Controller buttons a command can be bound to: (Rocket League's XboxTypeS_ name, dinput icon name).
pub const PAD_BUTTONS: [(&str, &str); 16] = [
    ("A", "south"), ("B", "east"), ("X", "west"), ("Y", "north"),
    ("LeftShoulder", "l1"), ("RightShoulder", "r1"), ("LeftTrigger", "l2"), ("RightTrigger", "r2"),
    ("LeftThumbStick", "l3"), ("RightThumbStick", "r3"), ("Back", "select"), ("Start", "start"),
    ("DPad_Up", "dpad_up"), ("DPad_Down", "dpad_down"), ("DPad_Left", "dpad_left"), ("DPad_Right", "dpad_right"),
];

/// `ini` with the command bound to `key` and the controller button `pad`, or
/// without each one that is None.
fn with_binding(ini: &str, action: &str, legacy: &[&str], key: Option<&str>, pad: Option<&str>) -> String {
    let tag = format!("Action=\"{action}\"");
    let nl = if ini.contains("
") { "
" } else { "
" };
    let mut out = Vec::new();
    let mut added = false;
    // the controller presets list the same action under both of these
    let mut pad_added = [false; 2];
    for line in ini.lines() {
        if line.trim_start().starts_with('[') {
            added = false;
            pad_added = [false; 2];
        }
        if line.contains(&tag) || legacy.iter().any(|old| line.contains(&format!("Action=\"{old}\""))) {
            continue;
        }
        out.push(line.to_string());
        if let Some(key) = key.filter(|_| !added && line.trim_start().starts_with("PCBindings") && line.contains(NEIGHBOUR)) {
            out.push(format!("PCBindings=( {tag}, Key=\"{key}\" )"));
            added = true;
        }
        for (done, prefix) in pad_added.iter_mut().zip(["GamepadBindings", "SteamInputBindings"]) {
            if let Some(pad) = pad.filter(|_| !*done && line.trim_start().starts_with(prefix) && line.contains(NEIGHBOUR)) {
                out.push(format!("{prefix}=( {tag}, Key=\"XboxTypeS_{pad}\" )"));
                *done = true;
            }
        }
    }
    let mut text = out.join(nl);
    if ini.ends_with('\n') {
        text.push_str(nl);
    }
    text
}

/// Remove the key binding.
pub fn unbind() -> Result<(), String> {
    set_binding(&action(), &LEGACY, None, None)
}

/// Bind the command to `key` and the controller button `pad` in TAInput.ini; None unbinds each.
pub fn set_binding(action: &str, legacy: &[&str], key: Option<&str>, pad: Option<&str>) -> Result<(), String> {
    let path = hebnix_sdk::utils::paths::documents_dir()
        .map(|d| d.join("My Games/Rocket League/TAGame/Config/TAInput.ini"))
        .ok_or("Could not find the Documents folder")?;
    let Ok(ini) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let updated = with_binding(&ini, action, legacy, key, pad);
    if updated == ini {
        return Ok(());
    }
    std::fs::write(&path, updated).map_err(|e| format!("Could not write TAInput.ini: {e}"))
}

/// The picker waiting for a button press (its id) and, once pressed, the button.
static CAPTURE: std::sync::Mutex<Option<(String, Option<Option<String>>)>> = std::sync::Mutex::new(None);

/// `pad` as the setting the binding takes: None for no button.
pub fn pad_setting(pad: &str) -> Option<&str> {
    Some(pad).filter(|pad| !pad.is_empty())
}

/// Controller button picker: press "Bind controller" and then a button, shown with its
/// prompt icon. Returns true when `pad` changed. `pad` holds a PAD_BUTTONS name, empty for none.
pub fn pad_picker(ui: &mut eframe::egui::Ui, id: &str, pad: &mut String) -> bool {
    use crate::i18n::t;
    let mut changed = false;
    let mut capture = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((owner, Some(result))) = capture.as_ref() {
        if owner == id {
            if let Some(button) = result {
                changed = *pad != *button;
                *pad = button.clone();
            }
            *capture = None;
        }
    }
    let waiting = capture.as_ref().is_some_and(|(owner, _)| owner == id);
    ui.horizontal(|ui| {
        ui.label(t("game-mod-controller-button"));
        if waiting {
            ui.spinner();
            ui.label(t("game-mod-press-a-button"));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        match PAD_BUTTONS.iter().find(|(name, _)| *name == pad.as_str()) {
            Some((name, icon)) => {
                if !crate::plugins::gamepad_icons::prompt_icon(ui, icon, 22.0) {
                    ui.label(*name);
                }
            }
            None => {
                ui.weak(t("game-mod-no-controller-button"));
            }
        }
        if ui.add_enabled(capture.is_none(), eframe::egui::Button::new(t("game-mod-bind-controller"))).clicked() {
            *capture = Some((id.to_string(), None));
            let (id, ctx) = (id.to_string(), ui.ctx().clone());
            std::thread::spawn(move || {
                let button = hebnix_sdk::input::detect_any_hotkey(Some(std::time::Duration::from_secs(6)))
                    .filter(|bind| bind.is_controller)
                    .and_then(|bind| crate::plugins::gamepad_icons::dinput_name_for(&bind))
                    .and_then(|icon| PAD_BUTTONS.iter().find(|(_, i)| *i == icon))
                    .map(|(name, _)| name.to_string());
                if let Ok(mut capture) = CAPTURE.lock() {
                    *capture = Some((id, Some(button)));
                }
                ctx.request_repaint();
            });
        }
        if !pad.is_empty() && ui.button(t("game-mod-clear-controller")).clicked() {
            pad.clear();
            changed = true;
        }
    });
    changed
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
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, key: &mut String, pad: &mut String) -> bool {
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
            key_changed |= pad_picker(ui, "forfeit_pad", pad);
        });
        if want != on || (key_changed && on) {
            let (cooked, backups, key, pad) = (cooked.to_path_buf(), backups.to_path_buf(), key.clone(), pad.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if !want {
                    restore(&cooked, &backups).and_then(|_| unbind()).map(|_| t("game-mod-forfeit-off"))
                } else {
                    enable(&cooked, &backups).and_then(|_| set_binding(&action(), &LEGACY, Some(&key), pad_setting(&pad))).map(|_| t("game-mod-forfeit-on"))
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

    const ACTION: &str = "Checkpoint_1";
    /// The function the dispatcher rewrites.
    const DONOR: &str = "PlayerController_TA.NetClientInputRate";
    const ACTIONS: &str = "PlayerInput_TA.GetDefaultActions";

    /// The body is the game's own statement with the player's PRI in place of
    /// Player.Actor.PlayerReplicationInfo: 5e 19 00 2e <cast> 01 <variable>
    /// then the skip, return property, size and the call.
    #[test]
    fn body_matches_the_games_own_call_shape() {
        let script = body(58203, -3726, 58051);
        let mut expected = vec![0xe7, 0x1f];
        expected.extend_from_slice(LOG_MARKER.as_bytes());
        expected.extend_from_slice(&[0x00, 0x48, 0x48, 0x16, 0x5e, 0x19, 0x00, 0x2e]);
        expected.extend_from_slice(&58203i32.to_le_bytes());
        expected.push(0x01);
        expected.extend_from_slice(&(-3726i32).to_le_bytes());
        expected.extend_from_slice(&[0x0a, 0x00, 0, 0, 0, 0, 0x00, 0x1c]);
        expected.extend_from_slice(&58051i32.to_le_bytes());
        expected.push(0x16);
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
        let bound = with_binding(ini, ACTION, &[], Some("F9"), None);
        assert_eq!(bound.matches("Action=\"Checkpoint_1\"").count(), 2);
        assert!(bound.contains("Spacebar\" )
PCBindings=( Action=\"Checkpoint_1\", Key=\"F9\" )
"));
        assert_eq!(with_binding(&bound, ACTION, &[], Some("F8"), None).matches("Key=\"F8\"").count(), 2);
        assert_eq!(with_binding(&bound, ACTION, &[], None, None), ini);
    }

    #[test]
    fn controller_binding_goes_after_each_pad_ready_up() {
        let ini = "[A]\nPCBindings=( Action=\"ReadyUp\", Key=\"Spacebar\" )\nGamepadBindings=( Action=\"ReadyUp\", Key=\"XboxTypeS_A\" )\nSteamInputBindings=( Action=\"ReadyUp\", Key=\"XboxTypeS_A\" )\n";
        let bound = with_binding(ini, ACTION, &[], Some("F9"), Some("DPad_Up"));
        assert!(bound.contains("GamepadBindings=( Action=\"Checkpoint_1\", Key=\"XboxTypeS_DPad_Up\" )"));
        assert!(bound.contains("SteamInputBindings=( Action=\"Checkpoint_1\", Key=\"XboxTypeS_DPad_Up\" )"));
        assert_eq!(bound.matches("Checkpoint_1").count(), 3);
        assert_eq!(with_binding(&bound, ACTION, &[], None, None), ini);
    }

    /// Patch a copy of the real package, reload it and look for the command.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        let backups = std::env::temp_dir().join("hebnix_forfeit_test");
        let _ = std::fs::remove_dir_all(&backups);
        apply(&mut upk, &backups, "b").unwrap();
        let out = std::env::temp_dir().join("hebnix_forfeit_test.upk");
        upk.save_fast(&out).unwrap();
        let reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        let index = object(&reloaded, DONOR).unwrap() as usize - 1;
        let _ = std::fs::remove_dir_all(&backups);
        assert!(is_patched(&reloaded));
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
            (payload(&upk, object(&upk, DONOR).unwrap() as usize - 1), payload(&upk, object(&upk, ACTIONS).unwrap() as usize - 1))
        };
        enable(&cooked, &backups).unwrap();
        assert!(enabled(&backups) && is_patched(&load()));
        enable(&cooked, &backups).unwrap();
        restore(&cooked, &backups).unwrap();
        let upk = load();
        assert!(!enabled(&backups) && !is_patched(&upk));
        assert_eq!(payload(&upk, object(&upk, DONOR).unwrap() as usize - 1), vanilla.0);
        assert_eq!(payload(&upk, object(&upk, ACTIONS).unwrap() as usize - 1), vanilla.1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
