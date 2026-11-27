//! the Browse Maps catalog, backed by the Steam Workshop through the hebnix
//! workshop api (search + manifests, see docs/workshop-api.md). maps download
//! straight from steam's CDN into the same cache the old catalog used, so the
//! cards, the in-game Workshop menu and LAN hosting work the same.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::steam_cdn;
use super::steam_download::{self, SearchItem, SearchSort};

/// the maps downloaded from the workshop, so they stay listed (View
/// downloaded, the menu map picker) without being in the current results
const DOWNLOADED_FILE: &str = "steam_maps.json";

/// every search result the player has seen, newest first, shown while
/// nothing is being searched for
const SEARCH_CACHE_FILE: &str = "steam_search_cache.json";
const SEARCH_CACHE_LIMIT: usize = 500;

/// one page of workshop results as catalog entries
#[derive(Debug)]
pub struct SteamPage {
    pub items: Vec<Value>,
    pub total: u64,
    pub next_cursor: Option<String>,
}

pub fn fetch_page(query: &str, cursor: Option<&str>) -> Result<SteamPage, String> {
    let page = steam_download::search(query, SearchSort::Popular, cursor)?;
    Ok(SteamPage {
        items: page.items.iter().map(to_catalog_entry).collect(),
        total: page.total,
        next_cursor: page.next_cursor,
    })
}

/// same shape as the old cdn catalog's entries, so the cards render it as is
pub fn to_catalog_entry(item: &SearchItem) -> Value {
    json!({
        "id": item.wid,
        "name": item.title,
        "author": item.author,
        "short_description": item.description,
        "banner_path": if item.preview_url.starts_with("https://") { item.preview_url.as_str() } else { "" },
        "download_count": item.subscriptions.to_string(),
        "file_size": item.file_size,
        "steam": true,
    })
}

pub fn is_steam_entry(entry: &Value) -> bool {
    entry.get("steam").and_then(Value::as_bool).unwrap_or(false)
}

/// a steam preview url shrunk to card size (steam's image server resizes
/// on request), so a page of cards is a few hundred KB, not tens of MB
pub fn thumbnail_url(url: &str) -> String {
    if url.starts_with("https://images.steamusercontent.com/") && !url.contains('?') {
        format!("{url}?imw=480&imh=270&ima=fit&impolicy=Letterbox")
    } else {
        url.to_string()
    }
}

pub fn load_downloaded(runtime_dir: &Path) -> Vec<Value> {
    load_entries(&runtime_dir.join(DOWNLOADED_FILE))
}

pub fn load_search_cache(runtime_dir: &Path) -> Vec<Value> {
    load_entries(&runtime_dir.join(SEARCH_CACHE_FILE))
}

/// remembers search results for later viewing (newer results replace older
/// records of the same map)
pub fn remember_search_results(runtime_dir: &Path, items: &[Value]) {
    let fresh: Vec<&Value> = items
        .iter()
        .filter(|item| is_steam_entry(item) && valid_wid(&super::id_of(item)))
        .collect();
    if fresh.is_empty() {
        return;
    }
    let mut maps = load_search_cache(runtime_dir);
    maps.retain(|map| !fresh.iter().any(|item| super::id_of(item) == super::id_of(map)));
    maps.splice(0..0, fresh.into_iter().cloned());
    maps.truncate(SEARCH_CACHE_LIMIT);
    let written = serde_json::to_vec(&maps)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(runtime_dir.join(SEARCH_CACHE_FILE), bytes).map_err(|e| e.to_string()));
    if let Err(error) = written {
        tracing::warn!("[Workshop] could not save the search cache: {error}");
    }
}

fn load_entries(path: &Path) -> Vec<Value> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| is_steam_entry(entry) && valid_wid(&super::id_of(entry)))
        .collect()
}

/// remembers a downloaded map (replacing an older record of it)
pub fn record_downloaded(runtime_dir: &Path, entry: &Value) -> Result<(), String> {
    let id = super::id_of(entry);
    let mut maps = load_downloaded(runtime_dir);
    maps.retain(|map| super::id_of(map) != id);
    maps.push(entry.clone());
    let text = serde_json::to_string_pretty(&maps).map_err(|e| e.to_string())?;
    std::fs::write(runtime_dir.join(DOWNLOADED_FILE), text).map_err(|e| e.to_string())
}

