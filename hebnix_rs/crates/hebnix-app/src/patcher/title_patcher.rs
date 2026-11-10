//! Local title and rank display patches for embedded Scaleform movies.
#[path = "title_patcher/abc.rs"]
pub(super) mod abc;
#[path = "title_patcher/ranks.rs"]
mod ranks;
pub use ranks::{RankPatchState, RankPick};
#[path = "title_patcher/movie.rs"]
pub(super) mod movie;
use super::{
    backup_guard,
    upk_package::{UpkPackage, strip},
    workshop_menu_ui,
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
    stage_loaded_package(
        UpkPackage::load(source)?,
        &file,
        destination,
        &|raw| movie::patch(raw, rules),
        false,
        &mut |_, _, _| Ok(()),
    )
}

fn stage_loaded_package(
    mut package: UpkPackage,
    file: &str,
    destination: &Path,
    edit: &MovieEdit<'_>,
    save_restored: bool,
    record: &mut impl FnMut(&UpkPackage, &Entry, &[u8]) -> Result<(), String>,
) -> Result<PackagePatch, String> {
    let mut entries = Vec::new();
    let mut methods = 0;
    for index in 0..package.exports.len() {
        // The Workshop LAN patch owns these movies and validates their exact contents.
        if workshop_menu_ui::managed_movie(file, &package, index).is_some() {
            continue;
        }
        let Some((replacement, count)) = rewrite_movie_export(&package, index, edit)? else {
            continue;
        };
        let entry = Entry {
            file: file.to_string(),
            export: index,
            original: hash(data(&package, index)?),
            patched: hash(&replacement),
        };
        record(&package, &entry, &replacement)?;
        entries.push(entry);
        package.replace_title_export(index, &replacement)?;
        methods += count;
    }
    if save_restored || !entries.is_empty() {
        save_verified(&package, destination)?;
    }
    Ok(PackagePatch { methods, entries })
}

type MovieEdit<'a> = dyn Fn(&[u8]) -> Result<Option<(Vec<u8>, usize)>, String> + 'a;

fn rewrite_movie_export(
    package: &UpkPackage,
    index: usize,
    edit: &MovieEdit<'_>,
) -> Result<Option<(Vec<u8>, usize)>, String> {
    rewrite_export(package, index, |_, raw| edit(raw))
}

