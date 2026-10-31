//! Native profile picture hooks. The built-in GFx callback host is created
//! directly; no Workshop menu, map, package, JSON list or feature is required.
use super::upk_package::UpkPackage as RawPackage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    ops::{Deref, DerefMut},
    path::Path,
    time::Instant,
};
const CALLBACK: &str = "GFxData_Workshop_TA.OnEventImageDownloaded";
const PATCH_VERSION: u32 = 5;
#[path = "profile_pictures/native.rs"]
mod native;

// This patch changes payload locations and unused name slots, never object
// identities. Build its symbol table once instead of walking all 135k exports
// for every bytecode operand. Keep the cache local to this patch operation.
struct UpkPackage {
    inner: RawPackage,
    objects: HashMap<String, i32>,
}
impl UpkPackage {
    fn load(path: &Path) -> Result<Self, String> {
        let inner = RawPackage::load(path)?;
        let import_count = inner.import_debug().len();
        let mut objects = HashMap::with_capacity(inner.exports.len() + import_count);
        for index in (1..=inner.exports.len() as i32).chain((1..=import_count as i32).map(|i| -i)) {
            objects
                .entry(inner.asset_path(index).to_ascii_lowercase())
                .or_insert(index);
        }
        Ok(Self { inner, objects })
    }
    fn find_object(&self, path: &str) -> Option<i32> {
        self.objects.get(&path.to_ascii_lowercase()).copied()
    }
}
impl Deref for UpkPackage {
    type Target = RawPackage;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for UpkPackage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

fn timing(stage: &str, started: Instant) {
    tracing::debug!(
        stage,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "Native avatar patch"
    );
    #[cfg(test)]
    eprintln!(
        "Native avatar patch {stage}: {:.3}s",
        started.elapsed().as_secs_f64()
    );
}

#[derive(Clone, Serialize, Deserialize)]
struct ExportSnapshot {
    path: String,
    original: Vec<u8>,
    patched: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
struct NameSnapshot {
    index: usize,
    original: String,
    patched: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    version: u32,
    build: String,
    source: String,
    exports: Vec<ExportSnapshot>,
    names: Vec<NameSnapshot>,
}

fn cooked(game: &Path) -> std::path::PathBuf {
    game.join("TAGame/CookedPCConsole")
}
fn receipt_path(game: &Path) -> std::path::PathBuf {
    cooked(game).join("Backups/ProfilePictures/receipt.json")
}
fn bytes<'a>(package: &'a UpkPackage, path: &str) -> Result<(usize, &'a [u8]), String> {
    let index = object(package, path)? as usize - 1;
    let e = &package.exports[index];
    Ok((
        index,
        &package.image[e.serial_offset..e.serial_offset + e.serial_size],
    ))
}
fn matches(package: &UpkPackage, receipt: &Receipt, on: bool) -> bool {
    receipt
        .names
        .iter()
        .all(|n| package.names.get(n.index) == Some(if on { &n.patched } else { &n.original }))
        && receipt.exports.iter().all(|e| {
            bytes(package, &e.path)
                .is_ok_and(|(_, b)| b == if on { &e.patched } else { &e.original })
        })
}
fn read_receipt(game: &Path) -> Result<Receipt, String> {
    serde_json::from_slice(&fs::read(receipt_path(game)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
pub fn installed(game: &Path) -> Result<bool, String> {
    let path = receipt_path(game);
    if !path.is_file() {
        return Ok(false);
    }
    let receipt = read_receipt(game)?;
    let package = UpkPackage::load(&super::game_path::resolve(&cooked(game), "TAGame.upk"))?;
    Ok(receipt.version == PATCH_VERSION && matches(&package, &receipt, true))
}
fn restore_snapshot(package: &mut UpkPackage, saved: &Receipt) -> Result<(), String> {
    for item in &saved.exports {
        let (index, _) = bytes(package, &item.path)?;
        package.replace_export_payload(index, &item.original)?;
    }
    package.rename_names_same_size(
        &saved
            .names
            .iter()
            .map(|n| (n.index, n.original.clone()))
            .collect::<Vec<_>>(),
    )
}
fn check_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing native profile pictures.".into())
    } else {
        Ok(())
    }
}
pub fn set_enabled(game: &Path, on: bool) -> Result<String, String> {
    set_enabled_checked(game, on, check_closed)
}
fn set_enabled_checked(
    game: &Path,
    on: bool,
    ensure_closed: impl Fn() -> Result<(), String>,
) -> Result<String, String> {
    let started = Instant::now();
    ensure_closed()?;
    let directory = cooked(game);
    let source = super::game_path::resolve(&directory, "TAGame.upk");
    let build = super::backup_guard::build_id(&directory)?;
    let source_digest = Sha256::digest(fs::read(&source).map_err(|e| e.to_string())?);
    let mut package = UpkPackage::load(&source)?;
    timing("load and index", started);
    let receipt = if receipt_path(game).exists() {
        Some(read_receipt(game)?)
    } else {
        None
    };
    let upgrading = on && receipt.as_ref().is_some_and(|r| r.version < PATCH_VERSION);
    if let Some(saved) = &receipt {
        if !(1..=PATCH_VERSION).contains(&saved.version)
            || saved.build != build
            || Path::new(&saved.source) != source
        {
            return Err("Rocket League changed since this profile picture patch was saved. Its old backup will not be restored.".into());
        }
        if matches(&package, saved, on) && !upgrading {
            return Ok(if on {
                "Native profile pictures are enabled."
            } else {
                "Native profile pictures are disabled."
            }
            .into());
        }
        if !matches(&package, saved, !on) && !(upgrading && matches(&package, saved, true)) {
            return Err("The profile picture UI functions were changed by another patch. Refusing to overwrite them.".into());
        }
    } else if !on {
        return Ok("Native profile pictures are disabled.".into());
    }
    // Fingerprint unchanged payloads once, rather than retaining/reloading a
    // second decompressed package. Validation still checks every export.
    let fingerprints: Vec<[u8; 32]> = package
        .exports
        .iter()
        .map(|entry| {
            Sha256::digest(
                &package.image[entry.serial_offset..entry.serial_offset + entry.serial_size],
            )
            .into()
        })
        .collect();
    if upgrading {
        let previous = receipt.as_ref().ok_or("Missing previous avatar receipt")?;
        if matches(&package, previous, true) {
            restore_snapshot(&mut package, previous)?;
        }
    }
    let saved = if on && receipt.is_some() && !upgrading {
        let saved = receipt.expect("receipt checked");
        package.rename_names_same_size(
            &saved
                .names
                .iter()
                .map(|n| (n.index, n.patched.clone()))
                .collect::<Vec<_>>(),
        )?;
        for item in &saved.exports {
            let (index, _) = bytes(&package, &item.path)?;
            package.replace_export_payload(index, &item.patched)?;
        }
        saved
    } else if on {
        let originals = native::TARGETS
            .iter()
            .map(|path| {
                let (index, payload) = bytes(&package, path)?;
                Ok((index, payload.to_vec()))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        let changes = native::patch(&mut package)?;
        let exports = changes
            .exports
            .iter()
            .map(|index| {
                let path = package.asset_path(*index as i32 + 1);
                Ok(ExportSnapshot {
                    original: originals
                        .get(index)
                        .ok_or("Unexpected profile picture patch target")?
                        .clone(),
                    patched: bytes(&package, &path)?.1.to_vec(),
                    path,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let names = changes
            .renamed
            .iter()
            .map(|(old, new)| NameSnapshot {
                index: package
                    .names
                    .iter()
                    .position(|n| n == new)
                    .expect("renamed name exists"),
                original: old.clone(),
                patched: new.clone(),
            })
            .collect();
        Receipt {
            version: PATCH_VERSION,
            build,
            source: source.to_string_lossy().into(),
            exports,
            names,
        }
    } else {
        let saved = receipt.ok_or("No profile picture snapshot exists")?;
        for item in &saved.exports {
            let (index, _) = bytes(&package, &item.path)?;
            package.replace_export_payload(index, &item.original)?;
        }
        package.rename_names_same_size(
            &saved
                .names
                .iter()
                .map(|n| (n.index, n.original.clone()))
                .collect::<Vec<_>>(),
        )?;
        saved
    };
    timing("prepare hooks", started);
    let temporary = source.with_extension("profile-pictures.tmp");
    package.save_fast(&temporary)?;
    timing("save package", started);
    let check = UpkPackage::load(&temporary)?;
    if !matches(&check, &saved, on) {
        let _ = fs::remove_file(&temporary);
        return Err("Profile picture package validation failed.".into());
    }
    // Every untouched export must survive. Name-table edits are restricted to
    // unused slots and preserve indices and the total serialized table size.
    if check.exports.len() != fingerprints.len() {
        let _ = fs::remove_file(&temporary);
        return Err("Profile picture staging changed the export count.".into());
    }
    let targets: HashSet<_> = saved
        .exports
        .iter()
        .map(|e| object(&package, &e.path).map(|i| i as usize - 1))
        .collect::<Result<_, _>>()?;
    for (index, expected) in fingerprints.iter().enumerate() {
        if targets.contains(&index) {
            continue;
        }
        let after = &check.exports[index];
        let actual: [u8; 32] = Sha256::digest(
            &check.image[after.serial_offset..after.serial_offset + after.serial_size],
        )
        .into();
        if *expected != actual {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Profile picture staging changed unrelated export {}",
                check.asset_path(index as i32 + 1)
            ));
        }
    }
    timing("validate every export", started);
    ensure_closed()?;
    if Sha256::digest(fs::read(&source).map_err(|e| e.to_string())?) != source_digest {
        let _ = fs::remove_file(&temporary);
        return Err("Another operation changed the game package while profile pictures were being prepared. Try again.".into());
    }
    let path = receipt_path(game);
    fs::create_dir_all(path.parent().ok_or("Invalid receipt path")?).map_err(|e| e.to_string())?;
    fs::write(
        &path,
        serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::copy(&temporary, &source).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(temporary);
    timing("complete", started);
    Ok(if on {
        "Native profile pictures enabled. Start Rocket League to load player pictures."
    } else {
        "Native profile pictures disabled. Other game patches were preserved."
    }
    .into())
}
#[derive(Clone, Default)]
struct Code {
    bytes: Vec<u8>,
    refs: usize,
}
impl Code {
    fn raw(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            refs: 0,
        }
    }
    fn reference(mut self, value: i32) -> Self {
        self.bytes.extend(value.to_le_bytes());
        self.refs += 1;
        self
    }
    fn then(mut self, other: Self) -> Self {
        self.bytes.extend(other.bytes);
        self.refs += other.refs;
        self
    }
    fn memory(&self) -> usize {
        self.bytes.len() + 4 * self.refs
    }
}

fn object(package: &UpkPackage, path: &str) -> Result<i32, String> {
    package
        .find_object(path)
        .ok_or_else(|| format!("Missing object: {path}"))
}
fn variable(package: &UpkPackage, token: u8, path: &str) -> Result<Code, String> {
    Ok(Code::raw(&[token]).reference(object(package, path)?))
}
fn context(receiver: Code, expression: Code, return_property: i32) -> Result<Code, String> {
    let skip = u16::try_from(expression.memory()).map_err(|_| "Context too large")?;
    Ok(Code::raw(&[0x5e, 0x19, 0x00])
        .then(receiver)
        .then(Code::raw(&skip.to_le_bytes()))
        .reference(return_property)
        .then(Code::raw(&[0x00]))
        .then(expression))
}
fn static_context(
    package: &UpkPackage,
    class: &str,
    expression: Code,
    return_property: i32,
) -> Result<Code, String> {
    let skip = u16::try_from(expression.memory()).map_err(|_| "Static context too large")?;
    Ok(Code::raw(&[0x5e, 0x12, 0x00, 0x20])
        .reference(object(package, class)?)
        .then(Code::raw(&skip.to_le_bytes()))
        .reference(return_property)
        .then(Code::raw(&[0]))
        .then(expression))
}
fn image_texture(package: &UpkPackage) -> Result<Code, String> {
    Ok(Code::raw(&[0x35])
        .reference(object(
            package,
            "IpDrv.OnlineImageDownloaderWeb.OnlineImageDownload.Texture",
        )?)
        .reference(object(
            package,
            "IpDrv.OnlineImageDownloaderWeb.OnlineImageDownload",
        )?)
        .then(Code::raw(&[0x00, 0x00]))
        .then(variable(package, 0x2b, &format!("{CALLBACK}.Image"))?))
}
fn field(package: &UpkPackage, receiver: Code, path: &str) -> Result<Code, String> {
    context(
        receiver,
        variable(package, 0x01, path)?,
        object(package, path)?,
    )
}
fn data_row(package: &UpkPackage, class: &str) -> Result<Code, String> {
    let store = field(
        package,
        variable(package, 0x01, "ProjectX.GFxDataRow_X.Shell")?,
        "ProjectX.GFxShell_X.DataStore",
    )?;
    let get = Code::raw(&[0x1c])
        .reference(object(package, "ProjectX.GFxDataStore_X.GetObject")?)
        .then(Code::raw(&[0x20]).reference(object(package, class)?))
        .then(Code::raw(&[0x48, 0x16]));
    Ok(Code::raw(&[0x2e])
        .reference(object(package, class)?)
        .then(context(
            store,
            get,
            object(package, "ProjectX.GFxDataStore_X.GetObject.ReturnValue")?,
        )?))
}
fn validate(code: &Code) -> Result<(), String> {
    struct Parser<'a> {
        b: &'a [u8],
        at: usize,
        refs: usize,
    }
    impl Parser<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let end = self.at.checked_add(n).ok_or("Bytecode overflow")?;
            let result = self
                .b
                .get(self.at..end)
                .ok_or_else(|| format!("Truncated bytecode at {}", self.at))?;
            self.at = end;
            Ok(result)
        }
        fn reference(&mut self) -> Result<(), String> {
            self.take(4)?;
            self.refs += 1;
            Ok(())
        }
        fn parameters(&mut self, depth: usize) -> Result<(), String> {
            loop {
                match self.b.get(self.at) {
                    Some(0x16) => {
                        self.at += 1;
                        return Ok(());
                    }
                    None => return Err("Unterminated function parameters".into()),
                    _ => self.expression(depth + 1)?,
                }
            }
        }
        fn expression(&mut self, depth: usize) -> Result<(), String> {
            if depth > 64 {
                return Err("Bytecode nesting exceeds limit".into());
            }
            let token = self.take(1)?[0];
            match token {
                0x01 | 0x20 | 0x2b => self.reference()?,
                0x04 | 0x5e => self.expression(depth + 1)?,
                0x0f => {
                    self.expression(depth + 1)?;
                    self.expression(depth + 1)?;
                }
                0x12 | 0x19 => {
                    self.take(1)?;
                    self.expression(depth + 1)?;
                    let skip = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                    self.reference()?;
                    self.take(1)?;
                    let start = self.at;
                    let refs = self.refs;
                    self.expression(depth + 1)?;
                    if self.at - start + 4 * (self.refs - refs) != skip {
                        return Err("Context skip/memory size mismatch".into());
                    }
                }
                0x1b => {
                    self.take(8)?;
                    self.parameters(depth + 1)?;
                }
                0x1c => {
                    self.reference()?;
                    self.parameters(depth + 1)?;
                }
                0x1f => while self.take(1)?[0] != 0 {},
                0x2e => {
                    self.reference()?;
                    self.expression(depth + 1)?;
                }
                0x35 => {
                    self.reference()?;
                    self.reference()?;
                    self.take(2)?;
                    self.expression(depth + 1)?;
                }
                0x38 => {
                    self.take(1)?;
                    self.expression(depth + 1)?;
                }
                0x43 => {
                    self.take(8)?;
                    self.reference()?;
                }
                0x45 => {
                    self.expression(depth + 1)?;
                    let yes_skip = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                    let start = self.at;
                    let refs = self.refs;
                    self.expression(depth + 1)?;
                    if self.at - start + 4 * (self.refs - refs) != yes_skip {
                        return Err("Conditional true skip/memory size mismatch".into());
                    }
                    let no_skip = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                    let start = self.at;
                    let refs = self.refs;
                    self.expression(depth + 1)?;
                    if self.at - start + 4 * (self.refs - refs) != no_skip {
                        return Err("Conditional false skip/memory size mismatch".into());
                    }
                }
                0x57 => {
                    if self.take(1)?[0] != 0x28 {
                        return Err("Unsupported array token".into());
                    }
                    self.expression(depth + 1)?;
                    self.take(2)?;
                    self.parameters(depth + 1)?;
                }
                0x2a => (),
                0x2c => {
                    self.take(1)?;
                }
                0x1d => {
                    self.take(4)?;
                }
                0x0b | 0x17 | 0x25 | 0x27 | 0x28 | 0x48 | 0x4c => (),
                0x80..=0xff => self.parameters(depth + 1)?,
                _ => {
                    return Err(format!(
                        "Unsupported proof token {token:02x} at {}",
                        self.at - 1
                    ));
                }
            }
            Ok(())
        }
    }
    let mut parser = Parser {
        b: &code.bytes,
        at: 0,
        refs: 0,
    };
    while parser.at < parser.b.len() {
        parser.expression(0)?;
    }
    if parser.refs != code.refs {
        return Err("Bytecode reference count mismatch".into());
    }
    Ok(())
}

fn name(package: &UpkPackage, value: &str) -> Result<Code, String> {
    let index = package
        .names
        .iter()
        .position(|n| n == value)
        .ok_or_else(|| format!("Missing name: {value}"))?;
    Ok(Code::raw(&[(index as u32).to_le_bytes(), [0; 4]].concat()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_copy_enabled(root: &Path, on: bool) -> Result<String, String> {
        // Only the isolated fixture may bypass the live-process guard.
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("hebnix-native-avatar-")
        );
        assert_eq!(
            fs::read(root.join("Binaries/Win64/RocketLeague.exe")).unwrap(),
            b"test build fingerprint"
        );
        set_enabled_checked(root, on, || Ok(()))
    }

    /// Uses a caller-supplied package copy; never writes to a game install.
    #[test]
    #[ignore = "requires HEBNIX_AVATAR_TEST_PACKAGE pointing to an unpatched TAGame.upk"]
    fn native_patch_round_trip_without_workshop_files() {
        let source = std::env::var_os("HEBNIX_AVATAR_TEST_PACKAGE").expect("test package path");
        let root = std::env::temp_dir().join(format!(
            "hebnix-native-avatar-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let directory = cooked(&root);
        fs::create_dir_all(&directory).unwrap();
        // Only a build fingerprint fixture. No installed executable is edited.
        fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
        fs::write(
            root.join("Binaries/Win64/RocketLeague.exe"),
            b"test build fingerprint",
        )
        .unwrap();
        fs::copy(source, directory.join("TAGame.upk")).unwrap();
        let before = UpkPackage::load(&directory.join("TAGame.upk")).unwrap();
        assert!(!root.join("TAGame/HbxWS.json").exists());
        assert!(!installed(&root).unwrap());
        set_copy_enabled(&root, true).unwrap();
        assert!(installed(&root).unwrap());
        let patched = UpkPackage::load(&directory.join("TAGame.upk")).unwrap();
        native::assert_friend_call_safety(&patched);
        drop(patched);
        // Upgrade all older receipts. Friends were untouched before version 3;
        // the match-row fallback was also absent in version 1.
        for version in [1, 2, 3, 4] {
            let mut legacy = read_receipt(&root).unwrap();
            let omitted: Vec<_> = legacy
                .exports
                .iter()
                .filter(|e| {
                    (version < 3 && e.path.starts_with("GFxData_Persona_TA."))
                        || (version == 1 && e.path == "GFxData_PRI_TA.UpdatePlayerAvatar")
                })
                .cloned()
                .collect();
            let mut legacy_package = UpkPackage::load(&directory.join("TAGame.upk")).unwrap();
            for export in &omitted {
                let (index, _) = bytes(&legacy_package, &export.path).unwrap();
                legacy_package
                    .replace_export_payload(index, &export.original)
                    .unwrap();
            }
            legacy_package.save(&directory.join("TAGame.upk")).unwrap();
            legacy.version = version;
            legacy
                .exports
                .retain(|e| !omitted.iter().any(|old| old.path == e.path));
            fs::write(receipt_path(&root), serde_json::to_vec(&legacy).unwrap()).unwrap();
            assert!(!installed(&root).unwrap());
            set_copy_enabled(&root, true).unwrap();
            assert!(installed(&root).unwrap());
            assert_eq!(
                read_receipt(&root).unwrap().exports.len(),
                native::TARGETS.len()
            );
        }
        set_copy_enabled(&root, false).unwrap();
        assert!(!installed(&root).unwrap());
        let after = UpkPackage::load(&directory.join("TAGame.upk")).unwrap();
        assert_eq!(before.names, after.names);
        for (index, export) in before.exports.iter().enumerate() {
            let restored = &after.exports[index];
            assert_eq!(
                before.image[export.serial_offset..export.serial_offset + export.serial_size],
                after.image[restored.serial_offset..restored.serial_offset + restored.serial_size],
                "{}",
                before.asset_path(index as i32 + 1)
            );
        }
        // Re-enabling must replay the receipt rather than consume more names.
        set_copy_enabled(&root, true).unwrap();
        assert!(installed(&root).unwrap());
        set_copy_enabled(&root, false).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
