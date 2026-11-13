//! Read-only discovery of accounts whose local saves can actually be parsed.
use super::service;
use hebnix_sdk::save_file;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Account {
    pub key: String,
    pub platform: String,
    pub id: String,
    pub name: String,
    pub save: Option<PathBuf>,
    stamp: Option<(u64, SystemTime)>,
}
impl Account {
    pub fn from_key(key: &str) -> Option<Self> {
        let (platform, id) = key.split_once(':')?;
        let valid = match platform {
            "epic" => service::valid_epic(id),
            "steam" => (10..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()),
            _ => false,
        };
        if !valid {
            return None;
        }
        let id = id.to_ascii_lowercase();
        Some(Self {
            key: format!("{platform}:{id}"),
            platform: platform.into(),
            id,
            name: "Current account".into(),
            save: None,
            stamp: None,
        })
    }
    pub fn from_primary(primary: &str) -> Option<Self> {
        let parts: Vec<_> = primary.split('|').collect();
        if parts.len() != 3 || parts[2].parse::<u8>().is_err() {
            return None;
        }
        let platform = match parts[0].to_ascii_lowercase().as_str() {
            "epic" | "epicgames" => "epic",
            "steam" => "steam",
            _ => return None,
        };
        Self::from_key(&format!("{platform}:{}", parts[1]))
    }
    pub fn picture_file(&self, root: &Path) -> PathBuf {
        root.join(format!("{}-{}.jpg", self.platform, self.id))
    }
}

fn save_identity(path: &Path) -> Option<Account> {
    let stem = path.file_stem()?.to_str()?.split('_').next()?;
    let platform = if service::valid_epic(stem) {
        "epic"
    } else {
        "steam"
    };
    Account::from_key(&format!("{platform}:{stem}"))
}
pub(crate) fn latest_saved() -> Option<Account> {
    save_file::find_save_accounts(None)
        .into_iter()
        .find_map(|entry| {
            let mut account = save_identity(&entry.path)?;
            if !entry.display_name.starts_with("Account (") {
                account.name = entry.display_name;
            }
            Some(account)
        })
}

