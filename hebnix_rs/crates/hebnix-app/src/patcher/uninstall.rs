//! Restore feature records before dropping any Hebnix data. Recovery files stay
//! in the game directory, including when a restore fails part way through.
use std::fs;
use std::path::Path;

pub fn restore(base: &Path, game: &Path) -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        return Err("Close Rocket League before restoring game changes.".into());
    }
    let cooked = game.join("TAGame/CookedPCConsole");
    if !cooked.is_dir() {
        return Err("The configured Rocket League folder could not be found. Correct it in Settings before restoring game changes.".into());
    }
    let backups = cooked.join("Backups");
    // Check without synchronize_install: uninstall must never discard recovery
    // files just because a newer game build has been installed.
    if backups.exists() {
        let saved = fs::read_to_string(backups.join("hebnix-install-build.sha256"))
            .map_err(|_| "Game backups have no build marker. Verify Rocket League files before uninstalling.")?;
        if saved.trim() != super::backup_guard::build_id(&cooked)? {
            return Err("Rocket League changed since the backups were made. Verify game files before uninstalling; backups have been kept.".into());
        }
    }
    #[cfg(windows)]
    if base.join("live-history-install.json").exists() {
        crate::replay_history_patch::uninstall(game, base)?;
    }
    let background = base.join("state/background_changer");
    if background.join("background_swaps.json").exists() {
        super::background_merger::run("reset", &cooked, &background, None, None)?;
    }
    if super::menu_map::active(&cooked)?.is_some() {
        super::menu_map::restore(&cooked)?;
    }
    super::title_patcher::restore_for_uninstall(base, &cooked)?;
    if !backups.exists() {
        return Ok(());
    }
    let (tx, _rx) = crossbeam_channel::unbounded();
    super::swapper::SwapperState::new(base).restore_all_active(&cooked, &backups, &tx)?;
    super::wheel_alignment::restore_for_uninstall(&cooked)?;
    for swap in super::ball_visual::active(&backups)? {
        super::ball_visual::restore(&cooked, &backups, swap.source)?;
    }
    for language in super::item_name::languages(&cooked) {
        if !super::item_name::load(&backups, &language)?
            .patches
            .is_empty()
        {
            super::item_name::rename(&cooked, &backups, &language, "", None)?;
        }
    }
    super::colours::restore_for_uninstall(&cooked, &backups)?;
    // Nested feature snapshots may include older changes to the same package.
    // Restore them before the shared pristine package snapshots at the root.
    super::thumbnail_patcher::restore_all(&cooked, &backups)?;
    for name in ["CarPatcher", "Anthems"] {
        restore_packages(&backups.join(name), &cooked)?;
    }
    super::decal_patcher::restore_for_uninstall(&cooked, &backups)?;
    super::heatseeker::restore(&cooked, &backups)?;
    restore_ball_regions(&backups, &cooked)?;
    restore_packages(&backups, &cooked)?;
    Ok(())
}

fn restore_ball_regions(backups: &Path, cooked: &Path) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    for entry in fs::read_dir(backups).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !super::patch_core::standard_ball::is_ball_tfc_backup(&name) {
            continue;
        }
        let (file, offset) = name
            .strip_suffix(".bin")
            .and_then(|s| s.rsplit_once('_'))
            .ok_or("Invalid ball texture recovery filename")?;
        let offset = offset.parse::<u64>().map_err(|e| e.to_string())?;
        let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
        let mut live = fs::OpenOptions::new()
            .write(true)
            .open(cooked.join(file))
            .map_err(|e| e.to_string())?;
        if offset
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > live.metadata().map(|m| m.len()).unwrap_or(0))
        {
            return Err(format!(
                "Ball texture backup is outside {file}; recovery files kept."
            ));
        }
        live.seek(SeekFrom::Start(offset))
            .and_then(|_| live.write_all(&bytes))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn restore_packages(backups: &Path, cooked: &Path) -> Result<(), String> {
    if !backups.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(backups).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(live) = name.strip_suffix(".bak") else {
            continue;
        };
        if ![".upk", ".tfc", ".bnk"]
            .iter()
            .any(|ext| live.ends_with(ext))
        {
            continue;
        }
        let target = super::game_path::resolve(cooked, live);
        if !target.is_file() {
            return Err(format!(
                "Cannot restore missing game file {live}; verify game files first."
            ));
        }
        fs::copy(entry.path(), &target).map_err(|e| format!("Could not restore {live}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("hebnix-uninstall-{:016x}", rand::random::<u64>()));
            fs::create_dir_all(root.join("Backups")).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn restores_packages_and_preserves_recovery_and_unrelated_files() {
        let fixture = Fixture::new();
        let cooked = &fixture.0;
        let backups = cooked.join("Backups");
        fs::write(backups.join("Example.upk.bak"), b"original").unwrap();
        fs::write(cooked.join("Example.upk"), b"patched").unwrap();
        fs::write(cooked.join("unrelated.txt"), b"keep").unwrap();
        fs::write(backups.join("unrelated.txt.bak"), b"do not restore").unwrap();
        restore_packages(&backups, cooked).unwrap();
        assert_eq!(fs::read(cooked.join("Example.upk")).unwrap(), b"original");
        assert_eq!(fs::read(cooked.join("unrelated.txt")).unwrap(), b"keep");
        assert!(backups.join("Example.upk.bak").is_file());
    }
    #[test]
    fn missing_game_file_stops_restoration_without_removing_its_backup() {
        let fixture = Fixture::new();
        let backups = fixture.0.join("Backups");
        fs::write(backups.join("Missing.upk.bak"), b"original").unwrap();
        assert!(restore_packages(&backups, &fixture.0).is_err());
        assert!(backups.join("Missing.upk.bak").is_file());
    }
}
