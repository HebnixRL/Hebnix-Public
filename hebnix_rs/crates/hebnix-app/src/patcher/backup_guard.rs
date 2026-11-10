use sha2::{Digest, Sha256};
use std::path::Path;
use std::{fs, io::Read};

const INSTALL_MARKER: &str = "hebnix-install-build.sha256";
const STALE_RESTORE_MESSAGE: &str = "Rocket League files changed. Resolve the game update prompt in Hebnix before patching or restoring game files.";

pub(crate) fn build_id(cooked_pc: &Path) -> Result<String, String> {
    let root = cooked_pc
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "Invalid Rocket League CookedPCConsole path".to_string())?;
    let exe = [
        root.join("TAGame")
            .join("Binaries")
            .join("Win64")
            .join("RocketLeague.exe"),
        root.join("Binaries").join("Win64").join("RocketLeague.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| format!("Could not find RocketLeague.exe below {}", root.display()))?;
    // every patch asks for this several times and hashing the exe is most of their wait,
    // so it is remembered until the exe's size or modified time changes
    static CACHE: std::sync::Mutex<
        Option<(
            std::path::PathBuf,
            u64,
            Option<std::time::SystemTime>,
            String,
        )>,
    > = std::sync::Mutex::new(None);
    let meta = fs::metadata(&exe).ok();
    let stamp = (
        meta.as_ref().map(|m| m.len()).unwrap_or(0),
        meta.and_then(|m| m.modified().ok()),
    );
    if let Some((path, len, modified, id)) = CACHE.lock().ok().and_then(|c| c.clone()) {
        if path == exe && (len, modified) == stamp && modified.is_some() {
            return Ok(id);
        }
    }
    let bytes = fs::read(&exe).map_err(|e| {
        format!(
            "Could not read {} to check backup compatibility: {e}",
            exe.display()
        )
    })?;
    let id = hex::encode(Sha256::digest(bytes));
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((exe, stamp.0, stamp.1, id.clone()));
    }
    Ok(id)
}

fn backup_root<'a>(backups_dir: &'a Path) -> &'a Path {
    backups_dir
        .ancestors()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("Backups"))
        })
        .unwrap_or(backups_dir)
}

fn remove_entry(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| format!("Could not discard stale backup {}: {e}", path.display()))
}

/// Deletes the whole Backups folder. Patched files stay patched, but Hebnix can
/// no longer restore them, so the game needs Verify game files to get clean ones.
pub fn delete_backups(cooked_pc: &Path) -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        return Err("Close Rocket League before deleting backups.".into());
    }
    let root = cooked_pc.join("Backups");
    match fs::remove_dir_all(&root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Could not delete {}: {e}", root.display())),
    }
}

pub(super) fn recorded_build(backups: &Path) -> Result<Option<String>, String> {
    let marker = backups.join(INSTALL_MARKER);
    match fs::read_to_string(&marker) {
        Ok(build) => {
            let build = build.trim();
            if build.len() != 64 || !build.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(None);
            }
            Ok(Some(build.to_owned()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read {}: {e}", marker.display())),
    }
}

pub(super) fn initialize_install(cooked: &Path, backups: &Path, build: &str) -> Result<(), String> {
    if build_id(cooked)? != build {
        return Err(STALE_RESTORE_MESSAGE.into());
    }
    fs::create_dir_all(backups)
        .map_err(|e| format!("Could not create {}: {e}", backups.display()))?;
    super::upk_history::replace(&backups.join(INSTALL_MARKER), build.as_bytes())
}

fn has_backups(root: &Path) -> Result<bool, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("Could not inspect {}: {e}", root.display())),
    };
    for entry in entries {
        if entry.map_err(|e| e.to_string())?.file_name() != INSTALL_MARKER {
            return Ok(true);
        }
    }
    Ok(false)
}

