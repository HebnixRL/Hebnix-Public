//! Menu access in game. Outside the main menu the Play menu disables cards
//! such as Private Match and LAN with "Access is disabled in your current game
//! mode". MenuTreeNodeEnabledChecker_TA.UpdateGFxNodeEnabled decides that from
//!
//!     bInMainMenu = ...GFxData_System_TA.UIState == "MainMenu";
//!
//! The patch turns it into `UIState != "MainMenX"`, which is always true, by
//! writing two bytes in place: the string == operator becomes != and one letter
//! of the literal changes. The function keeps its size and jump offsets.
use super::upk_package::UpkPackage;
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
const FUNCTION: &str = "MenuTreeNodeEnabledChecker_TA.UpdateGFxNodeEnabled";
/// LetBool, BoolVariable, LocalVariable: the start of `bInMainMenu = `.
const LET_LOCAL_BOOL: [u8; 3] = [0x14, 0x2d, 0x2b];
/// String == and != in this build's opcode map.
const STR_EQUAL: u8 = 0xcc;
const STR_NOT_EQUAL: u8 = 0xc0;
const LITERAL: &[u8] = b"\x1fMainMenu\x00";
const PATCHED_LITERAL: &[u8] = b"\x1fMainMenX\x00";

/// Where the operator and the literal sit in the function's bytes, and whether
/// they are patched. None when the function no longer looks as expected.
fn locate(bytes: &[u8]) -> Option<(usize, usize, bool)> {
    let find = |needle: &[u8]| {
        let mut hits = bytes.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle);
        let first = hits.next()?.0;
        hits.next().is_none().then_some(first)
    };
    let (literal, patched) = match (find(LITERAL), find(PATCHED_LITERAL)) {
        (Some(at), None) => (at, false),
        (None, Some(at)) => (at, true),
        _ => return None,
    };
    let operator = if patched { STR_NOT_EQUAL } else { STR_EQUAL };
    // the operator follows the local's 4-byte index and precedes the literal
    let op = (0..literal).rev().find(|&i| {
        i >= 7 && bytes[i] == operator && bytes[i - 7..i - 4] == LET_LOCAL_BOOL
    })?;
    Some((op, literal, patched))
}

fn function_range(upk: &UpkPackage) -> Result<(usize, usize), String> {
    let index = upk.find_object(FUNCTION).ok_or_else(|| format!("{FUNCTION} was not found in {PACKAGE}"))?;
    let export = usize::try_from(index - 1)
        .ok()
        .and_then(|i| upk.exports.get(i))
        .ok_or_else(|| format!("{FUNCTION} is not an export"))?;
    Ok((export.serial_offset, export.serial_size))
}

fn state(upk: &UpkPackage) -> Result<(usize, usize, bool), String> {
    let (start, size) = function_range(upk)?;
    let bytes = upk.image.get(start..start + size).ok_or("The menu check is truncated")?;
    let (op, literal, patched) = locate(bytes).ok_or("The menu check changed in this game build")?;
    Ok((start + op, start + literal, patched))
}

fn set_patched(upk: &mut UpkPackage, patched: bool) -> Result<(), String> {
    let (op, literal, current) = state(upk)?;
    if current == patched {
        return Ok(());
    }
    let (operator, text) = if patched { (STR_NOT_EQUAL, PATCHED_LITERAL) } else { (STR_EQUAL, LITERAL) };
    upk.patch(op, &[operator])?;
    upk.patch(literal, text)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("MenuAccess").join("state.json")
}

pub fn enabled(backups: &Path) -> bool {
    saved_path(backups).is_file()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing menu access in game.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads with
/// the wanted state.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.menu_access.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp).and_then(|saved| state(&saved).map(|(_, _, on)| on));
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
    backup_guard::check_install(cooked, backups)?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if !state(&upk)?.2 {
        set_patched(&mut upk, true)?;
        install(cooked, &live, &upk, true)?;
    }
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    let build = backup_guard::build_id(cooked)?;
    std::fs::write(path, serde_json::to_vec(&Saved { build }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if state(&upk)?.2 {
        set_patched(&mut upk, false)?;
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("MenuAccess"));
    Ok(())
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

    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path) {
        use crate::i18n::t;
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
        let on = enabled(backups);
        let mut want = on;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-menu-access")).on_hover_text(t("game-mod-menu-access-description"));
            ui.separator();
            ui.label(t("game-mod-menu-access-description"));
        });
        if want != on {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if want {
                    enable(&cooked, &backups).map(|_| t("game-mod-menu-access-on"))
                } else {
                    restore(&cooked, &backups).map(|_| t("game-mod-menu-access-off"))
                };
                let _ = tx.send(result);
            });
        }
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `InGFxNode.bEnabled = true; bInMainMenu = <ctx> == "MainMenu";` as in the live build.
    fn function() -> Vec<u8> {
        let mut bytes = vec![0x14, 0x5e, 0x19, 0x00, 0x2b, 0x1b, 0xc4, 0x00, 0x00, 0x27];
        bytes.extend([0x14, 0x2d, 0x2b, 0x19, 0xc4, 0x00, 0x00, STR_EQUAL, 0x5e, 0x19, 0x00, 0x1c, 0x00]);
        bytes.extend(LITERAL);
        bytes.extend([0x16, 0x57]);
        bytes
    }

    #[test]
    fn patch_and_restore_flip_only_the_operator_and_one_letter() {
        let original = function();
        let (op, literal, patched) = locate(&original).unwrap();
        assert!(!patched);
        assert_eq!(original[op], STR_EQUAL);
        let mut bytes = original.clone();
        bytes[op] = STR_NOT_EQUAL;
        bytes[literal..literal + PATCHED_LITERAL.len()].copy_from_slice(PATCHED_LITERAL);
        assert_eq!(original.iter().zip(&bytes).filter(|(a, b)| a != b).count(), 2);
        assert_eq!(locate(&bytes), Some((op, literal, true)));
    }

    #[test]
    fn an_unexpected_body_is_refused() {
        let mut bytes = function();
        bytes.extend(LITERAL);
        assert_eq!(locate(&bytes), None);
        let mut bytes = function();
        let op = bytes.iter().position(|&b| b == STR_EQUAL).unwrap();
        bytes[op] = 0x7a;
        assert_eq!(locate(&bytes), None);
    }
}
