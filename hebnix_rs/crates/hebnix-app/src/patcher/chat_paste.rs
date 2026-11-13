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
use super::title_patcher::{self, TitleOverride};

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

fn has_hook(movie: &[u8]) -> bool {
    let found = std::cell::Cell::new(false);
    let _ = title_patcher::movie::rewrite(movie, &|data| {
        if data.windows(8).any(|name| name == b"hbxPaste") {
            found.set(true);
        }
        Ok(None)
    });
    found.get()
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
fn install(cooked: &Path, before: &[u8], upk: &mut UpkPackage, swf: &[u8]) -> Result<(), String> {
    let live = super::game_path::resolve(cooked, PACKAGE);
    let temp = cooked.join(format!("{PACKAGE}.paste.tmp"));
    let result = MOVIE.with_movie(upk, swf, &temp).and_then(|()| {
        if digest(&MOVIE.movie(&UpkPackage::load(&temp)?)?) != digest(swf) {
            return Err(format!("Staged {PACKAGE} failed validation"));
        }
        super::upk_history::install_linked_from(cooked, &live, before, &std::fs::read(&temp).map_err(|e| e.to_string())?, "Chat Paste")
    });
    let _ = std::fs::remove_file(&temp);
    result
}

/// Put the patched movie in over the vanilla one.
#[cfg(test)]
fn patch(cooked: &Path, backups: &Path) -> Result<(), String> {
    patch_with_rules(cooked, backups, &[])
}

fn patch_with_rules(cooked: &Path, backups: &Path, rules: &[TitleOverride]) -> Result<(), String> {
    let _guard = title_patcher::UI_PATCH_LOCK.lock().map_err(|_| "UI patch lock unavailable")?;
    super::backup_guard::check_install(cooked, backups)?;
    let live = super::game_path::resolve(cooked, PACKAGE);
    let before = std::fs::read(&live).map_err(|e| e.to_string())?;
    let mut upk = UpkPackage::from_bytes(before.clone())?;
    let current = MOVIE.movie(&upk)?;
    let saved = saved_path(backups);
    let had_backup = saved.is_file();
    if digest(&current) == digest(MOVIE.patched) || (has_hook(&current) && !EARLIER.contains(&digest(&current).as_str())) {
        if !saved.is_file() {
            let original = MOVIE.recover_original(cooked, backups, &upk)
                .ok_or("Chat Paste is already installed, but its restore baseline is missing.")?;
            std::fs::create_dir_all(saved.parent().unwrap()).map_err(|e| e.to_string())?;
            super::upk_history::replace(&saved, &original)?;
        }
        return Ok(());
    }
    if EARLIER.contains(&digest(&current).as_str()) {
        // an earlier build of ours: the vanilla movie is already saved
        if std::fs::read(&saved).map(|swf| digest(&swf)).ok().as_deref() != Some(MOVIE.vanilla) {
            return Err("No clean copy of the HUD movie to update from. Verify the game files.".into());
        }
    } else if digest(&current) != MOVIE.vanilla {
        let index = usize::try_from(upk.find_object(MOVIE.export).ok_or("HUD movie is missing")? - 1)
            .map_err(|_| "Invalid HUD movie export")?;
        let existing = saved.is_file();
        let original = MOVIE.recover_original(cooked, backups, &upk)
            .ok_or("No compatible HUD baseline is available for Chat Paste.")?;
        std::fs::create_dir_all(saved.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
        super::upk_history::replace(&saved, &original)?;
        let result = title_patcher::rebase_hud_movie(cooked, &live, &before, upk, index, rules, &|raw| {
            if digest(raw) != MOVIE.vanilla {
                return Err("Chat Paste could not identify the original HUD code beneath Titles.".into());
            }
            let mut patched = MOVIE.patched.to_vec();
            patched[..3].copy_from_slice(b"GFX");
            Ok(Some((patched, 1)))
        }).and_then(|applied| {
            if applied { Ok(()) } else { Err("The HUD movie has edits outside the Titles restore record.".into()) }
        });
        if result.is_err() && !existing {
            let _ = std::fs::remove_file(&saved);
        }
        return result;
    } else {
        std::fs::create_dir_all(saved.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
        std::fs::write(&saved, &current).map_err(|e| e.to_string())?;
    }
    if let Err(error) = install(cooked, &before, &mut upk, MOVIE.patched) {
        if !had_backup && digest(&current) == MOVIE.vanilla {
            let _ = std::fs::remove_file(&saved);
        }
        return Err(error);
    }
    Ok(())
}

/// Turn chat paste on.
fn enable(cooked: &Path, backups: &Path, rules: &[TitleOverride]) -> Result<(), String> {
    ensure_closed()?;
    patch_with_rules(cooked, backups, rules)
}

/// Turn chat paste off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    restore_with_rules(cooked, backups, &[])
}

fn restore_with_rules(cooked: &Path, backups: &Path, rules: &[TitleOverride]) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    ensure_closed()?;
    super::backup_guard::check_install(cooked, backups)?;
    unpatch_with_rules(cooked, backups, rules)
}

/// Put the vanilla movie back and forget the patch.
#[cfg(test)]
fn unpatch(cooked: &Path, backups: &Path) -> Result<(), String> {
    unpatch_with_rules(cooked, backups, &[])
}

fn unpatch_with_rules(cooked: &Path, backups: &Path, rules: &[TitleOverride]) -> Result<(), String> {
    let _guard = title_patcher::UI_PATCH_LOCK.lock().map_err(|_| "UI patch lock unavailable")?;
    let live = super::game_path::resolve(cooked, PACKAGE);
    let before = std::fs::read(&live).map_err(|e| e.to_string())?;
    let mut upk = UpkPackage::from_bytes(before.clone())?;
    let current = MOVIE.movie(&upk)?;
    let current_hash = digest(&current);
    if has_hook(&current) || EARLIER.contains(&current_hash.as_str()) {
        let vanilla = std::fs::read(saved_path(backups)).map_err(|e| e.to_string())?;
        if digest(&vanilla) != MOVIE.vanilla {
            return Err("The saved Chat Paste baseline failed verification.".into());
        }
        if current_hash == digest(MOVIE.patched) || EARLIER.contains(&current_hash.as_str()) {
            install(cooked, &before, &mut upk, &vanilla)?;
        } else {
            let index = usize::try_from(upk.find_object(MOVIE.export).ok_or("HUD movie is missing")? - 1)
                .map_err(|_| "Invalid HUD movie export")?;
            if !title_patcher::rebase_hud_movie(cooked, &live, &before, upk, index, rules, &|raw| {
                if digest(raw) != digest(MOVIE.patched) {
                    return Err("The Chat Paste hook differs from its recorded HUD baseline.".into());
                }
                Ok(Some((vanilla.clone(), 1)))
            })? {
                return Err("Could not identify the Chat Paste edit in the shared HUD.".into());
            }
        }
    }
    let _ = std::fs::remove_file(saved_path(backups));
    Ok(())
}

/// Put the patch back after another feature rewrote GFX_Hud_SF.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) || MOVIE.status(cooked) {
        return Ok(());
    }
    ensure_closed()?;
    patch_with_rules(cooked, backups, &[])
}

