//! Closed-game save cleanup. Preflight every file before installing any change.
use hebnix_sdk::save_file::bakkes_code;
use hebnix_sdk::save_file::item_cleanup::plan_cleanup;
use hebnix_sdk::save_file::{loadout_code, loadout_edit};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

fn require_closed() -> Result<(), String> {
    if hebnix_sdk::process::detector::rocket_league_running_checked()? {
        Err("Rocket League restarted; close it and Check Again to finish save cleanup".into())
    } else {
        Ok(())
    }
}
fn regular(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        #[cfg(target_os = "windows")]
use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            #[cfg(target_os = "windows")]
use std::os::windows::io::AsRawHandle;
            #[cfg(target_os = "windows")]
use windows::Win32::Foundation::HANDLE;
            #[cfg(target_os = "windows")]
use windows::Win32::Storage::FileSystem::{
                FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo, GetFileInformationByHandleEx,
            };
            // OneDrive Files On-Demand uses cloud reparse tags even for local
            // files. Allow those while continuing to reject links/junctions.
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
            unsafe {
                GetFileInformationByHandleEx(
                    HANDLE(file.as_raw_handle()),
                    FileAttributeTagInfo,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of_val(&info) as u32,
                )
            }
            .map_err(|e| e.to_string())?;
            if !is_cloud_reparse_tag(info.ReparseTag) {
                return Err("save path is a reparse point; refusing to follow it".into());
            }
        }
    }
    if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
        return Err("save is not a supported regular file".into());
    }
    Ok(())
}

fn is_cloud_reparse_tag(tag: u32) -> bool {
    // IO_REPARSE_TAG_CLOUD and CLOUD_1 through CLOUD_F use bits 12..15.
    tag & !0x0000_f000 == 0x9000_001a
}

#[cfg(test)]
mod cloud_tests {
    use super::*;

    #[test]
    fn onedrive_cloud_tags_are_allowed_but_links_are_not() {
        for variant in 0..16 {
            assert!(is_cloud_reparse_tag(0x9000_001a | (variant << 12)));
        }
        for tag in [0, 0xa000_0003, 0xa000_000c, 0x8000_001b] {
            assert!(!is_cloud_reparse_tag(tag));
        }
    }
}
fn account_filename(path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("save"))
    {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let mut parts = stem.split('_');
    let base = parts.next().unwrap_or("");
    let valid = (base.len() == 32 && base.bytes().all(|b| b.is_ascii_hexdigit()))
        || ((10..=20).contains(&base.len()) && base.bytes().all(|b| b.is_ascii_digit()));
    valid
        && parts
            .next()
            .is_none_or(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
        && parts.next().is_none()
}
fn discover() -> Result<Vec<PathBuf>, String> {
    let docs = hebnix_sdk::utils::paths::documents_dir()
        .ok_or("Cannot locate Rocket League's Documents directory")?;
    let mut paths = Vec::new();
    for relative in [
        hebnix_sdk::utils::constants::SAVE_PATH_STEAM,
        hebnix_sdk::utils::constants::SAVE_PATH_EPIC,
    ] {
        let dir = docs.join(relative);
        if !dir.exists() {
            continue;
        }
        let root = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if !account_filename(&path) {
                continue;
            }
            regular(&path)?;
            let resolved = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
            if resolved.parent() != Some(root.as_path()) {
                return Err("save escaped its expected directory".into());
            }
            paths.push(resolved);
        }
    }
    // Match the existing save selector: the most recently saved local account.
    // Its numbered rotations must be cleaned too, or RL can recover the old item.
    let latest = paths
        .iter()
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .cloned();
    if let Some(latest) = latest {
        let account = latest
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap()
            .split('_')
            .next()
            .unwrap()
            .to_owned();
        paths.retain(|p| {
            p.parent() == latest.parent()
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.split('_').next() == Some(account.as_str()))
        });
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
struct Prepared {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
    backup: PathBuf,
    temp: PathBuf,
}
impl Drop for Prepared {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temp);
    }
}