fn rewrite_export(
    package: &UpkPackage,
    index: usize,
    edit: impl FnOnce(usize, &[u8]) -> Result<Option<(Vec<u8>, usize)>, String>,
) -> Result<Option<(Vec<u8>, usize)>, String> {
    if !matches!(
        strip(&package.class_of(&package.exports[index])),
        "SwfMovie" | "GFxMovieInfo"
    ) {
        return Ok(None);
    }
    let export = &package.exports[index];
    let (props, _) = package.serialized_props(export)?;
    let Some(raw_data) = props
        .iter()
        .find(|prop| prop.name == "RawData" && prop.tag_type == "ArrayProperty")
    else {
        return Ok(None);
    };
    let original = data(package, index)?;
    let start = raw_data.value_offset;
    let array = original
        .get(start..start + raw_data.size)
        .ok_or("Invalid title movie array")?;
    let len = super::upk_package::read_i32(array, 0)?;
    if len < 0 || len as usize + 4 != array.len() {
        return Err("Unsupported title movie array framing".into());
    }
    let Some((patched, count)) = edit(start, &array[4..])? else {
        return Ok(None);
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
    Ok(Some((replacement, count)))
}

fn ring_variant(
    package: &UpkPackage,
    index: usize,
    enabled: bool,
) -> Result<Option<Vec<u8>>, String> {
    Ok(rewrite_movie_export(package, index, &|raw| {
        movie::rewrite(raw, &|abc| abc::patch_boost_ring(abc, enabled))
    })?
    .map(|(payload, _)| payload))
}

pub(super) const BOOST_HUD: &str = "GFX_Hud_SF.upk";
pub(super) static UI_PATCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn toggle_boost_ring(cooked: &Path) -> Result<bool, String> {
    let _guard = UI_PATCH_LOCK
        .lock()
        .map_err(|_| "UI patch lock unavailable")?;
    if hebnix_sdk::process::is_rocket_league_running() {
        return Err("Close Rocket League before changing the boost ring.".into());
    }
    let live = super::game_path::resolve(cooked, BOOST_HUD);
    let before = fs::read(&live).map_err(|error| error.to_string())?;
    let mut package = UpkPackage::load(&live)?;
    let mut change = None;
    for index in 0..package.exports.len() {
        let variant = match ring_variant(&package, index, false)? {
            Some(payload) => Some((false, payload)),
            None => ring_variant(&package, index, true)?.map(|payload| (true, payload)),
        };
        if let Some((enabled, payload)) = variant {
            if change.is_some() {
                return Err("Found multiple boost ring handlers in the HUD.".into());
            }
            change = Some((index, enabled, payload));
        }
    }
    let (index, enabled, payload) =
        change.ok_or("Could not find the boost ring handler in the HUD.")?;
    package.replace_title_export(index, &payload)?;
    let staged = live.with_extension(format!("boost-ring-{:016x}.tmp", rand::random::<u64>()));
    let result = (|| {
        save_verified(&package, &staged)?;
        if fs::read(&live).map_err(|error| error.to_string())? != before {
            return Err("The HUD changed while the boost ring was being prepared.".into());
        }
        super::upk_history::install_copy(cooked, &staged, &live, "Team boost ring")?;
        Ok(enabled)
    })();
    let _ = fs::remove_file(staged);
    result
}

#[cfg(test)]
#[test]
fn team_ring_round_trips_real_hud_and_preserves_title_exports() {
    let Some(original) = std::env::var_os("HEBNIX_BOOST_HUD_FIXTURE").map(PathBuf::from) else {
        return;
    };
    let root = std::env::temp_dir().join(format!("hebnix-ring-{:016x}", rand::random::<u64>()));
    let cooked = root.join("TAGame/CookedPCConsole");
    fs::create_dir_all(&cooked).unwrap();
    let binaries = root.join("Binaries/Win64");
    fs::create_dir_all(&binaries).unwrap();
    fs::write(binaries.join("RocketLeague.exe"), b"fixture build").unwrap();
    let copy = cooked.join(BOOST_HUD);
    let rules = [TitleOverride {
        target_id: Some("boost-ring-test".into()),
        original: None,
        text: "Boost ring title test".into(),
        color: 0xff0000,
        glow: false,
        glow_color: None,
    }];
    let patched = stage_loaded_package(
        UpkPackage::load(&original).unwrap(),
        BOOST_HUD,
        &copy,
        &|raw| movie::patch(raw, &rules),
        false,
        &mut |_, _, _| Ok(()),
    )
    .unwrap();
    assert!(!patched.entries.is_empty());
    let titles = UpkPackage::load(&copy).unwrap();
    assert!(toggle_boost_ring(&cooked).unwrap());
    let ring_and_titles = UpkPackage::load(&copy).unwrap();
    let entries = patched.entries.iter().collect::<Vec<_>>();
    let (ring_only, _) = baseline(ring_and_titles, &root, &original, &entries).unwrap();
    assert!(entries.iter().any(|entry| {
        ring_variant(&ring_only, entry.export, false)
            .unwrap()
            .is_some()
    }));
    assert!(!toggle_boost_ring(&cooked).unwrap());
    let restored = UpkPackage::load(&copy).unwrap();
    for export in &titles.exports {
        assert_eq!(
            data(&restored, export.table_index).unwrap(),
            data(&titles, export.table_index).unwrap()
        );
    }
    fs::remove_dir_all(root).unwrap();
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

fn folder(base: &Path, cooked: &Path, feature: &str) -> PathBuf {
    let canonical = fs::canonicalize(cooked).unwrap_or_else(|_| cooked.to_path_buf());
    let identity = canonical
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    base.join(feature).join(&hash(identity.as_bytes())[..16])
}
pub(super) fn original_movie_package(cooked: &Path, file: &str) -> PathBuf {
    folder(&crate::config::base_dir(), cooked, "title_upk").join(format!("{file}.original"))
}

/// Older Titles builds edited Workshop's movie. Recover its original
/// export from the title journal when the live export still matches that edit.
pub(super) fn legacy_workshop_movie(
    cooked: &Path,
    file: &str,
    live: &UpkPackage,
    index: usize,
    movie_offset: usize,
) -> Option<Vec<u8>> {
    let base = crate::config::base_dir();
    let root = folder(&base, cooked, "title_upk");
    let manifest = load_manifest(&root).ok()?;
    let current = hash(data(live, index).ok()?);
    let entry = manifest.entries.iter().find(|entry| {
        entry.file.eq_ignore_ascii_case(file) && entry.export == index && entry.patched == current
    })?;
    let original = read_snapshot(&root, &entry.original).ok()??;
    movie_bytes(&original, movie_offset).ok().map(<[u8]>::to_vec)
}

fn load_manifest(root: &Path) -> Result<Manifest, String> {
    match fs::read(root.join("manifest.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid UI restore manifest: {e}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(error.to_string()),
    }
}
fn closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before applying or restoring UI patches.".into())
    } else {
        Ok(())
    }
}
// Content-addressed export snapshots let each feature undo only its own
// ActionScript edits, even after the other feature has been reapplied.
fn snapshot(root: &Path, digest: &str) -> Result<PathBuf, String> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid UI export checksum".into());
    }
    Ok(root.join("exports").join(digest))
}

