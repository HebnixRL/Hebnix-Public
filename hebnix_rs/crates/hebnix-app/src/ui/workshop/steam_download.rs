//! finds and downloads rocket league workshop maps from the steam workshop,
//! straight from the client, so a player can grab maps that aren't in the
//! hebnix catalog:
//!
//!  1. search goes through the hebnix workshop api (it holds the steam web
//!     api key), or the player pastes a workshop link/id,
//!  2. steam's public web api says what the item is (and that it really is
//!     a rocket league item -- nothing else is ever downloaded),
//!  3. the hebnix workshop api hands back the depot manifest and key, and
//!     the files come straight from steam's CDN (see steam_cdn.rs) -- no
//!     api key, steam login or .net runtime needed,
//!  4. the biggest non-boilerplate file is the map, it goes through the
//!     same import as a hand-picked map (see local_import.rs).

use crate::i18n::{t, t_args};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use eframe::egui;
use serde::Deserialize;
use serde_json::Value;

use super::local_import::{ImportMeta, LocalMap, import_map, write_item_vdf};
use super::steam_cdn;

const STEAM_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const STEAM_ITEM_URL: &str = "https://steamcommunity.com/sharedfiles/filedetails/?id=";
const RL_APPID: &str = "252950";
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LOG_LINES: usize = 200;

/// files the rocket league SDK bundles into every workshop map -- never the
/// map itself, however big they are
const BOILERPLATE: [&str; 3] = [
    "maptemplates.upk",
    "editorlandscaperesources.upk",
    "workshopiteminfo.json",
];
const IMAGE_EXTS: [&str; 6] = ["jpg", "jpeg", "jfif", "png", "webp", "bmp"];

// pure helpers

/// a workshop id from a bare number or a steam workshop url
pub fn parse_workshop_id(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.is_empty() && text.len() <= 20 && text.chars().all(|c| c.is_ascii_digit()) {
        return Some(text.to_string());
    }
    let start = text.find("?id=").or_else(|| text.find("&id="))? + 4;
    let digits: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (!digits.is_empty() && digits.len() <= 20).then_some(digits)
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkshopDetails {
    pub title: String,
    pub creator: String,
    pub description: String,
    pub preview_url: String,
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// reads steam's GetPublishedFileDetails reply, refusing anything that isn't
/// a rocket league workshop item
pub fn parse_details(reply: &Value, wid: &str) -> Result<WorkshopDetails, String> {
    let details = reply
        .pointer("/response/publishedfiledetails/0")
        .ok_or("Steam sent back nothing for that id.")?;
    if details.get("result").map(text_of).as_deref() != Some("1") {
        return Err("Steam has no workshop item with that id.".to_string());
    }
    let app = details
        .get("consumer_app_id")
        .map(text_of)
        .unwrap_or_default();
    if app != RL_APPID {
        return Err("That workshop item isn't for Rocket League.".to_string());
    }
    let field = |key: &str| details.get(key).map(text_of).unwrap_or_default();
    let title = field("title");
    Ok(WorkshopDetails {
        title: if title.trim().is_empty() {
            format!("Workshop Item {wid}")
        } else {
            title
        },
        creator: field("creator"),
        description: strip_bbcode(&field("description")),
        preview_url: field("preview_url"),
    })
}

/// steam descriptions use bbcode ([h1], [b], [url=..]); drop the tags
pub fn strip_bbcode(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close)
                if after[..close]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "=\"'.:/_ -".contains(c))
                    && !after[..close].is_empty() =>
            {
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn walk_files(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out);
        } else if let Ok(meta) = entry.metadata() {
            out.push((path, meta.len()));
        }
    }
}

