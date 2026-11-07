//! Bots in freeplay. The key adds one of the game's own bots to freeplay. It is
//! a block of the dispatcher (dispatcher.rs) with a key action of its own
//! (actions.rs).
use super::celebration::{call_of, context, jump, land, object};
use super::forfeit_command::{KEYS, set_binding};
use super::script_patch::Script;
use super::upk_package::UpkPackage;
use super::{actions, backup_guard, dispatcher, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
const DEFAULT_KEY: &str = "L";
/// The key action slot (see actions.rs), also the dispatcher block id.
const SLOT: u32 = 3;
const EVENT_CLASS: &str = "GameEvent_Soccar_TA";
const GET_EVENT: &str = "PlayerController_TA.GetGameEvent";
const IS_FREEPLAY: &str = "GameEvent_TA.IsPlayingFreeplay";
const SPAWN_BOT: &str = "GameEvent_TA.SpawnBot";
const RESET: &str = "GameEvent_TA.PlayerResetTraining";
const LOG_MARKER: &str = "HBXBOTSFREEPLAY";

/// Object references the body needs.
struct Refs {
    class: i32,
    get_event: i32,
    is_freeplay: i32,
    spawn_bot: i32,
    reset: i32,
}

/// `GameEvent_Soccar_TA(GetGameEvent())`.
fn event(refs: &Refs) -> Script {
    let mut out = Script::new();
    out.raw(&[0x2e]).object(refs.class).raw(&[0x1c]).object(refs.get_event).raw(&[0x16]);
    out
}

/// What the command runs, with LogInternal(marker) first to show in the log.
///
///     if (GameEvent.IsPlayingFreeplay())
///     {
///         GameEvent.SpawnBot();
///         GameEvent.PlayerResetTraining();
///     }
fn body(refs: &Refs) -> (Script, Vec<usize>) {
    let mut jumps = Vec::new();
    let mut out = Script::new();
    out.raw(&[0xe7, 0x1f]).raw(LOG_MARKER.as_bytes()).raw(&[0x00, 0x48, 0x48, 0x16]);
    let outer = jump(&mut out, 0x07, &mut jumps);
    out.append(&context(&event(refs), 0, &call_of(refs.is_freeplay)));
    out.append(&context(&event(refs), 0, &call_of(refs.spawn_bot)));
    out.append(&context(&event(refs), 0, &call_of(refs.reset)));
    land(&mut out, outer);
    (out, jumps)
}

fn is_patched(upk: &UpkPackage) -> bool {
    dispatcher::has(upk, SLOT as u8) && actions::has(upk, SLOT)
}

/// Add the dispatcher block and the key action that runs it.
pub fn apply(upk: &mut UpkPackage, backups: &Path, build: &str) -> Result<(), String> {
    let refs = Refs {
        class: object(upk, EVENT_CLASS)?,
        get_event: object(upk, GET_EVENT)?,
        is_freeplay: object(upk, IS_FREEPLAY)?,
        spawn_bot: object(upk, SPAWN_BOT)?,
        reset: object(upk, RESET)?,
    };
    let (script, jumps) = body(&refs);
    dispatcher::install(upk, backups, build, SLOT as u8, &script, &jumps)?;
    actions::remove(upk, SLOT)?;
    actions::add(upk, SLOT, &format!("{} {SLOT}", dispatcher::COMMAND))
}

/// The Action name TAInput.ini binds the key to.
fn action() -> String {
    actions::action_name(SLOT)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("BotsFreeplay").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

pub fn enabled(backups: &Path) -> bool {
    read_saved(backups).is_some()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the bots in freeplay command.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads and
/// has (or lacks) the command.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.bots_freeplay.tmp");
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

pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk) {
        return Ok(());
    }
    let build = backup_guard::build_id(cooked)?;
    apply(&mut upk, backups, &build)?;
    install(cooked, &live, &upk, true)?;
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec(&Saved { build }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if actions::has(&upk, SLOT) || dispatcher::has(&upk, SLOT as u8) {
        let build = backup_guard::build_id(cooked)?;
        if saved.build != build {
            return Err("The bots in freeplay backup belongs to another game build. Verify the game files.".into());
        }
        actions::remove(&mut upk, SLOT)?;
        dispatcher::remove(&mut upk, backups, &build, SLOT as u8)?;
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("BotsFreeplay"));
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
    apply(&mut upk, backups, &backup_guard::build_id(cooked)?)?;
    install(cooked, &live, &upk, true)
}

/// Unbind the key, for uninstall.
pub fn unbind() -> Result<(), String> {
    set_binding(&action(), &[], None)
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
            ui.checkbox(&mut want, t("game-mod-bots-freeplay")).on_hover_text(t("game-mod-bots-freeplay-description"));
            ui.label(t("game-mod-bots-freeplay-description"));
            ui.horizontal(|ui| {
                ui.label(t("game-mod-bots-freeplay-key"));
                egui::ComboBox::from_id_salt("bots_freeplay_key").selected_text(key.as_str()).show_ui(ui, |ui| {
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
                    restore(&cooked, &backups).and_then(|_| unbind()).map(|_| t("game-mod-bots-freeplay-off"))
                } else {
                    enable(&cooked, &backups).and_then(|_| set_binding(&action(), &[], Some(&key))).map(|_| t("game-mod-bots-freeplay-on"))
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

    #[test]
    fn body_ends_with_the_reset_and_its_jump_lands_at_the_end() {
        let refs = Refs { class: 100, get_event: 200, is_freeplay: 300, spawn_bot: 400, reset: 500 };
        let (script, jumps) = body(&refs);
        assert_eq!(jumps.len(), 1);
        let target = u16::from_le_bytes([script.bytes()[jumps[0]], script.bytes()[jumps[0] + 1]]) as usize;
        assert_eq!(target, script.mem_size());
        assert_eq!(script.mem_size(), script.len() + 4 * script.refs());
        for reference in [100i32, 200, 300, 400, 500] {
            assert!(script.bytes().windows(4).any(|w| w == reference.to_le_bytes()), "{reference}");
        }
    }
}