fn read_snapshot(root: &Path, digest: &str) -> Result<Option<Vec<u8>>, String> {
    match fs::read(snapshot(root, digest)?) {
        Ok(bytes) if hash(&bytes) == digest => Ok(Some(bytes)),
        Ok(_) => Err("UI export backup checksum failed".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn save_snapshot(root: &Path, digest: &str, bytes: &[u8]) -> Result<(), String> {
    let path = snapshot(root, digest)?;
    if !path.is_file() {
        fs::create_dir_all(root.join("exports")).map_err(|e| e.to_string())?;
        super::upk_history::replace(&path, bytes)?;
    }
    Ok(())
}

fn movie_bytes(payload: &[u8], start: usize) -> Result<&[u8], String> {
    let len = super::upk_package::read_i32(payload, start)?;
    let len = usize::try_from(len).map_err(|_| "Invalid UI movie length")?;
    payload
        .get(start + 4..start + 4 + len)
        .ok_or("Truncated UI movie backup".into())
}

fn capture_legacy_patch(package: &UpkPackage, root: &Path, entry: &Entry) -> Result<(), String> {
    if snapshot(root, &entry.patched)?.is_file() {
        return Ok(());
    }
    let live = data(package, entry.export)?;
    if hash(live) == entry.patched {
        return save_snapshot(root, &entry.patched, live);
    }
    if entry.file.eq_ignore_ascii_case(BOOST_HUD) {
        for enabled in [false, true] {
            if let Some(variant) = ring_variant(package, entry.export, enabled)?
                && hash(&variant) == entry.patched
            {
                return save_snapshot(root, &entry.patched, &variant);
            }
        }
    }
    Err("The existing UI patch no longer matches its restore record.".into())
}

fn baseline(
    mut live: UpkPackage,
    root: &Path,
    backup: &Path,
    entries: &[&Entry],
) -> Result<(UpkPackage, bool), String> {
    // A Workshop-owned movie can supersede a legacy Titles entry. Resolve
    // that ownership before opening a Titles backup we may no longer need.
    let entries: Vec<_> = entries.iter().copied().filter(|entry| {
        !workshop_menu_ui::managed_movie(&entry.file, &live, entry.export)
            .is_some_and(|movie| movie.has_known_movie(&live))
    }).collect();
    if entries.is_empty() {
        return Ok((live, false));
    }
    let original = UpkPackage::load(backup)?;
    if live.names != original.names || live.exports.len() != original.exports.len() {
        return Err("UI backup tables no longer match; no files changed.".into());
    }
    let mut restored = false;
    for entry in entries {
        let live_payload = data(&live, entry.export)?;
        let saved_original = read_snapshot(root, &entry.original)?;
        let original_payload = match &saved_original {
            Some(bytes) => bytes,
            None => data(&original, entry.export)?,
        };
        let live_hash = hash(live_payload);
        if hash(original_payload) != entry.original {
            return Err(format!(
                "{} has a UI backup that no longer matches its restore record.",
                entry.file
            ));
        }
        if live_hash == entry.original {
            continue;
        }
        if live_hash != entry.patched
            && let Some(patched) = read_snapshot(root, &entry.patched)?
        {
            let mut found_movie = false;
            let replacement = rewrite_export(&live, entry.export, |start, current| {
                found_movie = true;
                movie::revert(
                    current,
                    movie_bytes(original_payload, start)?,
                    movie_bytes(&patched, start)?,
                )
            })?;
            if !found_movie {
                return Err(format!("The backed-up UI movie is missing from {}.", entry.file));
            }
            if let Some((replacement, _)) = replacement {
                live.replace_title_export(entry.export, &replacement)?;
                restored = true;
            }
            continue;
        }
        let boost_hud = entry.file.eq_ignore_ascii_case(BOOST_HUD);
        let live_without_ring = if boost_hud {
            ring_variant(&live, entry.export, false)?
        } else {
            None
        };
        let matches = live_hash == entry.patched
            || (boost_hud
                && if let Some(payload) = &live_without_ring {
                    hash(payload) == entry.patched
                } else {
                    ring_variant(&live, entry.export, true)?
                        .is_some_and(|payload| hash(&payload) == entry.patched)
                });
        if !matches {
            return Err(format!(
                "{} was changed by another patch. Restore that edit before changing this UI patch.",
                entry.file
            ));
        }
        let adjusted_original = if boost_hud && saved_original.is_none() {
            ring_variant(&original, entry.export, live_without_ring.is_some())?
        } else {
            None
        };
        live.replace_title_export(
            entry.export,
            adjusted_original.as_deref().unwrap_or(original_payload),
        )?;
        restored = true;
    }
    // Keep the restored exports in memory. Reapply patches this same package,
    // then saves/reopens/verifies the final staged result exactly once.
    Ok((live, restored))
}

pub fn apply(base: &Path, cooked: &Path, rules: &[TitleOverride]) -> Result<String, String> {
    validate_rules(rules)?;
    run(base, cooked, false, Some(&|raw| movie::patch(raw, rules)))
}
pub fn restore(base: &Path, cooked: &Path) -> Result<String, String> {
    run(base, cooked, false, None)
}
pub(crate) fn restore_for_uninstall(base: &Path, cooked: &Path) -> Result<(), String> {
    if !load_manifest(&folder(base, cooked, "title_upk"))?
        .entries
        .is_empty()
    {
        restore(base, cooked)?;
    }
    if !load_manifest(&folder(base, cooked, "rank_upk"))?
        .entries
        .is_empty()
    {
        run(base, cooked, true, None)?;
    }
    Ok(())
}
fn run(
    base: &Path,
    cooked: &Path,
    ranks: bool,
    edit: Option<&MovieEdit<'_>>,
) -> Result<String, String> {
    let _guard = UI_PATCH_LOCK
        .lock()
        .map_err(|_| "UI patch lock unavailable")?;
    closed()?;
    let (feature, other_feature) = if ranks {
        ("rank_upk", "title_upk")
    } else {
        ("title_upk", "rank_upk")
    };
    let build = backup_guard::build_id(cooked)?;
    let root = folder(base, cooked, feature);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let other_root = folder(base, cooked, other_feature);
    let other = if edit.is_some() {
        load_manifest(&other_root)?
    } else {
        Manifest::default()
    };
    let other_entries = if other.build == build {
        &other.entries[..]
    } else {
        &[]
    };
    let mut previous = load_manifest(&root)?;
    if !previous.entries.is_empty() && previous.build != build {
        if edit.is_none() {
            return Err("Rocket League changed since these UI backups were made. Old backups will not be restored. Verify game files before applying the patch again.".into());
        }
        let mut inspected = HashSet::new();
        for entry in &previous.entries {
            if !valid_file(&entry.file) {
                return Err("Invalid UI backup filename".into());
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
                return Err("Old UI code is still installed after a game update. Verify Rocket League's game files before applying the patch again.".into());
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
                return Err("Invalid UI manifest filename".into());
            }
            groups.entry(entry.file.clone()).or_default().push(entry);
        }
        let mut files = if edit.is_some() {
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
            let (package, restored) = if let Some(old) = groups.get(&file) {
                baseline(package, &root, &backup, old)?
            } else {
                (package, false)
            };
            if let Some(edit) = edit {
                let patched = stage_loaded_package(
                    package,
                    &file,
                    &file_stage,
                    edit,
                    restored,
                    &mut |package, entry, replacement| {
                        if let Some(other) = other_entries.iter().find(|other| {
                            other.file.eq_ignore_ascii_case(&file) && other.export == entry.export
                        }) {
                            capture_legacy_patch(package, &other_root, other)?;
                        }
                        save_snapshot(&root, &entry.original, data(package, entry.export)?)?;
                        save_snapshot(&root, &entry.patched, replacement)
                    },
                )
                .map_err(|error| format!("{file}: {error}"))?;
                if patched.entries.is_empty() && !restored {
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
            } else if restored {
                save_verified(&package, &file_stage)?;
            } else {
                continue;
            }
            if hash(&fs::read(&live).map_err(|e| e.to_string())?) != live_hash {
                return Err(format!(
                    "{file} changed while UI packages were prepared; no UI files were installed."
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
        if edit.is_some() && methods == 0 {
            return Err("No compatible UI code was found; no game files changed.".into());
        }
        closed()?;
        for (live, _, rollback) in &pending {
            if fs::read(live).map_err(|e| e.to_string())?
                != fs::read(rollback).map_err(|e| e.to_string())?
            {
                return Err(
                    "A UI package changed during preparation; no UI files were installed.".into(),
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
                    "UI patch install failed: {error}. {}",
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
                "UI patch manifest could not be saved; game files restored: {error}"
            ));
        }
        let retained = manifest
            .entries
            .iter()
            .flat_map(|entry| [&entry.original, &entry.patched])
            .collect::<HashSet<_>>();
        for digest in previous
            .entries
            .iter()
            .flat_map(|entry| [&entry.original, &entry.patched])
        {
            if !retained.contains(digest)
                && let Ok(path) = snapshot(&root, digest)
            {
                let _ = fs::remove_file(path);
            }
        }
        if !ranks {
            let history_label = if edit.is_some() {
                "Title overrides"
            } else {
                "Restore Title overrides"
            };
            for (live, rollback) in &installed {
                super::upk_history::record_linked(
                    cooked,
                    live,
                    &fs::read(rollback).map_err(|e| e.to_string())?,
                    history_label,
                )?;
            }
        }
        Ok(match (ranks, edit.is_some()) {
            (true, true) => "Rank display applied. Start Rocket League to see your changes.".into(),
            (true, false) => "Rank display restored.".into(),
            (false, true) => String::new(),
            (false, false) => {
                "Restored original title UI code. Other exports were preserved.".into()
            }
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
