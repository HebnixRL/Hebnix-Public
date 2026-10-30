//! Local title presentation through embedded Scaleform code. No proxy or save writes.
#[path = "title_patcher/abc.rs"]
mod abc;
#[path = "title_patcher/movie.rs"]
mod movie;
use super::{
    backup_guard,
    upk_package::{UpkPackage, strip},
};
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TitleOverride {
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<TitleAppearance>,
    pub text: String,
    pub color: u32,
    pub glow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_color: Option<u32>,
}
/// Original catalog appearance, used when match data exposes resolved text instead of an ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TitleAppearance {
    pub text: String,
    pub color: Option<u32>,
    pub glow_color: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    file: String,
    export: usize,
    original: String,
    patched: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    build: String,
    entries: Vec<Entry>,
}
pub struct PackagePatch {
    pub methods: usize,
    entries: Vec<Entry>,
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn valid_file(name: &str) -> bool {
    !name.contains(['/', '\\', ':'])
        && name.to_ascii_lowercase().ends_with(".upk")
        && (name.to_ascii_lowercase().starts_with("gfx")
            || name.eq_ignore_ascii_case("Startup.upk"))
}
fn data<'a>(package: &'a UpkPackage, index: usize) -> Result<&'a [u8], String> {
    let export = package
        .exports
        .get(index)
        .ok_or("Invalid title export index")?;
    package
        .image
        .get(export.serial_offset..export.serial_offset.saturating_add(export.serial_size))
        .ok_or("Invalid title export bounds".into())
}
pub fn validate_rules(rules: &[TitleOverride]) -> Result<(), String> {
    if rules.is_empty() || rules.len() > 64 {
        return Err("Add between 1 and 64 title replacements.".into());
    }
    let mut targets = HashSet::new();
    for rule in rules {
        if rule.text.trim().is_empty()
            || rule.text.chars().count() > 64
            || rule.text.contains('\0')
            || rule.color > 0xffffff
            || rule.glow_color.is_some_and(|color| color > 0xffffff)
            || rule.original.as_ref().is_some_and(|appearance| {
                appearance.text.len() > 4096
                    || appearance.text.contains('\0')
                    || appearance.color.is_some_and(|color| color > 0xffffff)
                    || appearance.glow_color > 0xffffff
            })
        {
            return Err("Titles require 1–64 characters and a valid RGB colour.".into());
        }
        if rule
            .target_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.contains('\0'))
            || !targets.insert(rule.target_id.as_ref().map(|id| id.to_ascii_lowercase()))
        {
            return Err("Choose distinct title IDs for each replacement.".into());
        }
    }
    Ok(())
}

/// Patch only embedded movie exports in a staging file. Installed packages are
/// committed separately after the entire operation has been checked.
pub fn stage_package(
    source: &Path,
    destination: &Path,
    rules: &[TitleOverride],
) -> Result<PackagePatch, String> {
    validate_rules(rules)?;
    let file = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid title package name")?
        .to_string();
    stage_loaded_package(UpkPackage::load(source)?, &file, destination, rules)
}

