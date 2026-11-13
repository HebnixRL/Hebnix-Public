use super::{backup_guard, upk_history};
use crate::config::PatcherCfg;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const JOURNAL: &str = "hebnix-upk-update.json";

#[derive(Serialize, Deserialize)]
pub(crate) struct Recovery {
    archive_id: u64,
    pub(crate) items: PatcherCfg,
    pub(crate) swaps: Vec<serde_json::Value>,
    #[serde(default)]
    pub(crate) ball_visuals: Vec<(usize, usize, bool)>,
    #[serde(default)]
    pub(crate) alignments: Vec<(String, super::wheel_alignment::Settings)>,
    #[serde(default)]
    pub(crate) item_names: Vec<(String, Vec<super::item_name::Patch>)>,
    #[serde(default)]
    pub(crate) thumbnails: super::thumbnail_patcher::Manifest,
    pub(crate) build: String,
    pub(crate) prepared: bool,
    #[serde(default)]
    pub(crate) clearing: bool,
    #[serde(default)]
    instant_suite: bool,
    #[serde(default)]
    pub(crate) reset_pending: bool,
}

impl Recovery {
    pub(super) fn archive(&self, cooked: &Path) -> std::path::PathBuf {
        cooked.with_file_name(format!(".hebnix-stale-backups-{:016x}", self.archive_id))
    }

