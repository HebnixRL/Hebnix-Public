//! Where Rocket League's "restart" commands go, based on the persisted
//! RlLaunchCfg (see config.rs). Rocket League has no official Linux/Steam
//! listing anymore, so besides the real Steam game there is the generic
//! setup:
//!
//! - SteamNative: a real, owned Steam catalog listing. `steam://run`
//!   supports overriding the launch options with an extra argument
//!   directly.
//! - SteamShortcut: a Steam non-Steam-shortcut to any launcher (Slipstream,
//!   Heroic, a script). `steam://rungameid` starts it, but Steam won't add
//!   arguments to a non-Steam shortcut.

use std::path::PathBuf;

use crate::config::{RlLaunchCfg, RlLaunchMode};

/// a Steam non-Steam-shortcut found in shortcuts.vdf - offered to the user
/// during setup so they don't have to dig the numeric ID out of a generated
/// .desktop file by hand.
pub struct ShortcutCandidate {
    pub app_name: String,
    pub exe: String,
    pub rungameid: u64,
}

/// Steam's shortcut ID algorithm (reverse-engineered, used by every
/// third-party Steam shortcut tool): crc32(exe + appname), top bit forced
/// set, packed into the upper 32 bits of a 64-bit ID with a fixed
/// 0x02000000 suffix. `exe` must be exactly as stored in shortcuts.vdf,
/// quotes included. Verified live against a real shortcut's actual
/// steam://rungameid/<id> from its generated .desktop file.
pub fn compute_shortcut_id(exe: &str, app_name: &str) -> u64 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(exe.as_bytes());
    hasher.update(app_name.as_bytes());
    let top = (hasher.finalize() as u64) | 0x8000_0000;
    (top << 32) | 0x0200_0000
}

fn shortcuts_vdf_path() -> Option<PathBuf> {
    let steam_userdata = dirs::home_dir()?.join(".local/share/Steam/userdata");
    let entries = std::fs::read_dir(&steam_userdata).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("config/shortcuts.vdf");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// shortcuts.vdf is a simple untyped binary keyed map: each entry is a type
// byte (0x00 nested map, 0x01 string, 0x02 int32, 0x08 map-end) followed by
// a null-terminated key, then the value. No official spec, but this format
// is stable and widely relied on by other Steam shortcut tools.
fn parse_map(data: &[u8], mut i: usize) -> Option<(Vec<(String, VdfValue)>, usize)> {
    let mut entries = Vec::new();
    loop {
        let tag = *data.get(i)?;
        i += 1;
        if tag == 0x08 {
            return Some((entries, i));
        }
        let key_end = i + data[i..].iter().position(|b| *b == 0)?;
        let key = String::from_utf8_lossy(&data[i..key_end]).into_owned();
        i = key_end + 1;
        let value = match tag {
            0x00 => {
                let (nested, next) = parse_map(data, i)?;
                i = next;
                VdfValue::Map(nested)
            }
            0x01 => {
                let end = i + data[i..].iter().position(|b| *b == 0)?;
                let s = String::from_utf8_lossy(&data[i..end]).into_owned();
                i = end + 1;
                VdfValue::Str(s)
            }
            0x02 => {
                let bytes: [u8; 4] = data.get(i..i + 4)?.try_into().ok()?;
                i += 4;
                VdfValue::Int(i32::from_le_bytes(bytes))
            }
            _ => return None,
        };
        entries.push((key, value));
    }
}

enum VdfValue {
    Map(Vec<(String, VdfValue)>),
    Str(String),
    Int(i32),
}


/// every non-Steam shortcut, for the setup wizard to offer.
pub fn find_steam_shortcuts() -> Vec<ShortcutCandidate> {
    let Some(path) = shortcuts_vdf_path() else {
        return Vec::new();
    };
    let Ok(data) = std::fs::read(&path) else {
        return Vec::new();
    };
    parse_shortcuts(&data)
}

fn parse_shortcuts(data: &[u8]) -> Vec<ShortcutCandidate> {
    let Some((root, _)) = parse_map(data, 0) else {
        return Vec::new();
    };
    let Some((_, VdfValue::Map(shortcuts))) = root.into_iter().find(|(k, _)| k == "shortcuts")
    else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for (_, entry) in shortcuts {
        let VdfValue::Map(fields) = entry else {
            continue;
        };
        let mut app_name = None;
        let mut exe = None;
        let mut appid = None;
        for (key, value) in &fields {
            match (key.to_ascii_lowercase().as_str(), value) {
                ("appname", VdfValue::Str(s)) => app_name = Some(s.clone()),
                ("exe", VdfValue::Str(s)) => exe = Some(s.clone()),
                ("appid", VdfValue::Int(id)) => appid = Some(*id),
                _ => {}
            }
        }
        let (Some(app_name), Some(exe)) = (app_name, exe) else {
            continue;
        };
        // Current Steam stores the shortcut's own ID; only shortcuts made by
        // old Steam versions lack it and use the CRC scheme.
        let rungameid = match appid {
            Some(id) => ((id as u32 as u64) << 32) | 0x0200_0000,
            None => compute_shortcut_id(&exe, &app_name),
        };
        candidates.push(ShortcutCandidate {
            app_name,
            exe,
            rungameid,
        });
    }
    candidates
}

fn launch_uri(uri: &str) -> Result<(), String> {
    tracing::info!("rl_launch: opening {uri}");
    if uri.starts_with("steam://") {
        match std::process::Command::new("steam").arg(uri).spawn() {
            Ok(_) => return Ok(()),
            Err(error) => tracing::warn!(
                "rl_launch: 'steam' binary spawn failed ({error}), falling back to xdg-open"
            ),
        }
    }
    std::process::Command::new("xdg-open")
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            let message = format!("could not open '{uri}' via steam or xdg-open: {error}");
            tracing::warn!("rl_launch: {message}");
            message
        })
}