fn stage_loaded_package(
    mut package: UpkPackage,
    file: &str,
    destination: &Path,
    rules: &[TitleOverride],
) -> Result<PackagePatch, String> {
    let mut entries = Vec::new();
    let mut methods = 0;
    for index in 0..package.exports.len() {
        if !matches!(
            strip(&package.class_of(&package.exports[index])),
            "SwfMovie" | "GFxMovieInfo"
        ) {
            continue;
        }
        let export = &package.exports[index];
        let (props, _) = package.serialized_props(export)?;
        let Some(raw_data) = props
            .iter()
            .find(|prop| prop.name == "RawData" && prop.tag_type == "ArrayProperty")
        else {
            continue;
        };
        let original = data(&package, index)?.to_vec();
        let start = raw_data.value_offset;
        let array = original
            .get(start..start + raw_data.size)
            .ok_or("Invalid title movie array")?;
        let len = super::upk_package::read_i32(array, 0)?;
        if len < 0 || len as usize + 4 != array.len() {
            return Err("Unsupported title movie array framing".into());
        }
        let Some((patched, count)) = movie::patch(&array[4..], rules)? else {
            continue;
        };
        if raw_data.value_offset != raw_data.tag_offset + 24 {
            return Err("Unsupported title property tag".into());
        }
        let mut replacement = original[..start].to_vec();
        replacement[raw_data.tag_offset + 16..raw_data.tag_offset + 20]
            .copy_from_slice(&((patched.len() + 4) as u32).to_le_bytes());
        replacement.extend_from_slice(&(patched.len() as u32).to_le_bytes());
        replacement.extend(patched);
        replacement.extend_from_slice(&original[start + raw_data.size..]);
        entries.push(Entry {
            file: file.to_string(),
            export: index,
            original: hash(&original),
            patched: hash(&replacement),
        });
        package.replace_title_export(index, &replacement)?;
        methods += count;
    }
    if !entries.is_empty() {
        save_verified(&package, destination)?;
    }
    Ok(PackagePatch { methods, entries })
}

fn save_verified(package: &UpkPackage, destination: &Path) -> Result<(), String> {
    package.save_fast(destination)?;
    let check = UpkPackage::load(destination)?;
    if check.names != package.names || check.exports.len() != package.exports.len() {
        return Err("Title package table verification failed".into());
    }
    // Reopen and compare every export, including unrelated existing edits.
    for index in 0..check.exports.len() {
        if data(&check, index)? != data(package, index)? {
            return Err("Title export verification failed".into());
        }
    }
    Ok(())
}

fn folder(base: &Path, cooked: &Path) -> PathBuf {
    let canonical = fs::canonicalize(cooked).unwrap_or_else(|_| cooked.to_path_buf());
    let identity = canonical
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    base.join("title_upk")
        .join(&hash(identity.as_bytes())[..16])
}
fn load_manifest(root: &Path) -> Result<Manifest, String> {
    match fs::read(root.join("manifest.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid title restore manifest: {e}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(error.to_string()),
    }
}
fn closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before applying or restoring title patches.".into())
    } else {
        Ok(())
    }
}
fn baseline(mut live: UpkPackage, backup: &Path, entries: &[&Entry]) -> Result<UpkPackage, String> {
    let original = UpkPackage::load(backup)?;
    if live.names != original.names || live.exports.len() != original.exports.len() {
        return Err("Title backup tables no longer match; no files changed.".into());
    }
    for entry in entries {
        if hash(data(&live, entry.export)?) != entry.patched
            || hash(data(&original, entry.export)?) != entry.original
        {
            return Err(format!(
                "{} was changed by another patch. Restore that edit before changing titles.",
                entry.file
            ));
        }
        live.replace_title_export(entry.export, data(&original, entry.export)?)?;
    }
    // Keep the restored exports in memory. Reapply patches this same package,
    // then saves/reopens/verifies the final staged result exactly once.
    Ok(live)
}

