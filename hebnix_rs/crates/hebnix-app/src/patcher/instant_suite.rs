//! Instant Suite. When a match ends Rocket League shows the winner prompt
//! (GFX_WinnerMenu_SF, prompts.WinnerPrompt). As it announces the winner, the
//! patched movie asks Hebnix's preview server what to do (see
//! assets/instant-suite/README.md), so the action can change while the game
//! runs. Hebnix only answers right after the Stats API's MatchEnded:
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
use std::time::{Duration, Instant};

const PACKAGE: &str = "GFX_WinnerMenu_SF.upk";

const MOVIE: Movie = Movie {
    package: PACKAGE,
    export: "GFX_WinnerMenu.WinnerMenu",
    patched: include_bytes!("../../assets/instant-suite/winner_menu_ui.swf"),
    vanilla: "28f28cd0d1f94f3d4926a381a903514b4b14d69b90ad73a1676cd1b19cf60069",
};

pub const MODES: [&str; 4] = ["requeue", "freeplay", "menu", "workshop"];
pub const DEFAULT_MODE: &str = "requeue";

/// What the movie is told: the PNG width it reads, 0 for nothing.
static CODE: AtomicU8 = AtomicU8::new(0);
/// When the Stats API last said a match ended. The movie only gets an answer
/// right after one, so a winner left over from an earlier match does nothing.
static ENDED: Mutex<Option<Instant>> = Mutex::new(None);
const ENDED_FOR: Duration = Duration::from_secs(30);
/// The movie asks as the winner shows, the same moment MatchEnded is sent: wait this long for it.
const WAIT_FOR_END: Duration = Duration::from_secs(3);

pub fn match_ended() {
    if let Ok(mut ended) = ENDED.lock() {
        *ended = Some(Instant::now());
    }
}

fn ended_recently() -> bool {
    ENDED.lock().ok().and_then(|e| *e).is_some_and(|at| at.elapsed() < ENDED_FOR)
}

/// The width of the image the preview server answers the movie with, or None
/// to answer 404 so the winner screen stays. Blocks up to WAIT_FOR_END.
pub fn served_width() -> Option<u32> {
    let asked = Instant::now();
    while !ended_recently() {
        if asked.elapsed() >= WAIT_FOR_END {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // used once: the podium asks again later in the same match
    if let Ok(mut ended) = ENDED.lock() {
        *ended = None;
    }
    match CODE.load(Ordering::Relaxed) {
        0 => None,
        n => Some(u32::from(n)),
    }
}

/// Whether the preview server must run for Instant Suite.
pub fn wanted() -> bool {
    CODE.load(Ordering::Relaxed) != 0
}

fn code(mode: &str) -> u8 {
    MODES.iter().position(|m| *m == mode).map_or(0, |i| i as u8 + 1)
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
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
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
        return Err("The Instant Suite backup belongs to another game build. Verify the game files.".into());
    }
    let original = std::fs::read(folder(backups).join("original.bin")).map_err(|e| e.to_string())?;
    upk.replace_export_payload(saved.export, &original)?;
    install(cooked, &live, &upk)?;
    let _ = std::fs::remove_dir_all(folder(backups));
    Ok(())
}

/// Whether Instant Suite is on, in this build's form or an earlier one's.
fn status(cooked: &Path, backups: &Path) -> bool {
    read_saved(backups).is_some() || MOVIE.status(cooked)
}

/// Turn Instant Suite on.
pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    restore_legacy(cooked, backups)?;
    MOVIE.apply(cooked, backups)
}

/// Turn Instant Suite off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !status(cooked, backups) {
        return Ok(());
    }
    ensure_closed()?;
    restore_legacy(cooked, backups)?;
    MOVIE.restore(cooked, backups)
}

