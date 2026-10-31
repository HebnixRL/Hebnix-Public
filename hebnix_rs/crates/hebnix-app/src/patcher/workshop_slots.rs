//! fixed package names for LAN custom maps. the engine only resolves a package
//! by a name it indexed at launch, so every PC keeps the same pool of hbx_slotNN
//! placeholders. the host loads a map under a slot name, the joiner swaps that
//! slot to the same map while the game runs.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::workshop_menu::WorkshopMap;

pub const SLOTS: usize = 16;
const PREFIX: &str = "hbx_slot";
const HOST_DIR: &str = "HbxSlots";
const STATE: &str = "slots.json";
// a small vanilla package, copied to make each placeholder
const PLACEHOLDER: &str = "Woods_SFX.upk";

pub fn slot_name(n: usize) -> String {
    format!("{PREFIX}{n:02}")
}

pub fn is_slot(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .filter(|n| n.len() == 2)
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

/// the slot a map was given, for display
pub fn slot_of(cooked: &Path, map: &Path) -> Option<String> {
    let want = map.to_string_lossy();
    read_state(&host_dir(cooked)).into_iter().find(|(_, path)| *path == want).map(|(slot, _)| slot)
}

pub struct HostReport {
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// give each listed map a slot copy and point the list at it, so the game loads
/// the map under a package name every player has ready. maps keep a slot until
/// they are removed; more maps than slots leave the extra ones as they are.
pub fn assign_host(cooked: &Path, maps: &mut [WorkshopMap]) -> HostReport {
    let dir = host_dir(cooked);
    let mut report = HostReport { skipped: 0, errors: Vec::new() };
    if let Err(e) = fs::create_dir_all(&dir) {
        report.errors.push(e.to_string());
        return report;
    }
    let mut state = read_state(&dir);
    let listed: Vec<String> = maps.iter().map(|m| m.path.to_string_lossy().into_owned()).collect();
    state.retain(|slot, path| {
        let keep = listed.contains(path);
        if !keep {
            let _ = fs::remove_file(dir.join(format!("{slot}.udk")));
        }
        keep
    });
    for map in maps.iter_mut() {
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
        state.insert(slot, path);
        map.path = dest;
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
    let dest = cooked.join(format!("{slot}.upk"));
    let existed = dest.exists();
    if existed && same_file(&dest, map) {
        return Ok(Placed::Ready);
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

/// put the placeholder back, unless the game still holds the slot
pub fn release(cooked: &Path, slot: &str) {
    let dest = cooked.join(format!("{slot}.upk"));
    if dest.exists() && fs::remove_file(&dest).is_err() {
        return;
    }
    let _ = fs::copy(cooked.join(PLACEHOLDER), dest);
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
        assert!(is_slot("hbx_slot01") && is_slot("hbx_slot16"));
        for bad in ["hbx_slot00", "hbx_slot17", "hbx_slot1", "hbx_slot001", "Woods_SFX", "hbx_slot0a"] {
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
        let mut maps = vec![map("one"), map("two")];
        let originals: Vec<_> = maps.iter().map(|m| m.path.clone()).collect();
        assert_eq!(assign_host(&cooked, &mut maps).errors.len(), 0);
        assert_eq!(maps[0].path.file_name().unwrap(), "hbx_slot01.udk");
        assert_eq!(maps[1].path.file_name().unwrap(), "hbx_slot02.udk");
        assert_eq!(fs::read(&maps[1].path).unwrap(), b"two");
        assert_eq!(original(&maps[1].path), Some(originals[1].clone()));
        assert_eq!(slot_of(&cooked, &originals[0]).as_deref(), Some("hbx_slot01"));

        // dropping the first map keeps the second in its slot and frees the first
        let mut maps = vec![WorkshopMap { path: originals[1].clone(), ..map("two") }];
        assign_host(&cooked, &mut maps);
        assert_eq!(maps[0].path.file_name().unwrap(), "hbx_slot02.udk");
        assert!(!host_dir(&cooked).join("hbx_slot01.udk").exists());
        let mut maps = vec![WorkshopMap { path: originals[0].clone(), ..map("one") }, map("two")];
        assign_host(&cooked, &mut maps);
        assert_eq!(maps[0].path.file_name().unwrap(), "hbx_slot01.udk");
    }

    #[test]
    fn joiner_slot_swaps_and_releases() {
        let game = temp("join").join("TAGame");
        let cooked = game.join("CookedPCConsole");
        fs::write(cooked.join(PLACEHOLDER), b"placeholder").unwrap();
        let map = game.join("map.udk");
        fs::write(&map, b"the real map").unwrap();

        let slot_file = cooked.join("hbx_slot03.upk");
        assert!(matches!(place(&cooked, "hbx_slot03", &map, "Map"), Ok(Placed::Restart)));
        assert_eq!(fs::read(&slot_file).unwrap(), b"the real map");
        assert!(matches!(place(&cooked, "hbx_slot03", &map, "Map"), Ok(Placed::Ready)));
        assert_eq!(ensure_pool(&cooked), SLOTS - 1);
        release(&cooked, "hbx_slot03");
        assert_eq!(fs::read(&slot_file).unwrap(), b"placeholder");
        let other = game.join("other.udk");
        fs::write(&other, b"another, bigger map").unwrap();
        assert!(matches!(place(&cooked, "hbx_slot03", &other, "Other"), Ok(Placed::Swapped)));
        assert!(place(&cooked, "Woods_SFX", &map, "Map").is_err());
    }
}
