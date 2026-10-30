use crate::i18n::{t, t_args};
use crate::messages::AppMsg;
use crate::patcher::backup_guard;
use crate::patcher::painted_swap::{self, SwapPaint};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod browser;
mod item_name;
mod thumbnail_patcher;
#[cfg(test)]
mod browser_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwapCategory {
    Antennas,
    Anthems,
    Borders,
    Bodies,
    Boosts,
    Skins,
    Engines,
    Goals,
    Finishes,
    Banners,
    Toppers,
    Trails,
    Wheels,
}

impl SwapCategory {
    pub const ALL: [Self; 13] = [
        Self::Antennas,
        Self::Anthems,
        Self::Borders,
        Self::Bodies,
        Self::Boosts,
        Self::Skins,
        Self::Engines,
        Self::Goals,
        Self::Finishes,
        Self::Banners,
        Self::Toppers,
        Self::Trails,
        Self::Wheels,
    ];

    pub fn label(self) -> String {
        match self {
            Self::Antennas => t("label-antennas"),
            Self::Anthems => t("label-anthems"),
            Self::Borders => t("label-borders"),
            Self::Bodies => t("label-bodies"),
            Self::Boosts => t("label-boosts"),
            Self::Engines => t("label-engines"),
            Self::Goals => t("label-goals"),
            Self::Finishes => t("label-finishes"),
            Self::Banners => t("label-banners"),
            Self::Skins => t("label-decals"),
            Self::Toppers => t("label-toppers"),
            Self::Trails => t("label-trails"),
            Self::Wheels => t("label-wheels"),
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Antennas => "antennas",
            Self::Anthems => "anthems",
            Self::Borders => "borders",
            Self::Bodies => "bodies",
            Self::Boosts => "boosts",
            Self::Engines => "engines",
            Self::Goals => "goals",
            Self::Finishes => "finishes",
            Self::Banners => "banners",
            Self::Skins => "skins",
            Self::Toppers => "toppers",
            Self::Trails => "trails",
            Self::Wheels => "wheels",
        }
    }

    fn from_slug(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.slug().eq_ignore_ascii_case(value))
    }

    /// Catalogue `paintable` describes official inventory variants, not whether
    /// a visual asset can accept a local material colour override.
    pub fn supports_recolour(self) -> bool {
        !matches!(self, Self::Engines | Self::Anthems)
    }
}

#[derive(Clone)]
struct SwapItem {
    name: String,
    upk: String,
    path: Option<String>,
    thumbnail: Option<String>,
    /// Associated car image for engine cards; never an installation destination.
    body_preview: Option<String>,
    audio_bnk: Option<String>,
    upk_type: String,
    product_id: Option<i64>,
    car_key: Option<String>,
    car_name: Option<String>,
    car_product_id: Option<i64>,
    shared_package: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct ActiveSwap {
    category: String,
    source_name: String,
    source_upk: String,
    target_name: String,
    target_upk: String,
    #[serde(default)]
    target_bnk: Option<String>,
    #[serde(default)]
    paint: SwapPaint,
    #[serde(default)]
    target_thumbnail: Option<String>,
    #[serde(default)]
    source_asset: Option<String>,
    #[serde(default)]
    target_asset: Option<String>,
    #[serde(default)]
    asset_override: bool,
}

fn item_identity(item: &SwapItem) -> String {
    format!(
        "{}|{}",
        item.upk.to_ascii_lowercase(),
        item.path
            .as_deref()
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| item
                .product_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| item.name.to_ascii_lowercase()))
    )
}

fn can_recolour_owned(category: SwapCategory, item: &SwapItem, owned_ids: &HashSet<i64>) -> bool {
    category.supports_recolour() && item.product_id.is_some_and(|id| owned_ids.contains(&id))
}
fn thumbnail_kind(category: SwapCategory, item: &SwapItem) -> String {
    if category == SwapCategory::Skins
        && let Some(asset) = item.path.as_deref()
    {
        format!("skins|{asset}")
    } else {
        category.slug().into()
    }
}
fn thumbnail_key(category: SwapCategory, filename: &str, item: &SwapItem) -> String {
    format!(
        "{}|{}",
        thumbnail_kind(category, item).to_ascii_lowercase(),
        filename.to_ascii_lowercase()
    )
}
impl ActiveSwap {
    fn key(&self) -> String {
        if self.asset_override {
            format!(
                "{}|{}",
                self.target_upk.to_ascii_lowercase(),
                self.target_asset
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
            )
        } else {
            self.target_upk.clone()
        }
    }
    fn source_matches(&self, item: &SwapItem) -> bool {
        self.source_upk.eq_ignore_ascii_case(&item.upk)
            && self
                .source_asset
                .as_deref()
                .map(|asset| {
                    item.path
                        .as_deref()
                        .is_some_and(|path| asset.eq_ignore_ascii_case(path))
                })
                .unwrap_or_else(|| {
                    !item.shared_package || self.source_name.eq_ignore_ascii_case(&item.name)
                })
    }
    fn target_matches(&self, item: &SwapItem) -> bool {
        self.target_upk.eq_ignore_ascii_case(&item.upk)
            && self
                .target_asset
                .as_deref()
                .map(|asset| {
                    item.path
                        .as_deref()
                        .is_some_and(|path| asset.eq_ignore_ascii_case(path))
                })
                .unwrap_or_else(|| {
                    !item.shared_package || self.target_name.eq_ignore_ascii_case(&item.name)
                })
    }
}

/// One BakkesMod item: the swap category, the product equipped in the stock loadout,
/// the BakkesMod product to show instead, and its paint (0 = none).
pub struct SwapPick {
    pub category: SwapCategory,
    pub stock_product: i64,
    pub product: i64,
    pub paint: u8,
}

/// BakkesMod equip slot to swap category. Body is not applied by BakkesMod, and the
/// finish slots have no swap category here, so those return `None`.
pub fn category_for_equip_slot(slot: u8) -> Option<SwapCategory> {
    Some(match slot {
        1 => SwapCategory::Skins,
        2 => SwapCategory::Wheels,
        3 => SwapCategory::Boosts,
        4 => SwapCategory::Antennas,
        5 => SwapCategory::Toppers,
        13 => SwapCategory::Engines,
        14 => SwapCategory::Trails,
        15 => SwapCategory::Goals,
        18 => SwapCategory::Anthems,
        _ => return None,
    })
}

#[derive(Default)]
pub struct PresetApplyReport {
    pub applied: usize,
    pub skipped: Vec<String>,
    pub skipped_indexes: Vec<usize>,
}

fn patch_boost_bnk(source: &Path, target_backup: &Path, destination: &Path) -> Result<(), String> {
    let mut donor = fs::read(source).map_err(|error| format!("{}: {error}", source.display()))?;
    let target =
        fs::read(target_backup).map_err(|error| format!("{}: {error}", target_backup.display()))?;
    if donor.len() < 16 || target.len() < 16 || &donor[..4] != b"BKHD" || &target[..4] != b"BKHD" {
        return Err("Unsupported or truncated Wwise boost audio bank".into());
    }
    donor[12..16].copy_from_slice(&target[12..16]);
    let temporary = destination.with_extension("bnk.swapping.tmp");
    fs::write(&temporary, donor)
        .map_err(|error| format!("Failed to write {}: {error}", temporary.display()))?;
    fs::copy(&temporary, destination)
        .map_err(|error| format!("Failed to install {}: {error}", destination.display()))?;
    let _ = fs::remove_file(temporary);
    Ok(())
}

fn swap_compatible(category: SwapCategory, source: &SwapItem, target: &SwapItem) -> bool {
    if category != SwapCategory::Goals {
        return true;
    }
    match target.upk_type.as_str() {
        "2parts" => source.upk_type == "2parts",
        "3parts" => source.upk_type == "3parts" && source.upk.eq_ignore_ascii_case(&target.upk),
        _ => matches!(source.upk_type.as_str(), "simple" | "2parts"),
    }
}

fn inferred_thumbnail(category: SwapCategory, item: &SwapItem, cooked_pc: &Path) -> Option<String> {
    if let Some(name) = item.thumbnail.as_ref().filter(|name| !name.is_empty()) {
        return cooked_pc.join(name).is_file().then(|| name.clone());
    }
    if matches!(category, SwapCategory::Boosts | SwapCategory::Goals)
        && (category != SwapCategory::Goals || item.upk_type == "simple")
    {
        let lower = item.upk.to_ascii_lowercase();
        if lower.ends_with("_sf.upk") {
            let candidate = format!("{}_T_SF.upk", &item.upk[..item.upk.len() - 7]);
            return cooked_pc.join(&candidate).is_file().then_some(candidate);
        }
    }
    None
}

fn preview_thumbnail(category: SwapCategory, item: &SwapItem, cooked_pc: &Path) -> Option<String> {
    if category == SwapCategory::Engines {
        if let Some(name) = item.body_preview.as_ref().filter(|name| {
            cooked_pc.join(name).is_file()
                || cooked_pc
                    .join("Backups")
                    .join(format!("{name}.bak"))
                    .is_file()
        }) {
            return Some(name.clone());
        }
    }
    inferred_thumbnail(category, item, cooked_pc)
}