/// Page state: whether the patch is in, and the job writing the package.
#[derive(Default)]
pub struct State {
    job: Option<std::sync::mpsc::Receiver<Result<bool, String>>>,
    status: String,
    on: Option<bool>,
    /// an earlier build's byte patch, replaced once the game is closed
    legacy: Option<bool>,
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
        if self.on.is_none() && !self.busy() && cooked.is_dir() {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            self.legacy = Some(read_saved(&backups).is_some());
            self.spawn(move || Ok(status(&cooked, &backups)));
        }
        if self.legacy == Some(true) && self.on == Some(true) && !self.busy() && !hebnix_sdk::process::is_rocket_league_running() {
            self.legacy = Some(false);
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            self.spawn(move || enable(&cooked, &backups).map(|()| true));
        }
        let on = self.on == Some(true) && self.legacy != Some(true);
        CODE.store(if on { code(mode) } else { 0 }, Ordering::Relaxed);
        let flag = if on && mode == "workshop" { map } else { "" };
        if self.flagged.as_deref() != Some(flag) {
            self.flagged = Some(flag.to_string());
            super::workshop_menu::set_next(Path::new(flag));
            let cooked = cooked.to_path_buf();
            std::thread::spawn(move || drop(crate::ui::workshop::ingame_menu::refresh(&cooked)));
        }
    }

    /// Draw the toggle, the action picker and, for a Workshop map, the map picker.
    /// Returns true when a setting changed.
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, mode_setting: &mut String, map_setting: &mut String) -> bool {
        use crate::i18n::t;
        use eframe::egui;
        if !MODES.contains(&mode_setting.as_str()) {
            *mode_setting = DEFAULT_MODE.into();
        }
        let on = self.on == Some(true);
        let mut want = on;
        let mut changed = false;
        ui.add_enabled_ui(!self.busy() && self.on.is_some(), |ui| {
            ui.checkbox(&mut want, t("game-mod-instant-suite")).on_hover_text(t("game-mod-instant-suite-description"));
        });
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
    fn answers_only_once_after_a_match_end() {
        CODE.store(2, Ordering::Relaxed);
        match_ended();
        assert_eq!(served_width(), Some(2));
        // the same match's podium asks again: no second action
        let asked = Instant::now();
        assert_eq!(served_width(), None);
        assert!(asked.elapsed() >= WAIT_FOR_END);
        CODE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn every_mode_has_its_own_width() {
        let codes: Vec<u8> = MODES.iter().map(|m| code(m)).collect();
        assert_eq!(codes, [1, 2, 3, 4]);
        assert_eq!(code("nothing"), 0);
    }

    /// Switch Instant Suite on and off over a copy of the real package. Run with
    /// HEBNIX_WINNER_UPK set to a vanilla GFX_WinnerMenu_SF.upk.
    #[test]
    #[ignore]
    fn apply_and_restore_on_a_copy() {
        let source = std::env::var("HEBNIX_WINNER_UPK").unwrap();
        let root = std::env::temp_dir().join(format!("hebnix-winner-{}", std::process::id()));
        let dir = root.join("TAGame").join("CookedPCConsole");
        let backups = dir.join("Backups");
        std::fs::create_dir_all(&backups).unwrap();
        // backups are tied to the game build, read from the exe
        std::fs::create_dir_all(root.join("Binaries").join("Win64")).unwrap();
        std::fs::write(root.join("Binaries").join("Win64").join("RocketLeague.exe"), b"build").unwrap();
        std::fs::copy(&source, dir.join(PACKAGE)).unwrap();
        MOVIE.apply(&dir, &backups).unwrap();
        assert!(status(&dir, &backups));
        MOVIE.restore(&dir, &backups).unwrap();
        assert!(!status(&dir, &backups));
        assert_eq!(std::fs::read(dir.join(PACKAGE)).unwrap(), std::fs::read(&source).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn embedded_movie_is_not_the_vanilla_one() {
        assert_eq!(&MOVIE.patched[..3], b"FWS");
        assert!(MOVIE.patched.windows(6).any(|w| w == b"hbxAsk"));
    }
}
