//! decrypt + read RL .save files. the property tree is a serde_json::Value
//! with typed accessors on top via SaveData.

pub mod accessors;
pub mod bakkes_code;
pub mod binary_parser;
pub mod binary_serializer;
pub mod configuration;
pub mod crypto;
pub mod file_io;
pub mod item_cleanup;
pub mod loadout_code;
pub mod loadout_edit;
pub mod models;

pub use accessors::SaveData;
pub use configuration::{
    ConfigurationEditorKind, ConfigurationValue, list_configuration, restore_configuration,
    update_configuration, validate_configuration,
};
pub use file_io::{RawSave, assemble_savedata, parse_savedata};
pub use models::*;

use std::path::{Path, PathBuf};

use crate::utils::constants::{SAVE_PATH_EPIC, SAVE_PATH_STEAM};

/// decrypt + parse a .save into a typed SaveData
pub fn load(filepath: &Path, check_crc: bool) -> Result<SaveData, crypto::SaveError> {
    let raw = parse_savedata(filepath, check_crc)?;
    Ok(SaveData::from_raw(raw, filepath.to_path_buf()))
}

/// stem of an account save, with or without the rotation suffix
fn is_account_save_stem(stem: &str) -> bool {
    let base = match stem.split_once('_') {
        Some((base, suffix))
            if !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit()) =>
        {
            base
        }
        Some(_) => return false,
        None => stem,
    };
    let hex32 = base.len() == 32 && base.chars().all(|c| c.is_ascii_hexdigit());
    let numeric = (10..=20).contains(&base.len()) && base.chars().all(|c| c.is_ascii_digit());
    hex32 || numeric
}

/// newest account *.save in the DBE_Production dir
pub fn find_save_file(save_data_path: Option<&Path>) -> Option<PathBuf> {
    let dir = match save_data_path {
        Some(p) => p.to_path_buf(),
        None => detect_save_data_path()?,
    };
    account_save_files(&dir)
        .into_iter()
        .next()
        .map(|(_, path)| path)
}

/// Shared chronological ordering for default selection and account discovery.
fn account_save_files(dir: &Path) -> Vec<(std::time::SystemTime, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut saves: Vec<_> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("save"))
                .unwrap_or(false)
        })
        .filter(|e| {
            e.path()
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(is_account_save_stem)
        })
        .filter_map(|e| {
            let metadata = e.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let mtime = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            Some((mtime, e.path()))
        })
        .collect();
    saves.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    saves
}

/// The newest local save for one Rocket League account.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SaveAccount {
    pub account_id: String,
    pub display_name: String,
    pub path: PathBuf,
}

fn account_display_names(save_dir: &Path) -> std::collections::HashMap<String, String> {
    let mut names = std::collections::HashMap::new();
    let Some(tagame) = save_dir.parent().and_then(Path::parent) else {
        return names;
    };
    let logs = tagame.join("Logs");
    let Ok(entries) = std::fs::read_dir(logs) else {
        return names;
    };
    let id_re = regex::Regex::new(r"-epicuserid=([0-9a-fA-F]{32})").expect("valid epic id regex");
    let name_re =
        regex::Regex::new(r#"-epicusername=(?:"([^"]+)"|(\S+))"#).expect("valid epic name regex");
    let mut logs: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
        })
        .collect();
    logs.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    logs.reverse();
    for path in logs {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let Some(id) = id_re
                .captures(line)
                .and_then(|capture| capture.get(1))
                .map(|value| value.as_str().to_ascii_lowercase())
            else {
                continue;
            };
            if names.contains_key(&id) {
                continue;
            }
            if let Some(name) = name_re
                .captures(line)
                .and_then(|capture| capture.get(1).or_else(|| capture.get(2)))
                .map(|value| value.as_str().trim().to_string())
                .filter(|value| !value.is_empty())
            {
                names.insert(id, name);
            }
        }
    }
    names
}

/// Return the newest save per account, ordered from newest to oldest.
/// Names from launch logs are optional labels, never a requirement for discovery.
/// Generic profile names and historical tournament creator names are not used.
pub fn find_save_accounts(save_data_path: Option<&Path>) -> Vec<SaveAccount> {
    let Some(dir) = save_data_path
        .map(PathBuf::from)
        .or_else(detect_save_data_path)
    else {
        return Vec::new();
    };
    let names = account_display_names(&dir);
    let mut seen = std::collections::HashSet::new();
    let mut accounts = Vec::new();
    for (_, path) in account_save_files(&dir) {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let account_id = stem.split('_').next().unwrap_or(stem).to_ascii_lowercase();
        if !seen.insert(account_id.clone()) {
            continue;
        }
        let display_name = names
            .get(&account_id)
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Account ({account_id})"));
        accounts.push(SaveAccount {
            account_id,
            display_name,
            path,
        });
    }
    accounts
}

