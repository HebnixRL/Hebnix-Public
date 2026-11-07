//! Instant Suite. When a match ends Rocket League shows the winner prompt
//! (GFX_WinnerMenu_SF, prompts.WinnerPrompt). Its showPodiumSpotlight runs once
//! the game reaches the podium, so Hebnix appends one call to it, chosen by the mode:
//!
//!   requeue   this.onQuickRequeue()            queue again, as the Play Again button does
//!   freeplay  this.handleFreeplayRequest()     go to freeplay, as the Free Play button does
//!   menu      Shell.ExitToMainMenu()           leave to the main menu
//!
//! The edit is ActionScript bytecode in the package's SwfMovie, built from names
//! the movie already has. The movie's original export payload goes to
//! Backups/InstantSuite to restore it.
use super::title_patcher::{abc, movie};
use super::upk_package::{UpkPackage, strip};
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "GFX_WinnerMenu_SF.upk";
const CLASS: &str = "prompts:WinnerPrompt";
const METHOD: &str = "showPodiumSpotlight";

pub const MODES: [&str; 3] = ["requeue", "freeplay", "menu"];
pub const DEFAULT_MODE: &str = "requeue";

fn calls(mode: &str) -> Result<Vec<abc::Call<'static>>, String> {
    Ok(match mode {
        "requeue" => vec![abc::Call::This("onQuickRequeue")],
        "freeplay" => vec![abc::Call::This("handleFreeplayRequest")],
        "menu" => vec![abc::Call::Static { class: "tagame.api:Shell", name: "ExitToMainMenu" }],
        other => return Err(format!("Unknown Instant Suite mode {other}")),
    })
}