/// where RL is actually installed, straight from the launch config -
/// independent of the live process. Live detection
/// (`process::find_rocket_league()`) needs to read the running exe's path,
/// which EAC blocks almost immediately after launch, so this resolves the
/// install directory from data that doesn't need the live process at all:
/// Steam's appmanifest for SteamNative, the shortcut launcher's own files
/// for SteamShortcut.
pub fn resolve_install_root(cfg: &RlLaunchCfg) -> Option<PathBuf> {
    match cfg.mode {
        RlLaunchMode::Unconfigured | RlLaunchMode::EpicDirect => None,
        RlLaunchMode::SteamShortcut => shortcut_install_root(cfg),
        RlLaunchMode::SteamNative => {
            let id = std::env::var("HEBNIX_RL_APPID").unwrap_or_else(|_| cfg.steam_id.clone());
            for steamapps in hebnix_sdk::process::steam_library_steamapps_dirs() {
                let manifest = steamapps.join(format!("appmanifest_{id}.acf"));
                let Ok(text) = std::fs::read_to_string(&manifest) else {
                    continue;
                };
                // simple keyed text VDF: `"installdir"\t\t"Name"` on its own line
                if let Some(installdir) = text.lines().find_map(|line| {
                    let line = line.trim();
                    let rest = line.strip_prefix("\"installdir\"")?.trim();
                    let inner = rest.strip_prefix('"')?.strip_suffix('"')?;
                    Some(inner.to_string())
                }) {
                    return Some(steamapps.join("common").join(installdir));
                }
            }
            None
        }
    }
}

/// plain restart, no Workshop LAN address. `HEBNIX_RL_APPID` overrides
/// `cfg.steam_id` without needing to re-run the setup wizard.
pub fn restart(cfg: &RlLaunchCfg) -> Result<(), String> {
    tracing::info!("rl_launch: restart() mode={:?}", cfg.mode);
    match cfg.mode {
        RlLaunchMode::Unconfigured | RlLaunchMode::EpicDirect => Err(
            "Rocket League launch isn't set up yet - open Settings > Rocket League Launch Setup"
                .to_string(),
        ),
        RlLaunchMode::SteamNative | RlLaunchMode::SteamShortcut => {
            let id = std::env::var("HEBNIX_RL_APPID").unwrap_or_else(|_| cfg.steam_id.clone());
            launch_uri(&format!("steam://rungameid/{id}"))
        }
    }
}

/// Workshop multiplayer without `-multihome`: a plain restart.
pub fn restart_for_multiplayer_without_multihome(cfg: &RlLaunchCfg) -> Result<(), String> {
    tracing::info!("rl_launch: restart without -multihome, mode={:?}", cfg.mode);
    restart(cfg)
}

/// Workshop LAN "restart with -multihome=<address>".
/// `HEBNIX_RL_MULTIHOME_COMMAND_TEMPLATE` (with `{multihome}` /
/// `{multihome_encoded}` placeholders, shell-word-split) overrides
/// everything below without needing to re-run the setup wizard.
pub fn restart_multihome(cfg: &RlLaunchCfg, address: &str) -> Result<(), String> {
    tracing::info!(
        "rl_launch: restart_multihome() mode={:?} address={address}",
        cfg.mode
    );
    let raw = format!("-multihome={address}");
    let encoded = format!("-multihome%3D{address}");

    if let Ok(template) = std::env::var("HEBNIX_RL_MULTIHOME_COMMAND_TEMPLATE") {
        let rendered = template
            .replace("{multihome_encoded}", &encoded)
            .replace("{multihome}", &raw);
        let parts = shell_words::split(&rendered)
            .map_err(|error| format!("invalid HEBNIX_RL_MULTIHOME_COMMAND_TEMPLATE: {error}"))?;
        let (program, args) = parts
            .split_first()
            .ok_or_else(|| "HEBNIX_RL_MULTIHOME_COMMAND_TEMPLATE is empty".to_string())?;
        return std::process::Command::new(program)
            .args(args)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string());
    }

    match cfg.mode {
        RlLaunchMode::Unconfigured | RlLaunchMode::EpicDirect => Err(
            "Rocket League launch isn't set up yet - open Settings > Rocket League Launch Setup"
                .to_string(),
        ),
        RlLaunchMode::SteamNative => {
            let id = std::env::var("HEBNIX_RL_APPID").unwrap_or_else(|_| cfg.steam_id.clone());
            launch_uri(&format!("steam://run/{id}//{raw}/"))
        }
        RlLaunchMode::SteamShortcut => Err(
            "A non-Steam shortcut can't pass -multihome: Steam won't add arguments to it"
                .to_string(),
        ),
    }
}