/// find the right save data path (steam or epic). tries the running game
/// first, else picks whichever path has the newest .save.
pub fn detect_save_data_path() -> Option<PathBuf> {
    let docs = crate::utils::paths::documents_dir()?;
    preferred_save_data_path(
        &docs,
        crate::process::find_rocket_league().map(|rl| rl.save_data_path),
    )
}

fn preferred_save_data_path(docs: &Path, preferred: Option<PathBuf>) -> Option<PathBuf> {
    preferred
        .filter(|path| find_save_file(Some(path)).is_some())
        .or_else(|| newest_save_data_path(docs))
}

fn newest_save_data_path(docs: &Path) -> Option<PathBuf> {
    let mut best_path: Option<PathBuf> = None;
    let mut best_mtime: Option<std::time::SystemTime> = None;

    for rel in [SAVE_PATH_STEAM, SAVE_PATH_EPIC] {
        let candidate = docs.join(rel);
        if !candidate.is_dir() {
            continue;
        }
        if let Some(latest) = find_save_file(Some(&candidate)) {
            if let Ok(mtime) = latest.metadata().and_then(|m| m.modified()) {
                if best_mtime.map(|b| mtime > b).unwrap_or(true) {
                    best_mtime = Some(mtime);
                    best_path = Some(candidate);
                }
            }
        }
    }
    best_path
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn latest_save_fallback_uses_rotations_and_recency_instead_of_account_names() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("hebnix-latest-save-{}-{nonce}", std::process::id()));
        let directory = root.join(SAVE_PATH_EPIC);
        std::fs::create_dir_all(&directory).unwrap();
        let account_id = "f514e015f0504d8e9031c262f3d089ac";
        let older_account = "0123456789abcdef0123456789abcdef";
        let latest = directory.join(format!("{account_id}_2.save"));
        for (name, seconds) in [
            (format!("{account_id}.save"), 1),
            (format!("{account_id}_1.save"), 2),
            (format!("{older_account}.save"), 3),
            (format!("{account_id}_2.save"), 4),
        ] {
            let file = std::fs::File::create(directory.join(name)).unwrap();
            file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        }
        let accounts = find_save_accounts(Some(&directory));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].path, latest);
        assert_eq!(accounts[0].display_name, format!("Account ({account_id})"));
        assert_eq!(
            find_save_file(Some(&directory)),
            Some(accounts[0].path.clone())
        );

        let logs = root.join("My Games/Rocket League/TAGame/Logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("Launch.log"), format!(
            "-epicuserid={older_account} -epicusername=Aardvark\n-epicuserid={account_id} -epicusername=Its_Official_Jay"
        )).unwrap();
        let named = find_save_accounts(Some(&directory));
        assert_eq!(named[0].path, latest);
        assert_eq!(named[0].display_name, "Its_Official_Jay");
        assert_eq!(named[1].display_name, "Aardvark");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fallback_ignores_non_account_files_and_save_directories() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hebnix-save-candidates-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let id = "f514e015f0504d8e9031c262f3d089ac";
        let expected = root.join(format!("{id}_2.save"));
        std::fs::write(&expected, b"save").unwrap();
        for name in [
            format!("{id}_backup.save"),
            format!("{id}_.save"),
            format!("{id}_2.save.bak"),
            "Settings.save".into(),
        ] {
            std::fs::write(root.join(name), b"not an account save").unwrap();
        }
        std::fs::create_dir(root.join(format!("{id}_3.save"))).unwrap();
        assert_eq!(find_save_file(Some(&root)), Some(expected.clone()));
        let accounts = find_save_accounts(Some(&root));
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].path, expected);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rotating_saves_are_discoverable_without_launch_logs_or_a_base_save() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hebnix-unnamed-save-{}-{nonce}",
            std::process::id()
        ));
        let directory = root.join(SAVE_PATH_EPIC);
        std::fs::create_dir_all(&directory).unwrap();
        let account_id = "f514e015f0504d8e9031c262f3d089ac";
        let first = directory.join(format!("{account_id}_1.save"));
        let second = directory.join(format!("{account_id}_2.save"));
        // Discovery uses filenames and modification times, not file contents.
        for (path, seconds) in [(&first, 1), (&second, 2)] {
            let file = std::fs::File::create(path).unwrap();
            file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        }
        assert_eq!(find_save_file(Some(&directory)), Some(second.clone()));
        let accounts = find_save_accounts(Some(&directory));
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].account_id, account_id);
        assert_eq!(accounts[0].path, second);
        assert_eq!(accounts[0].display_name, format!("Account ({account_id})"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_platform_save_falls_back_to_existing_account_directory() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hebnix-save-fallback-{}-{nonce}",
            std::process::id()
        ));
        let steam = root.join(SAVE_PATH_STEAM);
        let epic = root.join(SAVE_PATH_EPIC);
        std::fs::create_dir_all(&steam).unwrap();
        std::fs::write(steam.join("0123456789abcdef0123456789abcdef.save"), b"save").unwrap();
        assert_eq!(
            preferred_save_data_path(&root, Some(epic.clone())),
            Some(steam.clone())
        );
        std::fs::create_dir_all(&epic).unwrap();
        // An existing but empty platform folder must also fall back.
        assert_eq!(
            preferred_save_data_path(&root, Some(epic.clone())),
            Some(steam)
        );
        std::fs::write(epic.join("0123456789abcdef0123456789abcdef.save"), b"save").unwrap();
        assert_eq!(
            preferred_save_data_path(&root, Some(epic.clone())),
            Some(epic)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn account_names_match_hex_ids_case_insensitively() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hebnix-save-account-{}-{nonce}",
            std::process::id()
        ));
        let save_dir = root.join(SAVE_PATH_EPIC);
        let logs = root.join("My Games/Rocket League/TAGame/Logs");
        std::fs::create_dir_all(&save_dir).unwrap();
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("Launch.log"),
            "-epicuserid=0123456789ABCDEF0123456789ABCDEF -epicusername=TestPlayer",
        )
        .unwrap();
        let path = save_dir.join("0123456789abcdef0123456789abcdef.save");
        std::fs::write(&path, b"save").unwrap();
        let accounts = find_save_accounts(Some(&save_dir));
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].display_name, "TestPlayer");
        assert_eq!(accounts[0].path, path);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn redirected_documents_support_save_discovery_reading_and_writing() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("hebnix-save-path-{}-{nonce}", std::process::id()));
        let docs = root.join("OneDrive/Documents");
        let logs = docs.join("My Games/Rocket League/TAGame/Logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("Launch.log"),
            "-epicuserid=0123456789abcdef0123456789abcdef -epicusername=TestPlayer",
        )
        .unwrap();
        let raw = RawSave {
            source_file: String::new(),
            encrypted_size: 0,
            crc_expected: 0,
            crc_calculated: 0,
            crc_match: true,
            foosball: 0xF005_BA11,
            magic: 0x7FFF_FFFF,
            engine_version: 1,
            licensee_version: 1,
            type_version: 1,
            object_types: Vec::new(),
            properties: serde_json::json!({}),
            objects: Vec::new(),
        };
        for relative in [SAVE_PATH_STEAM, SAVE_PATH_EPIC] {
            let directory = docs.join(relative);
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("0123456789abcdef0123456789abcdef.save");
            assemble_savedata(&raw, &path).unwrap();
            let detected = newest_save_data_path(&docs).unwrap();
            assert!(detected.starts_with(&docs));
            assert_eq!(find_save_file(Some(&directory)), Some(path.clone()));
            assert_eq!(find_save_accounts(Some(&directory))[0].path, path);
            // Missing log names must not hide otherwise discoverable saves.
            let unnamed = directory.join("fedcba9876543210fedcba9876543210.save");
            assemble_savedata(&raw, &unnamed).unwrap();
            let accounts = find_save_accounts(Some(&directory));
            assert_eq!(accounts.len(), 2);
            assert!(
                accounts
                    .iter()
                    .any(|account| account.display_name == "TestPlayer")
            );
            assert!(accounts.iter().any(|account| account.path == unnamed
                && account.display_name == "Account (fedcba9876543210fedcba9876543210)"));
            let mut parsed = parse_savedata(&path, true).unwrap();
            parsed.engine_version = 2;
            assemble_savedata(&parsed, &path).unwrap();
            assert_eq!(load(&path, true).unwrap().header.engine_version, 2);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
