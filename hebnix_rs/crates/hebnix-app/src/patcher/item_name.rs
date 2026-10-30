//! Product labels in encrypted UE3 coalesced localization files. Rebuild from
//! a verified snapshot so independent renames can be restored in any order.
use super::{backup_guard, upk_keys};
use aes::{
    Aes256,
    cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

const FOOTER: u32 = 0x82f93d67;
const WORKSHOP_SECTION: &str = "PlayMenuV4";
const WORKSHOP_FIELD: &str = "WorkshopDescription";
const WORKSHOP_DESCRIPTION: &str = "Play custom maps imported from Hebnix";

#[derive(Clone, Debug)]
pub(crate) struct Label {
    pub section: String,
    pub field: String,
    pub name: String,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Patch {
    pub section: String,
    pub field: String,
    pub original: String,
    pub name: String,
    aliases: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Manifest {
    original_hash: String,
    patched_hash: String,
    pub patches: Vec<Patch>,
}

fn valid_file(file: &str) -> bool {
    file.strip_prefix("Coalesced_")
        .and_then(|s| s.strip_suffix(".bin"))
        .is_some_and(|s| s.len() == 3 && s.bytes().all(|c| c.is_ascii_uppercase()))
}

pub(crate) fn languages(cooked: &Path) -> Vec<String> {
    let mut files: Vec<_> = fs::read_dir(cooked)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| valid_file(f))
        .collect();
    files.sort_by_key(|f| (f != "Coalesced_INT.bin", f.clone()));
    files
}

fn directory(backups: &Path, file: &str) -> Result<PathBuf, String> {
    if !valid_file(file) {
        return Err("Invalid item-name language filename".into());
    }
    Ok(backups.join("ItemName").join(file))
}

pub(crate) fn load(backups: &Path, file: &str) -> Result<Manifest, String> {
    match fs::read(directory(backups, file)?.join("patches.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Could not read item names: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn crypt(bytes: &[u8], key: &[u8; 32], encrypt: bool) -> Result<Vec<u8>, String> {
    if bytes.is_empty() || bytes.len() % 16 != 0 || bytes.len() > 32 * 1024 * 1024 {
        return Err("Invalid coalesced localization size".into());
    }
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut output = bytes.to_vec();
    for block in output.chunks_exact_mut(16) {
        if encrypt {
            cipher.encrypt_block(GenericArray::from_mut_slice(block));
        } else {
            cipher.decrypt_block(GenericArray::from_mut_slice(block));
        }
    }
    Ok(output)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn int(&mut self) -> Result<i32, String> {
        let b = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or("Truncated localization integer")?;
        self.at += 4;
        Ok(i32::from_le_bytes(b.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        let n = self.int()?;
        if !(0..=100_000).contains(&n) {
            return Err("Invalid localization count".into());
        }
        Ok(n as usize)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.int()?;
        let units = n
            .checked_abs()
            .ok_or("Invalid localization string length")? as usize;
        if units > 2_000_000 {
            return Err("Localization string is too large".into());
        }
        let size = units * if n < 0 { 2 } else { 1 };
        let b = self
            .bytes
            .get(self.at..self.at + size)
            .ok_or("Truncated localization string")?;
        self.at += size;
        if units == 0 {
            return Ok(String::new());
        }
        if n < 0 {
            if b[size - 2..] != [0, 0] {
                return Err("Unterminated localization string".into());
            }
            String::from_utf16(
                &b[..size - 2]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| e.to_string())
        } else {
            if b[size - 1] != 0 {
                return Err("Unterminated localization string".into());
            }
            // UE3 ANSI strings use byte characters; current localization uses UTF-16.
            Ok(b[..size - 1].iter().map(|&c| char::from(c)).collect())
        }
    }
}

fn parse(bytes: &[u8]) -> Result<(Vec<Label>, usize), String> {
    let mut r = Reader { bytes, at: 0 };
    let files = r.count()?;
    if files == 0 || files > 1024 {
        return Err("Invalid localization file count".into());
    }
    let mut labels = Vec::new();
    for _ in 0..files {
        let file = r.string()?;
        if !file.contains("\\Localization\\") {
            return Err("Unsupported localization filename".into());
        }
        let products = file
            .rsplit('\\')
            .next()
            .is_some_and(|s| s.to_ascii_lowercase().starts_with("products."))
            && file.to_ascii_lowercase().contains("\\tagame\\");
        let menu = file.to_ascii_lowercase().ends_with("\\tagame.int");
        for _ in 0..r.count()? {
            let section = r.string()?;
            for _ in 0..r.count()? {
                let key = r.string()?;
                let start = r.at;
                let name = r.string()?;
                if (products
                    && (key == "Label"
                        || (key == "Artist"
                            && section.to_ascii_lowercase().starts_with("anthem_"))))
                    || (menu && section == WORKSHOP_SECTION && key == WORKSHOP_FIELD)
                {
                    labels.push(Label {
                        section: section.clone(),
                        field: key,
                        name,
                        start,
                        end: r.at,
                    });
                }
            }
        }
    }
    // The four-byte coalesced sentinel precedes zero AES padding. Counting it
    // as padding both loses the sentinel on edits and rejects aligned files.
    if r.int()? as u32 != FOOTER
        || labels.is_empty()
        || bytes.len() - r.at >= 16
        || bytes[r.at..].iter().any(|&b| b != 0)
    {
        return Err("Unsupported localization data or padding".into());
    }
    Ok((labels, r.at))
}

fn decode(bytes: &[u8]) -> Result<(Vec<u8>, [u8; 32]), String> {
    if bytes.len() < 16 {
        return Err("Truncated localization file".into());
    }
    for (_, key) in upk_keys::embedded()? {
        let prefix = crypt(&bytes[..16], &key, false)?;
        let mut r = Reader {
            bytes: &prefix,
            at: 0,
        };
        if !r.count().is_ok_and(|n| n > 0 && n <= 1024) {
            continue;
        }
        let n = r.int()?;
        if !(-512..=-8).contains(&n) || prefix[8..12] != [b'.', 0, b'.', 0] {
            continue;
        }
        let plain = crypt(bytes, &key, false)?;
        parse(&plain)?;
        return Ok((plain, key));
    }
    Err("Could not decrypt Rocket League localization".into())
}

fn snapshot(backups: &Path, file: &str, manifest: &Manifest) -> Result<Vec<u8>, String> {
    let bytes = fs::read(directory(backups, file)?.join("original.bin"))
        .map_err(|e| format!("Item-name backup unavailable: {e}"))?;
    if hash(&bytes) != manifest.original_hash {
        return Err("Item-name backup has changed; restore cancelled".into());
    }
    Ok(bytes)
}

pub(crate) fn catalog(
    cooked: &Path,
    backups: &Path,
    file: &str,
) -> Result<(Vec<Label>, Manifest), String> {
    let mut manifest = load(backups, file)?;
    let live = fs::read(cooked.join(file)).map_err(|e| e.to_string())?;
    let original = if manifest.patches.is_empty() {
        live
    } else {
        check_live(&live, &manifest)?;
        snapshot(backups, file, &manifest)?
    };
    let (plain, _) = decode(&original)?;
    let labels = parse(&plain)?
        .0
        .into_iter()
        .filter(|l| l.field != WORKSHOP_FIELD)
        .collect();
    manifest.patches.retain(|p| p.field != WORKSHOP_FIELD);
    Ok((labels, manifest))
}

fn check_live(live: &[u8], manifest: &Manifest) -> Result<(), String> {
    if !manifest.patches.is_empty() && hash(live) != manifest.patched_hash {
        return Err("Localization was changed by another patch or game update. Refresh or undo that change before renaming or restoring.".into());
    }
    Ok(())
}

fn wide(text: &str) -> Vec<u8> {
    let units: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
    let mut out = (-(units.len() as i32)).to_le_bytes().to_vec();
    for unit in units {
        out.extend(unit.to_le_bytes());
    }
    out
}

fn rebuild(original: &[u8], patches: &[Patch]) -> Result<Vec<u8>, String> {
    if patches.is_empty() {
        return Ok(original.to_vec());
    }
    let (plain, key) = decode(original)?;
    let (labels, end) = parse(&plain)?;
    let mut edits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for patch in patches {
        for section in &patch.aliases {
            if !seen.insert((section.to_ascii_lowercase(), &patch.field)) {
                return Err("Overlapping item-name patches".into());
            }
            let label = labels
                .iter()
                .find(|l| {
                    l.section == *section && l.field == patch.field && l.name == patch.original
                })
                .ok_or_else(|| format!("Original product label is missing: {section}"))?;
            edits.push((label.start, label.end, wide(&patch.name)));
        }
    }
    edits.sort_by_key(|e| e.0);
    let mut output = Vec::new();
    let mut cursor = 0;
    for (start, end, value) in edits {
        output.extend_from_slice(&plain[cursor..start]);
        output.extend(value);
        cursor = end;
    }
    output.extend_from_slice(&plain[cursor..end]);
    output.resize(output.len().div_ceil(16) * 16, 0);
    parse(&output)?;
    crypt(&output, &key, true)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension(format!("name-{}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// `None` restores one item; an empty section restores every name in this language.
pub(crate) fn rename(
    cooked: &Path,
    backups: &Path,
    file: &str,
    section: &str,
    changes: Option<&[(&str, &str)]>,
) -> Result<(), String> {
    if let Some(changes) = changes {
        let mut fields = std::collections::HashSet::new();
        for (field, name) in changes {
            if !fields.insert(*field)
                || !matches!(*field, "Label" | "Artist")
                || (*field == "Artist" && !section.to_ascii_lowercase().starts_with("anthem_"))
                || (*field == "Label" && name.trim().is_empty())
                || name.chars().count() > 128
                || name.chars().any(char::is_control)
            {
                return Err("Item and song names must contain 1–128 characters. Artist names may be blank. Line breaks and control characters are not supported.".into());
            }
        }
    }
    edit(cooked, backups, file, section, changes)
}

/// Share the localization snapshot with item renames so either feature can
/// be restored without overwriting the other's changes.
pub(crate) fn workshop_description(cooked: &Path, backups: &Path, on: bool) -> Result<(), String> {
    edit(
        cooked,
        backups,
        "Coalesced_INT.bin",
        WORKSHOP_SECTION,
        on.then_some(&[(WORKSHOP_FIELD, WORKSHOP_DESCRIPTION)][..]),
    )
}

fn edit(
    cooked: &Path,
    backups: &Path,
    file: &str,
    section: &str,
    changes: Option<&[(&str, &str)]>,
) -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        return Err(
            "Close Rocket League before changing localization; restart it to load the text.".into(),
        );
    }
    let dir = directory(backups, file)?;
    backup_guard::check_install(cooked, backups)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lock = dir.join("editing.lock");
    let handle = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&lock)
        .map_err(|e| format!("Could not lock item names (another operation may be active): {e}"))?;
    let _lock = Lock(lock);
    let _handle = handle;
    let mut manifest = load(backups, file)?;
    let live = fs::read(cooked.join(file)).map_err(|e| e.to_string())?;
    check_live(&live, &manifest)?;
    let original = if manifest.patches.is_empty() {
        live.clone()
    } else {
        snapshot(backups, file, &manifest)?
    };
    let (plain, _) = decode(&original)?;
    let labels = parse(&plain)?.0;
    let was_empty = manifest.patches.is_empty();
    manifest.patches.retain(|p| {
        (section.is_empty() && p.field == WORKSHOP_FIELD)
            || (!section.is_empty()
                && (p.section != section
                    || changes.is_some_and(|c| !c.iter().any(|(field, _)| p.field == *field))))
    });
    if let Some(changes) = changes {
        for (field, name) in changes {
            let label = labels
                .iter()
                .find(|l| l.section == section && l.field == *field)
                .ok_or("Product label is missing")?;
            if *name != label.name {
                let mut aliases = vec![section.to_string()];
                // These are distinct stock/online/shop product records for the same body.
                for alias in [format!("{section}_Online"), format!("ShopItem_{section}")] {
                    if *field == "Label"
                        && labels.iter().any(|l| {
                            l.section.eq_ignore_ascii_case(&alias)
                                && l.field == *field
                                && l.name == label.name
                        })
                    {
                        aliases.push(
                            labels
                                .iter()
                                .find(|l| l.section.eq_ignore_ascii_case(&alias))
                                .unwrap()
                                .section
                                .clone(),
                        );
                    }
                }
                manifest.patches.push(Patch {
                    section: section.into(),
                    field: (*field).into(),
                    original: label.name.clone(),
                    name: (*name).into(),
                    aliases,
                });
            }
        }
    }
    let after = rebuild(&original, &manifest.patches)?;
    if was_empty {
        atomic_write(&dir.join("original.bin"), &original)?;
        manifest.original_hash = hash(&original);
    }
    manifest.patched_hash = hash(&after);
    let saved = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    atomic_write(&cooked.join(file), &after)?;
    if let Err(error) = atomic_write(&dir.join("patches.json"), &saved) {
        return match atomic_write(&cooked.join(file), &live) {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!(
                "{error}; rollback failed: {rollback}. The original backup is retained."
            )),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut out = 2i32.to_le_bytes().to_vec();
        out.extend(wide("..\\..\\TAGame\\Localization\\INT\\Products.int"));
        let sections = [
            ("Body_MuscleCar", vec![("Label", "Dominus")]),
            ("Body_MuscleCar_Online", vec![("Label", "Dominus")]),
            ("ShopItem_Body_MuscleCar", vec![("Label", "Dominus")]),
            ("EngineAudio_Dominus", vec![("Label", "Dominus")]),
            (
                "Body_Octane",
                vec![("Label", "Octane"), ("Description", "Unchanged")],
            ),
            ("Anthem_Test", vec![("Label", "Song"), ("Artist", "Artist")]),
        ];
        out.extend((sections.len() as i32).to_le_bytes());
        for (section, props) in sections {
            out.extend(wide(section));
            out.extend((props.len() as i32).to_le_bytes());
            for (k, v) in props {
                out.extend(wide(k));
                out.extend(wide(v));
            }
        }
        out.extend(wide("..\\..\\TAGame\\Localization\\INT\\TAGame.int"));
        out.extend(1i32.to_le_bytes());
        out.extend(wide(WORKSHOP_SECTION));
        out.extend(1i32.to_le_bytes());
        out.extend(wide(WORKSHOP_FIELD));
        out.extend(wide("Try out custom maps created in Steam Workshop"));
        out.extend(FOOTER.to_le_bytes());
        out.resize(out.len().div_ceil(16) * 16, 0);
        crypt(&out, &upk_keys::DEFAULT_UPK_KEY, true).unwrap()
    }

    struct Install {
        root: PathBuf,
        cooked: PathBuf,
        backups: PathBuf,
    }
    impl Install {
        fn new(bytes: &[u8]) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/item-name-tests")
                .join(format!("install-{}", rand::random::<u64>()));
            let cooked = root.join("TAGame/CookedPCConsole");
            fs::create_dir_all(&cooked).unwrap();
            fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
            fs::write(root.join("Binaries/Win64/RocketLeague.exe"), b"test build").unwrap();
            fs::write(cooked.join("Coalesced_INT.bin"), bytes).unwrap();
            Self {
                backups: cooked.join("Backups"),
                cooked,
                root,
            }
        }
        fn edit(&self, section: &str, changes: Option<&[(&str, &str)]>) -> Result<(), String> {
            rename(
                &self.cooked,
                &self.backups,
                "Coalesced_INT.bin",
                section,
                changes,
            )
        }
        fn labels(&self) -> Vec<Label> {
            let bytes = fs::read(self.cooked.join("Coalesced_INT.bin")).unwrap();
            parse(&decode(&bytes).unwrap().0).unwrap().0
        }
    }
    impl Drop for Install {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn workshop_roundtrip(bytes: &[u8]) {
        let install = Install::new(bytes);
        let set = |on| workshop_description(&install.cooked, &install.backups, on).unwrap();
        // Disabling older builds, which never changed the description, is safe.
        set(false);
        set(true);
        set(true);
        assert_eq!(
            install
                .labels()
                .iter()
                .find(|l| l.field == WORKSHOP_FIELD)
                .unwrap()
                .name,
            WORKSHOP_DESCRIPTION
        );
        install
            .edit("Body_Octane", Some(&[("Label", "Custom Octane")]))
            .unwrap();
        let (labels, manifest) =
            catalog(&install.cooked, &install.backups, "Coalesced_INT.bin").unwrap();
        assert!(labels.iter().all(|l| l.field != WORKSHOP_FIELD));
        assert!(manifest.patches.iter().all(|p| p.field != WORKSHOP_FIELD));
        set(false);
        assert_eq!(
            install
                .labels()
                .iter()
                .find(|l| l.section == "Body_Octane")
                .unwrap()
                .name,
            "Custom Octane"
        );
        set(true);
        install.edit("", None).unwrap();
        assert_eq!(
            install
                .labels()
                .iter()
                .find(|l| l.field == WORKSHOP_FIELD)
                .unwrap()
                .name,
            WORKSHOP_DESCRIPTION
        );
        set(false);
        set(false);
        assert_eq!(
            fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap(),
            bytes
        );
    }

    #[test]
    fn workshop_description_and_item_names_restore_independently() {
        workshop_roundtrip(&fixture());
    }

    #[test]
    #[ignore = "requires HEBNIX_ITEM_NAME_SOURCE; only edits a temporary copy"]
    fn installed_workshop_description_roundtrip() {
        workshop_roundtrip(&fs::read(std::env::var("HEBNIX_ITEM_NAME_SOURCE").unwrap()).unwrap());
    }

    #[test]
    fn body_aliases_and_independent_anthem_fields_restore_exactly() {
        let bytes = fixture();
        let install = Install::new(&bytes);
        install
            .edit("Body_MuscleCar", Some(&[("Label", "HSV Maloo")]))
            .unwrap();
        install
            .edit(
                "Anthem_Test",
                Some(&[("Label", "新しい曲 🎵"), ("Artist", "Custom Artist")]),
            )
            .unwrap();
        let labels = install.labels();
        for s in [
            "Body_MuscleCar",
            "Body_MuscleCar_Online",
            "ShopItem_Body_MuscleCar",
        ] {
            assert_eq!(
                labels.iter().find(|l| l.section == s).unwrap().name,
                "HSV Maloo"
            );
        }
        assert_eq!(
            labels
                .iter()
                .find(|l| l.section == "EngineAudio_Dominus")
                .unwrap()
                .name,
            "Dominus"
        );
        install
            .edit("Anthem_Test", Some(&[("Artist", "Artist")]))
            .unwrap();
        let labels = install.labels();
        assert_eq!(
            labels
                .iter()
                .find(|l| l.section == "Anthem_Test" && l.field == "Label")
                .unwrap()
                .name,
            "新しい曲 🎵"
        );
        assert_eq!(
            labels
                .iter()
                .find(|l| l.section == "Anthem_Test" && l.field == "Artist")
                .unwrap()
                .name,
            "Artist"
        );
        install.edit("Body_MuscleCar", None).unwrap();
        assert_eq!(
            load(&install.backups, "Coalesced_INT.bin")
                .unwrap()
                .patches
                .len(),
            1
        );
        install.edit("", None).unwrap();
        assert_eq!(
            fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap(),
            bytes
        );
    }

    #[test]
    fn conflicts_bad_inputs_and_corrupt_backup_preserve_live_data() {
        let bytes = fixture();
        let install = Install::new(&bytes);
        for invalid in ["", "\n", &"a".repeat(129)] {
            assert!(
                install
                    .edit("Body_Octane", Some(&[("Label", invalid)]))
                    .is_err()
            );
        }
        assert!(load(&install.backups, "../Coalesced_INT.bin").is_err());
        assert!(install.edit("Missing", Some(&[("Label", "X")])).is_err());
        install
            .edit("Body_Octane", Some(&[("Label", "Car")]))
            .unwrap();
        let dir = directory(&install.backups, "Coalesced_INT.bin").unwrap();
        let saved = fs::read(dir.join("patches.json")).unwrap();
        let live = fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap();
        #[cfg(windows)]
        {
            let manifest_path = dir.join("patches.json");
            let permissions = fs::metadata(&manifest_path).unwrap().permissions();
            let mut readonly = permissions.clone();
            readonly.set_readonly(true);
            fs::set_permissions(&manifest_path, readonly).unwrap();
            let result = install.edit("Body_Octane", Some(&[("Label", "Another Car")]));
            fs::set_permissions(&manifest_path, permissions).unwrap();
            assert!(result.is_err());
            assert_eq!(
                fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap(),
                live
            );
            assert_eq!(fs::read(&manifest_path).unwrap(), saved);
        }
        fs::write(install.cooked.join("Coalesced_INT.bin"), b"external change").unwrap();
        assert!(install.edit("", None).is_err());
        assert_eq!(
            fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap(),
            b"external change"
        );
        fs::write(install.cooked.join("Coalesced_INT.bin"), &live).unwrap();
        fs::write(dir.join("original.bin"), b"bad backup").unwrap();
        assert!(install.edit("", None).is_err());
        assert_eq!(
            fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap(),
            live
        );
        assert_eq!(fs::read(dir.join("patches.json")).unwrap(), saved);
    }

    #[test]
    fn rebuild_changes_only_target_value_bytes() {
        let bytes = fixture();
        let (plain, _) = decode(&bytes).unwrap();
        let (labels, end) = parse(&plain).unwrap();
        let label = labels.iter().find(|l| l.section == "Body_Octane").unwrap();
        let patch = Patch {
            section: label.section.clone(),
            field: "Label".into(),
            original: label.name.clone(),
            name: "Car with a longer name".into(),
            aliases: vec![label.section.clone()],
        };
        let (after, _) = decode(&rebuild(&bytes, &[patch]).unwrap()).unwrap();
        let (new_labels, new_end) = parse(&after).unwrap();
        let new_label = new_labels
            .iter()
            .find(|l| l.section == "Body_Octane")
            .unwrap();
        assert_eq!(&plain[..label.start], &after[..new_label.start]);
        assert_eq!(&plain[label.end..end], &after[new_label.end..new_end]);
        assert!(decode(&[0; 32]).is_err());
        assert!(parse(&[255; 32]).is_err());
    }

    #[test]
    #[ignore = "requires HEBNIX_ITEM_NAME_SOURCE pointing to an installed Coalesced_INT.bin; only edits a temporary copy"]
    fn installed_localization_roundtrip() {
        let source = std::env::var("HEBNIX_ITEM_NAME_SOURCE").unwrap();
        let source = Path::new(&source);
        for file in languages(source.parent().unwrap()) {
            let localized = fs::read(source.parent().unwrap().join(&file)).unwrap();
            let (plain, _) = decode(&localized).unwrap();
            let (labels, _) = parse(&plain).unwrap();
            assert!(labels.iter().any(|l| l.field == "Artist"), "{file}");
        }
        let bytes = fs::read(source).unwrap();
        let install = Install::new(&bytes);
        let original = install.labels();
        assert!(original.len() > 9000);
        install
            .edit("Body_MuscleCar", Some(&[("Label", "HSV Maloo")]))
            .unwrap();
        let anthem = original.iter().find(|l| l.field == "Artist").unwrap();
        install
            .edit(
                &anthem.section,
                Some(&[("Label", "Test Song 🎵"), ("Artist", "Test Artist")]),
            )
            .unwrap();
        let patched = install.labels();
        for before in &original {
            let after = patched
                .iter()
                .find(|l| l.section == before.section && l.field == before.field)
                .unwrap();
            let expected = if [
                "Body_MuscleCar",
                "Body_MuscleCar_Online",
                "ShopItem_Body_MuscleCar",
            ]
            .contains(&before.section.as_str())
                && before.field == "Label"
            {
                "HSV Maloo"
            } else if before.section == anthem.section {
                if before.field == "Artist" {
                    "Test Artist"
                } else {
                    "Test Song 🎵"
                }
            } else {
                &before.name
            };
            assert_eq!(
                after.name, expected,
                "{} / {}",
                before.section, before.field
            );
        }
        install.edit("Body_MuscleCar", None).unwrap();
        install
            .edit(&anthem.section, Some(&[("Artist", &anthem.name)]))
            .unwrap();
        install.edit("", None).unwrap();
        assert_eq!(
            fs::read(install.cooked.join("Coalesced_INT.bin")).unwrap(),
            bytes
        );
        println!(
            "Verified {} original product fields, body aliases, song/artist edits, and byte-exact restore",
            original.len()
        );
    }
}
