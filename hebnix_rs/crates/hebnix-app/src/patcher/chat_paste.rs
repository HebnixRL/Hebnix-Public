//! Chat paste. Rocket League's match chat box (GFX_Hud_SF, tagame.controls.
//! ChatTextInput) ignores Ctrl+V. The patched HUD movie (see
//! assets/chat-paste/README.md) inserts the clipboard at the caret through the
//! game's own Shell.PasteFromClipboard, line breaks turned into spaces.
//! The boost meter textures live in the same package, so only the movie is
//! swapped; the vanilla movie is kept in Backups/ChatPaste and swapped back to
//! restore. The boost patcher rebuilds the package from its backup and calls
//! `reapply`.
use super::upk_package::UpkPackage;
use super::workshop_menu_ui::{Movie, digest};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "GFX_Hud_SF.upk";

const MOVIE: Movie = Movie {
    package: PACKAGE,
    export: "GFX_Hud.HUD",
    patched: include_bytes!("../../assets/chat-paste/chat_paste_ui.swf"),
    vanilla: "a3011bd5268df871dc8fca3c7d7145269aa841cb91cc8ef4ff3ca7a6de3d33ac",
};

/// earlier builds of our movie, replaced or restored like the current one
const EARLIER: [&str; 2] = [
    "d442038ea61ecbe301e2c753a7f4420b57cd3da7ca9a5c3423f5975b8292bea6",
    "7e465e940ce8a524bbcf1f53047b2bbf3e1e241759002bd170d029bd25c801ff",
];

/// whether the live package holds any build of our movie
fn ours(cooked: &Path) -> bool {
    UpkPackage::load(&super::game_path::resolve(cooked, PACKAGE))
        .and_then(|upk| MOVIE.movie(&upk))
        .is_ok_and(|swf| digest(&swf) == digest(MOVIE.patched) || EARLIER.contains(&digest(&swf).as_str()))
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("ChatPaste").join("vanilla.swf")
}

/// Switched on, even while another feature has put a package without the patch back.
pub fn enabled(backups: &Path) -> bool {
    saved_path(backups).is_file()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing chat paste.".into())
    } else {
        Ok(())
    }
}

/// Swap `swf` into the live package after proving the staged copy loads and holds it.
fn install(cooked: &Path, upk: &mut UpkPackage, swf: &[u8]) -> Result<(), String> {
    let live = super::game_path::resolve(cooked, PACKAGE);
    let temp = cooked.join(format!("{PACKAGE}.paste.tmp"));
    let result = MOVIE.with_movie(upk, swf, &temp).and_then(|()| {
        if digest(&MOVIE.movie(&UpkPackage::load(&temp)?)?) != digest(swf) {
            return Err(format!("Staged {PACKAGE} failed validation"));
        }
        std::fs::copy(&temp, &live).map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
    });
    let _ = std::fs::remove_file(&temp);
    result
}

/// Put the patched movie in over the vanilla one.
fn patch(cooked: &Path, backups: &Path) -> Result<(), String> {
    let mut upk = UpkPackage::load(&super::game_path::resolve(cooked, PACKAGE))?;
    let current = MOVIE.movie(&upk)?;
    if digest(&current) == digest(MOVIE.patched) {
        return Ok(());
    }
    let saved = saved_path(backups);
    if EARLIER.contains(&digest(&current).as_str()) {
        // an earlier build of ours: the vanilla movie is already saved
        if std::fs::read(&saved).map(|swf| digest(&swf)).ok().as_deref() != Some(MOVIE.vanilla) {
            return Err("No clean copy of the HUD movie to update from. Verify the game files.".into());
        }
    } else if digest(&current) != MOVIE.vanilla {
        return Err("This Rocket League version's HUD is not the one chat paste was made for.".into());
    } else {
        std::fs::create_dir_all(saved.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
        std::fs::write(&saved, &current).map_err(|e| e.to_string())?;
    }
    install(cooked, &mut upk, MOVIE.patched)
}

/// Turn chat paste on.
pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    patch(cooked, backups)
}

/// Turn chat paste off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    ensure_closed()?;
    unpatch(cooked, backups)
}

/// Put the vanilla movie back and forget the patch.
fn unpatch(cooked: &Path, backups: &Path) -> Result<(), String> {
    if ours(cooked) {
        let vanilla = std::fs::read(saved_path(backups)).map_err(|e| e.to_string())?;
        if digest(&vanilla) != MOVIE.vanilla {
            return Err("The saved HUD movie is not the vanilla one. Verify the game files.".into());
        }
        let mut upk = UpkPackage::load(&super::game_path::resolve(cooked, PACKAGE))?;
        install(cooked, &mut upk, &vanilla)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("ChatPaste"));
    Ok(())
}

/// Put the patch back after another feature rewrote GFX_Hud_SF.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) || MOVIE.status(cooked) {
        return Ok(());
    }
    ensure_closed()?;
    patch(cooked, backups)
}

/// Page state: whether the patch is in, and the job writing the package.
#[derive(Default)]
pub struct State {
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    status: String,
    /// an earlier build's movie was checked for (and replaced) once the game was closed
    upgraded: bool,
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Draw the toggle.
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
        if on && !self.upgraded && !self.busy() && !hebnix_sdk::process::is_rocket_league_running() {
            self.upgraded = true;
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(reapply(&cooked, &backups).map(|_| String::new()));
            });
        }
        let mut want = on;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-chat-paste")).on_hover_text(t("game-mod-chat-paste-description"));
        });
        ui.label(t("game-mod-chat-paste-description"));
        if want != on {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if want {
                    enable(&cooked, &backups).map(|_| t("game-mod-chat-paste-on"))
                } else {
                    restore(&cooked, &backups).map(|_| t("game-mod-chat-paste-off"))
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

    #[test]
    fn embedded_movie_is_not_the_vanilla_one() {
        assert_eq!(&MOVIE.patched[..3], b"FWS");
        assert_ne!(digest(MOVIE.patched), MOVIE.vanilla);
        assert!(MOVIE.patched.windows(8).any(|w| w == b"hbxPaste"));
    }

    /// Switch chat paste on, rebuild the package as the boost patcher does,
    /// reapply, then restore, on a copy of the real package. Run with
    /// HEBNIX_HUD_UPK set to a vanilla GFX_Hud_SF.upk.
    #[test]
    #[ignore]
    fn enable_reapply_restore_on_a_copy() {
        let source = std::env::var("HEBNIX_HUD_UPK").unwrap();
        let dir = std::env::temp_dir().join(format!("hebnix-chat-paste-{}", std::process::id()));
        let backups = dir.join("Backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::copy(&source, dir.join(PACKAGE)).unwrap();
        let original = digest(&MOVIE.movie(&UpkPackage::load(&dir.join(PACKAGE)).unwrap()).unwrap());
        patch(&dir, &backups).unwrap();
        assert!(enabled(&backups) && MOVIE.status(&dir));
        std::fs::copy(&source, dir.join(PACKAGE)).unwrap();
        assert!(!MOVIE.status(&dir));
        patch(&dir, &backups).unwrap();
        assert!(MOVIE.status(&dir));
        unpatch(&dir, &backups).unwrap();
        assert!(!enabled(&backups) && !MOVIE.status(&dir));
        assert_eq!(digest(&MOVIE.movie(&UpkPackage::load(&dir.join(PACKAGE)).unwrap()).unwrap()), original);
        let _ = std::fs::remove_dir_all(dir);
    }
}
