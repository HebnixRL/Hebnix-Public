//! fixed package names for LAN custom maps. the engine only resolves a package
//! by a name it indexed at launch, so every PC keeps the same pool of hbx_slotNN
//! placeholders. the host loads a map under a slot name, the joiner swaps that
//! slot to the same map while the game runs.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::workshop_menu::WorkshopMap;

pub const SLOTS: usize = 128;
const PREFIX: &str = "hbx_slot";
const HOST_DIR: &str = "HbxSlots";
const STATE: &str = "slots.json";
// maps the player ticked for LAN, by path
const TICKED: &str = "lan_ready.json";
// a small vanilla package, copied to make each placeholder
const PLACEHOLDER: &str = "Woods_SFX.upk";

pub fn slot_name(n: usize) -> String {
    format!("{PREFIX}{n:03}")
}

pub fn is_slot(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .filter(|n| n.len() == 3)
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| (1..=SLOTS).contains(&n))
}

fn host_dir(cooked: &Path) -> PathBuf {
    cooked.parent().unwrap_or(cooked).join(HOST_DIR)
}

fn read_state(dir: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(dir.join(STATE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// paths of the maps ticked for LAN. before the first tick, the maps whose
/// slot copy exists count as ticked.
pub fn ticked(cooked: &Path) -> HashSet<String> {
    let dir = host_dir(cooked);
    match fs::read_to_string(dir.join(TICKED)).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()) {
        Some(list) => list.into_iter().collect(),
        None => read_state(&dir)
            .into_iter()
            .filter(|(slot, _)| is_slot(slot) && dir.join(format!("{slot}.udk")).is_file())
            .map(|(_, path)| path)
            .collect(),
    }
}

pub fn set_ticked(cooked: &Path, map: &Path, on: bool) -> Result<(), String> {
    let mut set = ticked(cooked);
    let path = map.to_string_lossy().into_owned();
    if on { set.insert(path) } else { set.remove(&path) };
    let dir = host_dir(cooked);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut list: Vec<_> = set.into_iter().collect();
    list.sort();
    let text = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    fs::write(dir.join(TICKED), text).map_err(|e| e.to_string())
}

/// same size and write time, which a copy or link keeps
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(a), Ok(b)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    let close = match (a.modified(), b.modified()) {
        (Ok(a), Ok(b)) => a.duration_since(b).unwrap_or_else(|e| e.duration()) < Duration::from_millis(50),
        _ => false,
    };
    a.len() == b.len() && close
}

/// hardlink where the drive allows it (NTFS), else a copy with a progress card
fn link_or_copy(src: &Path, dst: &Path, key: &str, label: &str) -> Result<(), String> {
    if fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    let result = (|| {
        let mut from = fs::File::open(src)?;
        let total = from.metadata()?.len().max(1);
        let mut to = fs::File::create(dst)?;
        let mut buffer = vec![0u8; 1 << 20];
        let (mut done, mut last) = (0u64, Instant::now() - Duration::from_secs(1));
        loop {
            let n = from.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            to.write_all(&buffer[..n])?;
            done += n as u64;
            if last.elapsed() > Duration::from_millis(250) {
                last = Instant::now();
                crate::toast::report(key, label, Some(done as f32 / total as f32), false);
            }
        }
        to.set_modified(from.metadata()?.modified()?)
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(dst);
        format!("could not copy {}: {e}", src.display())
    })
}

/// the hosted file is a slot copy; the map it stands for
pub fn original(hosted: &Path) -> Option<PathBuf> {
    let dir = hosted.parent()?;
    if dir.file_name()? != HOST_DIR {
        return None;
    }
    let stem = hosted.file_stem()?.to_string_lossy().into_owned();
    read_state(dir).remove(&stem).map(PathBuf::from)
}

/// the slot copy path a map was given, which only exists once the map is ticked
pub fn reserved(cooked: &Path, map: &Path) -> Option<PathBuf> {
    let slot = slot_of(cooked, map)?;
    let host = host_dir(cooked).join(format!("{slot}.udk"));
    (same_file(&host, map) && same_file(&cooked.join(format!("{slot}.upk")), map))
        .then_some(host)
}

/// A host slot is usable only when both copies hold the selected map.
pub fn filled(cooked: &Path, map: &Path) -> Option<PathBuf> {
    reserved(cooked, map)
}

/// Incoming slot protection rejects conflicting peers rather than unticking
/// local maps, so joining never requires an untick-driven menu refresh.
pub fn take_unticked() -> bool {
    false
}

/// the slot a map was given, for display
pub fn slot_of(cooked: &Path, map: &Path) -> Option<String> {
    let want = map.to_string_lossy();
    read_state(&host_dir(cooked)).into_iter().find(|(_, path)| *path == want).map(|(slot, _)| slot)
}

pub struct HostReport {
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// reserve a slot for every listed map and copy the ticked ones into theirs, so
/// a right-click can load the map under a package name every player has ready.
/// maps keep a slot until they are removed; unticked maps are never copied.
/// more maps than slots leave the extra ones without.
pub fn assign_host(cooked: &Path, maps: &[WorkshopMap]) -> HostReport {
    let dir = host_dir(cooked);
    let mut report = HostReport { skipped: 0, errors: Vec::new() };
    if let Err(e) = fs::create_dir_all(&dir) {
        report.errors.push(e.to_string());
        return report;
    }
    let mut state = read_state(&dir);
    let ticked = ticked(cooked);
    let listed: Vec<String> = maps.iter().map(|m| m.path.to_string_lossy().into_owned()).collect();
    state.retain(|slot, path| {
        // the old two digit slot names are dropped with their copies
        let keep = is_slot(slot) && listed.contains(path);
        if !keep {
            let _ = fs::remove_file(dir.join(format!("{slot}.udk")));
        }
        keep
    });
    for map in maps {
        let path = map.path.to_string_lossy().into_owned();
        let slot = match state.iter().find(|(_, p)| **p == path) {
            Some((slot, _)) => slot.clone(),
            None => match (1..=SLOTS).map(slot_name).find(|s| !state.contains_key(s)) {
                Some(slot) => slot,
                None => {
                    report.skipped += 1;
                    continue;
                }
            },
        };
        let dest = dir.join(format!("{slot}.udk"));
        if !ticked.contains(&path) {
            let _ = fs::remove_file(&dest);
            state.insert(slot, path);
            continue;
        }
        if !same_file(&dest, &map.path) {
            let key = format!("slot-{slot}");
            let label = format!("Preparing {} for LAN ({slot})", map.title);
            crate::toast::report(&key, &label, None, false);
            let copied = (|| {
                if dest.exists() {
                    fs::remove_file(&dest).map_err(|e| format!("{slot} is in use: {e}"))?;
                }
                link_or_copy(&map.path, &dest, &key, &label)
            })();
            match copied {
                Ok(()) => crate::toast::report(&key, &format!("{} is ready for LAN ({slot})", map.title), None, true),
                Err(e) => {
                    crate::toast::report(&key, &format!("LAN slot failed for {}: {e}", map.title), None, true);
                    report.errors.push(e);
                    state.remove(&slot);
                    continue;
                }
            }
        }
        // LAN resolves packages by bare name as well as the explicit Workshop
        // path. Keep both files consistent; this alone does not resolve the
        // duplicate NetIndex failures seen when hosting some renamed maps.
        if let Err(e) = place(cooked, &slot, &map.path, &map.title) {
            report.errors.push(e.clone());
            crate::toast::report(&format!("slot-{slot}"), &e, None, true);
            let _ = fs::remove_file(&dest);
            state.remove(&slot);
            continue;
        }
        state.insert(slot, path);
    }
    if let Ok(text) = serde_json::to_string_pretty(&state) {
        let _ = fs::write(dir.join(STATE), text);
    }
    report
}

/// make sure every slot has a placeholder before the game starts; how many were new
pub fn ensure_pool(cooked: &Path) -> usize {
    let source = cooked.join(PLACEHOLDER);
    if !source.is_file() {
        return 0;
    }
    (1..=SLOTS)
        .map(|n| cooked.join(format!("{}.upk", slot_name(n))))
        .filter(|dest| !dest.exists() && fs::copy(&source, dest).is_ok())
        .count()
}

pub enum Placed {
    /// The slot already contains the player's own ticked map.
    Own,
    Ready,
    Swapped,
    /// the slot did not exist when the game started
    Restart,
}

/// swap a slot to the host's map. fails while the game has the slot loaded.
pub fn place(cooked: &Path, slot: &str, map: &Path, title: &str) -> Result<Placed, String> {
    if !is_slot(slot) {
        return Err(format!("{slot} is not a map slot"));
    }
    let own = local_host_map(cooked, slot);
    if let Some(host) = &own {
        // Size/mtime is only a copy freshness hint, not map identity: distinct
        // packages can have the same length and timestamp.
        let source = original(&host).and_then(|p| fs::canonicalize(p).ok());
        if source.is_none() || source != fs::canonicalize(map).ok() {
            return Err(format!("{slot} is reserved for a local LAN map; untick it before joining a different map in this slot"));
        }
    }
    let dest = cooked.join(format!("{slot}.upk"));
    let existed = dest.exists();
    if existed && same_file(&dest, map) {
        return Ok(if own.is_some() { Placed::Own } else { Placed::Ready });
    }
    let key = format!("slot-{slot}");
    let label = format!("Loading {title} into {slot}");
    crate::toast::report(&key, &label, None, false);
    if existed {
        fs::remove_file(&dest).map_err(|e| format!("{slot} is in use by the game: {e}"))?;
    }
    if let Err(e) = link_or_copy(map, &dest, &key, &label) {
        release(cooked, slot);
        crate::toast::report(&key, &format!("Could not load {title}: {e}"), None, true);
        return Err(e);
    }
    let text = if existed {
        format!("{title} is ready to join ({slot})")
    } else {
        format!("{title} needs a Rocket League restart once to join ({slot})")
    };
    crate::toast::report(&key, &text, None, true);
    Ok(if existed { Placed::Swapped } else { Placed::Restart })
}

fn local_host_map(cooked: &Path, slot: &str) -> Option<PathBuf> {
    let dir = host_dir(cooked);
    let state = read_state(&dir);
    let path = state.get(slot)?;
    let host = dir.join(format!("{slot}.udk"));
    (ticked(cooked).contains(path) && host.is_file()).then_some(host)
}

/// Restore the local host map, or the placeholder, unless the game holds it.
pub fn release(cooked: &Path, slot: &str) {
    let source = local_host_map(cooked, slot).unwrap_or_else(|| cooked.join(PLACEHOLDER));
    let dest = cooked.join(format!("{slot}.upk"));
    if same_file(&source, &dest) {
        return;
    }
    if dest.exists() && fs::remove_file(&dest).is_err() {
        return;
    }
    let _ = link_or_copy(&source, &dest, &format!("slot-{slot}"), "Restoring LAN slot");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hbx-slots-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("TAGame").join("CookedPCConsole")).unwrap();
        dir
    }

    #[test]
    fn only_pool_names_are_slots() {
        assert!(is_slot("hbx_slot001") && is_slot("hbx_slot128"));
        for bad in ["hbx_slot000", "hbx_slot129", "hbx_slot01", "hbx_slot1", "hbx_slot0001", "Woods_SFX", "hbx_slot0a0"] {
            assert!(!is_slot(bad), "{bad}");
        }
    }

    #[test]
    fn host_slots_are_kept_per_map_and_freed_when_a_map_goes() {
        let game = temp("host").join("TAGame");
        let cooked = game.join("CookedPCConsole");
        let map = |name: &str| {
            let path = game.join(format!("{name}.udk"));
            fs::write(&path, name.as_bytes()).unwrap();
            WorkshopMap { id: 1, title: name.into(), author: String::new(), description: String::new(), path, preview_url: String::new() }
        };
        let maps = vec![map("one"), map("two")];
        let originals: Vec<_> = maps.iter().map(|m| m.path.clone()).collect();
        // every map reserves a slot, but nothing is copied until ticked
        assert_eq!(assign_host(&cooked, &maps).errors.len(), 0);
        let dir = host_dir(&cooked);
        assert_eq!(reserved(&cooked, &originals[0]), None);
        assert_eq!(reserved(&cooked, &originals[1]), None);
        assert!(!dir.join("hbx_slot001.udk").exists());
        for path in &originals {
            set_ticked(&cooked, path, true).unwrap();
        }
        assert_eq!(assign_host(&cooked, &maps).errors.len(), 0);
        assert_eq!(reserved(&cooked, &originals[1]), Some(dir.join("hbx_slot002.udk")));
        assert_eq!(filled(&cooked, &originals[1]), Some(dir.join("hbx_slot002.udk")));
        assert!(matches!(place(&cooked, "hbx_slot002", &originals[1], "Own map"), Ok(Placed::Own)));
        assert_eq!(fs::read(dir.join("hbx_slot002.udk")).unwrap(), b"two");
        assert_eq!(fs::read(cooked.join("hbx_slot002.upk")).unwrap(), b"two");
        release(&cooked, "hbx_slot002");
        assert_eq!(fs::read(cooked.join("hbx_slot002.upk")).unwrap(), b"two");
        assert!(place(&cooked, "hbx_slot002", &originals[0], "Conflicting peer").is_err());
        assert!(!take_unticked());
        assert!(ticked(&cooked).contains(originals[1].to_string_lossy().as_ref()));
        assert_eq!(original(&dir.join("hbx_slot002.udk")), Some(originals[1].clone()));
        assert_eq!(slot_of(&cooked, &originals[0]).as_deref(), Some("hbx_slot001"));

        // unticking drops the copy and keeps the reserved name
        set_ticked(&cooked, &originals[1], false).unwrap();
        assign_host(&cooked, &maps);
        assert!(!dir.join("hbx_slot002.udk").exists());
        assert_eq!(slot_of(&cooked, &originals[1]).as_deref(), Some("hbx_slot002"));
        // dropping the first map frees its slot and keeps the second's
        let maps = vec![WorkshopMap { path: originals[1].clone(), ..map("two") }];
        assign_host(&cooked, &maps);
        assert!(!dir.join("hbx_slot001.udk").exists());
        assert_eq!(slot_of(&cooked, &originals[0]), None);
        assert_eq!(slot_of(&cooked, &originals[1]).as_deref(), Some("hbx_slot002"));
    }

    #[test]
    fn joiner_slot_swaps_and_releases() {
        let game = temp("join").join("TAGame");
        let cooked = game.join("CookedPCConsole");
        fs::write(cooked.join(PLACEHOLDER), b"placeholder").unwrap();
        let map = game.join("map.udk");
        fs::write(&map, b"the real map").unwrap();

        let slot_file = cooked.join("hbx_slot003.upk");
        assert!(matches!(place(&cooked, "hbx_slot003", &map, "Map"), Ok(Placed::Restart)));
        assert_eq!(fs::read(&slot_file).unwrap(), b"the real map");
        assert!(matches!(place(&cooked, "hbx_slot003", &map, "Map"), Ok(Placed::Ready)));
        assert_eq!(ensure_pool(&cooked), SLOTS - 1);
        release(&cooked, "hbx_slot003");
        assert_eq!(fs::read(&slot_file).unwrap(), b"placeholder");
        let other = game.join("other.udk");
        fs::write(&other, b"another, bigger map").unwrap();
        assert!(matches!(place(&cooked, "hbx_slot003", &other, "Other"), Ok(Placed::Swapped)));
        assert!(place(&cooked, "Woods_SFX", &map, "Map").is_err());
    }
}