/// the map inside a downloaded workshop item: the biggest file that isn't an
/// image or SDK boilerplate, preferring real .upk/.udk files
pub fn find_map_file(dir: &Path) -> Option<PathBuf> {
    let mut files = Vec::new();
    walk_files(dir, &mut files);
    files.retain(|(path, _)| {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        !IMAGE_EXTS.contains(&extension_of(path).as_str()) && !BOILERPLATE.contains(&name.as_str())
    });
    let biggest = |candidates: Vec<&(PathBuf, u64)>| {
        candidates
            .into_iter()
            .max_by_key(|(_, size)| *size)
            .map(|(path, _)| path.clone())
    };
    let packages: Vec<_> = files
        .iter()
        .filter(|(path, _)| matches!(extension_of(path).as_str(), "upk" | "udk"))
        .collect();
    biggest(packages).or_else(|| biggest(files.iter().collect()))
}

/// a preview image that came inside the download, preferring one named
/// "preview", then the biggest. tiny icons are skipped.
pub fn find_bundled_preview(dir: &Path) -> Option<PathBuf> {
    let mut files = Vec::new();
    walk_files(dir, &mut files);
    files
        .into_iter()
        .filter(|(path, size)| *size > 1024 && IMAGE_EXTS.contains(&extension_of(path).as_str()))
        .max_by_key(|(path, size)| {
            let named = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().contains("preview"));
            (named, *size)
        })
        .map(|(path, _)| path)
}

// download

struct Downloaded {
    map_file: PathBuf,
    preview: Option<PathBuf>,
    details: WorkshopDetails,
    /// what the map's own WorkshopItemInfo.json says, if the download had one
    sidecar: Option<ItemInfo>,
}

/// the title/author/description from the WorkshopItemInfo.json the SDK puts
/// in every workshop upload
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ItemInfo {
    pub title: String,
    pub author: String,
    pub description: String,
}

fn json_text(object: &Value, key: &str) -> String {
    object
        .as_object()
        .and_then(|map| map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
        .map(|(_, v)| text_of(v))
        .unwrap_or_default()
}

/// reads a WorkshopItemInfo.json (keys in any letter case); None if it has
/// nothing useful in it
pub fn parse_item_info(text: &str) -> Option<ItemInfo> {
    let json: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let info = ItemInfo {
        title: json_text(&json, "title"),
        author: json_text(&json, "author"),
        description: strip_bbcode(&json_text(&json, "description")),
    };
    (info != ItemInfo::default()).then_some(info)
}

pub(super) fn read_item_info(dir: &Path) -> Option<ItemInfo> {
    let mut files = Vec::new();
    walk_files(dir, &mut files);
    files
        .into_iter()
        .filter(|(path, size)| {
            *size < 1024 * 1024
                && path.file_name().is_some_and(|n| {
                    n.to_string_lossy()
                        .eq_ignore_ascii_case("workshopiteminfo.json")
                })
        })
        .find_map(|(path, _)| parse_item_info(&std::fs::read_to_string(path).ok()?))
}

type Log<'a> = &'a (dyn Fn(&str) + Sync);

pub(super) fn fetch_details(wid: &str) -> Result<WorkshopDetails, String> {
    let response = ureq::post(STEAM_DETAILS_URL)
        .timeout(Duration::from_secs(15))
        .send_form(&[("itemcount", "1"), ("publishedfileids[0]", wid)])
        .map_err(|e| format!("Steam lookup failed: {e}"))?;
    let mut body = String::new();
    response
        .into_reader()
        .take(2 * 1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    let reply: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    parse_details(&reply, wid)
}


fn download_preview(url: &str, dest: &Path) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    let Ok(response) = ureq::get(url).timeout(Duration::from_secs(20)).call() else {
        return false;
    };
    if !response.content_type().starts_with("image/") {
        return false;
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_IMAGE_BYTES)
        .read_to_end(&mut bytes)
        .is_ok()
        && std::fs::write(dest, bytes).is_ok()
}


fn download_item(wid: &str, work_dir: &Path, log: Log) -> Result<Downloaded, String> {
    log(&t("download-item-looking-up-the-workshop-item-on"));
    let details = fetch_details(wid)?;
    log(&format!("Found \"{}\".", details.title));

    log(&t("download-item-requesting-the-download-manifest"));
    let item = steam_cdn::fetch_manifest(wid)?;

    let out_dir = work_dir.join("content");
    log(&t("download-item-downloading-the-map"));
    let last_step = std::sync::atomic::AtomicU64::new(u64::MAX);
    steam_cdn::download(&item, &out_dir, &|done, total| {
        // one line per 10%, not one per chunk
        let step = if total == 0 { 10 } else { done * 10 / total };
        if last_step.swap(step, Ordering::Relaxed) != step {
            log(&format!(
                "{}% ({} of {})",
                step * 10,
                format_size(done),
                format_size(total)
            ));
        }
    })?;

    let map_file = find_map_file(&out_dir).ok_or("The download had no map file in it.")?;
    let preview = find_bundled_preview(&out_dir).or_else(|| {
        let fetched = work_dir.join("preview.jpg");
        download_preview(&details.preview_url, &fetched).then_some(fetched)
    });
    let sidecar = read_item_info(&out_dir);
    Ok(Downloaded {
        map_file,
        preview,
        details,
        sidecar,
    })
}

pub fn format_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

// search

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSort {
    Popular,
    Trending,
    Recent,
}

impl SearchSort {
    const ALL: [SearchSort; 3] = [SearchSort::Popular, SearchSort::Trending, SearchSort::Recent];

    fn api_value(self) -> &'static str {
        match self {
            SearchSort::Popular => "popular",
            SearchSort::Trending => "trending",
            SearchSort::Recent => "recent",
        }
    }

    fn label(self) -> String {
        match self {
            SearchSort::Popular => t("steam-search-sort-popular"),
            SearchSort::Trending => t("steam-search-sort-trending"),
            SearchSort::Recent => t("steam-search-sort-recent"),
        }
    }
}