/// Plans each save with `planner` (`None` = unchanged), then backs up and installs the
/// changed ones. `tag` names the backups. Returns how many saves changed.
fn execute(
    paths: &[PathBuf],
    tag: &str,
    mut check: impl FnMut() -> Result<(), String>,
    mut planner: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>, String>,
) -> Result<usize, String> {
    check()?;
    let mut prepared = Vec::new();
    for path in paths {
        regular(path)?;
        let before = std::fs::read(path).map_err(|e| e.to_string())?;
        let Some(after) = planner(&before).map_err(|e| format!("{}: {e}", path.display()))? else {
            continue;
        };
        let token = format!("{tag}-{:032x}", rand::random::<u128>());
        let name = path
            .file_name()
            .ok_or("invalid save filename")?
            .to_string_lossy();
        let backup = path.with_file_name(format!("{name}.hebnix-{token}.bak"));
        let temp = path.with_file_name(format!("{name}.hebnix-{token}.tmp"));
        prepared.push(Prepared {
            path: path.clone(),
            before,
            after,
            backup,
            temp,
        });
        check()?;
    }
    // Immutable backups and flushed replacement files exist before the first install.
    for p in &prepared {
        check()?;
        write_new(&p.backup, &p.before)?;
        write_new(&p.temp, &p.after)?;
        if std::fs::read(&p.backup).map_err(|e| e.to_string())? != p.before
            || std::fs::read(&p.temp).map_err(|e| e.to_string())? != p.after
        {
            return Err("save backup/staging verification failed".into());
        }
    }
    for p in &prepared {
        check()?;
        regular(&p.path)?;
        // Deny writers while validating the snapshot. Windows requires this
        // handle to close before MoveFileEx can replace the destination.
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            #[cfg(target_os = "windows")]
use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1 | 4);
        }
        let _lock = options
            .open(&p.path)
            .map_err(|e| format!("Save is busy; retry after closing other save editors: {e}"))?;
        if std::fs::read(&p.path).map_err(|e| e.to_string())? != p.before {
            return Err("Save changed during cleanup; originals backed up, Check Again".into());
        }
        check()?;
        drop(_lock);
        super::store::replace(&p.temp, &p.path)?;
        if std::fs::read(&p.path).map_err(|e| e.to_string())? != p.after {
            return Err("Installed save verification failed; recovery backup retained".into());
        }
        check()?;
    }
    check()?;
    Ok(prepared.len())
}
pub fn clear_saved_instances(
    targets: &BTreeSet<String>,
    generation: u64,
) -> Result<String, String> {
    require_closed()?;
    if targets.is_empty() {
        return Ok("No tracked instances require save cleanup.".into());
    }
    let paths = discover()?;
    if paths.is_empty() {
        return Err(
            "No Rocket League account saves found; cannot verify saved-item cleanup".into(),
        );
    }
    let (mut instances, mut slots) = (0, 0);
    let changed = execute(&paths, &format!("clear-{generation}"), require_closed, |raw| {
        let plan = plan_cleanup(raw, targets)?;
        instances += plan.report.inventory_instances;
        slots += plan.report.equipped_slots;
        Ok(plan.report.changed().then_some(plan.bytes))
    })?;
    Ok(format!(
        "Cleaned {changed} files for the most recently saved account ({instances} inventory entries, {slots} equipped slots, including rotating copies). Original saves backed up beside each changed file."
    ))
}

/// The account save that loadout presets are read from and written to. Uses the same
/// account discovery as the owned-items view. Rotating copies (`<account>_N.save`)
/// are only used when the base save is missing; their preset positions can differ.
fn main_save() -> Result<PathBuf, String> {
    let accounts = hebnix_sdk::save_file::find_save_accounts(None);
    let account = accounts
        .first()
        .ok_or("No Rocket League account save found")?;
    let stem = account
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.split('_').next())
        .unwrap_or_default();
    let base = account.path.with_file_name(format!("{stem}.save"));
    Ok(if base.is_file() { base } else { account.path.clone() })
}

pub fn loadout_export(position: usize) -> Result<String, String> {
    let raw = std::fs::read(main_save()?).map_err(|e| e.to_string())?;
    loadout_edit::export(&raw, position)
}

pub fn loadout_preset(position: usize) -> Result<loadout_code::Preset, String> {
    let raw = std::fs::read(main_save()?).map_err(|e| e.to_string())?;
    loadout_edit::export_preset(&raw, position)
}

pub fn loadout_export_bakkes(position: usize) -> Result<String, String> {
    let raw = std::fs::read(main_save()?).map_err(|e| e.to_string())?;
    let preset = loadout_edit::export_preset(&raw, position)?;
    bakkes_code::encode(&bakkes_code::from_preset(&preset)?)
}

pub fn apply_loadout_edit(edit: &loadout_edit::Edit) -> Result<String, String> {
    require_closed()?;
    let path = main_save()?;
    let changed = execute(&[path.clone()], "loadout", require_closed, |raw| {
        loadout_edit::plan(raw, edit)
    })?;
    Ok(if changed == 1 {
        format!("Saved preset change to {}. Original save backed up beside it.", path.display())
    } else {
        "No change needed.".into()
    })
}

pub fn loadout_names() -> Result<Vec<String>, String> {
    let save = hebnix_sdk::save_file::load(&main_save()?, false).map_err(|e| e.to_string())?;
    Ok(save.loadout_sets().into_iter().map(|set| set.name).collect())
}
