//! Non-Steam shortcut support for RlLaunchMode::SteamShortcut (see
//! config.rs): finds the shortcuts in Steam's shortcuts.vdf so the setup
//! wizard can offer them. Steam and Epic dispatch stay in sys/windows.rs.

use std::path::PathBuf;


/// a Steam non-Steam-shortcut found in shortcuts.vdf - offered in the setup
/// wizard so the user doesn't have to dig the numeric ID out by hand.
pub struct ShortcutCandidate {
    pub app_name: String,
    pub exe: String,
    pub rungameid: u64,
}

/// Steam's shortcut ID algorithm (reverse-engineered, used by every
/// third-party Steam shortcut tool): crc32(exe + appname), top bit forced
/// set, packed into the upper 32 bits of a 64-bit ID with a fixed
/// 0x02000000 suffix.
pub fn compute_shortcut_id(exe: &str, app_name: &str) -> u64 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(exe.as_bytes());
    hasher.update(app_name.as_bytes());
    let top = (hasher.finalize() as u64) | 0x8000_0000;
    (top << 32) | 0x0200_0000
}

/// Steam's own install directory, from the same registry key Steam itself
/// writes on install (HKCU\Software\Valve\Steam, "SteamPath").
fn steam_install_dir() -> Option<PathBuf> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

fn shortcuts_vdf_path() -> Option<PathBuf> {
    let userdata = steam_install_dir()?.join("userdata");
    let entries = std::fs::read_dir(&userdata).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("config").join("shortcuts.vdf");
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