fn profile_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty()
        || ["default", "player", "profile", "defaultprofile", "player 1"]
            .contains(&name.to_ascii_lowercase().as_str())
    {
        None
    } else {
        Some(name.into())
    }
}
pub(super) fn discover(previous: &[Account]) -> Vec<Account> {
    let mut directories = Vec::new();
    if let Some(docs) = hebnix_sdk::utils::paths::documents_dir() {
        for relative in [
            hebnix_sdk::utils::constants::SAVE_PATH_EPIC,
            hebnix_sdk::utils::constants::SAVE_PATH_STEAM,
        ] {
            directories.push(docs.join(relative));
        }
    }
    if let Some(path) = save_file::detect_save_data_path() {
        if !directories.contains(&path) {
            directories.push(path);
        }
    }
    discover_in(&directories, previous)
}
fn discover_in(directories: &[PathBuf], previous: &[Account]) -> Vec<Account> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for directory in directories {
        for entry in save_file::find_save_accounts(Some(directory)) {
            let Some(mut account) = save_identity(&entry.path) else {
                continue;
            };
            if seen.contains(&account.key) {
                continue;
            }
            let stamp = std::fs::metadata(&entry.path)
                .ok()
                .and_then(|m| Some((m.len(), m.modified().ok()?)));
            if let Some(cached) = previous.iter().find(|a| {
                a.key == account.key
                    && a.save.as_ref() == Some(&entry.path)
                    && a.stamp == stamp
                    && stamp.is_some()
            }) {
                let mut cached = cached.clone();
                if !entry.display_name.starts_with("Account (") {
                    cached.name = entry.display_name;
                }
                result.push(cached);
                seen.insert(account.key);
                continue;
            }
            let Ok(save) = save_file::load(&entry.path, true) else {
                continue;
            };
            let saved_name = save
                .profile()
                .and_then(|profile| profile_name(&profile.profile_name));
            account.name = if entry.display_name.starts_with("Account (") {
                saved_name.unwrap_or(entry.display_name)
            } else {
                entry.display_name
            };
            account.save = Some(entry.path);
            account.stamp = stamp;
            seen.insert(account.key.clone());
            result.push(account);
        }
    }
    result
}
pub(crate) fn current() -> Option<Account> {
    if !hebnix_sdk::process::is_rocket_league_running() {
        return None;
    }
    let session = hebnix_sdk::log::local_player_session()?;
    let primary = session.primary_id?;
    let mut account = Account::from_primary(&primary)?;
    if let Some(name) = session.username.and_then(|n| profile_name(&n)) {
        account.name = name;
    }
    Some(account)
}
pub(super) fn choose(
    saved: &[Account],
    selected: Option<&str>,
    running: bool,
    live: Option<Account>,
) -> Option<Account> {
    if running {
        return live.map(|mut live| {
            if let Some(saved) = saved.iter().find(|a| a.key == live.key) {
                if live.name == "Current account" {
                    live.name = saved.name.clone();
                }
                live.save = saved.save.clone();
            }
            live
        });
    }
    saved
        .iter()
        .find(|a| Some(a.key.as_str()) == selected)
        .or_else(|| saved.first())
        .cloned()
}
pub(super) fn authorize(target: &Account) -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        if current()
            .as_ref()
            .is_some_and(|live| live.key == target.key)
        {
            return Ok(());
        }
        return Err("The logged-in account changed. Please try again.".into());
    }
    let path = target
        .save
        .as_deref()
        .ok_or("Open Rocket League to upload")?;
    if save_identity(path)
        .as_ref()
        .is_none_or(|owner| owner.key != target.key)
    {
        return Err("The selected save account changed.".into());
    }
    save_file::load(path, true).map_err(|_| {
        "This save is no longer readable. Open Rocket League to upload.".to_string()
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(id: char) -> Account {
        Account::from_key(&format!("epic:{}", id.to_string().repeat(32))).unwrap()
    }
    #[test]
    fn running_account_always_wins_and_unknown_login_does_not_use_old_selection() {
        let saved = vec![account('a'), account('b')];
        assert_eq!(
            choose(&saved, Some(&saved[0].key), true, Some(saved[1].clone()))
                .unwrap()
                .key,
            saved[1].key
        );
        assert!(choose(&saved, Some(&saved[0].key), true, None).is_none());
        assert_eq!(
            choose(&saved, Some(&saved[1].key), false, None)
                .unwrap()
                .key,
            saved[1].key
        );
    }
    #[test]
    fn account_keys_are_platform_specific_and_cannot_escape_storage() {
        assert!(Account::from_primary("Steam|76561198000000000|0").is_some());
        for bad in ["epic:../secret", "steam:../secret", "psn:12345", "epic:abc"] {
            assert!(Account::from_key(bad).is_none());
        }
        assert!(Account::from_primary("Epic|aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|0|extra").is_none());
    }
    #[test]
    fn readable_saves_supply_named_offline_accounts() {
        let root =
            std::env::temp_dir().join(format!("hebnix-readable-account-{}", rand::random::<u64>()));
        let saves = root.join("TAGame/SaveDataEpic/DBE_Production");
        let logs = root.join("TAGame/Logs");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("Launch.log"),
            "-epicuserid=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa -epicusername=\"Saved Player\"",
        )
        .unwrap();
        let raw = save_file::RawSave {
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
        save_file::assemble_savedata(&raw, &saves.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.save"))
            .unwrap();
        let found = discover_in(&[saves], &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Saved Player");
        assert!(found[0].save.is_some());
        assert_eq!(
            choose(&found, None, false, None).unwrap().key,
            "epic:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn filenames_alone_do_not_enable_offline_upload() {
        let root =
            std::env::temp_dir().join(format!("hebnix-unreadable-save-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.save");
        std::fs::write(path, b"not a save").unwrap();
        assert!(discover_in(&[root.clone()], &[]).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
