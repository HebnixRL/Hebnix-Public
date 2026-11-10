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
}

impl Recovery {
    fn archive(&self, cooked: &Path) -> std::path::PathBuf {
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
        })
    }

    pub(crate) fn rebase_if_needed(&mut self, cooked: &Path, repatch: bool) -> Result<(), String> {
        let build = backup_guard::build_id(cooked)?;
        if build == self.build && (!self.prepared || !backup_guard::update_required(cooked)?) {
            return Ok(());
        }
        for backups in [cooked.join("Backups"), self.archive(cooked)] {
            ensure_no_old_patch(cooked, &backups, repatch)?;
        }
        self.remove_archive(cooked)?;
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
        if !repatch && archive.exists() {
            ensure_no_old_patch(cooked, &archive, false)?;
        }
        if self.prepared {
            return Ok(());
        }
        if !archive.exists() {
            ensure_no_old_patch(cooked, &backups, repatch)?;
            if backups.exists() {
                fs::rename(&backups, &archive).map_err(|e| e.to_string())?;
            }
        }
        if repatch && keep_hud(cooked, &archive)? {
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
            upk_history::carry_hud_history(&archive, &backups, &self.build)?;
        }
        backup_guard::initialize_install(cooked, &backups, &self.build)?;
        self.prepared = true;
        if let Err(error) = self.save(cooked) {
            self.prepared = false;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn finish(cooked: &Path) -> Result<(), String> {
        if let Some(recovery) = Self::load(cooked)? {
            recovery.remove_archive(cooked)?;
        }
        fs::remove_file(cooked.join(JOURNAL)).map_err(|e| e.to_string())
    }
}

pub(crate) fn write_item_state(root: &Path, bytes: &[u8]) -> Result<(), String> {
    upk_history::replace(&root.join("patcher.json"), bytes)
}

pub(crate) fn apply_alignment(
    cooked: &Path,
    file: &str,
    settings: super::wheel_alignment::Settings,
) -> Result<(), String> {
    if super::wheel_alignment::recovery_settings(cooked)?
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
            upk_history::install(&cooked, &live, b"patched", "Fixture", None).unwrap();
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
        assert!(recovery.prepare(&cooked, true).unwrap_err().contains("A.upk"));
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
    if let Some(file) = upk_history::remaining_patch(cooked, backups, repatch && keep_hud(cooked, backups)?)? {
        return Err(format!(
            "{file} still contains an older Hebnix patch. Verify Rocket League's game files, then try again."
        ));
    }
    Ok(())
}

// Repatch All rebuilds item patches. An unchanged HUD belongs to separate
// features, and its restore points are still valid on the same game build.
fn keep_hud(cooked: &Path, backups: &Path) -> Result<bool, String> {
    let Some(recorded) = backup_guard::recorded_build(backups)? else {
        return Ok(false);
    };
    if recorded != backup_guard::build_id(cooked)? {
        return Ok(false);
    }
    upk_history::unchanged_hud(cooked, backups, &recorded)
}