/// Rocket League's folder for a non-Steam shortcut. Slipstream keeps a
/// config.json next to its executable naming the game exe
/// (`rocket_league_path`); under Proton that is a Windows path on drive Z:,
/// the root of the Linux file system. Otherwise the game was most likely
/// installed with legendary, so its installed.json is asked (the standalone
/// one, then the copy Heroic keeps).
fn shortcut_install_root(cfg: &RlLaunchCfg) -> Option<PathBuf> {
    let exe = cfg.shortcut_exe.trim().trim_matches('"');
    let from_config = (!exe.is_empty())
        .then(|| std::path::Path::new(exe).parent().map(|dir| dir.join("config.json")))
        .flatten()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json.get("rocket_league_path")?.as_str().and_then(game_root_from_exe));
    if from_config.is_some() {
        return from_config;
    }
    let home = dirs::home_dir()?;
    [
        home.join(".config/legendary/installed.json"),
        home.join(".config/heroic/legendaryConfig/legendary/installed.json"),
    ]
    .into_iter()
    .find_map(|path| {
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        // Sugar is Rocket League's Epic catalog name
        let install_path = json.get("Sugar")?.get("install_path")?.as_str()?;
        Some(PathBuf::from(install_path))
    })
}

/// `Z:\games\rocketleague\Binaries\Win64\RocketLeague.exe` (or the same as a
/// Linux path) to `/games/rocketleague`.
fn game_root_from_exe(exe: &str) -> Option<PathBuf> {
    let unix = match exe.get(..2) {
        Some(drive) if drive.eq_ignore_ascii_case("z:") => exe[2..].replace('\\', "/"),
        _ if exe.starts_with('/') => exe.to_string(),
        _ => return None,
    };
    let path = PathBuf::from(unix);
    // .../Binaries/Win64/RocketLeague.exe
    let binaries = path.parent()?.parent()?;
    binaries
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Binaries"))
        .then(|| binaries.parent().map(PathBuf::from))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// shortcuts.vdf as Steam writes it: one shortcut with its own appid
    /// (-833557446, i.e. 3461409850) and one old one without.
    #[test]
    fn shortcut_ids_use_the_stored_appid() {
        fn string(key: &str, value: &str) -> Vec<u8> {
            [&[0x01][..], key.as_bytes(), &[0], value.as_bytes(), &[0]].concat()
        }
        let mut data = vec![0x00];
        data.extend(b"shortcuts\0");
        data.extend([0x00, b'0', 0]);
        data.extend([&[0x02][..], b"appid\0", &(-833557446i32).to_le_bytes()].concat());
        data.extend(string("AppName", "Rocket League"));
        data.extend(string("Exe", "\"/games/Slipstream.exe\""));
        data.push(0x08);
        data.extend([0x00, b'1', 0]);
        data.extend(string("AppName", "Old"));
        data.extend(string("Exe", "\"/games/old\""));
        data.push(0x08);
        data.extend([0x08, 0x08]);
        let found = parse_shortcuts(&data);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].rungameid, 14866642103835820032);
        assert_eq!(found[1].rungameid, compute_shortcut_id("\"/games/old\"", "Old"));
    }

    #[test]
    fn slipstream_game_root() {
        assert_eq!(
            game_root_from_exe(r"Z:\mnt\game\rocketleague\Binaries\Win64\RocketLeague.exe"),
            Some(PathBuf::from("/mnt/game/rocketleague"))
        );
        assert_eq!(
            game_root_from_exe("/mnt/game/rocketleague/Binaries/Win64/RocketLeague.exe"),
            Some(PathBuf::from("/mnt/game/rocketleague"))
        );
        assert_eq!(game_root_from_exe(r"C:\Games\rocketleague\Binaries\Win64\RocketLeague.exe"), None);
        assert_eq!(game_root_from_exe("/mnt/game/RocketLeague.exe"), None);
    }
}
