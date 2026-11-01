//! opt-in LAN title and package preload experiment, independent of map swapping
use super::upk_package::UpkPackage;
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
};

const TITLE: &str = "GFxData_LanServer_TA.GetLocalizedMapName";
const JOIN: &str = "GFxData_LanServer_TA.GetJoinOptions";
const RECEIPT: &str = "WorkshopLanTrial.json";
const BRIDGE: &str = "HbxLanTrial";
// object name forms tried in order: absolute, game-relative, bare stem, absolute with extension
const CANDIDATES: [&str; 1] = [".object"];

#[derive(Clone, Default)]
struct Code {
    bytes: Vec<u8>,
    refs: usize,
}
impl Code {
    fn raw(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.into(),
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
        self.bytes.len() + self.refs * 4
    }
}
fn object(p: &UpkPackage, path: &str) -> Result<i32, String> {
    p.find_object(path)
        .ok_or_else(|| format!("missing object: {path}"))
}
fn variable(p: &UpkPackage, token: u8, path: &str) -> Result<Code, String> {
    Ok(Code::raw(&[token]).reference(object(p, path)?))
}
fn text(value: &str) -> Code {
    Code::raw(&[0x1f])
        .then(Code::raw(value.as_bytes()))
        .then(Code::raw(&[0]))
}
fn concat(a: Code, b: Code) -> Code {
    Code::raw(&[0xf8]).then(a).then(b).then(Code::raw(&[0x16]))
}
fn bridge_file(p: &UpkPackage, extension: &str) -> Result<Code, String> {
    // sidecar peers appear to the game as 127.10.0.n:7777
    let key = Code::raw(&[0xc9])
        .then(variable(p, 0x01, "GFxData_LanServer_TA.ServerId")?)
        .then(text(":"))
        .then(text("_"))
        .then(Code::raw(&[0x48, 0x16]));
    Ok(concat(
        concat(text(r"..\..\TAGame\HbxLanTrial\"), key),
        text(extension),
    ))
}
fn read_file(p: &UpkPackage, extension: &str, destination: Code) -> Result<Code, String> {
    let index =
        p.names
            .iter()
            .position(|n| n == "LoadFileToString")
            .ok_or("enable the in-game Workshop menu before staging this trial")? as i32;
    let call = Code::raw(&[0x1b])
        .then(Code::raw(&index.to_le_bytes()))
        .then(Code::raw(&[0; 4]))
        .then(bridge_file(p, extension)?)
        .then(destination)
        .then(Code::raw(&[0x16]));
    let skip = u16::try_from(call.memory()).map_err(|e| e.to_string())?;
    Ok(Code::raw(&[0x5e, 0x12, 0, 0x20])
        .reference(object(p, "Core.FileSystem")?)
        .then(Code::raw(&skip.to_le_bytes()))
        .reference(0)
        .then(Code::raw(&[0]))
        .then(call))
}
fn guarded(condition: Code, body: Code, start: usize) -> Result<Code, String> {
    let end =
        u16::try_from(start + 3 + condition.memory() + body.memory()).map_err(|e| e.to_string())?;
    Ok(Code::raw(&[0x07])
        .then(Code::raw(&end.to_le_bytes()))
        .then(condition)
        .then(body))
}
fn prefix(p: &UpkPackage, path: &str) -> Result<Code, String> {
    let destination = variable(p, 0x2b, &format!("{path}.ReturnValue"))?;
    if path == TITLE {
        return guarded(
            read_file(p, ".title", destination.clone())?,
            Code::raw(&[0x04]).then(destination),
            0,
        );
    }
    // each candidate name form is tried and logged, so the log shows which one
    // the engine resolves; loading TheWorld asks it to find the custom package
    let request = Code::raw(&[0xe7])
        .then(concat(
            text("HBX LAN join preparation: "),
            variable(p, 0x01, "GFxData_LanServer_TA.ServerId")?,
        ))
        .then(Code::raw(&[0x48, 0x48, 0x16]));
    let mut offset = request.memory();
    let mut code = request;
    for (n, extension) in CANDIDATES.iter().enumerate() {
        let load = Code::raw(&[0x1c])
            .reference(object(p, "Core.Object.DynamicLoadObject")?)
            .then(destination.clone())
            .then(Code::raw(&[0x20]).reference(object(p, "Core.Object")?))
            .then(Code::raw(&[0x27, 0x16]));
        let message = concat(
            concat(text(&format!("HBX LAN preload {}: ", n + 1)), destination.clone()),
            concat(text(" = "), Code::raw(&[0x38, 0x56]).then(load)),
        );
        let log = Code::raw(&[0xe7])
            .then(message)
            .then(Code::raw(&[0x48, 0x48, 0x16]));
        let guard = guarded(read_file(p, extension, destination.clone())?, log, offset)?;
        offset += guard.memory();
        code = code.then(guard);
    }
    Ok(code)
}
fn payload(p: &UpkPackage, path: &str) -> Result<Vec<u8>, String> {
    let index = object(p, path)? as usize - 1;
    let e = &p.exports[index];
    Ok(p.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec())
}
fn patched(p: &UpkPackage, path: &str, original: &[u8]) -> Result<Vec<u8>, String> {
    let expected = if path == TITLE {
        (99i32, 63i32)
    } else {
        (255, 183)
    };
    if original.len() < 48
        || original[40..44] != expected.0.to_le_bytes()
        || original[44..48] != expected.1.to_le_bytes()
    {
        return Err(format!("{path} changed layout; refusing to patch"));
    }
    // both originals contain straight-line return expressions, with no jumps
    let code = prefix(p, path)?;
    let mut result = original[..48].to_vec();
    result[40..44].copy_from_slice(&(expected.0 + code.memory() as i32).to_le_bytes());
    result[44..48].copy_from_slice(&(expected.1 + code.bytes.len() as i32).to_le_bytes());
    result.extend(code.bytes);
    result.extend(&original[48..]);
    Ok(result)
}

/// patch only the two LAN functions, preserving all other uncommitted features
pub fn set_mode(cooked: &Path, mode: &str) -> Result<(), String> {
    if !["off", "labels", "join"].contains(&mode) {
        return Err("mode must be off, labels or join".into());
    }
    let live = cooked.join("TAGame.upk");
    let receipt_path = cooked.join("Backups").join(RECEIPT);
    let mut p = UpkPackage::load(&live)?;
    let mut receipt = if receipt_path.exists() {
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(&receipt_path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
    } else {
        if mode == "off" {
            return Ok(());
        }
        let mut exports = serde_json::Map::new();
        for path in [TITLE, JOIN] {
            let original = payload(&p, path)?;
            let changed = patched(&p, path, &original)?;
            exports.insert(
                path.into(),
                serde_json::json!({"original": original, "patched": changed}),
            );
        }
        serde_json::json!({"version": 1, "mode": "off", "exports": exports})
    };
    let mut needs_write = false;
    for path in [TITLE, JOIN] {
        let original: Vec<u8> =
            serde_json::from_value(receipt["exports"][path]["original"].clone())
                .map_err(|e| e.to_string())?;
        let stored: Vec<u8> = serde_json::from_value(receipt["exports"][path]["patched"].clone())
            .map_err(|e| e.to_string())?;
        let current = payload(&p, path)?;
        // rebuild so a newer build replaces the code an older build saved in the receipt
        let changed = patched(&p, path, &original)?;
        if current != original && current != stored && current != changed {
            return Err(format!(
                "{path} was changed by another patch; leaving it alone"
            ));
        }
        if stored != changed {
            receipt["exports"][path]["patched"] = serde_json::json!(changed);
            needs_write = true;
        }
        let enabled = mode != "off" && (path == TITLE || mode == "join");
        let bytes = if enabled { &changed } else { &original };
        if current != *bytes {
            p.replace_export_payload(object(&p, path)? as usize - 1, bytes)?;
            needs_write = true;
        }
    }
    if !needs_write && receipt["mode"] == mode {
        return Ok(());
    }
    // persist the originals before writing the game package
    fs::create_dir_all(receipt_path.parent().unwrap()).map_err(|e| e.to_string())?;
    let old_mode = receipt["mode"].clone();
    fs::write(&receipt_path, receipt.to_string()).map_err(|e| e.to_string())?;
    let temp = cooked.join("TAGame.lan-trial.tmp");
    p.save(&temp)?;
    let check = UpkPackage::load(&temp)?;
    for path in [TITLE, JOIN] {
        if payload(&check, path)? != payload(&p, path)? {
            return Err(format!("saved {path} failed verification"));
        }
    }
    fs::copy(&temp, &live).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(temp);
    receipt["mode"] = mode.into();
    if let Err(error) = fs::write(&receipt_path, receipt.to_string()) {
        return Err(format!(
            "package saved but trial mode could not be recorded (was {old_mode}): {error}"
        ));
    }
    Ok(())
}

pub fn enabled(cooked: &Path) -> bool {
    fs::read(cooked.join("Backups").join(RECEIPT))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|r| r["mode"] == "labels" || r["mode"] == "join")
}

pub fn reapply(cooked: &Path) -> Result<(), String> {
    let mode = fs::read(cooked.join("Backups").join(RECEIPT))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|receipt| receipt["mode"].as_str().map(str::to_owned));
    match mode.as_deref() {
        Some(mode @ ("labels" | "join")) => set_mode(cooked, mode),
        _ => Ok(()),
    }
}

pub struct Offer {
    pub ip: Ipv4Addr,
    pub title: String,
    pub map: Option<PathBuf>,
    /// package name the host loaded the map under, a map slot or empty
    pub package: String,
}

/// find an existing custom map by package name, independent of its catalog id
pub fn find_local_map(roots: &[PathBuf], stem: &str) -> Option<PathBuf> {
    if stem.is_empty() {
        return None;
    }
    let mut pending: Vec<_> = roots.iter().cloned().map(|path| (path, 0usize)).collect();
    let mut visited = HashSet::new();
    let mut found = None;
    let mut entries_seen = 0;
    while let Some((folder, depth)) = pending.pop() {
        let Ok(folder) = fs::canonicalize(folder) else {
            continue;
        };
        if !visited.insert(folder.clone()) {
            continue;
        }
        if visited.len() > 256 {
            return None;
        }
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            entries_seen += 1;
            if entries_seen > 4096 {
                return None;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() && depth < 4 {
                pending.push((path, depth + 1));
            } else if kind.is_file()
                && path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(stem))
                && path.extension().is_some_and(|s| {
                    ["udk", "upk", "umap"]
                        .iter()
                        .any(|ext| s.to_string_lossy().eq_ignore_ascii_case(ext))
                })
            {
                let path = fs::canonicalize(path).ok()?;
                if found.as_ref().is_some_and(|previous| previous != &path) {
                    return None;
                }
                found = Some(path);
            }
        }
    }
    found
}
fn unicode_file(path: &Path, value: &str) -> Result<(), String> {
    let bytes: Vec<u8> = std::iter::once(0xfeffu16)
        .chain(value.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    if fs::read(path).ok().as_deref() == Some(&bytes) {
        return Ok(());
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    // Windows rename cannot replace an existing destination
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, path).map_err(|e| e.to_string())
}

/// put the map in the engine's package cache as <name>.upk (hardlink, else copy);
/// false when a different package already holds the name
fn stage_package(cooked: &Path, map: &Path, name: &str, marker: &Path) -> Result<bool, String> {
    let dest = cooked.join(format!("{name}.upk"));
    if dest.exists() {
        let same = |a: &Path, b: &Path| {
            matches!((fs::metadata(a), fs::metadata(b)), (Ok(a), Ok(b)) if a.len() == b.len())
        };
        // only a package Hebnix placed, or an identical one, may be reused
        if !marker.exists() && !same(&dest, map) {
            return Ok(false);
        }
    } else if fs::hard_link(map, &dest).is_err() {
        fs::copy(map, &dest).map_err(|e| e.to_string())?;
    }
    fs::write(marker, b"").map_err(|e| e.to_string())?;
    Ok(true)
}

/// one line per synced peer title, with whether its map is ready to join
pub fn offer_status(cooked: &Path) -> Vec<String> {
    let Some(folder) = cooked.parent().map(|game| game.join(BRIDGE)) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut lines: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "title"))
        .filter_map(|p| {
            let bytes = fs::read(&p).ok()?;
            let units: Vec<u16> = bytes
                .get(2..)?
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let object = fs::read(p.with_extension("object")).ok().map(|b| {
                let units: Vec<u16> = b.get(2..).unwrap_or_default().chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&units)
            });
            let state = match object.as_deref().and_then(|o| o.strip_suffix(".TheWorld")) {
                Some(name) if super::workshop_slots::is_slot(name) => format!("ready to join ({name})"),
                Some(_) => "ready to join".to_string(),
                None => "not on this PC yet".to_string(),
            };
            Some(format!("{} - {state}", String::from_utf16_lossy(&units)))
        })
        .collect();
    lines.sort();
    lines
}