/// `payload` of the movie export with the mode's call appended, or None when
/// this export is not the winner prompt's movie.
fn patched_payload(upk: &UpkPackage, index: usize, mode: &str) -> Result<Option<Vec<u8>>, String> {
    let export = &upk.exports[index];
    if !matches!(strip(&upk.class_of(export)), "SwfMovie" | "GFxMovieInfo") {
        return Ok(None);
    }
    let (props, _) = upk.serialized_props(export)?;
    let Some(raw) = props.iter().find(|p| p.name == "RawData" && p.tag_type == "ArrayProperty") else {
        return Ok(None);
    };
    let original = payload(upk, index);
    let start = raw.value_offset;
    let array = original.get(start..start + raw.size).ok_or("Invalid movie array")?;
    let len = super::upk_package::read_i32(array, 0)?;
    if len < 0 || len as usize + 4 != array.len() {
        return Err("Unsupported movie array framing".into());
    }
    if raw.value_offset != raw.tag_offset + 24 {
        return Err("Unsupported movie property tag".into());
    }
    let calls = calls(mode)?;
    let edit = |abc_bytes: &[u8]| abc::append_calls(abc_bytes, CLASS, METHOD, &calls);
    let Some((movie, _)) = movie::rewrite(&array[4..], &edit)? else {
        return Ok(None);
    };
    let mut out = original[..start].to_vec();
    out[raw.tag_offset + 16..raw.tag_offset + 20].copy_from_slice(&((movie.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(&(movie.len() as u32).to_le_bytes());
    out.extend(movie);
    out.extend_from_slice(&original[start + raw.size..]);
    Ok(Some(out))
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

/// Patch the winner prompt's movie. Returns the export index and its vanilla payload.
fn apply(upk: &mut UpkPackage, mode: &str) -> Result<(usize, Vec<u8>), String> {
    for index in 0..upk.exports.len() {
        if let Some(patched) = patched_payload(upk, index, mode)? {
            let original = payload(upk, index);
            upk.replace_export_payload(index, &patched)?;
            return Ok((index, original));
        }
    }
    Err(format!("{CLASS} was not found in {PACKAGE}"))
}

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

/// The active mode, or None when Instant Suite is off.
pub fn mode(backups: &Path) -> Option<String> {
    read_saved(backups).map(|s| s.mode)
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing Instant Suite.".into())
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
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
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

/// Turn Instant Suite on with `mode`, or change its mode.
pub fn enable(cooked: &Path, backups: &Path, mode: &str) -> Result<(), String> {
    ensure_closed()?;
    calls(mode)?;
    let live = game_path::resolve(cooked, PACKAGE);
    let dir = folder(backups);
    let mut upk = UpkPackage::load(&live)?;
    let build = backup_guard::build_id(cooked)?;
    if let Some(saved) = read_saved(backups) {
        // Changing mode: start again from the vanilla payload.
        if saved.build != build {
            return Err("The Instant Suite backup belongs to another game build. Verify the game files.".into());
        }
        let original = std::fs::read(dir.join("original.bin")).map_err(|e| e.to_string())?;
        upk.replace_export_payload(saved.export, &original)?;
    }
    let (export, original) = apply(&mut upk, mode)?;
    install(cooked, &live, &upk)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if read_saved(backups).is_none() {
        std::fs::write(dir.join("original.bin"), original).map_err(|e| e.to_string())?;
    }
    let saved = Saved { build, mode: mode.into(), export };
    std::fs::write(dir.join("state.json"), serde_json::to_vec(&saved).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Turn Instant Suite off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if saved.build != backup_guard::build_id(cooked)? {
        return Err("The Instant Suite backup belongs to another game build. Verify the game files.".into());
    }
    let original = std::fs::read(folder(backups).join("original.bin")).map_err(|e| e.to_string())?;
    upk.replace_export_payload(saved.export, &original)?;
    install(cooked, &live, &upk)?;
    let _ = std::fs::remove_dir_all(folder(backups));
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

    /// Draw the toggle and mode picker. Returns true when `mode` changed.
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, mode_setting: &mut String) -> bool {
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
        if !MODES.contains(&mode_setting.as_str()) {
            *mode_setting = DEFAULT_MODE.into();
        }
        let active = mode(backups);
        let on = active.is_some();
        let mut want = on;
        let mut changed = false;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-instant-suite")).on_hover_text(t("game-mod-instant-suite-description"));
            ui.label(t("game-mod-instant-suite-description"));
            ui.horizontal(|ui| {
                ui.label(t("game-mod-instant-suite-mode"));
                egui::ComboBox::from_id_salt("instant_suite_mode")
                    .selected_text(t(&format!("game-mod-instant-suite-mode-{mode_setting}")))
                    .show_ui(ui, |ui| {
                        for option in MODES {
                            changed |= ui
                                .selectable_value(mode_setting, option.to_string(), t(&format!("game-mod-instant-suite-mode-{option}")))
                                .changed();
                        }
                    });
            });
        });
        if want != on || (changed && on) {
            let (cooked, backups, mode_now) = (cooked.to_path_buf(), backups.to_path_buf(), mode_setting.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if !want {
                    restore(&cooked, &backups).map(|_| t("game-mod-instant-suite-off"))
                } else {
                    enable(&cooked, &backups, &mode_now).map(|_| t("game-mod-instant-suite-on"))
                };
                let _ = tx.send(result);
            });
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

    /// Patch a copy of the real winner-menu package for each mode, reload it
    /// and check the movie changed and parses. Run with HEBNIX_WINNER_UPK set
    /// to a vanilla GFX_WinnerMenu_SF.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_WINNER_UPK").unwrap();
        for mode in MODES {
            let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
            let (index, original) = apply(&mut upk, mode).unwrap();
            assert_ne!(payload(&upk, index), original);
            let out = std::env::temp_dir().join(format!("hebnix_instant_{mode}.upk"));
            upk.save_fast(&out).unwrap();
            let reloaded = UpkPackage::load(&out).unwrap();
            // Keep the patched copies for inspection when asked.
            match std::env::var("HEBNIX_WINNER_KEEP") {
                Ok(dir) => {
                    let _ = std::fs::copy(&out, Path::new(&dir).join(format!("{mode}.upk")));
                    let _ = std::fs::remove_file(out);
                }
                Err(_) => {
                    let _ = std::fs::remove_file(out);
                }
            }
            assert_eq!(payload(&reloaded, index), payload(&upk, index));
            // A second pass must refuse to stack on the patched movie.
            let mut again = reloaded;
            assert!(apply(&mut again, mode).is_err());
        }
    }

    #[test]
    fn unknown_mode_is_rejected() {
        assert!(calls("nothing").is_err());
        assert!(MODES.iter().all(|m| calls(m).is_ok()));
    }
}