pub(super) fn retain_recovery_backup(cooked: &Path, archive: &Path) -> Result<(), String> {
    let saved = saved_path(&cooked.join("Backups"));
    let source = saved_path(archive);
    if saved.is_file() || !source.is_file() {
        return Ok(());
    }
    let movie = MOVIE.movie(&UpkPackage::load(&super::game_path::resolve(cooked, PACKAGE))?)?;
    if !has_hook(&movie) && !EARLIER.contains(&digest(&movie).as_str()) {
        return Ok(());
    }
    let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
    if digest(&bytes) != MOVIE.vanilla {
        return Err("The archived Chat Paste baseline failed verification.".into());
    }
    std::fs::create_dir_all(saved.parent().unwrap()).map_err(|error| error.to_string())?;
    super::upk_history::replace(&saved, &bytes)
}

pub(super) fn recover_after_update(cooked: &Path, archive: &Path) -> Result<(), String> {
    retain_recovery_backup(cooked, archive)?;
    let backups = cooked.join("Backups");
    if enabled(archive) && !enabled(&backups) {
        enable(cooked, &backups, &[])?;
    }
    Ok(())
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
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, rules: Vec<TitleOverride>, mut report: impl FnMut(&str)) {
        use crate::i18n::t;
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    if let Err(error) = &result {
                        report(error);
                    }
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
                    enable(&cooked, &backups, &rules).map(|_| t("game-mod-chat-paste-on"))
                } else {
                    restore_with_rules(&cooked, &backups, &rules).map(|_| t("game-mod-chat-paste-off"))
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

    #[test]
    #[ignore]
    fn titles_and_chat_paste_restore_independently() {
        let source = std::env::var_os("HEBNIX_HUD_UPK").unwrap();
        let root = std::env::temp_dir().join(format!("hebnix-shared-hud-{}", std::process::id()));
        let cooked = root.join("TAGame/CookedPCConsole");
        let binaries = root.join("Binaries/Win64");
        let backups = cooked.join("Backups");
        let base = crate::config::base_dir();
        std::fs::create_dir_all(&cooked).unwrap();
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::write(binaries.join("RocketLeague.exe"), b"fixture").unwrap();
        let live = cooked.join(PACKAGE);
        std::fs::copy(source, &live).unwrap();
        let read = || MOVIE.movie(&UpkPackage::load(&live).unwrap()).unwrap();
        let rules = [TitleOverride {
            target_id: None,
            original: None,
            text: "Shared HUD".into(),
            color: 0x123456,
            glow: false,
            glow_color: None,
        }];
        title_patcher::apply(&base, &cooked, &rules).unwrap();
        let titles_only = digest(&read());
        patch_with_rules(&cooked, &backups, &rules).unwrap();
        assert!(has_hook(&read()));
        unpatch(&cooked, &backups).unwrap();
        assert_eq!(digest(&read()), titles_only);
        patch_with_rules(&cooked, &backups, &rules).unwrap();
        title_patcher::restore(&base, &cooked).unwrap();
        assert_eq!(digest(&read()), digest(MOVIE.patched));
        unpatch(&cooked, &backups).unwrap();
        assert_eq!(digest(&read()), MOVIE.vanilla);
    }

    /// Switch chat paste on, rebuild the package as the boost patcher does,
    /// reapply, then restore, on a copy of the real package. Run with
    /// HEBNIX_HUD_UPK set to a vanilla GFX_Hud_SF.upk.
    #[test]
    #[ignore]
    fn enable_reapply_restore_on_a_copy() {
        let source = std::env::var("HEBNIX_HUD_UPK").unwrap();
        let dir = std::env::temp_dir().join(format!("hebnix-chat-paste-{}", std::process::id()));
        let dir = dir.join("TAGame/CookedPCConsole");
        let backups = dir.join("Backups");
        let binaries = dir.join("../../Binaries/Win64");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::write(binaries.join("RocketLeague.exe"), b"fixture").unwrap();
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
        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }
}