/// the package name a downloaded map was saved with, which a LAN join
/// matches on
pub fn downloaded_file(runtime_dir: &Path, id: &str) -> Option<String> {
    load_downloaded(runtime_dir)
        .into_iter()
        .find(|map| super::id_of(map) == id)
        .and_then(|map| map.get("file").and_then(Value::as_str).map(str::to_string))
        .filter(|file| crate::multiplayer_lan::valid_package_stem(file))
}

fn valid_wid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// downloads a workshop item from steam's CDN and caches its map as
/// `<cache_dir>/<wid>.upk`. returns the map's package name (its file name
/// without the extension).
pub fn download_map(
    wid: &str,
    cache_dir: &Path,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<String, String> {
    if !valid_wid(wid) {
        return Err("That isn't a Steam Workshop map.".to_string());
    }
    let item = steam_cdn::fetch_manifest(wid)?;
    let work_dir: PathBuf = cache_dir.join(format!("steam_tmp_{wid}"));
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
    let result = (|| {
        steam_cdn::download(&item, &work_dir, progress)?;
        let map_file = steam_download::find_map_file(&work_dir)
            .ok_or("The download had no map file in it.")?;
        let stem: String = map_file
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let cached = cache_dir.join(format!("{wid}.upk"));
        std::fs::rename(&map_file, &cached)
            .or_else(|_| std::fs::copy(&map_file, &cached).map(|_| ()))
            .map_err(|e| format!("Could not save the map: {e}"))?;
        Ok(stem)
    })();
    let _ = std::fs::remove_dir_all(&work_dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_items_become_catalog_entries() {
        let item = SearchItem {
            wid: "1759763738".into(),
            title: "Speed Jump: Trials 3".into(),
            file_size: 59_224_775,
            subscriptions: 120_000,
            author: "dmc".into(),
            description: "Jump".into(),
            preview_url: "https://images.steamusercontent.com/ugc/1/ABC/".into(),
        };
        let entry = to_catalog_entry(&item);
        assert_eq!(super::super::id_of(&entry), "1759763738");
        assert_eq!(entry["name"], "Speed Jump: Trials 3");
        assert_eq!(entry["author"], "dmc");
        assert_eq!(entry["download_count"], "120000");
        assert!(is_steam_entry(&entry));
        let plain = SearchItem { preview_url: "http://x/y.png".into(), ..item };
        assert_eq!(to_catalog_entry(&plain)["banner_path"], "");
    }

    #[test]
    fn steam_previews_are_fetched_at_card_size() {
        assert_eq!(
            thumbnail_url("https://images.steamusercontent.com/ugc/1/ABC/"),
            "https://images.steamusercontent.com/ugc/1/ABC/?imw=480&imh=270&ima=fit&impolicy=Letterbox"
        );
        assert_eq!(thumbnail_url("https://example.com/a.png"), "https://example.com/a.png");
    }

    #[test]
    fn downloaded_maps_are_remembered_with_their_package_name() {
        let dir = std::env::temp_dir().join(format!("hebnix-steam-catalog-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut entry = json!({"id": "42", "name": "A", "steam": true});
        record_downloaded(&dir, &entry).unwrap();
        entry["file"] = json!("sjt3");
        record_downloaded(&dir, &entry).unwrap();
        assert_eq!(load_downloaded(&dir).len(), 1);
        assert_eq!(downloaded_file(&dir, "42").as_deref(), Some("sjt3"));
        record_downloaded(&dir, &json!({"id": "../x", "steam": true})).unwrap();
        assert_eq!(load_downloaded(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_results_are_cached_newest_first_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("hebnix-steam-cache-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let map = |id: &str, name: &str| json!({"id": id, "name": name, "steam": true});
        remember_search_results(&dir, &[map("1", "a"), map("2", "b")]);
        remember_search_results(&dir, &[map("3", "c"), map("2", "b2"), json!({"id": "x", "steam": true})]);
        let names: Vec<_> = load_search_cache(&dir).iter().map(|m| m["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["c", "b2", "a"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
