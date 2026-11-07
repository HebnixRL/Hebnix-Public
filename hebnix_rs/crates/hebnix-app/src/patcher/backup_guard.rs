use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const INSTALL_MARKER: &str = "hebnix-install-build.sha256";
const STALE_RESTORE_MESSAGE: &str = "Rocket League was updated. Hebnix discarded all old patch backups and active entries instead of restoring files from the previous build. Apply the item again to create a fresh backup.";

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
    static CACHE: std::sync::Mutex<Option<(std::path::PathBuf, u64, Option<std::time::SystemTime>, String)>> =
        std::sync::Mutex::new(None);
    let meta = fs::metadata(&exe).ok();
    let stamp = (meta.as_ref().map(|m| m.len()).unwrap_or(0), meta.and_then(|m| m.modified().ok()));
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

fn discard_root(backups_dir: &Path) -> Result<(), String> {
    let root = backup_root(backups_dir);
    let install_marker = root.join(INSTALL_MARKER);
    for entry in
        fs::read_dir(root).map_err(|e| format!("Could not inspect {}: {e}", root.display()))?
    {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path != install_marker {
            remove_entry(&path)?;
        }
    }
    Ok(())
}

/// Synchronize every Hebnix backup belonging to this Rocket League install.
///
/// A missing marker is deliberately treated as untrusted when backup entries already exist.
/// This makes upgrades from older Hebnix releases fail safe: an unknown backup can never be
/// copied over files from a newer Rocket League build.
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

pub fn synchronize_install(cooked_pc: &Path, backups_dir: &Path) -> Result<bool, String> {
    let root = backup_root(backups_dir);
    fs::create_dir_all(root).map_err(|e| format!("Could not create {}: {e}", root.display()))?;
    let current = build_id(cooked_pc)?;
    let marker = root.join(INSTALL_MARKER);
    let recorded = fs::read_to_string(&marker).unwrap_or_default();
    let entries: Vec<_> = fs::read_dir(root)
        .map_err(|e| format!("Could not inspect {}: {e}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != &marker)
        .collect();
    let stale = !entries.is_empty() && recorded.trim() != current;
    if stale {
        discard_root(backups_dir)?;
    }
    if recorded.trim() != current {
        fs::write(&marker, &current)
            .map_err(|e| format!("Could not write {}: {e}", marker.display()))?;
    }
    Ok(stale)
}

/// Guard a restore that only needs install-wide protection and has no exclusive backup names.
pub fn check_install(cooked_pc: &Path, backups_dir: &Path) -> Result<(), String> {
    if synchronize_install(cooked_pc, backups_dir)? {
        Err(STALE_RESTORE_MESSAGE.into())
    } else {
        Ok(())
    }
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
        discard_root(backups_dir)?;
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
) -> Result<bool, String> {
    let discarded = synchronize_install(cooked_pc, backups_dir)?;
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
        return Ok(discarded);
    }
    if fs::read_to_string(&marker).is_ok_and(|saved| saved.trim() == current) {
        return Ok(discarded);
    }

    discard_root(backups_dir)?;
    fs::create_dir_all(backups_dir)
        .map_err(|e| format!("Could not recreate {}: {e}", backups_dir.display()))?;
    fs::write(backups_dir.join(marker_name), current)
        .map_err(|e| format!("Could not write {}: {e}", marker.display()))?;
    Ok(true)
}