/// bounded peer metadata crosses into the game through local data files only
pub fn sync_offers(cooked: &Path, offers: &[Offer]) -> Result<(), String> {
    if !enabled(cooked) {
        return Ok(());
    }
    let folder = cooked.parent().ok_or("missing TAGame folder")?.join(BRIDGE);
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let mut keep = HashSet::new();
    // two hosts can use the same slot for different maps, the first one gets it
    let mut claimed: HashMap<String, PathBuf> = HashMap::new();
    for offer in offers.iter().take(32) {
        let key = format!("{}_7777", offer.ip);
        let title = folder.join(format!("{key}.title"));
        unicode_file(
            &title,
            &offer
                .title
                .chars()
                .filter(|c| !c.is_control())
                .take(128)
                .collect::<String>(),
        )?;
        keep.insert(title);
        if let Some(map) = &offer.map {
            let map = fs::canonicalize(map).map_err(|e| e.to_string())?;
            let name = map
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let marker;
            if super::workshop_slots::is_slot(&offer.package) {
                let name = offer.package.clone();
                if claimed.get(&name).is_some_and(|other| *other != map) {
                    continue;
                }
                claimed.insert(name.clone(), map.clone());
                if let Err(error) = super::workshop_slots::place(cooked, &name, &map, &offer.title) {
                    crate::toast::report(&format!("slot-{name}"), &error, None, true);
                    continue;
                }
                let marker = folder.join(format!("{name}.staged"));
                fs::write(&marker, b"").map_err(|e| e.to_string())?;
                keep.insert(marker);
                let path = folder.join(format!("{key}.object"));
                unicode_file(&path, &format!("{name}.TheWorld"))?;
                keep.insert(path);
                continue;
            }
            // the engine finds a package by bare name, and only indexes .upk files
            marker = folder.join(format!("{name}.staged"));
            if !stage_package(cooked, &map, &name, &marker)? {
                continue;
            }
            keep.insert(marker);
            let path = folder.join(format!("{key}.object"));
            unicode_file(&path, &format!("{name}.TheWorld"))?;
            keep.insert(path);
        }
    }
    for entry in fs::read_dir(&folder).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("title" | "object" | "staged")
        ) && !keep.contains(&path)
        {
            if path.extension().is_some_and(|e| e == "staged") {
                if let Some(stem) = path.file_stem() {
                    // a slot goes back to its placeholder, anything else was only ours to stage
                    if super::workshop_slots::is_slot(&stem.to_string_lossy()) {
                        super::workshop_slots::release(cooked, &stem.to_string_lossy());
                    } else {
                        let _ = fs::remove_file(cooked.join(stem).with_extension("upk"));
                    }
                }
            }
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "hebnix-lan-trial-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn read_text(path: &Path) -> String {
        let bytes = fs::read(path).unwrap();
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
    #[test]
    fn package_lookup_crosses_catalog_ids_and_rejects_ambiguous_copies() {
        let root = temp();
        let selected = root.join("mods");
        let fallback = root.join("HebnixWorkshopMaps");
        let local = fallback.join("hebnix_667").join("SpeedJumpRings3.udk");
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        fs::create_dir_all(&selected).unwrap();
        fs::write(&local, b"local package").unwrap();
        let roots = [selected.clone(), fallback];
        assert_eq!(
            find_local_map(&roots, "speedjumprings3"),
            Some(fs::canonicalize(&local).unwrap())
        );
        assert!(find_local_map(&roots, "missing_package").is_none());
        fs::write(selected.join("speedjumprings3.upk"), b"different package").unwrap();
        assert!(find_local_map(&roots, "speedjumprings3").is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn peer_titles_and_local_paths_are_separate_and_stale_peers_are_removed() {
        let root = temp();
        let cooked = root.join("TAGame/CookedPCConsole");
        fs::create_dir_all(cooked.join("Backups")).unwrap();
        fs::write(cooked.join("Backups").join(RECEIPT), r#"{"mode":"join"}"#).unwrap();
        let map = root.join("original_name.udk");
        fs::write(&map, b"local test map").unwrap();
        let folder = root.join("TAGame").join(BRIDGE);
        sync_offers(
            &cooked,
            &[
                Offer {
                    ip: "127.10.0.1".parse().unwrap(),
                    title: "Readable workshop title \u{2603}".into(),
                    map: Some(map),
                    package: String::new(),
                },
                Offer {
                    ip: "127.10.0.2".parse().unwrap(),
                    title: "Not downloaded".into(),
                    map: None,
                    package: String::new(),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            read_text(&folder.join("127.10.0.1_7777.title")),
            "Readable workshop title \u{2603}"
        );
        assert!(
            read_text(&folder.join("127.10.0.1_7777.object")).ends_with("original_name.TheWorld")
        );
        assert!(!folder.join("127.10.0.2_7777.object").exists());
        assert_eq!(fs::read(cooked.join("original_name.upk")).unwrap(), b"local test map");
        assert_eq!(
            offer_status(&cooked),
            [
                "Not downloaded - not on this PC yet",
                "Readable workshop title \u{2603} - ready to join"
            ]
        );
        // a different package already using the name is never replaced or removed
        fs::write(cooked.join("taken.upk"), b"game package").unwrap();
        fs::write(root.join("taken.udk"), b"peer map data").unwrap();
        sync_offers(
            &cooked,
            &[Offer {
                ip: "127.10.0.3".parse().unwrap(),
                title: "Taken".into(),
                map: Some(root.join("taken.udk")),
                package: String::new(),
            }],
        )
        .unwrap();
        assert!(!folder.join("127.10.0.3_7777.object").exists());
        sync_offers(&cooked, &[]).unwrap();
        assert!(!cooked.join("original_name.upk").exists());
        assert_eq!(fs::read(cooked.join("taken.upk")).unwrap(), b"game package");
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);

        // a host that loaded the map under a slot name: the joiner's slot is swapped
        fs::write(cooked.join("Woods_SFX.upk"), b"placeholder").unwrap();
        super::super::workshop_slots::ensure_pool(&cooked);
        let slotted = |ip: &str, title: &str, path: &Path| Offer {
            ip: ip.parse().unwrap(),
            title: title.into(),
            map: Some(path.to_path_buf()),
            package: "hbx_slot004".into(),
        };
        sync_offers(&cooked, &[slotted("127.10.0.4", "Slotted", &root.join("taken.udk"))]).unwrap();
        assert_eq!(fs::read(cooked.join("hbx_slot004.upk")).unwrap(), b"peer map data");
        assert_eq!(read_text(&folder.join("127.10.0.4_7777.object")), "hbx_slot004.TheWorld");
        assert_eq!(offer_status(&cooked), ["Slotted - ready to join (hbx_slot004)"]);
        // another host using the same slot for a different map does not take it over
        fs::write(root.join("other.udk"), b"other map").unwrap();
        sync_offers(
            &cooked,
            &[
                slotted("127.10.0.4", "Slotted", &root.join("taken.udk")),
                slotted("127.10.0.5", "Clash", &root.join("other.udk")),
            ],
        )
        .unwrap();
        assert!(!folder.join("127.10.0.5_7777.object").exists());
        assert_eq!(fs::read(cooked.join("hbx_slot004.upk")).unwrap(), b"peer map data");
        sync_offers(&cooked, &[]).unwrap();
        assert_eq!(fs::read(cooked.join("hbx_slot004.upk")).unwrap(), b"placeholder");
        fs::remove_dir_all(root).unwrap();
    }

    /// exercises real package serialization and restores every unrelated export
    #[test]
    #[ignore]
    fn real_package_roundtrip() {
        let source = std::env::var("HEBNIX_LAN_TRIAL_SOURCE").unwrap();
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        fs::copy(source, root.join("TAGame.upk")).unwrap();
        let before = UpkPackage::load(&root.join("TAGame.upk")).unwrap();
        set_mode(&root, "labels").unwrap();
        let labels = UpkPackage::load(&root.join("TAGame.upk")).unwrap();
        assert_eq!(
            payload(&labels, JOIN).unwrap(),
            payload(&before, JOIN).unwrap()
        );
        assert_ne!(
            payload(&labels, TITLE).unwrap(),
            payload(&before, TITLE).unwrap()
        );
        set_mode(&root, "join").unwrap();
        let joined = UpkPackage::load(&root.join("TAGame.upk")).unwrap();
        assert_ne!(
            payload(&joined, JOIN).unwrap(),
            payload(&before, JOIN).unwrap()
        );
        let saved = fs::read(root.join("TAGame.upk")).unwrap();
        reapply(&root).unwrap();
        assert_eq!(fs::read(root.join("TAGame.upk")).unwrap(), saved);
        let mut rebuilt = UpkPackage::load(&root.join("TAGame.upk")).unwrap();
        let index = object(&rebuilt, TITLE).unwrap() as usize - 1;
        rebuilt
            .replace_export_payload(index, &payload(&before, TITLE).unwrap())
            .unwrap();
        rebuilt.save(&root.join("TAGame.upk")).unwrap();
        reapply(&root).unwrap();
        let reapplied = UpkPackage::load(&root.join("TAGame.upk")).unwrap();
        assert_eq!(
            payload(&reapplied, TITLE).unwrap(),
            payload(&joined, TITLE).unwrap()
        );
        set_mode(&root, "off").unwrap();
        let after = UpkPackage::load(&root.join("TAGame.upk")).unwrap();
        assert_eq!(before.names, after.names);
        for (index, e) in before.exports.iter().enumerate() {
            let a = &after.exports[index];
            assert_eq!(
                &before.image[e.serial_offset..e.serial_offset + e.serial_size],
                &after.image[a.serial_offset..a.serial_offset + a.serial_size],
                "export {index}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