pub fn apply(base: &Path, cooked: &Path, rules: &[TitleOverride]) -> Result<String, String> {
    run(base, cooked, Some(rules))
}
pub fn restore(base: &Path, cooked: &Path) -> Result<String, String> {
    run(base, cooked, None)
}
pub(crate) fn restore_for_uninstall(base: &Path, cooked: &Path) -> Result<(), String> {
    if !load_manifest(&folder(base, cooked))?.entries.is_empty() {
        restore(base, cooked)?;
    }
    Ok(())
}
fn run(base: &Path, cooked: &Path, rules: Option<&[TitleOverride]>) -> Result<String, String> {
    closed()?;
    if let Some(rules) = rules {
        validate_rules(rules)?;
    }
    let build = backup_guard::build_id(cooked)?;
    let root = folder(base, cooked);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut previous = load_manifest(&root)?;
    if !previous.entries.is_empty() && previous.build != build {
        if rules.is_none() {
            return Err("Rocket League changed since these title backups were made. Old backups will not be restored. Verify game files before applying titles again.".into());
        }
        let mut inspected = HashSet::new();
        for entry in &previous.entries {
            if !valid_file(&entry.file) {
                return Err("Invalid title backup filename".into());
            }
            if !inspected.insert(entry.file.clone()) || !cooked.join(&entry.file).is_file() {
                continue;
            }
            let current = UpkPackage::load(&cooked.join(&entry.file))?;
            if previous
                .entries
                .iter()
                .filter(|old| old.file == entry.file)
                .any(|old| data(&current, old.export).is_ok_and(|bytes| hash(bytes) == old.patched))
            {
                return Err("Old title code is still installed after a game update. Verify Rocket League's game files before applying titles again.".into());
            }
        }
        // Keep the old build recoverable for inspection without ever restoring
        // its assets into the new game. Fresh backups come from current files.
        let archive = root.join(format!("old-build-{:016x}", rand::random::<u64>()));
        fs::create_dir(&archive).map_err(|e| e.to_string())?;
        for file in inspected {
            let backup = root.join(format!("{file}.original"));
            if backup.is_file() {
                fs::copy(&backup, archive.join(format!("{file}.original")))
                    .map_err(|e| e.to_string())?;
            }
        }
        fs::rename(root.join("manifest.json"), archive.join("manifest.json"))
            .map_err(|e| e.to_string())?;
        previous = Manifest::default();
    }
    let stage = root.join(format!("stage-{:016x}", rand::random::<u64>()));
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let mut preserve_stage = false;
    let result = (|| {
        let mut groups: HashMap<String, Vec<&Entry>> = HashMap::new();
        for entry in &previous.entries {
            if !valid_file(&entry.file) {
                return Err("Invalid title manifest filename".into());
            }
            groups.entry(entry.file.clone()).or_default().push(entry);
        }
        let mut files = if rules.is_some() {
            fs::read_dir(cooked)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .filter(|name| valid_file(name))
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        } else {
            groups.keys().cloned().collect()
        };
        files.sort();
        files.dedup();
        let mut pending = Vec::new();
        let mut entries = Vec::new();
        let mut methods = 0;
        for file in files {
            let live = cooked.join(&file);
            let live_hash = hash(&fs::read(&live).map_err(|e| e.to_string())?);
            let file_stage = stage.join(&file);
            let backup = root.join(format!("{file}.original"));
            let package = UpkPackage::load(&live)?;
            let package = if let Some(old) = groups.get(&file) {
                baseline(package, &backup, old)?
            } else {
                package
            };
            if let Some(rules) = rules {
                let patched = stage_loaded_package(package, &file, &file_stage, rules)
                    .map_err(|error| format!("{file}: {error}"))?;
                if patched.entries.is_empty() {
                    continue;
                }
                if !groups.contains_key(&file) {
                    fs::copy(&live, &backup).map_err(|e| e.to_string())?;
                }
                entries.extend(patched.entries.into_iter().map(|mut entry| {
                    entry.file = file.clone();
                    entry
                }));
                methods += patched.methods;
            } else {
                save_verified(&package, &file_stage)?;
            }
            if hash(&fs::read(&live).map_err(|e| e.to_string())?) != live_hash {
                return Err(format!(
                    "{file} changed while title packages were prepared; no title files were installed."
                ));
            }
            let rollback = stage.join(format!("rollback-{file}"));
            fs::copy(&live, &rollback).map_err(|e| e.to_string())?;
            if hash(&fs::read(&rollback).map_err(|e| e.to_string())?) != live_hash {
                return Err(format!(
                    "{file} changed while its recovery copy was being prepared."
                ));
            }
            pending.push((live, file_stage, rollback));
        }
        if rules.is_some() && methods == 0 {
            return Err("No compatible title UI code was found; no game files changed.".into());
        }
        closed()?;
        for (live, _, rollback) in &pending {
            if fs::read(live).map_err(|e| e.to_string())?
                != fs::read(rollback).map_err(|e| e.to_string())?
            {
                return Err(
                    "A UI package changed during preparation; no title files were installed."
                        .into(),
                );
            }
        }
        let mut installed = Vec::new();
        for (live, staged, rollback) in &pending {
            // Include the current file in rollback even if a copy fails midway.
            installed.push((live, rollback));
            if let Err(error) = closed().and_then(|_| {
                fs::copy(staged, live)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }) {
                let mut errors = Vec::new();
                for (live, rollback) in installed.iter().rev() {
                    if let Err(e) = fs::copy(rollback, live) {
                        errors.push(e.to_string());
                    }
                }
                preserve_stage = !errors.is_empty();
                return Err(format!(
                    "Title install failed: {error}. {}",
                    if errors.is_empty() {
                        "Original files restored.".into()
                    } else {
                        format!("Rollback needs attention: {}", errors.join("; "))
                    }
                ));
            }
        }
        let manifest = Manifest { build, entries };
        let manifest_result = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let next = root.join("manifest.next");
                fs::write(&next, bytes).map_err(|e| e.to_string())?;
                fs::rename(next, root.join("manifest.json")).map_err(|e| e.to_string())
            });
        if let Err(error) = manifest_result {
            for (live, rollback) in installed.iter().rev() {
                fs::copy(rollback, live).map_err(|e| {
                    preserve_stage = true;
                    format!("Manifest failed: {error}; rollback failed: {e}")
                })?;
            }
            return Err(format!(
                "Title manifest could not be saved; game files restored: {error}"
            ));
        }
        Ok(if rules.is_some() {
            String::new()
        } else {
            "Restored original title UI code. Other exports were preserved.".into()
        })
    })();
    if preserve_stage {
        return Err(format!(
            "{} Recovery copies retained at {}",
            result.unwrap_err(),
            stage.display()
        ));
    }
    let _ = fs::remove_dir_all(&stage);
    result
}