// Preview availability must never determine whether an installed item is listed.
fn source_upks_available(_category: SwapCategory, item: &SwapItem, cooked_pc: &Path) -> bool {
    cooked_pc.join(&item.upk).is_file()
}
fn prettify_car_key(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            let mut chars = lower.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const CARD_NAME_LIMIT: usize = 30;

fn shorten_for_card(text: &str) -> String {
    let mut chars = text.chars();
    let shortened: String = chars.by_ref().take(CARD_NAME_LIMIT).collect();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn normalized_label(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_universal_car(name: &str) -> bool {
    matches!(
        normalized_label(name).as_str(),
        "universal" | "universalcar"
    )
}

fn item_label(category: SwapCategory, item: &SwapItem) -> String {
    if category != SwapCategory::Skins {
        return item.name.clone();
    }

    let car_name = item.car_name.as_deref().unwrap_or("Unknown car");
    let decal_already_names_car = item
        .name
        .split_once(':')
        .is_some_and(|(prefix, _)| normalized_label(prefix) == normalized_label(car_name));
    if decal_already_names_car {
        item.name.clone()
    } else {
        format!("{car_name} · {}", item.name)
    }
}

#[derive(Clone)]
struct ResolvedItem {
    available: bool,
    thumbnail: Option<String>,
}

struct Resolution {
    path: PathBuf,
    generation: u64,
    items: Option<Arc<Vec<ResolvedItem>>>,
    cars: Option<Arc<Vec<(String, String, Option<i64>)>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkthroughStep {
    SelectSource,
    SelectTarget,
}

pub struct SwapperState {
    base_dir: PathBuf,
    catalogs: HashMap<SwapCategory, Arc<Vec<SwapItem>>>,
    errors: HashMap<SwapCategory, String>,
    target_index: HashMap<String, usize>,
    target_search: HashMap<String, String>,
    selected_car: Option<String>,
    match_swapped_item: bool,
    car_search: String,
    search_input: HashMap<SwapCategory, String>,
    page: HashMap<SwapCategory, usize>,
    active: Vec<ActiveSwap>,
    view_patched: bool,
    walkthrough_mode: bool,
    walkthrough_step: WalkthroughStep,
    walkthrough_category: Option<SwapCategory>,
    walkthrough_source: Option<usize>,
    walkthrough_source_car: Option<String>,
    walkthrough_target: Option<usize>,
    walkthrough_search: HashMap<SwapCategory, String>,
    walkthrough_page: HashMap<SwapCategory, usize>,
    walkthrough_applied_at: Option<Instant>,
    owned_only: bool,
    thumbnails: HashMap<String, Option<Arc<[u8]>>>,
    failed_thumbnails: HashMap<String, String>,
    thumbnail_jobs: Sender<(String, PathBuf, String, egui::Context)>,
    thumbnail_results: Receiver<(String, Result<Arc<[u8]>, String>)>,
    resolution_jobs: Sender<(SwapCategory, PathBuf, Vec<SwapItem>, u64, egui::Context)>,
    resolution_results: Receiver<(
        SwapCategory,
        u64,
        Vec<ResolvedItem>,
        Vec<(String, String, Option<i64>)>,
    )>,
    resolutions: HashMap<SwapCategory, Resolution>,
    resolution_generation: u64,
    swap_paint: HashMap<String, SwapPaint>,
    recolour_category: SwapCategory,
    recolour_search: String,
    recolour_selected: Option<String>,
    recolour_paint: SwapPaint,
    recolour_page: usize,
    recolour_show_applied: bool,
    recolour_status: Option<Result<String, String>>,
    swap_speed: HashMap<String, f32>,
    /// set from the Items animation speed tab; shows the speed picker on swap rows
    pub speed_enabled: bool,
    thumbnail_editor: thumbnail_patcher::State,
    name_editor: item_name::State,
}

impl SwapperState {
    pub fn new(base_dir: &Path) -> Self {
        let (thumbnail_jobs, jobs) = unbounded::<(String, PathBuf, String, egui::Context)>();
        let (results, thumbnail_results) = unbounded();
        std::thread::spawn(move || {
            while let Ok((key, path, category, context)) = jobs.recv() {
                let result = if let Some(kind) = category.strip_prefix("raw|") {
                    crate::cosmetic_thumbnail::extract_raw_png(&path, kind)
                } else {
                    crate::cosmetic_thumbnail::extract_png(&path, &category)
                }.map(Into::into);
                let _ = results.send((key, result));
                context.request_repaint();
            }
        });
        let (resolution_jobs, resolution_requests) =
            unbounded::<(SwapCategory, PathBuf, Vec<SwapItem>, u64, egui::Context)>();
        let (resolution_sender, resolution_results) = unbounded();
        std::thread::spawn(move || {
            while let Ok((category, cooked_pc, items, generation, context)) =
                resolution_requests.recv()
            {
                let resolved = items
                    .iter()
                    .map(|item| ResolvedItem {
                        available: source_upks_available(category, item, &cooked_pc),
                        thumbnail: preview_thumbnail(category, item, &cooked_pc),
                    })
                    .collect();
                let mut cars = if category == SwapCategory::Skins {
                    items
                        .iter()
                        .filter_map(|item| {
                            Some((
                                item.car_key.clone()?,
                                item.car_name.clone()?,
                                item.car_product_id,
                            ))
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                cars.sort_by(|left, right| {
                    let left_universal = is_universal_car(&left.1);
                    let right_universal = is_universal_car(&right.1);
                    right_universal.cmp(&left_universal).then_with(|| {
                        left.1
                            .to_ascii_lowercase()
                            .cmp(&right.1.to_ascii_lowercase())
                    })
                });
                cars.dedup_by(|left, right| left.0 == right.0);
                if resolution_sender
                    .send((category, generation, resolved, cars))
                    .is_err()
                {
                    break;
                }
                context.request_repaint();
            }
        });
        Self {
            base_dir: base_dir.to_path_buf(),
            catalogs: HashMap::new(),
            errors: HashMap::new(),
            target_index: HashMap::new(),
            target_search: HashMap::new(),
            selected_car: None,
            match_swapped_item: true,
            car_search: String::new(),
            search_input: HashMap::new(),
            page: HashMap::new(),
            active: Vec::new(),
            view_patched: false,
            walkthrough_mode: true,
            walkthrough_step: WalkthroughStep::SelectSource,
            walkthrough_category: None,
            walkthrough_source: None,
            walkthrough_source_car: None,
            walkthrough_target: None,
            walkthrough_search: HashMap::new(),
            walkthrough_page: HashMap::new(),
            walkthrough_applied_at: None,
            owned_only: false,
            thumbnails: HashMap::new(),
            failed_thumbnails: HashMap::new(),
            thumbnail_jobs,
            thumbnail_results,
            resolution_jobs,
            resolution_results,
            resolutions: HashMap::new(),
            resolution_generation: 0,
            swap_paint: HashMap::new(),
            recolour_category: SwapCategory::Bodies,
            recolour_search: String::new(),
            recolour_selected: None,
            recolour_paint: SwapPaint::Default,
            recolour_page: 0,
            recolour_show_applied: false,
            recolour_status: None,
            swap_speed: HashMap::new(),
            speed_enabled: false,
            thumbnail_editor: thumbnail_patcher::State::default(),
            name_editor: item_name::State::default(),
        }
    }

    fn collect_thumbnails(&mut self) {
        while let Ok((key, result)) = self.thumbnail_results.try_recv() {
            match result {
                Ok(png) => {
                    self.thumbnails.insert(key, Some(png));
                }
                Err(error) => {
                    self.failed_thumbnails.insert(key.clone(), error);
                    self.thumbnails.insert(key, None);
                }
            }
        }
    }

    fn thumbnail_status(&mut self, ui: &mut egui::Ui, category: SwapCategory) {
        let prefix = format!("{}|", category.slug());
        let failures: Vec<_> = self
            .failed_thumbnails
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect();
        if failures.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.weak(t_args(
                "thumbnail-status-failures-previews-unavailable-items-rema",
                &[("failures", (failures.len()).to_string().into())],
            ));
            if ui
                .small_button(t("thumbnail-status-retry-previews"))
                .clicked()
            {
                for key in failures {
                    self.failed_thumbnails.remove(&key);
                    self.thumbnails.remove(&key);
                }
            }
        });
    }

    fn queue_thumbnail(
        &mut self,
        ui: &egui::Ui,
        category: SwapCategory,
        filename: &str,
        cooked_pc: &Path,
        item: &SwapItem,
    ) {
        let key = thumbnail_key(category, filename, item);
        if !self.thumbnails.contains_key(&key) {
            self.thumbnails.insert(key.clone(), None);
            let _ = self.thumbnail_jobs.send((
                key,
                {
                    let backup = cooked_pc.join("Backups").join(format!("{filename}.bak"));
                    if backup.is_file() {
                        backup
                    } else {
                        cooked_pc.join(filename)
                    }
                },
                thumbnail_kind(category, item),
                ui.ctx().clone(),
            ));
        }
    }

    fn resolved_items(
        &mut self,
        ui: &mut egui::Ui,
        category: SwapCategory,
        cooked_pc: &Path,
        items: &[SwapItem],
    ) -> Option<Arc<Vec<ResolvedItem>>> {
        while let Ok((ready_category, generation, resolved, cars)) =
            self.resolution_results.try_recv()
        {
            if let Some(entry) = self.resolutions.get_mut(&ready_category) {
                if entry.generation == generation {
                    entry.items = Some(Arc::new(resolved));
                    entry.cars = Some(Arc::new(cars));
                }
            }
        }
        if self
            .resolutions
            .get(&category)
            .is_none_or(|entry| entry.path != cooked_pc)
        {
            self.resolution_generation += 1;
            let generation = self.resolution_generation;
            self.resolutions.insert(
                category,
                Resolution {
                    path: cooked_pc.to_path_buf(),
                    generation,
                    items: None,
                    cars: None,
                },
            );
            let _ = self.resolution_jobs.send((
                category,
                cooked_pc.to_path_buf(),
                items.to_vec(),
                generation,
                ui.ctx().clone(),
            ));
        }
        let resolved = self
            .resolutions
            .get(&category)
            .and_then(|entry| entry.items.clone());
        if resolved.is_none() {
            ui.weak(t("resolved-items-preparing-items"));
        }
        resolved
    }

    pub fn owned_only(&self) -> bool {
        self.owned_only
    }

    pub fn set_owned_only(&mut self, enabled: bool) {
        self.owned_only = enabled;
    }

    pub fn set_catalogs(&mut self, catalogs: &HashMap<String, Value>) -> Result<(), String> {
        let bodies = catalogs
            .get(SwapCategory::Bodies.slug())
            .ok_or_else(|| "The bodies catalog was not downloaded".to_string())?;
        let mut parsed = HashMap::new();
        for category in SwapCategory::ALL {
            let root = catalogs
                .get(category.slug())
                .ok_or_else(|| format!("The {} catalog was not downloaded", category.slug()))?;
            parsed.insert(
                category,
                Arc::new(Self::parse_catalog(category, root, bodies)?),
            );
        }
        self.catalogs = parsed;
        self.errors.clear();
        self.thumbnails.clear();
        self.failed_thumbnails.clear();
        self.resolutions.clear();
        Ok(())
    }

    fn parse_catalog(
        category: SwapCategory,
        root: &Value,
        bodies: &Value,
    ) -> Result<Vec<SwapItem>, String> {
        let mut items = Vec::new();
        if category == SwapCategory::Skins {
            let body_ids = bodies
                .get("bodies")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|body| {
                    let name = body.get("name")?.as_str()?.to_ascii_lowercase();
                    let id = body.get("id").and_then(|id| {
                        id.as_i64()
                            .or_else(|| id.as_str().and_then(|text| text.parse().ok()))
                    })?;
                    Some((name, id))
                })
                .collect::<HashMap<_, _>>();
            if let Some(cars) = root.get("cars").and_then(Value::as_object) {
                for (car_name, car) in cars {
                    if let Some(skins) = car.get("skins").and_then(Value::as_array) {
                        // The `skins` array can include universal decals whose display name
                        // names another body (for example, OCTANE contains "Hakkaa:
                        // Glitched"). Inferring the body from the first `Car: Decal` entry
                        // therefore mislabeled, and effectively hid, OCTANE. The object key
                        // is the catalog's authoritative body identifier.
                        let display_name = prettify_car_key(car_name);
                        let car_product_id =
                            body_ids.get(&display_name.to_ascii_lowercase()).copied();
                        for skin in skins {
                            Self::push_item(
                                &mut items,
                                skin,
                                Some((car_name, &display_name, car_product_id)),
                            );
                        }
                    }
                }
            }
        } else if let Some(array) = root.get(category.slug()).and_then(Value::as_array) {
            for value in array {
                Self::push_item(&mut items, value, None);
            }
        }
        let mut seen = HashSet::new();
        if category == SwapCategory::Engines {
            for item in &mut items {
                item.body_preview = crate::cosmetic_thumbnail::body_preview_for_engine(
                    &item.name, &item.upk, bodies,
                );
            }
        }
        items.retain(|item| {
            seen.insert(if category == SwapCategory::Skins {
                format!(
                    "{}|{}",
                    item_identity(item),
                    item.car_key.as_deref().unwrap_or("")
                )
            } else {
                item.upk.to_ascii_lowercase()
            })
        });
        if category == SwapCategory::Skins {
            let mut packages: HashMap<String, HashSet<String>> = HashMap::new();
            for item in &items {
                packages
                    .entry(item.upk.to_ascii_lowercase())
                    .or_default()
                    .insert(item_identity(item));
            }
            for item in &mut items {
                item.shared_package = packages[&item.upk.to_ascii_lowercase()].len() > 1;
            }
        }
        items.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
        if items.is_empty() {
            Err(format!(
                "No swappable UPKs found in the {} API catalog",
                category.slug()
            ))
        } else {
            Ok(items)
        }
    }

    fn push_item(items: &mut Vec<SwapItem>, value: &Value, car: Option<(&str, &str, Option<i64>)>) {
        let Some(name) = value.get("name").and_then(Value::as_str) else {
            return;
        };
        let Some(upk) = value.get("upk_path").and_then(Value::as_str) else {
            return;
        };
        if !upk.to_ascii_lowercase().ends_with(".upk") {
            return;
        }
        items.push(SwapItem {
            name: name.to_string(),
            upk: upk.to_string(),
            path: value
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_string),
            thumbnail: value
                .get("thumbnail")
                .and_then(Value::as_str)
                .map(str::to_string),
            body_preview: None,
            audio_bnk: value
                .get("audio_bnk")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            upk_type: value
                .get("upk_type")
                .and_then(Value::as_str)
                .unwrap_or("simple")
                .to_string(),
            product_id: value.get("id").and_then(|id| {
                id.as_i64()
                    .or_else(|| id.as_str().and_then(|text| text.parse().ok()))
            }),
            car_key: car.map(|(key, _, _)| key.to_string()),
            car_name: car.map(|(_, name, _)| name.to_string()),
            car_product_id: car.and_then(|(_, _, id)| id),
            shared_package: false,
        });
    }

    fn manifest_path(backups_dir: &Path) -> PathBuf {
        backups_dir.join("swapper_swaps.json")
    }

    fn load_active(&mut self, backups_dir: &Path) {
        self.active = fs::read(Self::manifest_path(backups_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        self.active.retain(|swap| {
            backups_dir
                .join(format!("{}.bak", swap.target_upk))
                .is_file()
        });
    }

    fn save_active(&self, backups_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(backups_dir).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(&self.active).map_err(|error| error.to_string())?;
        fs::write(Self::manifest_path(backups_dir), bytes).map_err(|error| error.to_string())
    }

    fn apply_swap(
        &mut self,
        category: SwapCategory,
        source: &SwapItem,
        target: &SwapItem,
        paint: SwapPaint,
        speed: f32,
        cooked_pc: &Path,
        backups_dir: &Path,
    ) -> Result<(), String> {
        backup_guard::synchronize_install(cooked_pc, backups_dir)?;
        if target.shared_package
            && self.active.iter().any(|swap| {
                !swap.asset_override && swap.target_upk.eq_ignore_ascii_case(&target.upk)
            })
        {
            self.restore_swap(&target.upk, cooked_pc, backups_dir)?;
        }
        let same_item = item_identity(source) == item_identity(target);
        if same_item && paint == SwapPaint::Default {
            return Err("Choose two different items".into());
        }
        if paint != SwapPaint::Default && !category.supports_recolour() {
            return Err("This item category has no visual colours to change".into());
        }
        if !swap_compatible(category, source, target) {
            return Err(format!(
                "{} ({}) is not compatible with the {} target layout ({})",
                source.name, source.upk_type, target.name, target.upk_type
            ));
        }
        fs::create_dir_all(backups_dir).map_err(|error| error.to_string())?;
        let source_live = cooked_pc.join(&source.upk);
        let target_live = cooked_pc.join(&target.upk);
        let source_backup = backups_dir.join(format!("{}.bak", source.upk));
        let target_backup = backups_dir.join(format!("{}.bak", target.upk));
        // A source may already be replaced in this install.  Its .bak is the
        // pristine donor and is sufficient even when the live file is absent.
        if !source_live.is_file() && !source_backup.is_file() {
            return Err(format!("Source UPK not found: {}", source_live.display()));
        }
        if !target_live.is_file() && !target_backup.is_file() {
            return Err(format!("Target UPK not found: {}", target_live.display()));
        }
        if !target_backup.exists() {
            fs::copy(&target_live, &target_backup)
                .map_err(|error| format!("Failed to back up {}: {error}", target.upk))?;
        }
        // If the source is itself an active target, copy its pristine backup.
        let copy_source = if source_backup.is_file() {
            &source_backup
        } else {
            &source_live
        };
        let asset_override = category == SwapCategory::Skins && target.shared_package;
        if asset_override {
            let source_asset = source
                .path
                .as_deref()
                .ok_or("This decal has no catalog asset path")?;
            let target_asset = target
                .path
                .as_deref()
                .ok_or("This shared decal has no catalog asset path")?;
            let (carrier, asset) = super::decal_variant::make_carrier(
                copy_source,
                source_asset,
                target_asset,
                cooked_pc,
                &self.base_dir,
            )?;
            painted_swap::bake(&cooked_pc.join(&carrier), &source.upk, paint)?;
            if crate::speed_patch::is_active(speed) {
                crate::speed_patch::apply(&cooked_pc.join(&carrier), None, speed)?;
            }
            super::decal_variant::link_product(
                &cooked_pc.join(carrier),
                &asset,
                &target_live,
                &target_backup,
                target_asset,
                &self.base_dir,
            )?;
        } else if same_item {
            painted_swap::recolour_for_target(copy_source, &target_live, &source.upk, paint)?;
        } else {
            painted_swap::patch_for_target(
                copy_source,
                &target_backup,
                &target_live,
                &self.base_dir,
                source.path.as_deref(),
                target.path.as_deref(),
                &source.upk,
                paint,
            )
            .map_err(|error| {
                format!("Failed to patch {} for {}: {error}", source.upk, target.upk)
            })?;
        }
        if !asset_override && crate::speed_patch::is_active(speed) {
            match crate::speed_patch::apply(&target_live, Some(&target_backup), speed) {
                Ok(0) => {}
                Ok(count) => {
                    tracing::info!("[Speed] {}: scaled {count} animation values", target.upk)
                }
                Err(error) => tracing::warn!("[Speed] {}: {error}", target.upk),
            }
        }
        let mut target_bnk = None;
        if category == SwapCategory::Boosts && !same_item {
            if let (Some(source_name), Some(target_name)) =
                (source.audio_bnk.as_deref(), target.audio_bnk.as_deref())
            {
                let source_live = cooked_pc.join(source_name);
                let target_live = cooked_pc.join(target_name);
                let target_backup = backups_dir.join(format!("{target_name}.bak"));
                if !source_live.is_file() || !target_live.is_file() {
                    let _ = fs::copy(
                        backups_dir.join(format!("{}.bak", target.upk)),
                        cooked_pc.join(&target.upk),
                    );
                    return Err(format!(
                        "Boost audio bank missing: {} or {}",
                        source_live.display(),
                        target_live.display()
                    ));
                }
                if !target_backup.is_file() {
                    fs::copy(&target_live, &target_backup)
                        .map_err(|error| format!("Failed to back up {target_name}: {error}"))?;
                }
                if let Err(error) = patch_boost_bnk(&source_live, &target_backup, &target_live) {
                    let _ = fs::copy(
                        backups_dir.join(format!("{}.bak", target.upk)),
                        cooked_pc.join(&target.upk),
                    );
                    return Err(error);
                }
                target_bnk = Some(target_name.to_string());
            }
        }
        let mut target_thumbnail = None;
        if !asset_override
            && let (Some(source_name), Some(target_name)) = (
                inferred_thumbnail(category, source, cooked_pc),
                inferred_thumbnail(category, target, cooked_pc),
            )
        {
            if !source_name.eq_ignore_ascii_case(&target_name) {
                let source_live = cooked_pc.join(&source_name);
                let target_live = cooked_pc.join(&target_name);
                let source_backup = backups_dir.join(format!("{source_name}.bak"));
                let target_backup = backups_dir.join(format!("{target_name}.bak"));
                if !target_backup.is_file() {
                    fs::copy(&target_live, &target_backup).map_err(|error| {
                        format!("Failed to back up thumbnail {target_name}: {error}")
                    })?;
                }
                let thumbnail_source = if source_backup.is_file() {
                    &source_backup
                } else {
                    &source_live
                };
                let thumbnail_result = (|| {
                    let donor_asset = crate::cosmetic_upk::thumbnail_asset_path(
                        thumbnail_source,
                        &self.base_dir,
                        source.path.as_deref(),
                    )?;
                    let target_asset = crate::cosmetic_upk::thumbnail_asset_path(
                        &target_backup,
                        &self.base_dir,
                        target.path.as_deref(),
                    )?;
                    crate::cosmetic_upk::patch_for_target(
                        thumbnail_source,
                        &target_backup,
                        &target_live,
                        &self.base_dir,
                        Some(&donor_asset),
                        Some(&target_asset),
                    )
                })();
                if let Err(error) = thumbnail_result {
                    let _ = fs::copy(
                        backups_dir.join(format!("{}.bak", target.upk)),
                        cooked_pc.join(&target.upk),
                    );
                    if let Some(target_bnk) = target_bnk.as_deref() {
                        let _ = fs::copy(
                            backups_dir.join(format!("{target_bnk}.bak")),
                            cooked_pc.join(target_bnk),
                        );
                    }
                    let _ = fs::copy(&target_backup, &target_live);
                    return Err(format!("Thumbnail patch failed: {error}"));
                }
                target_thumbnail = Some(target_name);
            }
        }
        if same_item {
            // Recolouring replaces the previous visual swap with this item's
            // own asset. Restore its original audio/preview too, retaining the
            // backup tracking until Restore cleans up the whole change.
            if let Some(previous) = self.active.iter().find(|swap| swap.target_matches(target)) {
                for (name, tracked) in [
                    (&previous.target_bnk, &mut target_bnk),
                    (&previous.target_thumbnail, &mut target_thumbnail),
                ] {
                    if let Some(name) = name {
                        fs::copy(backups_dir.join(format!("{name}.bak")), cooked_pc.join(name))
                            .map_err(|error| format!("Failed to restore {name}: {error}"))?;
                        *tracked = Some(name.clone());
                    }
                }
            }
        }
        self.active.retain(|swap| !swap.target_matches(target));
        self.active.push(ActiveSwap {
            category: category.slug().to_string(),
            source_name: source.name.clone(),
            source_upk: source.upk.clone(),
            target_name: target.name.clone(),
            target_upk: target.upk.clone(),
            target_bnk,
            paint,
            target_thumbnail,
            source_asset: source.path.clone(),
            target_asset: target.path.clone(),
            asset_override,
        });
        self.save_active(backups_dir)
    }

    fn restore_all(
        &mut self,
        category: SwapCategory,
        cooked_pc: &Path,
        backups_dir: &Path,
    ) -> Result<usize, String> {
        backup_guard::check_install(cooked_pc, backups_dir)?;
        self.load_active(backups_dir);
        let keys: Vec<_> = self
            .active
            .iter()
            .filter(|swap| swap.category == category.slug())
            .map(ActiveSwap::key)
            .collect();
        let mut restored = 0;
        let mut errors = Vec::new();
        for key in keys {
            match self.restore_swap(&key, cooked_pc, backups_dir) {
                Ok(()) => restored += 1,
                Err(error) => errors.push(error),
            }
        }
        if errors.is_empty() {
            Ok(restored)
        } else {
            Err(format!("Restored {restored} swaps; {}", errors.join("; ")))
        }
    }
    fn restore_item(
        &mut self,
        target: &SwapItem,
        cooked_pc: &Path,
        backups_dir: &Path,
    ) -> Result<(), String> {
        let key = self
            .active
            .iter()
            .find(|active| active.target_matches(target))
            .map(ActiveSwap::key)
            .ok_or("No active swap was found for this item")?;
        self.restore_swap(&key, cooked_pc, backups_dir)
    }
    fn restore_swap(
        &mut self,
        target_upk: &str,
        cooked_pc: &Path,
        backups_dir: &Path,
    ) -> Result<(), String> {
        backup_guard::check_install(cooked_pc, backups_dir)?;
        if let Some(active) = self
            .active
            .iter()
            .find(|swap| swap.key().eq_ignore_ascii_case(target_upk))
            .cloned()
            && active.asset_override
        {
            super::decal_variant::restore_product(
                &cooked_pc.join(&active.target_upk),
                &backups_dir.join(format!("{}.bak", active.target_upk)),
                active
                    .target_asset
                    .as_deref()
                    .ok_or("Missing decal variant identity")?,
            )?;
            self.active.retain(|swap| swap.key() != active.key());
            return self.save_active(backups_dir);
        }
        let backup = backups_dir.join(format!("{target_upk}.bak"));
        let live = cooked_pc.join(target_upk);
        if !backup.is_file() {
            return Err(format!("Backup not found: {}", backup.display()));
        }
        if live.exists() {
            fs::remove_file(&live)
                .map_err(|error| format!("Failed to remove {}: {error}", live.display()))?;
        }
        if let Err(error) = fs::copy(&backup, &live) {
            return Err(format!("Failed to restore {target_upk}: {error}"));
        }
        fs::remove_file(&backup)
            .map_err(|error| format!("Failed to remove {}: {error}", backup.display()))?;
        if let Some(target_bnk) = self
            .active
            .iter()
            .find(|swap| swap.target_upk.eq_ignore_ascii_case(target_upk))
            .and_then(|swap| swap.target_bnk.as_deref())
        {
            let bnk_backup = backups_dir.join(format!("{target_bnk}.bak"));
            let bnk_live = cooked_pc.join(target_bnk);
            if bnk_backup.is_file() {
                fs::copy(&bnk_backup, &bnk_live)
                    .map_err(|error| format!("Failed to restore {target_bnk}: {error}"))?;
                fs::remove_file(&bnk_backup).map_err(|error| {
                    format!("Failed to remove {}: {error}", bnk_backup.display())
                })?;
            }
        }
        if let Some(thumbnail) = self
            .active
            .iter()
            .find(|swap| swap.target_upk.eq_ignore_ascii_case(target_upk))
            .and_then(|swap| swap.target_thumbnail.as_deref())
        {
            let thumb_backup = backups_dir.join(format!("{thumbnail}.bak"));
            let thumb_live = cooked_pc.join(thumbnail);
            if thumb_backup.is_file() {
                fs::copy(&thumb_backup, &thumb_live)
                    .map_err(|error| format!("Failed to restore {thumbnail}: {error}"))?;
                fs::remove_file(&thumb_backup).map_err(|error| {
                    format!("Failed to remove {}: {error}", thumb_backup.display())
                })?;
            }
        }
        self.active
            .retain(|swap| !swap.target_upk.eq_ignore_ascii_case(target_upk));
        self.save_active(backups_dir)
    }

    fn apply_recolour(
        &mut self,
        category: SwapCategory,
        item: &SwapItem,
        paint: SwapPaint,
        owned_ids: &HashSet<i64>,
        cooked_pc: &Path,
        backups_dir: &Path,
    ) -> Result<(), String> {
        if !can_recolour_owned(category, item, owned_ids) {
            return Err("Select an owned visual item to recolour".into());
        }
        self.load_active(backups_dir);
        if paint == SwapPaint::Default {
            self.restore_item(item, cooked_pc, backups_dir)
        } else {
            self.apply_swap(category, item, item, paint, 1.0, cooked_pc, backups_dir)
        }
    }

    pub fn render_recolour_controls(&mut self, ui: &mut egui::Ui, tx: &Sender<AppMsg>) {
        let before = self.recolour_category;
        egui::ComboBox::from_id_salt("recolour_category")
            .selected_text(self.recolour_category.label())
            .show_ui(ui, |ui| {
                for category in SwapCategory::ALL
                    .into_iter()
                    .filter(|c| c.supports_recolour())
                {
                    ui.selectable_value(&mut self.recolour_category, category, category.label());
                }
            });
        if ui.button(t("app-reload-catalogs")).clicked() {
            let _ = tx.send(AppMsg::ReloadCatalogs);
        }
        let filter_changed = ui
            .checkbox(&mut self.recolour_show_applied, t("recolour-show-recoloured"))
            .on_hover_text(t("recolour-show-recoloured-hint"))
            .changed();
        if before != self.recolour_category {
            self.recolour_selected = None;
            self.recolour_status = None;
        }
        if before != self.recolour_category || filter_changed {
            self.recolour_page = 0;
        }
    }

    pub fn render_recolour(
        &mut self,
        ui: &mut egui::Ui,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        owned_ids: &HashSet<i64>,
    ) {
        self.collect_thumbnails();
        self.load_active(backups_dir);
        ui.add_space(8.0);
        if browser::search(ui, &mut self.recolour_search, t("recolour-search")) {
            self.recolour_page = 0;
        }
        if let Some(status) = &self.recolour_status {
            match status {
                Ok(message) => {
                    ui.label(message);
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            }
        }
        if owned_ids.is_empty() {
            self.recolour_selected = None;
            ui.weak(t("recolour-no-inventory"));
            return;
        }
        let category = self.recolour_category;
        let Some(items) = self.catalogs.get(&category).cloned() else {
            ui.label(
                self.errors
                    .get(&category)
                    .cloned()
                    .unwrap_or_else(|| t("recolour-loading-catalogue")),
            );
            return;
        };
        let Some(resolved) = self.resolved_items(ui, category, cooked_pc, &items) else {
            return;
        };
        let owned: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                can_recolour_owned(category, item, owned_ids)
                    && resolved[*index].available
                    && (!self.recolour_show_applied
                        || self.active.iter().any(|active| {
                            active.category == category.slug()
                                && active.target_matches(item)
                                && active.paint != SwapPaint::Default
                        }))
            })
            .map(|(index, _)| index)
            .collect();
        // Account/catalogue changes must invalidate the selection before an action.
        let selected = self.recolour_selected.as_ref().and_then(|key| {
            owned
                .iter()
                .copied()
                .find(|&index| item_identity(&items[index]) == *key)
        });
        if selected.is_none() {
            self.recolour_selected = None;
        }
        if let Some(index) = selected {
            let item = &items[index];
            ui.separator();
            ui.strong(item_label(category, item));
            ui.push_id(("recolour_paint", category, item_identity(item)), |ui| {
                painted_swap::controls(ui, &mut self.recolour_paint)
            });
            let active = self
                .active
                .iter()
                .find(|active| active.target_matches(item));
            if let Some(active) = active {
                ui.weak(t_args(
                    "recolour-applied",
                    &[
                        ("item", active.source_name.clone().into()),
                        ("paint", active.paint.description().into()),
                    ],
                ));
            }
            let mut requested = None;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.recolour_paint != SwapPaint::Default || active.is_some(),
                        egui::Button::new(if self.recolour_paint == SwapPaint::Default {
                            t("recolour-restore-original")
                        } else {
                            t("recolour-apply")
                        }),
                    )
                    .clicked()
                {
                    requested = Some(self.recolour_paint);
                }
                if ui
                    .add_enabled(active.is_some(), egui::Button::new(t("app-restore")))
                    .clicked()
                {
                    requested = Some(SwapPaint::Default);
                }
            });
            if let Some(paint) = requested {
                if !crate::messages::block_item_action_if_game_running(tx) {
                    let result = self
                        .apply_recolour(category, item, paint, owned_ids, cooked_pc, backups_dir)
                        .map(|()| {
                            if paint == SwapPaint::Default {
                                t_args(
                                    "recolour-restored",
                                    &[("item", item_label(category, item).into())],
                                )
                            } else {
                                t_args(
                                    "recolour-recoloured",
                                    &[
                                        ("item", item_label(category, item).into()),
                                        ("paint", paint.description().into()),
                                    ],
                                )
                            }
                        });
                    let log = match &result {
                        Ok(message) => format!("[Recolour] {message}"),
                        Err(error) => format!("[Recolour] Error: {error}"),
                    };
                    let _ = tx.send(AppMsg::Log(log));
                    self.recolour_status = Some(result);
                }
            }
        }
        ui.separator();
        let query = self.recolour_search.to_ascii_lowercase();
        let filtered: Vec<usize> = owned
            .into_iter()
            .filter(|&index| {
                query.is_empty()
                    || item_label(category, &items[index])
                        .to_ascii_lowercase()
                        .contains(&query)
                    || items[index].upk.to_ascii_lowercase().contains(&query)
            })
            .collect();
        let columns = browser::columns(ui, false);
        let page_size = columns * browser::ROWS;
        browser::pagination(ui, &mut self.recolour_page, filtered.len(), page_size);
        if filtered.is_empty() {
            ui.weak(t("recolour-no-matches"));
            return;
        }
        let visible = &filtered[self.recolour_page * page_size
            ..((self.recolour_page + 1) * page_size).min(filtered.len())];
        for &index in visible {
            if let Some(filename) = resolved[index].thumbnail.as_deref() {
                self.queue_thumbnail(ui, category, filename, cooked_pc, &items[index]);
            }
        }
        let tile_height = browser::tile_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("recolour_items")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in visible.chunks(columns) {
                    ui.columns(columns, |columns| {
                        for (column, &index) in row.iter().enumerate() {
                            let item = &items[index];
                            let thumbnail = self.browser_preview(category, item, &resolved[index]);
                            let id =
                                egui::Id::new(("recolour_item", category, item_identity(item)));
                            if browser::tile(
                                &mut columns[column],
                                id,
                                category,
                                item,
                                thumbnail,
                                selected == Some(index),
                                self.active.iter().any(|active| {
                                    active.category == category.slug()
                                        && active.target_matches(item)
                                }),
                                tile_height,
                            )
                            .clicked()
                            {
                                self.recolour_selected = Some(item_identity(item));
                                self.recolour_paint = self
                                    .active
                                    .iter()
                                    .find(|active| active.target_matches(item))
                                    .map_or(SwapPaint::Default, |active| active.paint);
                                self.recolour_status = None;
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });
    }

    pub fn active_count(&mut self, backups_dir: &Path) -> usize {
        self.load_active(backups_dir);
        self.active.len()
    }

    /// Swap entries for a preset: each pick shows `product` where `stock_product` is equipped.
    /// Returns the entries and the reasons picks were skipped.
    pub fn bakkes_preset_swaps(&self, picks: &[SwapPick]) -> (Vec<Value>, Vec<String>) {
        let mut swaps = Vec::new();
        let mut skipped = Vec::new();
        for pick in picks {
            let Some(items) = self.catalogs.get(&pick.category) else {
                skipped.push(format!("{}: catalog unavailable", pick.category.slug()));
                continue;
            };
            let find = |id: i64| items.iter().find(|item| item.product_id == Some(id));
            let (Some(source), Some(target)) = (find(pick.stock_product), find(pick.product)) else {
                skipped.push(format!(
                    "{}: product {} or stock {} is not in the catalog",
                    pick.category.slug(),
                    pick.product,
                    pick.stock_product
                ));
                continue;
            };
            let paint = match pick.paint {
                0 => SwapPaint::Default,
                paint => SwapPaint::Preset(paint),
            };
            let swap = ActiveSwap {
                category: pick.category.slug().to_string(),
                source_name: source.name.clone(),
                source_upk: source.upk.clone(),
                target_name: target.name.clone(),
                target_upk: target.upk.clone(),
                target_bnk: None,
                paint,
                target_thumbnail: None,
                source_asset: source.path.clone(),
                target_asset: target.path.clone(),
                asset_override: false,
            };
            swaps.push(serde_json::to_value(swap).expect("swap serializes"));
        }
        (swaps, skipped)
    }

    pub fn apply_preset_swaps(
        &mut self,
        swaps: &[Value],
        cooked_pc: &Path,
        backups_dir: &Path,
    ) -> PresetApplyReport {
        self.load_active(backups_dir);
        let mut report = PresetApplyReport::default();
        for (index, value) in swaps.iter().enumerate() {
            let saved = match serde_json::from_value::<ActiveSwap>(value.clone()) {
                Ok(saved) => saved,
                Err(error) => {
                    report.skipped.push(format!("Invalid swap entry: {error}"));
                    report.skipped_indexes.push(index);
                    continue;
                }
            };
            let Some(category) = SwapCategory::from_slug(&saved.category) else {
                report.skipped.push(format!(
                    "{}: unknown category '{}'",
                    saved.source_name, saved.category
                ));
                report.skipped_indexes.push(index);
                continue;
            };
            let Some(items) = self.catalogs.get(&category).cloned() else {
                report.skipped.push(format!(
                    "{} -> {}: the {} catalog is unavailable",
                    saved.source_name,
                    saved.target_name,
                    category.slug()
                ));
                report.skipped_indexes.push(index);
                continue;
            };
            let Some(source) = items
                .iter()
                .find(|item| saved.source_matches(item))
                .cloned()
            else {
                report.skipped.push(format!(
                    "{}: item is not in the local {} catalog",
                    saved.source_name,
                    category.slug()
                ));
                report.skipped_indexes.push(index);
                continue;
            };
            let Some(target) = items
                .iter()
                .find(|item| saved.target_matches(item))
                .cloned()
            else {
                report.skipped.push(format!(
                    "{}: target is not in the local {} catalog",
                    saved.target_name,
                    category.slug()
                ));
                report.skipped_indexes.push(index);
                continue;
            };

            if self
                .active
                .iter()
                .any(|active| active.target_matches(&target))
                && let Err(error) = self.restore_item(&target, cooked_pc, backups_dir)
            {
                report.skipped.push(format!(
                    "{} -> {}: could not restore the current change: {error}",
                    source.name, target.name
                ));
                report.skipped_indexes.push(index);
                continue;
            }

            match self.apply_swap(
                category,
                &source,
                &target,
                saved.paint,
                1.0,
                cooked_pc,
                backups_dir,
            ) {
                Ok(()) => report.applied += 1,
                Err(error) => {
                    report
                        .skipped
                        .push(format!("{} -> {}: {error}", source.name, target.name));
                    report.skipped_indexes.push(index);
                }
            }
        }
        report
    }

    pub fn restore_all_active(
        &mut self,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
    ) -> Result<usize, String> {
        if crate::messages::block_item_action_if_game_running(tx) {
            return Ok(0);
        }
        self.load_active(backups_dir);
        let targets = self.active.iter().map(ActiveSwap::key).collect::<Vec<_>>();
        let mut restored = 0;
        for target in targets {
            self.restore_swap(&target, cooked_pc, backups_dir)?;
            restored += 1;
        }
        Ok(restored)
    }

    pub fn render_active_swaps(
        &mut self,
        ui: &mut egui::Ui,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
    ) {
        self.load_active(backups_dir);
        self.collect_thumbnails();
        if self.active.is_empty() {
            return;
        }
        ui.strong(t("active-swaps-item-swaps"));
        ui.add_space(4.0);
        let fallback = crate::ui::browser::fallback_image(&self.base_dir);
        let active = self.active.clone();
        let mut restore = None;
        for row in active.chunks(4) {
            ui.columns(4, |columns| {
                for (column, swap) in row.iter().enumerate() {
                    let category = SwapCategory::ALL
                        .into_iter()
                        .find(|category| category.slug() == swap.category);
                    let source_item = category.and_then(|category| {
                        self.catalogs
                            .get(&category)
                            .and_then(|items| items.iter().find(|item| swap.source_matches(item)))
                    });
                    let target_item = category.and_then(|category| {
                        self.catalogs
                            .get(&category)
                            .and_then(|items| items.iter().find(|item| swap.target_matches(item)))
                    });
                    // Active cards represent the desired item, so prefer its pristine
                    // catalog thumbnail. A patched target thumbnail can be unreadable by
                    // the preview extractor even though Rocket League accepts the package.
                    let source_path = source_item
                        .and_then(|item| preview_thumbnail(category?, item, cooked_pc))
                        .map(|filename| {
                            let backup = backups_dir.join(format!("{filename}.bak"));
                            if backup.is_file() {
                                backup
                            } else {
                                cooked_pc.join(filename)
                            }
                        })
                        .or_else(|| {
                            swap.target_thumbnail
                                .as_deref()
                                .map(|filename| cooked_pc.join(filename))
                                .filter(|path| path.is_file())
                        });
                    let mut image_for = |path: Option<PathBuf>, role: &str| {
                        let Some(path) = path else {
                            return Some(fallback.clone());
                        };
                        let asset = if role == "source" {
                            swap.source_asset.as_deref()
                        } else {
                            swap.target_asset.as_deref()
                        };
                        let kind = if swap.category == "skins" {
                            asset
                                .map(|asset| format!("skins|{asset}"))
                                .unwrap_or_else(|| swap.category.clone())
                        } else {
                            swap.category.clone()
                        };
                        let key = format!("active|{role}|{kind}|{}", path.display());
                        if !self.thumbnails.contains_key(&key) {
                            self.thumbnails.insert(key.clone(), None);
                            let _ = self.thumbnail_jobs.send((
                                key.clone(),
                                path,
                                kind,
                                columns[column].ctx().clone(),
                            ));
                        }
                        self.thumbnails
                            .get(&key)
                            .and_then(Clone::clone)
                            .or_else(|| {
                                self.failed_thumbnails
                                    .contains_key(&key)
                                    .then(|| fallback.clone())
                            })
                    };
                    let image = image_for(source_path, "source");
                    let source_name = source_item
                        .map(|item| item.name.as_str())
                        .unwrap_or(&swap.source_name);
                    let target_name = target_item
                        .map(|item| item.name.as_str())
                        .unwrap_or(&swap.target_name);
                    egui::Frame::group(columns[column].style()).show(&mut columns[column], |ui| {
                        ui.vertical_centered(|ui| {
                            let thumbnail =
                                |ui: &mut egui::Ui,
                                 uri: String,
                                 bytes: Option<Arc<[u8]>>,
                                 size: egui::Vec2| {
                                    if let Some(bytes) = bytes {
                                        ui.add(
                                            egui::Image::from_bytes(uri, bytes)
                                                .fit_to_exact_size(size),
                                        );
                                    } else {
                                        ui.add_sized(size, egui::Spinner::new());
                                    }
                                };
                            thumbnail(
                                ui,
                                format!("bytes://active/{}", swap.target_upk),
                                image,
                                egui::vec2(120.0, 76.0),
                            );
                            ui.strong(source_name);
                            ui.weak(t_args(
                                "active-swaps-replaced-target-name",
                                &[("target_name", target_name.to_string().into())],
                            ));
                            if ui
                                .add_sized(
                                    [ui.available_width(), 24.0],
                                    egui::Button::new(t("app-restore")),
                                )
                                .clicked()
                            {
                                restore = Some(swap.key());
                            }
                        });
                    });
                }
            });
            ui.add_space(6.0);
        }
        if let Some(target) = restore {
            if crate::messages::block_item_action_if_game_running(tx) {
                return;
            }
            match self.restore_swap(&target, cooked_pc, backups_dir) {
                Ok(()) => {
                    let _ = tx.send(AppMsg::Log(format!("[Swapper] Restored {target}")));
                }
                Err(error) => {
                    let _ = tx.send(AppMsg::Log(format!("[Swapper] Error: {error}")));
                }
            }
        }
    }

    fn render_walkthrough_target(
        &mut self,
        ui: &mut egui::Ui,
        category: SwapCategory,
        items: &Arc<Vec<SwapItem>>,
        resolved: &Arc<Vec<ResolvedItem>>,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        owned_ids: &HashSet<i64>,
    ) {
        let Some(source_index) = self.walkthrough_source.filter(|index| *index < items.len())
        else {
            self.walkthrough_step = WalkthroughStep::SelectSource;
            self.walkthrough_target = None;
            return;
        };
        let source = &items[source_index];
        let selected_car = self.selected_car.clone();
        let owned_only = self.owned_only;
        let target_allowed = |index: usize, target: &SwapItem| {
            index != source_index
                && resolved[index].available
                && swap_compatible(category, source, target)
                && (category != SwapCategory::Skins
                    || selected_car
                        .as_ref()
                        .is_some_and(|car| target.car_key.as_ref() == Some(car)))
                && (!owned_only || target.product_id.is_some_and(|id| owned_ids.contains(&id)))
        };
        let query = self
            .walkthrough_search
            .get(&category)
            .map(String::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let filtered: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                target_allowed(*index, item)
                    && (query.is_empty()
                        || item_label(category, item)
                            .to_ascii_lowercase()
                            .contains(&query)
                        || item.upk.to_ascii_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect();

        if self
            .walkthrough_target
            .is_some_and(|index| !filtered.contains(&index))
        {
            self.walkthrough_target = None;
        }

        let column_count = browser::columns(ui, false);
        let page_size = column_count * browser::ROWS;
        let mut apply = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_sized([110.0, 28.0], egui::Button::new(t("tab-walkthrough-back")))
                .clicked()
            {
                self.walkthrough_step = WalkthroughStep::SelectSource;
                self.walkthrough_target = None;
            }
            ui.separator();
            ui.strong(t_args(
                "tab-walkthrough-selected",
                &[("item", item_label(category, source).into())],
            ));
            if let Some(target_index) = self.walkthrough_target {
                let key = format!("{}|{}", category.slug(), item_identity(&source));
                let selected_paint = self.swap_paint.entry(key.clone()).or_default();
                if category.supports_recolour() {
                    ui.push_id(("walkthrough_paint", &key), |ui| {
                        painted_swap::controls(ui, selected_paint);
                    });
                }
                if self.speed_enabled {
                    let selected_speed = self.swap_speed.entry(key).or_insert(1.0);
                    ui.push_id("walkthrough_speed", |ui| {
                        crate::speed_patch::speed_slider(ui, selected_speed);
                    });
                }
                apply = ui
                    .add_sized(
                        [110.0, 28.0],
                        egui::Button::new(t("ball-apply")).fill(ui.visuals().selection.bg_fill),
                    )
                    .on_hover_text(item_label(category, &items[target_index]))
                    .clicked();
            }
        });
        ui.add_space(8.0);

        if self.walkthrough_step == WalkthroughStep::SelectSource {
            return;
        }
        if filtered.is_empty() {
            browser::pagination(ui, &mut 0, 0, page_size);
            ui.vertical_centered(|ui| ui.weak(t("tab-walkthrough-no-replacements")));
            return;
        }

        let page = self.walkthrough_page.entry(category).or_insert(0);
        browser::pagination(ui, page, filtered.len(), page_size);
        let visible = &filtered[*page * page_size..((*page + 1) * page_size).min(filtered.len())];
        for &target_index in visible {
            if let Some(filename) = resolved[target_index].thumbnail.as_deref() {
                self.queue_thumbnail(ui, category, filename, cooked_pc, &items[target_index]);
            }
        }
        let tile_height = browser::tile_height(ui);
        egui::ScrollArea::vertical()
            .id_salt(("swapper_walkthrough_targets", category))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in visible.chunks(column_count) {
                    ui.columns(column_count, |columns| {
                        for (column, &target_index) in row.iter().enumerate() {
                            let target = &items[target_index];
                            let thumbnail =
                                self.browser_preview(category, target, &resolved[target_index]);
                            let tile_id = egui::Id::new((
                                "walkthrough_target",
                                category,
                                item_identity(target),
                            ));
                            if browser::tile(
                                &mut columns[column],
                                tile_id,
                                category,
                                target,
                                thumbnail,
                                self.walkthrough_target == Some(target_index),
                                self.active.iter().any(|swap| {
                                    swap.category == category.slug() && swap.target_matches(target)
                                }),
                                tile_height,
                            )
                            .clicked()
                            {
                                self.walkthrough_target = Some(target_index);
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });

        if apply {
            let Some(target_index) = self.walkthrough_target else {
                return;
            };
            if crate::messages::block_item_action_if_game_running(tx) {
                return;
            }
            let source = items[source_index].clone();
            let target = items[target_index].clone();
            let key = format!("{}|{}", category.slug(), item_identity(&source));
            let paint = *self.swap_paint.entry(key.clone()).or_default();
            let speed = *self.swap_speed.entry(key).or_insert(1.0);
            match self.apply_swap(
                category,
                &source,
                &target,
                paint,
                speed,
                cooked_pc,
                backups_dir,
            ) {
                Ok(()) => {
                    let _ = tx.send(AppMsg::Log(format!(
                        "[Swapper] {} -> {} (replaced {})",
                        source.name, target.name, target.upk
                    )));
                    if category == SwapCategory::Skins {
                        self.selected_car = self.walkthrough_source_car.clone();
                        self.page.insert(category, 0);
                    }
                    self.walkthrough_step = WalkthroughStep::SelectSource;
                    self.walkthrough_source = None;
                    self.walkthrough_source_car = None;
                    self.walkthrough_target = None;
                    self.walkthrough_search.entry(category).or_default().clear();
                    self.walkthrough_page.insert(category, 0);
                    self.walkthrough_applied_at = Some(Instant::now());
                }
                Err(error) => {
                    let _ = tx.send(AppMsg::Log(format!("[Swapper] Error: {error}")));
                }
            }
        }
    }

    pub fn render_tab(
        &mut self,
        ui: &mut egui::Ui,
        category: SwapCategory,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        owned_ids: &HashSet<i64>,
    ) -> bool {
        let mut owned_filter_requested = false;
        self.load_active(backups_dir);
        self.collect_thumbnails();
        if self.walkthrough_mode && self.walkthrough_category != Some(category) {
            self.walkthrough_category = Some(category);
            self.walkthrough_step = WalkthroughStep::SelectSource;
            self.walkthrough_source = None;
            self.walkthrough_source_car = None;
            self.walkthrough_target = None;
        }
        ui.horizontal_top(|ui| {
            ui.heading(category.label());
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                |ui| {
                    if ui.button(t("app-reload-catalogs")).clicked() {
                        let _ = tx.send(AppMsg::ReloadCatalogs);
                        self.load_active(backups_dir);
                    }
                    if ui.button(t("app-restore-all")).clicked() {
                        if !crate::messages::block_item_action_if_game_running(tx) {
                            match self.restore_all(category, cooked_pc, backups_dir) {
                                Ok(count) => {
                                    let _ = tx.send(AppMsg::Log(format!(
                                        "[Swapper] Restored {count} {} swap(s).",
                                        category.label()
                                    )));
                                }
                                Err(error) => {
                                    let _ =
                                        tx.send(AppMsg::Log(format!("[Swapper] Error: {error}")));
                                }
                            }
                        }
                    }
                    if ui
                        .checkbox(&mut self.view_patched, t("ball-show-applied"))
                        .changed()
                    {
                        self.page.insert(category, 0);
                    }
                    let previous_mode = self.walkthrough_mode;
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.walkthrough_mode,
                            false,
                            t("tab-manual-mode"),
                        );
                        ui.selectable_value(
                            &mut self.walkthrough_mode,
                            true,
                            t("tab-walkthrough-mode"),
                        );
                    });
                    if self.walkthrough_mode != previous_mode {
                        self.walkthrough_category = Some(category);
                        self.walkthrough_step = WalkthroughStep::SelectSource;
                        self.walkthrough_source = None;
                        self.walkthrough_source_car = None;
                        self.walkthrough_target = None;
                    }
                },
            );
        });
        ui.weak(t(&format!("items-browser-subtitle-{}", category.slug())));
        ui.add_space(8.0);
        let selecting_target =
            self.walkthrough_mode && self.walkthrough_step == WalkthroughStep::SelectTarget;
        let input = if selecting_target {
            self.walkthrough_search.entry(category).or_default()
        } else {
            self.search_input.entry(category).or_default()
        };
        if browser::search(
            ui,
            input,
            t_args(
                "tab-search-category",
                &[("category", category.label().to_lowercase().into())],
            ),
        ) {
            if selecting_target {
                self.walkthrough_page.insert(category, 0);
            } else {
                self.page.insert(category, 0);
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut self.owned_only, t("tab-show-only-owned-replacements"))
                .changed()
            {
                owned_filter_requested = self.owned_only;
            }
            if self.owned_only && owned_ids.is_empty() {
                ui.weak(t("items-browser-no-owned-products"));
            }
        });
        ui.add_space(6.0);

        if self.walkthrough_mode {
            let applied_duration = Duration::from_secs(1);
            if let Some(applied_at) = self.walkthrough_applied_at {
                let elapsed = applied_at.elapsed();
                if elapsed < applied_duration {
                    ui.ctx().request_repaint_after(applied_duration - elapsed);
                    egui::Frame::new()
                        .fill(ui.visuals().selection.bg_fill)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.strong(
                                egui::RichText::new(t("tab-walkthrough-applied-swap"))
                                    .color(ui.visuals().selection.stroke.color),
                            );
                        });
                    ui.add_space(8.0);
                } else {
                    self.walkthrough_applied_at = None;
                }
            }
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(if self.walkthrough_step == WalkthroughStep::SelectSource {
                            t("tab-walkthrough-find-item")
                        } else {
                            t("tab-walkthrough-select-owned")
                        });
                    });
                });
            ui.add_space(10.0);
        }

        self.thumbnail_status(ui, category);
        let Some(items) = self.catalogs.get(&category).cloned() else {
            ui.colored_label(
                egui::Color32::from_rgb(231, 76, 60),
                self.errors
                    .get(&category)
                    .map(String::as_str)
                    .unwrap_or("Catalog could not be loaded"),
            );
            return owned_filter_requested;
        };
        let Some(resolved) = self.resolved_items(ui, category, cooked_pc, &items) else {
            return owned_filter_requested;
        };
        if category == SwapCategory::Skins {
            let cars = self
                .resolutions
                .get(&category)
                .and_then(|entry| entry.cars.clone())
                .unwrap();
            let car_allowed = |car: &(String, String, Option<i64>)| {
                !self.owned_only
                    || is_universal_car(&car.1)
                    || car.2.is_some_and(|id| owned_ids.contains(&id))
            };
            if self.selected_car.as_ref().is_some_and(|selected| {
                !cars
                    .iter()
                    .any(|car| &car.0 == selected && car_allowed(car))
            }) {
                self.selected_car = None;
            }
            if self.selected_car.is_none() {
                self.selected_car = cars
                    .iter()
                    .find(|car| car_allowed(car) && is_universal_car(&car.1))
                    .or_else(|| cars.iter().find(|car| car_allowed(car)))
                    .map(|car| car.0.clone());
            }
            let selected_text = self
                .selected_car
                .as_ref()
                .and_then(|selected| cars.iter().find(|car| &car.0 == selected))
                .map(|car| car.1.as_str())
                .unwrap_or("Select car...");
            let previous_car = self.selected_car.clone();
            ui.horizontal_wrapped(|ui| {
                ui.strong(t("tab-car"));
                egui::ComboBox::from_id_salt("swapper_decal_car")
                    .width(280.0)
                    .height(320.0)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        ui.set_min_height(300.0);
                        ui.horizontal(|ui| {
                            ui.label(t("spoofer-filter"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.car_search)
                                    .hint_text(t("tab-search-cars"))
                                    .desired_width(180.0),
                            );
                            if ui.small_button(t("spoofer-clear")).clicked() {
                                self.car_search.clear();
                            }
                        });
                        ui.separator();
                        let query = self.car_search.trim().to_ascii_lowercase();
                        for (key, name, id) in cars.iter() {
                            if (!self.owned_only
                                || is_universal_car(name)
                                || id.is_some_and(|id| owned_ids.contains(&id)))
                                && (query.is_empty() || name.to_ascii_lowercase().contains(&query))
                            {
                                ui.selectable_value(
                                    &mut self.selected_car,
                                    Some(key.clone()),
                                    name,
                                );
                            }
                        }
                    });
                if self.walkthrough_mode {
                    let mut enabled = true;
                    ui.add_enabled(
                        false,
                        egui::Checkbox::new(&mut enabled, t("tab-match-selected-car")),
                    )
                    .on_hover_text(t("tab-limit-replacement-decals-to-the-selected"));
                } else if ui
                    .checkbox(&mut self.match_swapped_item, t("tab-match-selected-car"))
                    .on_hover_text(t("tab-limit-replacement-decals-to-the-selected"))
                    .changed()
                {
                    self.page.insert(category, 0);
                }
            });
            if self.selected_car != previous_car {
                self.page.insert(category, 0);
                self.walkthrough_page.insert(category, 0);
                self.walkthrough_target = None;
            }
            ui.add_space(6.0);
        }
        if self.walkthrough_mode && self.walkthrough_step == WalkthroughStep::SelectTarget {
            self.render_walkthrough_target(
                ui,
                category,
                &items,
                &resolved,
                cooked_pc,
                backups_dir,
                tx,
                owned_ids,
            );
            return owned_filter_requested;
        }
        let query = self
            .search_input
            .get(&category)
            .cloned()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let filtered: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                let matches_search = query.is_empty()
                    || item_label(category, item)
                        .to_ascii_lowercase()
                        .contains(&query)
                    || item.upk.to_ascii_lowercase().contains(&query);
                let matches_car = category != SwapCategory::Skins
                    || self
                        .selected_car
                        .as_ref()
                        .is_some_and(|car| item.car_key.as_ref() == Some(car));
                let is_applied = self
                    .active
                    .iter()
                    .any(|swap| swap.category == category.slug() && swap.source_matches(item));
                resolved[*index].available
                    && matches_search
                    && matches_car
                    && (!self.view_patched || is_applied)
            })
            .map(|(index, _)| index)
            .collect();
        if self.walkthrough_mode
            && self
                .walkthrough_source
                .is_some_and(|index| !filtered.contains(&index))
        {
            self.walkthrough_source = None;
        }
        let column_count = browser::columns(ui, !self.walkthrough_mode);
        let page_size = column_count * browser::ROWS;
        if filtered.is_empty() {
            browser::pagination(ui, &mut 0, 0, page_size);
            ui.vertical_centered(|ui| {
                ui.weak(if self.view_patched {
                    t("tab-no-applied-items-match-the-search")
                } else {
                    t("tab-no-items-match-the-search")
                })
            });
            return owned_filter_requested;
        }

        let page = self.page.entry(category).or_insert(0);
        browser::pagination(ui, page, filtered.len(), page_size);
        let visible = &filtered[*page * page_size..((*page + 1) * page_size).min(filtered.len())];
        for &source_index in visible {
            if let Some(filename) = resolved[source_index].thumbnail.as_deref() {
                self.queue_thumbnail(ui, category, &filename, cooked_pc, &items[source_index]);
            }
        }
        if self.walkthrough_mode {
            if let Some(source_index) = self.walkthrough_source {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(t_args(
                        "tab-walkthrough-selected",
                        &[("item", item_label(category, &items[source_index]).into())],
                    ));
                    if ui
                        .add_sized(
                            [110.0, 28.0],
                            egui::Button::new(t("ball-next")).fill(ui.visuals().selection.bg_fill),
                        )
                        .clicked()
                    {
                        self.walkthrough_step = WalkthroughStep::SelectTarget;
                        self.walkthrough_target = None;
                        self.walkthrough_page.insert(category, 0);
                    }
                });
                ui.add_space(8.0);
            }
            let tile_height = browser::tile_height(ui);
            egui::ScrollArea::vertical()
                .id_salt(("swapper_walkthrough_sources", category))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for row in visible.chunks(column_count) {
                        ui.columns(column_count, |columns| {
                            for (column, &source_index) in row.iter().enumerate() {
                                let source = &items[source_index];
                                let thumbnail =
                                    self.browser_preview(category, source, &resolved[source_index]);
                                let tile_id = egui::Id::new((
                                    "walkthrough_source",
                                    category,
                                    item_identity(source),
                                ));
                                if browser::tile(
                                    &mut columns[column],
                                    tile_id,
                                    category,
                                    source,
                                    thumbnail,
                                    self.walkthrough_source == Some(source_index),
                                    self.active.iter().any(|swap| {
                                        swap.category == category.slug()
                                            && swap.source_matches(source)
                                    }),
                                    tile_height,
                                )
                                .clicked()
                                {
                                    self.walkthrough_source = Some(source_index);
                                    self.walkthrough_source_car = self.selected_car.clone();
                                    self.walkthrough_target = None;
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            return owned_filter_requested;
        }
        let mut action: Option<(usize, usize, bool, SwapPaint, f32)> = None;
        egui::ScrollArea::vertical()
            .id_salt(("swapper_grid", category))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in visible.chunks(column_count) {
                    ui.columns(column_count, |columns| {
                        for (column, &source_index) in row.iter().enumerate() {
                            let source = &items[source_index];
                            let key = format!("{}|{}", category.slug(), item_identity(&source));
                            let thumbnail =
                                self.browser_preview(category, source, &resolved[source_index]);
                            let target_index = self.target_index.entry(key.clone()).or_insert(0);
                            let selected_car = self.selected_car.clone();
                            let match_swapped_item = self.match_swapped_item;
                            let target_allowed = |index: usize, target: &SwapItem| {
                                index != source_index
                                    && resolved[index].available
                                    && swap_compatible(category, source, target)
                                    && (category != SwapCategory::Skins
                                        || !match_swapped_item
                                        || selected_car.as_ref().is_some_and(|car| {
                                            target.car_key.as_ref() == Some(car)
                                        }))
                                    && (!self.owned_only
                                        || target
                                            .product_id
                                            .is_some_and(|id| owned_ids.contains(&id)))
                            };
                            if *target_index >= items.len()
                                || !target_allowed(*target_index, &items[*target_index])
                            {
                                *target_index = items
                                    .iter()
                                    .enumerate()
                                    .position(|(index, item)| target_allowed(index, item))
                                    .unwrap_or(0);
                            }
                            let has_target = items
                                .get(*target_index)
                                .is_some_and(|item| target_allowed(*target_index, item));
                            let card = browser::frame(&columns[column], false).show(
                                &mut columns[column],
                                |ui| {
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                    let tile_width = ui.available_width();
                                    ui.set_min_height(238.0);
                                    ui.vertical_centered(|ui| {
                                        let source_label = item_label(category, source);
                                        browser::preview(ui, &key, thumbnail, 100.0);
                                        browser::name(ui, &source_label, &source.upk);
                                        if category == SwapCategory::Skins {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&source.upk).weak().small(),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&source.upk);
                                        }
                                        if self.active.iter().any(|swap| {
                                            swap.category == category.slug()
                                                && swap.source_matches(source)
                                        }) {
                                            browser::applied(ui);
                                        }
                                        ui.add_space(4.0);
                                        ui.label(
                                            egui::RichText::new(
                                                if category == SwapCategory::Skins {
                                                    t("tab-replace-with-decal")
                                                } else {
                                                    t("tab-replace-item")
                                                },
                                            )
                                            .size(11.0)
                                            .color(ui.visuals().weak_text_color()),
                                        );
                                        ui.add_enabled_ui(has_target, |ui| {
                                            ui.set_width(tile_width);
                                            let replacement_label =
                                                item_label(category, &items[*target_index]);
                                            let selected_text = browser::replacement_text(
                                                ui,
                                                &replacement_label,
                                                tile_width,
                                            );
                                            egui::ComboBox::from_id_salt((
                                                "swap_target_card",
                                                &key,
                                            ))
                                            .width(tile_width)
                                            .truncate()
                                            .height(300.0)
                                            .close_behavior(
                                                egui::PopupCloseBehavior::CloseOnClickOutside,
                                            )
                                            .selected_text(selected_text)
                                            .show_ui(ui, |ui| {
                                                ui.set_min_height(280.0);
                                                let target_filter = self
                                                    .target_search
                                                    .entry(key.clone())
                                                    .or_default();
                                                ui.horizontal(|ui| {
                                                    ui.label(t("spoofer-filter"));
                                                    ui.add(
                                                        egui::TextEdit::singleline(target_filter)
                                                            .hint_text(t("tab-search-items"))
                                                            .desired_width(150.0),
                                                    );
                                                    if ui.small_button(t("spoofer-clear")).clicked()
                                                    {
                                                        target_filter.clear();
                                                    }
                                                });
                                                ui.separator();
                                                let target_query =
                                                    target_filter.to_ascii_lowercase();
                                                for (index, item) in items
                                                    .iter()
                                                    .enumerate()
                                                    .filter(|(index, item)| {
                                                        target_allowed(*index, item)
                                                            && (target_query.is_empty()
                                                                || item_label(category, item)
                                                                    .to_ascii_lowercase()
                                                                    .contains(&target_query)
                                                                || item
                                                                    .upk
                                                                    .to_ascii_lowercase()
                                                                    .contains(&target_query))
                                                    })
                                                {
                                                    let label = item_label(category, item);
                                                    ui.selectable_value(
                                                            target_index,
                                                            index,
                                                            &label,
                                                        )
                                                        .on_hover_text(format!(
                                                            "{label}\n{}",
                                                            item.upk
                                                        ));
                                                }
                                            })
                                            .response
                                            .on_hover_text(item_label(
                                                category,
                                                &items[*target_index],
                                            ));
                                        });
                                        if !has_target {
                                            ui.weak(t("tab-no-owned-replacement-is-available"));
                                            return;
                                        }
                                        let selected_paint =
                                            self.swap_paint.entry(key.clone()).or_default();
                                        if category.supports_recolour() {
                                            ui.push_id(("swap_paint", &key), |ui| {
                                                painted_swap::controls(ui, selected_paint);
                                            });
                                        }
                                        let paint = *selected_paint;
                                        let mut speed = 1.0;
                                        if self.speed_enabled {
                                            let selected_speed =
                                                self.swap_speed.entry(key.clone()).or_insert(1.0);
                                            ui.push_id(("swap_speed", &key), |ui| {
                                                crate::speed_patch::speed_slider(
                                                    ui,
                                                    selected_speed,
                                                );
                                            });
                                            speed = *selected_speed;
                                        }
                                        let active = self.active.iter().find(|swap| {
                                            swap.paint == paint
                                                && swap.category == category.slug()
                                                && swap.source_matches(source)
                                                && swap.target_matches(&items[*target_index])
                                        });
                                        if let Some(active) = active {
                                            ui.weak(t_args(
                                                "tab-set-as-active",
                                                &[(
                                                    "active",
                                                    active.target_name.to_string().into(),
                                                )],
                                            ));
                                        }
                                        if ui
                                            .add_sized(
                                                [ui.available_width(), 24.0],
                                                egui::Button::new(if active.is_some() {
                                                    t("app-restore")
                                                } else {
                                                    t("ball-apply")
                                                }),
                                            )
                                            .clicked()
                                        {
                                            action = Some((
                                                source_index,
                                                *target_index,
                                                active.is_some(),
                                                paint,
                                                speed,
                                            ));
                                        }
                                    });
                                },
                            );
                            browser::hover_outline(&columns[column], card.response.rect);
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        if let Some((source_index, target_index, restoring, paint, speed)) = action {
            if crate::messages::block_item_action_if_game_running(tx) {
                return owned_filter_requested;
            }
            let source = items[source_index].clone();
            let target = items[target_index].clone();
            let result = if restoring {
                self.restore_item(&target, cooked_pc, backups_dir)
            } else {
                self.apply_swap(
                    category,
                    &source,
                    &target,
                    paint,
                    speed,
                    cooked_pc,
                    backups_dir,
                )
            };
            match result {
                Ok(()) => {
                    let message = if restoring {
                        format!("[Swapper] Restored {} ({})", target.name, target.upk)
                    } else {
                        format!(
                            "[Swapper] {} -> {} (replaced {})",
                            source.name, target.name, target.upk
                        )
                    };
                    let _ = tx.send(AppMsg::Log(message));
                }
                Err(error) => {
                    let _ = tx.send(AppMsg::Log(format!("[Swapper] Error: {error}")));
                }
            }
        }
        owned_filter_requested
    }
}

#[cfg(test)]
mod recolour_tests {
    use super::*;
    use serde_json::json;

    fn catalog_item(category: SwapCategory, file: &str) -> SwapItem {
        SwapperState::parse_catalog(category, &json!({category.slug(): [
            {"name":"Test item", "upk_path":file, "id":123, "paintable":false, "audio_bnk":"unchanged.bnk"}
        ]}), &Value::Null).unwrap().remove(0)
    }

    #[test]
    fn recolour_eligibility_uses_visual_category_and_ownership() {
        let owned = HashSet::from([123]);
        for category in SwapCategory::ALL
            .into_iter()
            .filter(|c| *c != SwapCategory::Skins)
        {
            let mut item = catalog_item(category, "test_SF.upk");
            assert_eq!(
                can_recolour_owned(category, &item, &owned),
                !matches!(category, SwapCategory::Engines | SwapCategory::Anthems)
            );
            assert!(!can_recolour_owned(category, &item, &HashSet::new()));
            item.product_id = None;
            assert!(!can_recolour_owned(category, &item, &owned));
        }
        assert!(SwapCategory::Skins.supports_recolour());
    }

    #[test]
    fn recolour_rejects_unowned_items_and_audio_before_accessing_files() {
        let path = Path::new("nonexistent-recolour-test");
        let mut state = SwapperState::new(path);
        let body = catalog_item(SwapCategory::Bodies, "body.upk");
        for (category, owned) in [
            (SwapCategory::Bodies, HashSet::new()),
            (SwapCategory::Engines, HashSet::from([123])),
            (SwapCategory::Anthems, HashSet::from([123])),
        ] {
            assert_eq!(
                state
                    .apply_recolour(category, &body, SwapPaint::Crimson, &owned, path, path)
                    .unwrap_err(),
                "Select an owned visual item to recolour"
            );
        }
    }

    #[test]
    fn recolour_round_trip_and_preset_replay_with_installed_assets() {
        let Some(game) = std::env::var_os("HEBNIX_RECOLOUR_GAME_DIR").map(PathBuf::from) else {
            return;
        };
        let root =
            std::env::temp_dir().join(format!("hebnix-recolour-test-{}", rand::random::<u64>()));
        let cooked = root.join("TAGame/CookedPCConsole");
        let backups = cooked.join("Backups");
        fs::create_dir_all(&cooked).unwrap();
        fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
        fs::write(root.join("Binaries/Win64/RocketLeague.exe"), b"test build").unwrap();
        let mut state = SwapperState::new(&root);
        let owned = HashSet::from([123]);
        for (category, file) in [
            (SwapCategory::Bodies, "Body_Backfire_SF.upk"),
            (SwapCategory::Wheels, "wheel_hiro_SF.upk"),
            (SwapCategory::Boosts, "Boost_Standard_SF.upk"),
        ] {
            let backup = game.join("Backups").join(format!("{file}.bak"));
            let pristine = fs::read(if backup.is_file() {
                backup
            } else {
                game.join(file)
            })
            .unwrap();
            let live = cooked.join(file);
            fs::write(&live, &pristine).unwrap();
            let item = catalog_item(category, file);
            state
                .catalogs
                .insert(category, Arc::new(vec![item.clone()]));
            state
                .apply_recolour(
                    category,
                    &item,
                    SwapPaint::Custom([32, 128, 240]),
                    &owned,
                    &cooked,
                    &backups,
                )
                .unwrap();
            let coloured = fs::read(&live).unwrap();
            assert_ne!(coloured, pristine);
            assert_eq!(
                fs::read(backups.join(format!("{file}.bak"))).unwrap(),
                pristine
            );
            let saved = serde_json::to_value(&state.active[0]).unwrap();
            // Simulate a prior swap with companion audio/thumbnail changes.
            // Recolouring the original item must restore and keep tracking them.
            if category == SwapCategory::Boosts {
                for name in ["previous.bnk", "preview.upk"] {
                    fs::write(cooked.join(name), b"swapped").unwrap();
                    fs::write(backups.join(format!("{name}.bak")), b"original").unwrap();
                }
                state.active[0].target_bnk = Some("previous.bnk".into());
                state.active[0].target_thumbnail = Some("preview.upk".into());
                state.save_active(&backups).unwrap();
            }
            state
                .apply_recolour(
                    category,
                    &item,
                    SwapPaint::Crimson,
                    &owned,
                    &cooked,
                    &backups,
                )
                .unwrap();
            assert_ne!(fs::read(&live).unwrap(), coloured);
            if category == SwapCategory::Boosts {
                for name in ["previous.bnk", "preview.upk"] {
                    assert_eq!(fs::read(cooked.join(name)).unwrap(), b"original");
                }
            }
            // A failed paint must preserve the previously installed colour.
            let previous = fs::read(&live).unwrap();
            assert!(
                state
                    .apply_recolour(
                        category,
                        &item,
                        SwapPaint::Preset(0),
                        &owned,
                        &cooked,
                        &backups
                    )
                    .is_err()
            );
            assert_eq!(fs::read(&live).unwrap(), previous);
            state
                .apply_recolour(
                    category,
                    &item,
                    SwapPaint::Default,
                    &owned,
                    &cooked,
                    &backups,
                )
                .unwrap();
            assert_eq!(fs::read(&live).unwrap(), pristine);
            assert!(state.active.is_empty());
            let report = state.apply_preset_swaps(&[saved], &cooked, &backups);
            assert_eq!(report.applied, 1, "{:?}", report.skipped);
            assert_eq!(fs::read(&live).unwrap(), coloured);
            let (tx, _rx) = unbounded();
            state.restore_all_active(&cooked, &backups, &tx).unwrap();
            assert_eq!(fs::read(&live).unwrap(), pristine);
            assert!(!cooked.join("unchanged.bnk").exists());
        }
        fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod variant_catalog_tests {
    use super::*;
    #[test]
    fn all_season_twelve_ranks_remain_separate_in_one_package() {
        let root: Value =
            serde_json::from_str(include_str!("../../testdata/season12-variants.json")).unwrap();
        let mut with_alias = root.clone();
        let alias = with_alias["cars"]["UNIVERSAL"]["skins"][0].clone();
        with_alias["cars"]["UNIVERSAL"]["skins"]
            .as_array_mut()
            .unwrap()
            .push(alias);
        let items = SwapperState::parse_catalog(
            SwapCategory::Skins,
            &with_alias,
            &serde_json::json!({"bodies":[]}),
        )
        .unwrap();
        assert_eq!(items.len(), 7);
        assert_eq!(
            items
                .iter()
                .map(item_identity)
                .collect::<HashSet<_>>()
                .len(),
            7
        );
        assert_eq!(
            items
                .iter()
                .map(|item| thumbnail_key(SwapCategory::Skins, "skin_SeasonTwelve_T_SF.upk", item))
                .collect::<HashSet<_>>()
                .len(),
            7
        );
        for rank in [
            "Bronze",
            "Silver",
            "Gold",
            "Platinum",
            "Diamond",
            "Champion",
            "Grand Champion",
        ] {
            assert!(
                items
                    .iter()
                    .any(|item| item.name == format!("Season 12 - {rank}") && item.shared_package)
            );
        }
    }
    #[test]
    fn preset_identity_distinguishes_rank_variants() {
        let root: Value =
            serde_json::from_str(include_str!("../../testdata/season12-variants.json")).unwrap();
        let items = SwapperState::parse_catalog(
            SwapCategory::Skins,
            &root,
            &serde_json::json!({"bodies":[]}),
        )
        .unwrap();
        let bronze = items
            .iter()
            .find(|item| item.product_id == Some(4659))
            .unwrap();
        let silver = items
            .iter()
            .find(|item| item.product_id == Some(4681))
            .unwrap();
        let active = ActiveSwap {
            category: "skins".into(),
            source_name: bronze.name.clone(),
            source_upk: bronze.upk.clone(),
            target_name: silver.name.clone(),
            target_upk: silver.upk.clone(),
            target_bnk: None,
            paint: SwapPaint::Default,
            target_thumbnail: None,
            source_asset: bronze.path.clone(),
            target_asset: silver.path.clone(),
            asset_override: true,
        };
        let saved: ActiveSwap =
            serde_json::from_slice(&serde_json::to_vec(&active).unwrap()).unwrap();
        assert!(saved.source_matches(bronze));
        assert!(!saved.source_matches(silver));
        assert!(saved.target_matches(silver));
        assert!(!saved.target_matches(bronze));
        assert_eq!(saved.key(), item_identity(silver));
    }
}

#[cfg(test)]
mod theme_content_space_tests {
    use super::*;
    use crate::theme;

    fn body_grid_rect(base: &Path, theme_name: &str) -> egui::Rect {
        let ctx=egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        theme::apply_theme(&ctx,&dir,&dir,theme_name).unwrap();
        let p=theme::presentation(&ctx);
        let cooked=base.join("CookedPCConsole");
        let backups=cooked.join("Backups");
        let mut swapper=SwapperState::new(base);
        let items:Vec<_>=(0..16).map(|i|SwapItem {
            name:format!("Test body {i}"),upk:format!("test-{i}.upk"),path:None,thumbnail:None,body_preview:None,
            audio_bnk:None,upk_type:"simple".into(),product_id:None,
            car_key:None,car_name:None,car_product_id:None,shared_package:false,
        }).collect();
        swapper.catalogs.insert(SwapCategory::Bodies,Arc::new(items));
        swapper.resolutions.insert(SwapCategory::Bodies,Resolution {
            path:cooked.clone(),generation:0,items:Some(Arc::new(vec![ResolvedItem{available:true,thumbnail:None};16])),
            cars:Some(Arc::new(Vec::new())),
        });
        let (tx,_rx)=unbounded();
        let mut output=egui::FullOutput::default();
        let page = |ui: &mut egui::Ui, swapper: &mut SwapperState| {
            if p.as_ref().is_some_and(|p|p.layout.show_page_heading) {ui.heading("Items");ui.add_space(8.0);}
            ui.horizontal(|ui|{for label in ["Item Swapper","Item Spawner","Animation Speed","Wheel Alignment","Ball Appearance"] {let _=ui.selectable_label(label=="Item Swapper",label);}});
            ui.separator();
            theme::secondary_navigation(ui,"space_test_categories").show(ui,|ui| {
                egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui,|ui| {
                    for label in ["Ball Patcher","Boost Patcher","Decal Patcher","Car Patcher"] {let _=ui.selectable_label(false,label);}
                    ui.separator();
                    for category in SwapCategory::ALL {let _=ui.selectable_label(category==SwapCategory::Bodies,category.label());}
                });
            });
            egui::CentralPanel::default().frame(crate::i18n::layout::subtab_content_frame()).show(ui,|ui| {
                assert!(!swapper.render_tab(ui,SwapCategory::Bodies,&cooked,&backups,&tx,&HashSet::new()));
            });
        };
        for _ in 0..3 {
            output=ctx.run_ui(egui::RawInput {
                screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1250.0,730.0))),..Default::default()
            },|ui|{
                if let Some(p)=&p {
                    theme::show_shell(ui,p,|ui,part|match part {
                        theme::ShellPart::Navigation(vertical)=>{
                            let nav = |ui:&mut egui::Ui|for key in ["Console","Maps","Spoofer","Colours","Items","Settings","Plugins","Replays","About"] {
                                let width=vertical.then(||ui.available_width());
                                let _=p.nav_item(ui,key=="Items",key,key,width);
                            };
                            if vertical {ui.heading("Hebnix");ui.add_space(10.0);nav(ui);}
                            else {theme::toolbar(ui,nav);}
                        },
                        theme::ShellPart::Content=>page(ui,&mut swapper),
                    });
                } else {
                    egui::CentralPanel::default().show(ui,|ui|{
                        ui.horizontal(|ui|for key in ["Console","Maps","Spoofer","Colours","Items","Settings","Plugins","Replays","About"] {let _=ui.selectable_label(key=="Items",key);});
                        ui.separator();
                        page(ui,&mut swapper);
                    });
                }
            });
        }
        fn has_body(shape:&egui::Shape)->bool {
            match shape {
                egui::Shape::Text(text)=>text.galley.text()=="Test body 0",
                egui::Shape::Vec(shapes)=>shapes.iter().any(has_body),
                _=>false,
            }
        }
        output.shapes.iter().find(|shape|has_body(&shape.shape)).expect("Bodies grid must render").clip_rect
    }

    #[test]
    fn hebnix_2_recovers_item_scroll_space_without_changing_existing_themes() {
        let base=std::env::temp_dir().join(format!("hebnix-content-space-{}",std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let legacy=body_grid_rect(&base,"Dark");
        let ocean=body_grid_rect(&base,"Ocean Studio Dark");
        let compact=body_grid_rect(&base,"Hebnix 2");
        println!("Bodies grid: default={legacy:?}; Ocean={ocean:?}; Hebnix 2={compact:?}");
        assert!(compact.width()>ocean.width()+180.0,"The duplicate navigation column must be recovered");
        assert!(compact.height()>ocean.height()+60.0,"The grid must gain vertical space");
        assert!(compact.area()>=legacy.area()*0.95,"The new theme should use space comparably to the default");
        assert!(compact.bottom()>710.0,"The scroll region must reach the bottom of the window");
        let ctx=egui::Context::default();
        let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        theme::apply_theme(&ctx,&dir,&dir,"Hebnix 2").unwrap();
        let blue=ctx.style_of(ctx.theme()).visuals.widgets.inactive.bg_fill;
        assert!(blue.b()>blue.r()+20,"Ordinary buttons must have blue accents");
        let temp=std::env::temp_dir().canonicalize().unwrap();
        let resolved=base.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp)&&resolved!=temp);
        fs::remove_dir_all(resolved).unwrap();
    }
}
