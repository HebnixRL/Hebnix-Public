//! Goal explosion size. Rocket League calls Ball_TA.SetExplosionFXActor with
//! the explosion's actor when a goal is scored. The patch adds
//!
//!     ExplosionFX.SetDrawScale(scale);
//!
//! before that function returns, the way Ball_TA.SetBallScale scales the
//! ball. The function is relocated, so Hebnix keeps its original payload in
//! Backups/GoalExplosion and puts it back to restore. The avatar and Workshop
//! patches only touch their own exports and carry this one along; Colours
//! rebuilds TAGame.upk from a pristine copy and calls `reapply`.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FUNCTION: &str = "Ball_TA.SetExplosionFXActor";
const SET_DRAW_SCALE: &str = "Engine.Actor.SetDrawScale";
/// Return, Nothing, EndOfScript: how every function here ends.
const END: [u8; 3] = [0x04, 0x0b, 0x4c];

pub const MIN_SCALE: f32 = 0.1;
pub const MAX_SCALE: f32 = 10.0;

/// The statement `ExplosionFX.SetDrawScale(scale)`, copied from the game's own
/// context calls: Context, no fail flag, the object (a local, 0x2b), skip
/// size of the call, no return property, then the final-function call.
fn statement(parameter: i32, set_draw_scale: i32, scale: f32) -> Script {
    let mut call = Script::new();
    call.raw(&[0x1c]).object(set_draw_scale).raw(&[0x1e]).raw(&scale.to_le_bytes()).raw(&[0x16]);
    let mut out = Script::new();
    out.raw(&[0x5e, 0x19, 0x00, 0x2b]).object(parameter);
    out.raw(&(call.mem_size() as u16).to_le_bytes());
    out.object(0).raw(&[0x00]).append(&call);
    out
}

pub fn apply(upk: &mut UpkPackage, scale: f32) -> Result<(), String> {
    if !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return Err(format!("Goal explosion scale must be between {MIN_SCALE} and {MAX_SCALE}"));
    }
    let object = |path: &str| upk.find_object(path).ok_or_else(|| format!("{path} was not found"));
    let parameter = object(&format!("{FUNCTION}.ExplosionFX"))?;
    let set_draw_scale = object(SET_DRAW_SCALE)?;
    let function = object(FUNCTION)?;
    let export = &upk.exports[function as usize - 1];
    let old = upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec();
    let mut script = Script::of_function(&old)?;
    if script.bytes().windows(4).any(|w| w == set_draw_scale.to_le_bytes()) {
        return Err(format!("{FUNCTION} already scales the explosion"));
    }
    if !script.bytes().ends_with(&END) {
        return Err(format!("{FUNCTION} changed layout in this Rocket League version"));
    }
    script.split_off(END.len());
    script.append(&statement(parameter, set_draw_scale, scale)).raw(&END);
    replace_script(upk, FUNCTION, &script)
}


const PACKAGE: &str = "TAGame.upk";

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
    scale: f32,
    /// SetExplosionFXActor's export payload before the patch.
    original: Vec<u8>,
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("GoalExplosion").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

/// The scale the patch is set to, if it is on.
pub fn current(backups: &Path) -> Option<f32> {
    read_saved(backups).map(|saved| saved.scale)
}

fn function_index(upk: &UpkPackage) -> Result<usize, String> {
    upk.find_object(FUNCTION)
        .filter(|index| *index > 0)
        .map(|index| index as usize - 1)
        .ok_or_else(|| format!("{FUNCTION} was not found"))
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

fn is_patched(upk: &UpkPackage, index: usize) -> Result<bool, String> {
    let set_draw_scale = upk
        .find_object(SET_DRAW_SCALE)
        .ok_or_else(|| format!("{SET_DRAW_SCALE} was not found"))?;
    Ok(Script::of_function(&payload(upk, index))?
        .bytes()
        .windows(4)
        .any(|w| w == set_draw_scale.to_le_bytes()))
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the goal explosion scale.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads and
/// has (or lacks) the patch.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.goal_explosion.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp)
        .and_then(|saved| function_index(&saved).and_then(|i| is_patched(&saved, i)));
    if check != Ok(patched) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

/// Turn the patch on, or change its scale.
pub fn enable(cooked: &Path, backups: &Path, scale: f32) -> Result<(), String> {
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    let index = function_index(&upk)?;
    let build = backup_guard::build_id(cooked)?;
    let original = if is_patched(&upk, index)? {
        let saved = read_saved(backups)
            .filter(|saved| saved.build == build)
            .ok_or("The goal explosion patch has no matching backup. Verify the game files.")?;
        upk.replace_export_payload(index, &saved.original)?;
        saved.original
    } else {
        payload(&upk, index)
    };
    apply(&mut upk, scale)?;
    install(cooked, &live, &upk, true)?;
    let saved = Saved { build, scale, original };
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec(&saved).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Turn the patch off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    let index = function_index(&upk)?;
    if is_patched(&upk, index)? {
        if saved.build != backup_guard::build_id(cooked)? {
            return Err("The goal explosion backup belongs to another game build. Verify the game files.".into());
        }
        upk.replace_export_payload(index, &saved.original)?;
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("GoalExplosion"));
    Ok(())
}

/// Put the patch back after another feature rewrote TAGame.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    match read_saved(backups) {
        Some(saved) => enable(cooked, backups, saved.scale),
        None => Ok(()),
    }
}

/// Page state: the slider and the job writing the package.
pub struct State {
    scale: f32,
    loaded: bool,
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    status: String,
}