#[derive(Default)]
pub struct TitlePatcherState {
    result: Option<Receiver<Result<String, String>>>,
    pub status: String,
    pending: Option<(PathBuf, Option<Vec<TitleOverride>>)>,
    applied: Option<(PathBuf, Vec<TitleOverride>)>,
}
impl TitlePatcherState {
    /// Report only file transactions completed successfully in this session.
    pub fn applied_matches(&self, cooked: &Path, rules: &[TitleOverride]) -> Option<bool> {
        if self.busy() {
            return None;
        }
        self.applied
            .as_ref()
            .filter(|(path, _)| path == cooked)
            .map(|(_, applied)| applied == rules)
    }

    pub fn busy(&self) -> bool {
        self.result.is_some()
    }
    pub fn poll(&mut self) {
        if let Some(result) = self.result.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.result = None;
            if result.is_ok() {
                self.applied = self
                    .pending
                    .take()
                    .and_then(|(path, rules)| rules.map(|rules| (path, rules)));
            } else {
                self.pending = None;
                self.applied = None;
            }
            self.status = result.unwrap_or_else(|e| format!("Title patch failed: {e}"));
        }
    }
    pub fn start(
        &mut self,
        base: PathBuf,
        cooked: PathBuf,
        rules: Option<Vec<TitleOverride>>,
        ctx: egui::Context,
    ) {
        if self.busy() {
            return;
        }
        let (tx, rx) = unbounded();
        self.result = Some(rx);
        self.pending = Some((cooked.clone(), rules.clone()));
        self.status = "Preparing title UI packages…".into();
        std::thread::spawn(move || {
            let result = if let Some(rules) = rules {
                apply(&base, &cooked, &rules)
            } else {
                restore(&base, &cooked)
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
}