    fn remove_archive(&self, cooked: &Path) -> Result<(), String> {
        match fs::remove_dir_all(self.archive(cooked)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(crate) fn load(cooked: &Path) -> Result<Option<Self>, String> {
        match fs::read(cooked.join(JOURNAL)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub(crate) fn save(&self, cooked: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        upk_history::replace(&cooked.join(JOURNAL), &bytes)
    }

    pub(crate) fn capture(cooked: &Path, items: PatcherCfg) -> Result<Self, String> {
        let swaps = match fs::read(cooked.join("Backups/swapper_swaps.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        let backups = cooked.join("Backups");
        let mut item_names = Vec::new();
        for file in super::item_name::languages(cooked) {
            let patches = super::item_name::load(&backups, &file)?.patches;
            if !patches.is_empty() {
                item_names.push((file, patches));
            }
        }
        Ok(Self {
            archive_id: rand::random(),
            items,
            swaps,
            ball_visuals: super::ball_visual::active(&backups)?
                .into_iter()
                .map(|swap| (swap.source, swap.donor, swap.colours))
                .collect(),
            alignments: super::wheel_alignment::recovery_settings(cooked)?,
            item_names,
            thumbnails: super::thumbnail_patcher::load(&backups)?,
            build: backup_guard::build_id(cooked)?,
            prepared: false,
            clearing: false,
            instant_suite: super::instant_suite::recorded_enabled(&backups)
                || super::instant_suite::status(cooked, &backups),
            reset_pending: false,
        })
    }

    pub(crate) fn rebase_if_needed(&mut self, cooked: &Path, repatch: bool) -> Result<(), String> {
        let build = backup_guard::build_id(cooked)?;
        if build == self.build && (!self.prepared || !backup_guard::update_required(cooked)?) {
            return Ok(());
        }
        let backups = cooked.join("Backups");
        let archive = self.archive(cooked);
        ensure_no_old_patch(cooked, &backups, repatch)?;
        if archive.is_dir() {
            ensure_no_old_patch(cooked, &archive, repatch)?;
            // Preserve feature receipts across a second update. The current
            // tree may include partial item work from the interrupted run.
            if backups.is_dir() {
                let interrupted = archive.with_extension(format!("interrupted-{:016x}", rand::random::<u64>()));
                fs::rename(&backups, interrupted).map_err(|error| error.to_string())?;
            }
        }
        self.build = build;
        self.prepared = false;
        self.save(cooked)
    }

    pub(crate) fn prepare(&mut self, cooked: &Path, repatch: bool) -> Result<(), String> {
        if backup_guard::build_id(cooked)? != self.build {
            return Err("Rocket League changed again during recovery. Verify the game files and restart Hebnix.".into());
        }
        let backups = cooked.join("Backups");
        let archive = self.archive(cooked);
        if self.prepared {
            if !repatch && archive.is_dir() {
                ensure_no_old_patch(cooked, &archive, false)?;
            }
            return Ok(());
        }
        if !archive.exists() {
            ensure_no_old_patch(cooked, &backups, repatch)?;
            if backups.exists() {
                fs::rename(&backups, &archive).map_err(|e| e.to_string())?;
            }
        }
        backup_guard::initialize_install(cooked, &backups, &self.build)?;
        let retained = retained_ui(cooked, &archive, true)?;
        if retained.contains(&super::title_patcher::BOOST_HUD.to_ascii_lowercase()) {
            for name in ["GFX_Hud_SF.upk.bak", "boost-build.sha256", "ChatPaste/vanilla.swf"] {
                let source = archive.join(name);
                if source.is_file() {
                    let destination = backups.join(name);
                    if let Some(parent) = destination.parent() {
                        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    }
                    let bytes = fs::read(&source).map_err(|error| error.to_string())?;
                    upk_history::replace(&destination, &bytes)?;
                }
            }
        }
        if !retained.is_empty() {
            upk_history::carry_ui_history(&archive, &backups, &self.build, &retained)?;
        }
        self.prepared = true;
        self.reset_pending = true;
        if let Err(error) = self.save(cooked) {
            self.prepared = false;
            self.reset_pending = false;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn finish(cooked: &Path) -> Result<(), String> {
        if hebnix_sdk::process::is_rocket_league_running() {
            return Err("Close Rocket League before finishing recovery.".into());
        }
        if let Some(recovery) = Self::load(cooked)? {
            if recovery.build != backup_guard::build_id(cooked)? {
                return Err("Rocket League changed again before recovery finished.".into());
            }
            let archive = recovery.archive(cooked);
            if archive.is_dir() {
                let mut errors = Vec::new();
                for (feature, apply) in [
                    ("Profile pictures", super::profile_pictures::recover_after_update as fn(&Path, &Path) -> Result<(), String>),
                    ("Player name bridge", super::profile_pictures::name_bridge::retain_recovery_state),
                    ("Workshop", super::workshop_menu::recover_after_update),
                    ("Freeplay", super::workshop_host::recover_freeplay),
                    ("UPK Studio colours", super::colours::recover_after_update),
                    ("LAN chat commands", super::lan_commands::recover_after_update),
                    ("Chat Paste", super::chat_paste::recover_after_update),
                    ("Game Mods", recover_game_mods),
                ] {
                    if hebnix_sdk::process::is_rocket_league_running() {
                        return Err("Close Rocket League before finishing recovery.".into());
                    }
                    if let Err(error) = apply(cooked, &archive) {
                        errors.push(format!("{feature}: {error}"));
                    }
                }
                if let Err(error) = super::instant_suite::recover_after_update(cooked, &archive, recovery.instant_suite) {
                    errors.push(format!("Instant Suite: {error}"));
                }
                if !errors.is_empty() {
                    return Err(errors.join("\n"));
                }
            }
            recovery.remove_archive(cooked)?;
        }
        fs::remove_file(cooked.join(JOURNAL)).map_err(|e| e.to_string())
    }
}

fn recover_game_mods(cooked: &Path, archive: &Path) -> Result<(), String> {
    let backups = cooked.join("Backups");
    if archive.join("Dispatcher/state.json").is_file() {
        let package = super::upk_package::UpkPackage::load(&super::game_path::resolve(cooked, "TAGame.upk"))?;
        super::dispatcher::retain_recovery_state(cooked, archive, &package)?;
    }
    for (name, enabled, apply) in [
        ("Forfeit command", super::forfeit_command::enabled as fn(&Path) -> bool,
            super::forfeit_command::enable as fn(&Path, &Path) -> Result<(), String>),
        ("Celebration", super::celebration::enabled, super::celebration::enable),
        ("Bots in Freeplay", super::bots_freeplay::enabled, super::bots_freeplay::enable),
    ] {
        if enabled(archive) && !enabled(&backups) {
            apply(cooked, &backups).map_err(|error| format!("{name}: {error}"))?;
        }
    }
    super::chat_export::recover_after_update(cooked, archive)?;
    if let Some(scale) = super::goal_explosion::current(archive)
        && super::goal_explosion::current(&backups).is_none()
    {
        super::goal_explosion::recover_after_update(cooked, archive, scale)?;
    }
    if let Some(gain) = super::goal_explosion_volume::current(archive)
        && super::goal_explosion_volume::current(&backups).is_none()
    {
        super::goal_explosion_volume::recover_after_update(cooked, archive, gain)?;
    }
    Ok(())
}

pub(crate) fn write_item_state(root: &Path, bytes: &[u8]) -> Result<(), String> {
    upk_history::replace(&root.join("patcher.json"), bytes)
}

pub(crate) fn apply_alignment(
    cooked: &Path,
    file: &str,
    settings: super::wheel_alignment::Settings,
    force: bool,
) -> Result<(), String> {
    if !force && super::wheel_alignment::recovery_settings(cooked)?
        .iter()
        .any(|(name, saved)| name == file && *saved == settings)
    {
        return Ok(());
    }
    super::wheel_alignment::transaction(
        cooked,
        file,
        settings,
        super::wheel_alignment::TransactionAction::Apply,
    )
    .map(|_| ())
}

pub(crate) fn apply_names(
    cooked: &Path,
    file: &str,
    patches: &[super::item_name::Patch],
) -> Result<(), String> {
    let backups = cooked.join("Backups");
    for patch in patches {
        if !matches!(patch.field.as_str(), "Label" | "Artist") {
            continue;
        }
        super::item_name::rename(
            cooked,
            &backups,
            file,
            &patch.section,
            Some(&[(&patch.field, &patch.name)]),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Install(std::path::PathBuf);

    impl Install {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("hebnix-update-{}", rand::random::<u64>()));
            fs::create_dir_all(root.join("TAGame/CookedPCConsole")).unwrap();
            fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
            fs::write(
                root.join("Binaries/Win64/RocketLeague.exe"),
                b"initial build",
            )
            .unwrap();
            Self(root)
        }

        fn cooked(&self) -> std::path::PathBuf {
            self.0.join("TAGame/CookedPCConsole")
        }
    }

    impl Drop for Install {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_external_replacements_without_mistaking_normal_patches_for_updates() {
        let install = Install::new();
        let cooked = install.cooked();
        let backups = cooked.join("Backups");
        assert!(!backup_guard::update_required(&cooked).unwrap());
        for name in ["A.upk", "B.upk"] {
            let live = cooked.join(name);
            fs::write(&live, b"vanilla").unwrap();
            upk_history::install_linked(&cooked, &live, b"patched", "Fixture").unwrap();
            fs::write(backups.join(format!("{name}.bak")), b"vanilla").unwrap();
        }
        assert!(!backup_guard::update_required(&cooked).unwrap());

        fs::write(cooked.join("B.upk"), b"new game bytes").unwrap();
        assert!(backup_guard::update_required(&cooked).unwrap());
        assert!(
            upk_history::install(
                &cooked,
                &cooked.join("B.upk"),
                b"stale patch",
                "Fixture",
                None
            )
            .is_err()
        );
        let mut recovery = Recovery::capture(&cooked, PatcherCfg::default()).unwrap();
        recovery.save(&cooked).unwrap();
        assert!(recovery.prepare(&cooked, false).unwrap_err().contains("A.upk"));
        assert!(backups.join("A.upk.bak").is_file());

        fs::write(cooked.join("A.upk"), b"new game bytes").unwrap();
        recovery.prepare(&cooked, true).unwrap();
        assert!(Recovery::load(&cooked).unwrap().unwrap().prepared);
        assert!(!backups.join("A.upk.bak").exists());
        assert_eq!(fs::read(cooked.join("B.upk")).unwrap(), b"new game bytes");
        assert_eq!(
            fs::read(recovery.archive(&cooked).join("B.upk.bak")).unwrap(),
            b"vanilla"
        );
        Recovery::finish(&cooked).unwrap();
        assert!(!backup_guard::update_required(&cooked).unwrap());
    }

    #[test]
    fn changed_build_blocks_patch_and_restore_without_discarding_active_manifest() {
        let install = Install::new();
        let cooked = install.cooked();
        let backups = cooked.join("Backups");
        backup_guard::check_install(&cooked, &backups).unwrap();
        fs::write(backups.join("swapper_swaps.json"), b"[]").unwrap();
        fs::write(
            install.0.join("Binaries/Win64/RocketLeague.exe"),
            b"updated build v2",
        )
        .unwrap();
        assert!(backup_guard::update_required(&cooked).unwrap());
        assert!(backup_guard::check_install(&cooked, &backups).is_err());
        assert!(backup_guard::prepare(&cooked, &backups, "fixture.sha256", |_| true).is_err());
        assert_eq!(fs::read(backups.join("swapper_swaps.json")).unwrap(), b"[]");
    }

    #[test]
    fn recovery_keeps_updated_stock_and_resets_a_surviving_item_patch() {
        let install = Install::new();
        let cooked = install.cooked();
        let backups = cooked.join("Backups");
        for file in ["Updated.upk", "Survivor.upk"] {
            let live = cooked.join(file);
            fs::write(&live, b"old stock").unwrap();
            upk_history::install_linked(&cooked, &live, b"old patch", "Custom decal").unwrap();
            fs::write(backups.join(format!("{file}.bak")), b"old stock").unwrap();
        }
        fs::write(cooked.join("Updated.upk"), b"new stock").unwrap();
        fs::write(install.0.join("Binaries/Win64/RocketLeague.exe"), b"new Rocket League build").unwrap();

        let mut recovery = Recovery::capture(&cooked, PatcherCfg::default()).unwrap();
        recovery.prepare(&cooked, true).unwrap();
        assert!(Recovery::load(&cooked).unwrap().unwrap().reset_pending);
        assert_eq!(fs::read(cooked.join("Updated.upk")).unwrap(), b"new stock");
        assert_eq!(fs::read(cooked.join("Survivor.upk")).unwrap(), b"old stock");
        assert!(!backups.join("Updated.upk.bak").exists());
        assert_eq!(fs::read(recovery.archive(&cooked).join("Updated.upk.bak")).unwrap(), b"old stock");
    }

    #[test]
    fn compares_versions_for_named_legacy_upk_backups() {
        let install = Install::new();
        let cooked = install.cooked();
        let backups = cooked.join("Backups");
        backup_guard::check_install(&cooked, &backups).unwrap();
        fs::create_dir(backups.join("Nested")).unwrap();
        let old = [0xc1, 0x83, 0x2a, 0x9e, 0x64, 3, 32, 0];
        let new = [0xc1, 0x83, 0x2a, 0x9e, 0x64, 3, 33, 0];
        fs::write(backups.join("Nested/A.UPK.BAK"), old).unwrap();
        fs::write(cooked.join("A.UPK"), new).unwrap();
        assert!(backup_guard::update_required(&cooked).unwrap());
    }
}

fn ensure_no_old_patch(cooked: &Path, backups: &Path, repatch: bool) -> Result<(), String> {
    if repatch {
        if let Some(recorded) = backup_guard::recorded_build(backups)? {
            let retained = retained_ui(cooked, backups, true)?;
            upk_history::restore_repatch_baselines(cooked, backups, &recorded, &retained)?;
            if let Some(file) = upk_history::remaining_repatch_item(cooked, backups, &recorded)? {
                return Err(format!("Could not identify a clean baseline for {file}. Verify Rocket League's game files and retry repatch."));
            }
        } else if let Some(file) = upk_history::remaining_patch(cooked, backups, &Default::default())? {
            return Err(format!("The build for the patch in {file} is unknown. Verify Rocket League's game files and retry repatch."));
        }
        return Ok(());
    }
    if let Some(recorded) = backup_guard::recorded_build(backups)? {
        let retained = retained_ui(cooked, backups, true)?;
        upk_history::restore_repatch_baselines(cooked, backups, &recorded, &retained)?;
    }
    let retained = retained_ui(cooked, backups, true)?;
    if let Some(file) = upk_history::remaining_patch(cooked, backups, &retained)? {
        return Err(format!(
            "Could not undo the recorded patch in {file}. Its matching baseline is missing or the patch history is incomplete."
        ));
    }
    Ok(())
}

// Item recovery leaves UI features installed when their package and build
// still match the recorded patch.
fn retained_ui(
    cooked: &Path,
    backups: &Path,
    repatch: bool,
) -> Result<std::collections::HashSet<String>, String> {
    if repatch
        && let Some(recorded) = backup_guard::recorded_build(backups)?
        && recorded == backup_guard::build_id(cooked)?
    {
        return upk_history::unchanged_ui_packages(cooked, backups, &recorded);
    }
    Ok(Default::default())
}