fn upk_version(path: &Path) -> Result<Option<[u8; 4]>, String> {
    let mut header = [0; 8];
    let result = fs::File::open(path).and_then(|mut file| file.read_exact(&mut header));
    match result {
        Ok(()) if header[..4] == [0xc1, 0x83, 0x2a, 0x9e] => {
            Ok(Some(header[4..].try_into().unwrap()))
        }
        Ok(()) => Ok(None),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

pub(super) fn different_version(backup: &Path, live: &Path) -> Result<bool, String> {
    let (Some(saved), Some(installed)) = (upk_version(backup)?, upk_version(live)?) else {
        return Ok(false);
    };
    if saved != installed {
        return Ok(true);
    }
    // TAGame patches modify exports in place and retain the package GUID.
    // Cosmetic replacements can legitimately substitute the donor's GUID.
    if live
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("TAGame.upk"))
    {
        fn guid(path: &Path) -> Result<Option<[u8; 16]>, String> {
            let mut header = Vec::new();
            fs::File::open(path)
                .and_then(|file| file.take(4096).read_to_end(&mut header))
                .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
            Ok(super::colours::package_guid(&header).ok())
        }
        return Ok(
            matches!((guid(backup)?, guid(live)?), (Some(saved), Some(installed)) if saved != installed),
        );
    }
    Ok(false)
}

fn changed_backup_versions(
    cooked: &Path,
    dir: &Path,
    tracked: &std::collections::HashSet<String>,
) -> Result<usize, String> {
    let mut changed = 0;
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() && name != super::upk_history::DIR {
            changed += changed_backup_versions(cooked, &entry.path(), tracked)?;
        } else if kind.is_file() {
            let lower = name.to_ascii_lowercase();
            let package = if lower.ends_with(".upk.bak") {
                &name[..name.len() - ".bak".len()]
            } else if lower.ends_with(".upk") {
                &name
            } else {
                continue;
            };
            if !tracked.contains(&package.to_ascii_lowercase())
                && different_version(&entry.path(), &super::game_path::resolve(cooked, package))?
            {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// Check package history even when the executable is unchanged, such as after
/// a launcher verifies files or ships a content-only update.
pub(crate) fn update_required(cooked: &Path) -> Result<bool, String> {
    let root = cooked.join("Backups");
    if !has_backups(&root)? {
        return Ok(false);
    }
    let Some(recorded) = recorded_build(&root)? else {
        return Ok(true);
    };
    let current = build_id(cooked)?;
    let (tracked, replaced) = super::upk_history::replaced_packages(cooked, &root, &recorded)?;
    let different_versions = changed_backup_versions(cooked, &root, &tracked)?;
    Ok(recorded != current || replaced > 0 || different_versions > 0)
}

pub fn check_install(cooked: &Path, backups_dir: &Path) -> Result<(), String> {
    let root = backup_root(backups_dir);
    let current = build_id(cooked)?;
    let recorded = recorded_build(root)?;
    if recorded.as_deref() != Some(&current) {
        if has_backups(root)? {
            return Err(STALE_RESTORE_MESSAGE.into());
        }
        initialize_install(cooked, root, &current)?;
    }
    Ok(())
}

/// Refuse legacy and cross-build backups before they can replace current game files.
pub fn check(
    cooked_pc: &Path,
    backups_dir: &Path,
    marker_name: &str,
    has_backups: impl Fn(&str) -> bool,
) -> Result<(), String> {
    check_install(cooked_pc, backups_dir)?;
    fs::create_dir_all(backups_dir)
        .map_err(|e| format!("Could not create {}: {e}", backups_dir.display()))?;
    let current = build_id(cooked_pc)?;
    let marker = backups_dir.join(marker_name);
    let existing = fs::read_dir(backups_dir)
        .map_err(|e| format!("Could not inspect {}: {e}", backups_dir.display()))?
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_str().is_some_and(&has_backups));
    if !existing {
        fs::write(&marker, &current)
            .map_err(|e| format!("Could not write {}: {e}", marker.display()))?;
        return Ok(());
    }
    let recorded = fs::read_to_string(&marker).unwrap_or_default();
    if recorded.trim() != current {
        return Err(STALE_RESTORE_MESSAGE.into());
    }
    Ok(())
}

/// Recreate outdated feature backups from installed game packages before applying a patch.
/// Old backups and their active-state entries are discarded rather than retained or restored.
pub fn prepare(
    cooked_pc: &Path,
    backups_dir: &Path,
    marker_name: &str,
    has_backups: impl Fn(&str) -> bool,
) -> Result<(), String> {
    check_install(cooked_pc, backups_dir)?;
    fs::create_dir_all(backups_dir)
        .map_err(|e| format!("Could not create {}: {e}", backups_dir.display()))?;
    let current = build_id(cooked_pc)?;
    let marker = backups_dir.join(marker_name);
    let old_files: Vec<_> = fs::read_dir(backups_dir)
        .map_err(|e| format!("Could not inspect {}: {e}", backups_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_str().is_some_and(&has_backups))
        .map(|entry| entry.path())
        .collect();
    if old_files.is_empty() {
        fs::write(&marker, current)
            .map_err(|e| format!("Could not write {}: {e}", marker.display()))?;
        return Ok(());
    }
    if fs::read_to_string(&marker).is_ok_and(|saved| saved.trim() == current) {
        return Ok(());
    }

    // A feature marker can be missing on older installs. The install marker
    // has already been validated, so only invalidate that feature's snapshots.
    for path in old_files {
        remove_entry(&path)?;
    }
    fs::write(&marker, current)
        .map_err(|e| format!("Could not write {}: {e}", marker.display()))?;
    Ok(())
}