/// one search result from the workshop api
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct SearchItem {
    pub wid: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub subscriptions: u64,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub preview_url: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub total: u64,
    /// pass back to get the next page; None on the last one
    pub next_cursor: Option<String>,
}

/// reads the workshop api's search reply, dropping anything without a valid
/// workshop id
pub fn parse_search(reply: &Value) -> Result<SearchPage, String> {
    let items: Vec<SearchItem> = reply
        .get("items")
        .and_then(Value::as_array)
        .ok_or("The search reply had no results list.")?
        .iter()
        .filter_map(|item| serde_json::from_value::<SearchItem>(item.clone()).ok())
        .filter(|item| parse_workshop_id(&item.wid).as_deref() == Some(item.wid.as_str()))
        .map(|mut item| {
            if item.title.trim().is_empty() {
                item.title = format!("Workshop Item {}", item.wid);
            }
            item
        })
        .collect();
    Ok(SearchPage {
        total: reply.get("total").and_then(Value::as_u64).unwrap_or(items.len() as u64),
        next_cursor: reply
            .get("next_cursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string),
        items,
    })
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(super) fn search(query: &str, sort: SearchSort, cursor: Option<&str>) -> Result<SearchPage, String> {
    let mut path = format!("/search?q={}&sort={}", encode(query.trim()), sort.api_value());
    if let Some(cursor) = cursor {
        path.push_str(&format!("&cursor={}", encode(cursor)));
    }
    parse_search(&steam_cdn::api_get(&path)?)
}

// ui

#[derive(Default)]
struct Shared {
    log: Vec<String>,
    finished: Option<Result<LocalMap, String>>,
    /// the finished map has been handed to the catalog
    delivered: bool,
    results: Vec<SearchItem>,
    total: u64,
    next_cursor: Option<String>,
    search_error: Option<String>,
}

pub struct SteamDownloader {
    input: String,
    query: String,
    sort: SearchSort,
    /// a search has been run, so the results area is shown
    searched: bool,
    busy: Arc<AtomicBool>,
    searching: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
}

impl Default for SteamDownloader {
    fn default() -> Self {
        Self {
            input: String::new(),
            query: String::new(),
            sort: SearchSort::Popular,
            searched: false,
            busy: Arc::new(AtomicBool::new(false)),
            searching: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }
}

impl SteamDownloader {
    /// runs a search; with `more`, the next page is added to the results
    fn search(&mut self, more: bool, ctx: &egui::Context) {
        if self.searching.swap(true, Ordering::Relaxed) {
            return;
        }
        self.searched = true;
        let cursor = if more {
            self.shared.lock().ok().and_then(|s| s.next_cursor.clone())
        } else {
            None
        };
        if let Ok(mut shared) = self.shared.lock() {
            shared.search_error = None;
            if !more {
                shared.results.clear();
                shared.total = 0;
                shared.next_cursor = None;
            }
        }
        let (query, sort) = (self.query.clone(), self.sort);
        let searching = self.searching.clone();
        let shared = self.shared.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let page = search(&query, sort, cursor.as_deref());
            if let Ok(mut shared) = shared.lock() {
                match page {
                    Ok(page) => {
                        for item in page.items {
                            if !shared.results.iter().any(|r| r.wid == item.wid) {
                                shared.results.push(item);
                            }
                        }
                        shared.total = page.total;
                        shared.next_cursor = page.next_cursor;
                    }
                    Err(error) => shared.search_error = Some(error),
                }
            }
            searching.store(false, Ordering::Relaxed);
            repaint.request_repaint();
        });
    }

    fn start(&self, wid: String, cache_dir: PathBuf, runtime_dir: PathBuf, ctx: &egui::Context) {
        if self.busy.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Ok(mut shared) = self.shared.lock() {
            shared.log.clear();
            shared.finished = None;
            shared.delivered = false;
        }
        let busy = self.busy.clone();
        let shared = self.shared.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let log = {
                let shared = shared.clone();
                let repaint = repaint.clone();
                move |line: &str| {
                    if let Ok(mut shared) = shared.lock() {
                        shared.log.push(line.to_string());
                        if shared.log.len() > MAX_LOG_LINES {
                            shared.log.remove(0);
                        }
                    }
                    repaint.request_repaint();
                }
            };
            let work_dir = std::env::temp_dir().join(format!("hebnix_workshop_{wid}"));
            let _ = std::fs::remove_dir_all(&work_dir);
            let result = std::fs::create_dir_all(&work_dir)
                .map_err(|e| e.to_string())
                .and_then(|_| download_item(&wid, &work_dir, &log))
                .and_then(|downloaded| {
                    log(&t("start-saving-the-map-its-details-and"));
                    let sidecar = downloaded.sidecar.as_ref();
                    // the author's own name from the map's info file beats
                    // steam's bare creator id
                    let author = sidecar
                        .map(|s| s.author.clone())
                        .filter(|a| !a.trim().is_empty())
                        .unwrap_or_else(|| {
                            if downloaded.details.creator.is_empty() {
                                String::new()
                            } else {
                                format!("Steam user {}", downloaded.details.creator)
                            }
                        });
                    let description = if downloaded.details.description.is_empty() {
                        sidecar.map(|s| s.description.clone()).unwrap_or_default()
                    } else {
                        downloaded.details.description.clone()
                    };
                    let map = import_map(
                        &cache_dir,
                        &runtime_dir,
                        &downloaded.map_file,
                        &ImportMeta {
                            name: downloaded.details.title.clone(),
                            author,
                            description,
                        },
                        downloaded.preview.as_deref(),
                    )?;
                    // the details also go next to the map as a workshop .vdf
                    write_item_vdf(&cache_dir, &map, &wid)?;
                    Ok(map)
                });
            let _ = std::fs::remove_dir_all(&work_dir);
            if let Ok(mut shared) = shared.lock() {
                shared.finished = Some(result);
            }
            busy.store(false, Ordering::Relaxed);
            repaint.request_repaint();
        });
    }

    fn render_search(&mut self, ui: &mut egui::Ui, busy: bool, start_with: &mut Option<String>) {
        let searching = self.searching.load(Ordering::Relaxed);
        let mut run_search = false;
        ui.horizontal_wrapped(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(t("steam-search-hint"))
                    .desired_width(260.0)
                    .char_limit(100),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run_search = true;
            }
            egui::ComboBox::from_id_salt("steam_search_sort")
                .selected_text(self.sort.label())
                .show_ui(ui, |ui| {
                    for sort in SearchSort::ALL {
                        if ui.selectable_value(&mut self.sort, sort, sort.label()).changed() {
                            run_search = true;
                        }
                    }
                });
            if ui
                .add_enabled(!searching, egui::Button::new(t("steam-search-button")))
                .clicked()
            {
                run_search = true;
            }
            if searching {
                ui.spinner();
            }
        });
        if run_search {
            self.search(false, ui.ctx());
        }
        if !self.searched {
            return;
        }

        let mut load_more = false;
        if let Ok(shared) = self.shared.lock() {
            if let Some(error) = &shared.search_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            if shared.results.is_empty() && !searching && shared.search_error.is_none() {
                ui.small(t("steam-search-no-results"));
            }
            if !shared.results.is_empty() {
                ui.small(t_args(
                    "steam-search-shown-of-total",
                    &[
                        ("shown", shared.results.len().to_string().into()),
                        ("total", shared.total.max(shared.results.len() as u64).to_string().into()),
                    ],
                ));
                egui::ScrollArea::vertical()
                    .id_salt("steam_search_results")
                    .max_height(300.0)
                    .show(ui, |ui| crate::panel::followed(ui, |ui| {
                        for item in &shared.results {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(!busy, egui::Button::new(t("render-download")))
                                    .clicked()
                                {
                                    *start_with = Some(item.wid.clone());
                                }
                                ui.strong(&item.title);
                                if item.file_size > 0 {
                                    ui.small(format_size(item.file_size));
                                }
                                if item.subscriptions > 0 {
                                    ui.small(t_args(
                                        "steam-search-subscribers",
                                        &[("count", item.subscriptions.to_string().into())],
                                    ));
                                }
                                ui.hyperlink_to(
                                    t("steam-search-steam-page"),
                                    format!("{STEAM_ITEM_URL}{}", item.wid),
                                );
                            });
                        }
                        if shared.next_cursor.is_some()
                            && ui
                                .add_enabled(!searching, egui::Button::new(t("steam-search-more")))
                                .clicked()
                        {
                            load_more = true;
                        }
                        crate::panel::follow_focus(ui);
                    }));
            }
        }
        if load_more {
            self.search(true, ui.ctx());
        }
    }

    /// draws the download section. returns the map once it has been
    /// downloaded and imported.
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        cache_dir: &Path,
        runtime_dir: &Path,
    ) -> Option<LocalMap> {
        let busy = self.busy.load(Ordering::Relaxed);

        ui.heading(t("render-download-from-the-steam-workshop"));
        ui.separator();
        ui.label(t("steam-download-intro"));
        ui.add_space(6.0);

        let mut start_with = None;
        self.render_search(ui, busy, &mut start_with);
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label(t("render-workshop-link-or-id"));
            ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .hint_text(t("render-https-steamcommunity-com-sharedfiles-fil"))
                    .desired_width(360.0),
            );
        });
        let wid = parse_workshop_id(&self.input);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(wid.is_some() && !busy, egui::Button::new(t("render-download-map")))
                .clicked()
            {
                start_with = wid.clone();
            }
            if busy {
                ui.spinner();
            } else if wid.is_none() && !self.input.trim().is_empty() {
                ui.small(t("render-that-doesn-t-look-like-a"));
            }
        });

        let mut imported = None;
        if let Ok(shared) = self.shared.lock() {
            match &shared.finished {
                Some(Ok(map)) => {
                    ui.colored_label(
                        egui::Color32::LIGHT_GREEN,
                        t_args(
                            "render-imported-map-find-it-under-browse",
                            &[("map", map.name.to_string().into())],
                        ),
                    );
                }
                Some(Err(error)) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                None => {}
            }
            if !shared.log.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("steam_download_log")
                    .max_height(140.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| crate::panel::followed(ui, |ui| {
                        for line in &shared.log {
                            ui.small(line);
                        }
                    }));
            }
        }
        // hand a finished map over exactly once
        if !busy {
            if let Ok(mut shared) = self.shared.lock() {
                if !shared.delivered {
                    if let Some(Ok(map)) = &shared.finished {
                        imported = Some(map.clone());
                        shared.delivered = true;
                    }
                }
            }
        }
        if let Some(wid) = start_with {
            self.start(
                wid,
                cache_dir.to_path_buf(),
                runtime_dir.to_path_buf(),
                ui.ctx(),
            );
        }
        imported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hebnix_steam_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, size: usize) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![1u8; size]).unwrap();
    }

    #[test]
    fn workshop_ids_come_from_numbers_and_links() {
        assert_eq!(
            parse_workshop_id("2968144588").as_deref(),
            Some("2968144588")
        );
        assert_eq!(
            parse_workshop_id(
                " https://steamcommunity.com/sharedfiles/filedetails/?id=2968144588&searchtext=x "
            )
            .as_deref(),
            Some("2968144588")
        );
        assert_eq!(
            parse_workshop_id("https://steamcommunity.com/workshop/filedetails/?l=english&id=42")
                .as_deref(),
            Some("42")
        );
        assert_eq!(parse_workshop_id(""), None);
        assert_eq!(parse_workshop_id("not a link"), None);
        assert_eq!(parse_workshop_id("12abc"), None);
    }

    #[test]
    fn only_rocket_league_items_are_accepted() {
        let reply = |app: Value, result: i64| {
            json!({ "response": { "publishedfiledetails": [{
                "result": result, "consumer_app_id": app, "title": "Cool Map",
                "creator": "7656", "description": "[h1]Hi[/h1] there [b]friend[/b]",
                "preview_url": "https://x/y.jpg"
            }]}})
        };
        let ok = parse_details(&reply(json!(252950), 1), "1").unwrap();
        assert_eq!(ok.title, "Cool Map");
        assert_eq!(ok.description, "Hi there friend");
        // steam sometimes sends ids as strings
        assert!(parse_details(&reply(json!("252950"), 1), "1").is_ok());
        assert!(
            parse_details(&reply(json!(730), 1), "1")
                .unwrap_err()
                .contains("Rocket League")
        );
        assert!(parse_details(&reply(json!(252950), 9), "1").is_err());
        assert!(parse_details(&json!({}), "1").is_err());
    }

    #[test]
    fn the_item_info_file_is_read_in_any_letter_case() {
        let info = parse_item_info(
            "\u{feff}{\"Title\":\"Cool Map\",\"AUTHOR\":\"Someone\",\"description\":\"[b]Fun[/b] map\"}",
        )
        .unwrap();
        assert_eq!(info.title, "Cool Map");
        assert_eq!(info.author, "Someone");
        assert_eq!(info.description, "Fun map");
        assert_eq!(parse_item_info("{}"), None);
        assert_eq!(parse_item_info("not json"), None);
    }

    #[test]
    fn the_item_info_file_is_found_inside_the_download() {
        let dir = temp_dir("iteminfo");
        write(&dir, "sub/Cool.upk", 100);
        std::fs::write(dir.join("sub/WorkshopItemInfo.json"), "{\"Author\":\"Me\"}").unwrap();
        assert_eq!(read_item_info(&dir).unwrap().author, "Me");
        assert_eq!(read_item_info(&temp_dir("noinfo")), None);
    }

    #[test]
    fn bbcode_is_stripped_but_plain_brackets_stay() {
        assert_eq!(
            strip_bbcode("[url=https://a.b]link[/url] and [i]x[/i]"),
            "link and x"
        );
        assert_eq!(strip_bbcode("scores [1, 2] here"), "scores [1, 2] here");
        assert_eq!(strip_bbcode("open [ bracket"), "open [ bracket");
    }

    #[test]
    fn the_map_is_the_biggest_real_file() {
        let dir = temp_dir("map");
        write(&dir, "MapTemplates.upk", 9_000);
        write(&dir, "EditorLandscapeResources.upk", 8_000);
        write(&dir, "WorkshopItemInfo.json", 5_000);
        write(&dir, "art/Preview.jpg", 20_000);
        write(&dir, "sub/Cool.upk", 3_000);
        write(&dir, "sub/notes.txt", 7_000);
        assert_eq!(
            find_map_file(&dir).unwrap().file_name().unwrap(),
            "Cool.upk"
        );
    }

    #[test]
    fn without_a_package_the_biggest_other_file_is_used() {
        let dir = temp_dir("fallback");
        write(&dir, "a.bin", 10);
        write(&dir, "b.bin", 500);
        assert_eq!(find_map_file(&dir).unwrap().file_name().unwrap(), "b.bin");
        assert_eq!(find_map_file(&temp_dir("empty")), None);
    }

    #[test]
    fn the_bundled_preview_prefers_its_name() {
        let dir = temp_dir("preview");
        write(&dir, "icon.png", 100);
        write(&dir, "big.png", 9_000);
        write(&dir, "MapPreview.jpg", 3_000);
        assert_eq!(
            find_bundled_preview(&dir).unwrap().file_name().unwrap(),
            "MapPreview.jpg"
        );
        assert_eq!(find_bundled_preview(&temp_dir("nopic")), None);
    }

    #[test]
    fn search_results_need_a_real_workshop_id() {
        let reply = json!({
            "total": 40,
            "next_cursor": "AoJ4",
            "items": [
                {"wid": "1759763738", "title": "Speed Jump: Trials 3", "file_size": 59224775, "subscriptions": 120000},
                {"wid": "../etc", "title": "bad"},
                {"wid": "42", "title": "  "},
                {"title": "no id"}
            ]
        });
        let page = parse_search(&reply).unwrap();
        assert_eq!(page.total, 40);
        assert_eq!(page.next_cursor.as_deref(), Some("AoJ4"));
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].file_size, 59224775);
        assert_eq!(page.items[1].title, "Workshop Item 42");
        assert!(parse_search(&json!({})).is_err());
        let last = parse_search(&json!({"items": [], "next_cursor": ""})).unwrap();
        assert_eq!(last.next_cursor, None);
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(format_size(59_224_775), "56 MB");
        assert_eq!(format_size(3 * 1024 * 1024 / 2), "1.5 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024 / 2), "2.5 GB");
    }

    /// the whole search + download path against a real workshop api and
    /// steam's CDN. run with
    /// `HEBNIX_WORKSHOP_API_URL=http://127.0.0.1:8798 HEBNIX_TEST_WID=1759763738
    /// cargo test -p hebnix-app --bin hebnix -- --ignored downloads_a_map_end_to_end`
    #[test]
    #[ignore = "needs a workshop api and network access"]
    fn downloads_a_map_end_to_end() {
        let wid = std::env::var("HEBNIX_TEST_WID").expect("set HEBNIX_TEST_WID");
        let page = search("speed jump", SearchSort::Popular, None).unwrap();
        assert!(!page.items.is_empty());
        let dir = temp_dir("end_to_end");
        let log = |line: &str| eprintln!("{line}");
        let downloaded = download_item(&wid, &dir, &log).unwrap();
        let size = std::fs::metadata(&downloaded.map_file).unwrap().len();
        eprintln!("map {} ({size} bytes)", downloaded.map_file.display());
        assert!(size > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