impl Default for State {
    fn default() -> Self {
        Self { scale: 3.0, loaded: false, job: None, status: String::new() }
    }
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path) {
        use crate::i18n::{t, t_args};
        use eframe::egui;
        if !self.loaded {
            self.loaded = true;
            if let Some(scale) = current(backups) {
                self.scale = scale;
            }
        }
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
        ui.heading(t("nav-goal-explosion-scale"));
        ui.label(t("goal-explosion-description"));
        ui.add_space(8.0);
        ui.add_enabled(
            !self.busy(),
            egui::Slider::new(&mut self.scale, MIN_SCALE..=MAX_SCALE).logarithmic(true).text("x"),
        );
        ui.horizontal(|ui| {
            let apply_clicked = ui
                .add_enabled(!self.busy(), egui::Button::new(t("goal-explosion-apply")))
                .clicked();
            let restore_clicked = ui
                .add_enabled(
                    !self.busy() && current(backups).is_some(),
                    egui::Button::new(t("goal-explosion-restore")),
                )
                .clicked();
            if apply_clicked || restore_clicked {
                let (cooked, backups, scale) = (cooked.to_path_buf(), backups.to_path_buf(), self.scale);
                let (tx, rx) = std::sync::mpsc::channel();
                self.job = Some(rx);
                self.status.clear();
                std::thread::spawn(move || {
                    let result = if apply_clicked {
                        enable(&cooked, &backups, scale).map(|_| {
                            t_args("goal-explosion-applied", &[("scale", format!("{scale:.2}").into())])
                        })
                    } else {
                        restore(&cooked, &backups).map(|_| t("goal-explosion-restored"))
                    };
                    let _ = tx.send(result);
                });
            }
        });
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        } else if let Some(scale) = current(backups) {
            ui.label(t_args("goal-explosion-active", &[("scale", format!("{scale:.2}").into())]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::script_patch::function_payload;

    #[test]
    fn statement_matches_the_games_own_call_shape() {
        // Ball_TA.OnWarnBallReset: 5e 19 00 01 <obj> 13 00 | 00 00 00 00 00 | 1c <fn> 01 <obj> 16
        let s = statement(0x2ad2, -1986, 2.0);
        assert_eq!(
            s.bytes(),
            [
                0x5e, 0x19, 0x00, 0x2b, 0xd2, 0x2a, 0x00, 0x00, 0x0f, 0x00, 0, 0, 0, 0, 0x00, 0x1c, 0x3e, 0xf8,
                0xff, 0xff, 0x1e, 0, 0, 0, 0x40, 0x16
            ]
        );
        // call = 1 + 8 + 5 + 1 in memory; the statement adds the object and the return property
        assert_eq!(s.mem_size(), s.len() + 4 * 3);
    }

    /// Patch a copy of the real package and read it back. Run with
    /// HEBNIX_UPK_TEST_SOURCE set to a vanilla TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(std::path::Path::new(&source)).unwrap();
        let index = upk.find_object(FUNCTION).unwrap() as usize - 1;
        let before = {
            let e = &upk.exports[index];
            upk.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec()
        };
        apply(&mut upk, 3.0).unwrap();
        assert!(apply(&mut upk, 3.0).is_err(), "second apply must refuse");
        let out = std::env::temp_dir().join("hebnix_goal_explosion_test.upk");
        upk.save(&out).unwrap();
        let reloaded = UpkPackage::load(&out).unwrap();
        let e = &reloaded.exports[index];
        let after = reloaded.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec();
        let added = statement(reloaded.find_object(&format!("{FUNCTION}.ExplosionFX")).unwrap(), reloaded.find_object(SET_DRAW_SCALE).unwrap(), 3.0);
        assert_eq!(after.len(), before.len() + added.len());
        let rebuilt = function_payload(&before, &{
            let mut s = Script::of_function(&before).unwrap();
            s.split_off(3);
            s.append(&added).raw(&END);
            s
        })
        .unwrap();
        assert_eq!(after, rebuilt);
        let _ = std::fs::remove_file(out);
    }

    /// enable, change the scale, restore, on a copy of the real package laid
    /// out like an install. Run with HEBNIX_UPK_TEST_SOURCE set to a vanilla
    /// TAGame.upk and Rocket League closed.
    #[test]
    #[ignore]
    fn enable_change_restore() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let root = std::env::temp_dir().join("hebnix_goal_explosion_install");
        let _ = std::fs::remove_dir_all(&root);
        let cooked = root.join("TAGame").join("CookedPCConsole");
        let backups = cooked.join("Backups");
        std::fs::create_dir_all(&cooked).unwrap();
        std::fs::create_dir_all(root.join("TAGame/Binaries/Win64")).unwrap();
        std::fs::write(root.join("TAGame/Binaries/Win64/RocketLeague.exe"), b"exe").unwrap();
        std::fs::copy(&source, cooked.join("TAGame.upk")).unwrap();
        let vanilla = {
            let upk = UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
            let index = function_index(&upk).unwrap();
            payload(&upk, index)
        };
        let state = || {
            let upk = UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
            let index = function_index(&upk).unwrap();
            (is_patched(&upk, index).unwrap(), payload(&upk, index))
        };
        enable(&cooked, &backups, 3.0).unwrap();
        assert_eq!(current(&backups), Some(3.0));
        let (patched, first) = state();
        assert!(patched && first.len() > vanilla.len());
        enable(&cooked, &backups, 5.0).unwrap();
        let (patched, second) = state();
        assert!(patched && second.len() == first.len() && second != first);
        assert_eq!(read_saved(&backups).unwrap().original, vanilla);
        restore(&cooked, &backups).unwrap();
        assert_eq!(current(&backups), None);
        let (patched, restored) = state();
        assert!(!patched);
        assert_eq!(restored, vanilla);
        let _ = std::fs::remove_dir_all(&root);
    }
}
