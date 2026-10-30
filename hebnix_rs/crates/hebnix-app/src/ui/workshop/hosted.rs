//! The map this player is hosting a LAN match on right now, announced to the
//! other players so they can download it into their Workshop menu.
//!
//! The game logs the hosted file's path (`hebnix_sdk::log::hosted_map`). Maps
//! Hebnix installed sit in a `hebnix_<id>` folder and keep that id; any other
//! map (Steam workshop, hand-made) is shared as `local_<hash>`, the same id an
//! imported map gets, so peers verify the download against its own name.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::multiplayer_lan::{SlotMap, hash_file, is_local_map_id, local_map_id, valid_map_id};

use super::ingame_menu::META;

/// `SlotMap::slot` of the hosted map. Not a map slot, so older versions ignore it.
pub const HOSTING_SLOT: &str = "Hosting";

#[derive(Default)]
struct State {
    path: Option<String>,
    map: Option<SlotMap>,
}

/// Looks the hosted map up once per hosted file; hashing and copying a big map
/// into the cache happens on a worker so replying to a peer never waits on it.
#[derive(Clone)]
pub struct HostedMap {
    state: Arc<Mutex<State>>,
    cache_dir: PathBuf,
}

impl HostedMap {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { state: Arc::default(), cache_dir }
    }

    /// The map being hosted, once it is ready to be served.
    pub fn current(&self) -> Option<SlotMap> {
        let path = hebnix_sdk::log::hosted_map(None);
        let mut state = self.state.lock().ok()?;
        if state.path != path {
            state.path = path.clone();
            state.map = None;
            if let Some(path) = path {
                let (me, cache_dir) = (self.state.clone(), self.cache_dir.clone());
                std::thread::spawn(move || {
                    let map = describe(Path::new(&path), &cache_dir);
                    if let Ok(mut state) = me.lock() {
                        if state.path.as_deref() == Some(path.as_str()) {
                            state.map = map;
                        }
                    }
                });
            }
        }
        state.map.clone()
    }
}

/// Name the hosted file for peers and make sure the cache can serve it.
fn describe(file: &Path, cache_dir: &Path) -> Option<SlotMap> {
    let folder = file.parent()?;
    let id = folder
        .file_name()?
        .to_str()?
        .strip_prefix(crate::patcher::workshop_menu::MAP_DIR_PREFIX)
        .filter(|id| valid_map_id(id) && !is_local_map_id(id))
        .map(str::to_string)
        .or_else(|| hash_file(file).ok().map(|hash| local_map_id(&hash)))?;
    let cached = cache_dir.join(format!("{id}.upk"));
    if !cached.is_file() {
        std::fs::create_dir_all(cache_dir).ok()?;
        std::fs::copy(file, &cached).ok()?;
    }
    let meta: serde_json::Value = std::fs::read_to_string(folder.join(META))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let text = |key: &str| meta[key].as_str().unwrap_or_default().to_string();
    let stem = file.file_stem()?.to_string_lossy().to_string();
    Some(SlotMap {
        slot: HOSTING_SLOT.to_string(),
        local: is_local_map_id(&id),
        name: Some(text("title")).filter(|t| !t.is_empty()).unwrap_or(stem),
        author: text("author"),
        description: text("description"),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hebnix_maps_keep_their_id_and_other_maps_are_hashed() {
        let dir = std::env::temp_dir().join("hebnix_hosted_test");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = dir.join("cache");
        let ours = dir.join("hebnix_abc123");
        let steam = dir.join("1843822064");
        for folder in [&ours, &steam] {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("n.udk"), b"map bytes").unwrap();
        }
        std::fs::write(ours.join(META), r#"{"title":"Noob","author":"dmc"}"#).unwrap();

        let map = describe(&ours.join("n.udk"), &cache).unwrap();
        assert_eq!((map.id.as_str(), map.name.as_str(), map.author.as_str(), map.local), ("abc123", "Noob", "dmc", false));
        assert!(cache.join("abc123.upk").is_file());

        let map = describe(&steam.join("n.udk"), &cache).unwrap();
        assert!(map.local && map.id.starts_with("local_") && map.name == "n");
        assert!(cache.join(format!("{}.upk", map.id)).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
