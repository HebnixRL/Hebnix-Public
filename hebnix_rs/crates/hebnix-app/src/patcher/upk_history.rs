//! Restore points for live game packages. Backup blobs are shared by content.
use super::{backup_guard, painted_swap, wheel_alignment};
use crate::i18n::{t, t_args};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
};

pub(super) const DIR: &str = "UPKHistory";

static INSTALL: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Recipe {
    pub baseline: String,
    #[serde(default)]
    pub caption: String,
    pub donor: String,
    pub paint: painted_swap::SwapPaint,
    pub alignment: Option<wheel_alignment::Settings>,
    pub owned_paint: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SwapPoint {
    pub active: Option<super::swapper::ActiveSwap>,
    pub sidecars: Vec<(String, String)>,
    pub backups: Vec<(String, String)>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SwapTransition {
    pub before: SwapPoint,
    pub after: SwapPoint,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    file: String,
    label: String,
    before: String,
    after: String,
    #[serde(default)]
    recipe: Option<Recipe>,
    #[serde(default)]
    swap: Option<SwapTransition>,
    #[serde(default)]
    linked: bool,
    #[serde(default)]
    unix_seconds: i64,
    #[serde(default)]
    cleared: bool,
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn root(cooked: &Path) -> Result<PathBuf, String> {
    Ok(cooked
        .join("Backups")
        .join(DIR)
        .join(backup_guard::build_id(cooked)?))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("{}.pending", rand::random::<u64>()));
    let result = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, path));
    let _ = fs::remove_file(temporary);
    result.map_err(|e| format!("{}: {e}", path.display()))
}

fn valid_file(file: &str) -> bool {
    !file.contains(['/', '\\', ':'])
        && file.to_ascii_lowercase().ends_with(".upk")
        && !file.eq_ignore_ascii_case("TAGame.upk")
}

fn location(cooked: &Path, live: &Path) -> Option<String> {
    let relative = live.strip_prefix(cooked).ok()?;
    let file = relative.file_name()?.to_str()?;
    (valid_file(file)
        && !relative.components().any(|part| {
            let part = part.as_os_str().to_string_lossy();
            part.eq_ignore_ascii_case("Backups") || part.starts_with('.')
        }))
    .then(|| relative.to_string_lossy().into_owned())
}

fn blob_path(store: &Path, id: &str) -> Result<PathBuf, String> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid UPK restore point".into());
    }
    Ok(store.join("blobs").join(format!("{id}.upk.bak")))
}

pub(super) fn store_blob(store: &Path, bytes: &[u8]) -> Result<String, String> {
    let id = hash(bytes);
    let path = blob_path(store, &id)?;
    if !path.is_file() {
        fs::create_dir_all(store.join("blobs")).map_err(|e| e.to_string())?;
        replace(&path, bytes)?;
    }
    Ok(id)
}

fn load_blob(store: &Path, id: &str) -> Result<Vec<u8>, String> {
    let bytes = read(&blob_path(store, id)?)?;
    if hash(&bytes) != id {
        return Err("UPK restore point failed verification".into());
    }
    Ok(bytes)
}

pub(super) fn baseline(cooked: &Path, bytes: &[u8]) -> Result<String, String> {
    store_blob(&root(cooked)?, bytes)
}

pub(super) fn swap_point(
    cooked: &Path,
    backups: &Path,
    file: &str,
    sidecars: &[String],
    active: Option<super::swapper::ActiveSwap>,
) -> Result<SwapPoint, String> {
    let store = root(cooked)?;
    let files = sidecars
        .iter()
        .map(|name| {
            Ok((
                name.clone(),
                store_blob(&store, &read(&cooked.join(name))?)?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut saved = Vec::new();
    if active.is_some() {
        for name in std::iter::once(file).chain(sidecars.iter().map(String::as_str)) {
            let backup = backups.join(format!("{name}.bak"));
            if backup.is_file() {
                saved.push((name.to_owned(), store_blob(&store, &read(&backup)?)?));
            }
        }
    }
    Ok(SwapPoint {
        active,
        sidecars: files,
        backups: saved,
    })
}

pub(super) fn recover_swap_point(
    cooked: &Path,
    backups: &Path,
    point: &SwapPoint,
) -> Result<(), String> {
    let store = root(cooked)?;
    for (name, id) in &point.sidecars {
        replace(&cooked.join(name), &load_blob(&store, id)?)?;
    }
    for (name, id) in &point.backups {
        replace(
            &backups.join(format!("{name}.bak")),
            &load_blob(&store, id)?,
        )?;
    }
    Ok(())
}

pub(super) fn record_swap(
    cooked: &Path,
    live: &Path,
    before: &[u8],
    label: &str,
    recipe: Option<Recipe>,
    swap: SwapTransition,
) -> Result<(), String> {
    let after = read(live)?;
    commit(
        cooked,
        live,
        before,
        &after,
        label,
        recipe,
        Some(swap),
        false,
        false,
    )
}

fn entries(store: &Path) -> Result<Vec<Entry>, String> {
    let mut files = match fs::read_dir(store.join("events")) {
        Ok(files) => files
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    files.sort_unstable_by_key(|file| std::cmp::Reverse(file.file_name()));
    let mut cleared = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for file in files
        .into_iter()
        .filter(|file| file.path().extension().is_some_and(|ext| ext == "json"))
    {
        let entry: Entry =
            serde_json::from_slice(&read(&file.path())?).map_err(|e| e.to_string())?;
        let key = entry.file.to_ascii_lowercase();
        if entry.cleared {
            cleared.insert(key);
        } else if !cleared.contains(&key) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

pub(super) fn current_recipe(cooked: &Path, live: &Path) -> Result<Option<Recipe>, String> {
    let Some(file) = location(cooked, live) else {
        return Ok(None);
    };
    let entry = entries(&root(cooked)?)?
        .into_iter()
        .find(|entry| entry.file.eq_ignore_ascii_case(&file));
    match entry {
        Some(entry) if hash(&read(live)?) == entry.after => Ok(entry.recipe),
        _ => Ok(None),
    }
}

pub(super) fn recipe_input(cooked: &Path, recipe: &Recipe) -> Result<Vec<u8>, String> {
    load_blob(&root(cooked)?, &recipe.baseline)
}

pub(super) fn render_recipe(cooked: &Path, recipe: &Recipe) -> Result<Vec<u8>, String> {
    let store = root(cooked)?;
    fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let stage = store.join(format!("recipe-{}.upk.bak", rand::random::<u64>()));
    let result = (|| {
        let input = blob_path(&store, &recipe.baseline)?;
        let bytes = load_blob(&store, &recipe.baseline)?;
        if let Some(alignment) = recipe
            .alignment
            .filter(|value| *value != wheel_alignment::Settings::default())
        {
            wheel_alignment::build(&input, &stage, alignment)?;
        } else {
            replace(&stage, &bytes)?;
        }
        if recipe.donor.is_empty() {
            return Err("Missing UPK asset identity".into());
        }
        if recipe.paint != painted_swap::SwapPaint::Default {
            if recipe.owned_paint {
                painted_swap::bake_owned(&stage, &recipe.donor, recipe.paint)?;
            } else {
                painted_swap::bake(&stage, &recipe.donor, recipe.paint)?;
            }
        }
        read(&stage)
    })();
    let _ = fs::remove_file(stage);
    result
}

/// Install a staged payload and keep both sides of the change.
pub(super) fn install(
    cooked: &Path,
    live: &Path,
    after: &[u8],
    label: &str,
    recipe: Option<Recipe>,
) -> Result<(), String> {
    check_live_package(cooked, live)?;
    let before = read(live)?;
    commit(
        cooked, live, &before, after, label, recipe, None, true, false,
    )
}

pub(super) fn install_copy(
    cooked: &Path,
    source: &Path,
    live: &Path,
    label: &str,
) -> Result<(), String> {
    install(cooked, live, &read(source)?, label, None)
}

/// Record operations which already have their own transaction and verification.
pub(super) fn record(
    cooked: &Path,
    live: &Path,
    before: &[u8],
    label: &str,
    recipe: Option<Recipe>,
) -> Result<(), String> {
    let after = read(live)?;
    commit(
        cooked, live, before, &after, label, recipe, None, false, false,
    )
}

pub(super) fn install_linked(
    cooked: &Path,
    live: &Path,
    after: &[u8],
    label: &str,
) -> Result<(), String> {
    check_live_package(cooked, live)?;
    let before = read(live)?;
    commit(cooked, live, &before, after, label, None, None, true, true)
}

pub(super) fn record_linked(
    cooked: &Path,
    live: &Path,
    before: &[u8],
    label: &str,
) -> Result<(), String> {
    let after = read(live)?;
    commit(cooked, live, before, &after, label, None, None, false, true)
}

/// Commit a feature's staged package against the exact input it edited.
pub(super) fn install_linked_from(
    cooked: &Path,
    live: &Path,
    before: &[u8],
    after: &[u8],
    label: &str,
) -> Result<(), String> {
    commit(cooked, live, before, after, label, None, None, true, true)
}

fn commit(
    cooked: &Path,
    live: &Path,
    before: &[u8],
    after: &[u8],
    label: &str,
    recipe: Option<Recipe>,
    swap: Option<SwapTransition>,
    install: bool,
    linked: bool,
) -> Result<(), String> {
    let _guard = INSTALL.lock().map_err(|e| e.to_string())?;
    commit_locked(
        cooked, live, before, after, label, recipe, swap, install, linked,
    )
}

fn commit_locked(
    cooked: &Path,
    live: &Path,
    before: &[u8],
    after: &[u8],
    label: &str,
    recipe: Option<Recipe>,
    swap: Option<SwapTransition>,
    install: bool,
    linked: bool,
) -> Result<(), String> {
    if read(live)? != if install { before } else { after } {
        return Err("The package changed while preparing the restore point".into());
    }
    if before == after && swap.is_none() {
        return Ok(());
    }
    let Some(file) = location(cooked, live) else {
        return if install {
            replace(live, after)
        } else {
            Ok(())
        };
    };
    let backups = cooked.join("Backups");
    if !backups.join("hebnix-install-build.sha256").exists()
        && (!backups.exists()
            || fs::read_dir(&backups)
                .map_err(|error| error.to_string())?
                .all(|entry| entry.is_ok_and(|entry| entry.file_name() == DIR)))
    {
        backup_guard::check_install(cooked, &backups)?;
    }
    let store = root(cooked)?;
    let rows = entries(&store)?;
    let history: Vec<_> = rows
        .iter()
        .filter(|row| row.file.eq_ignore_ascii_case(&file))
        .collect();
    let after_hash = hash(after);
    let cleared = restored_original(cooked, &history, &after_hash, swap.as_ref());
    let entry = Entry {
        file,
        label: label.into(),
        before: store_blob(&store, before)?,
        after: store_blob(&store, after)?,
        recipe,
        swap,
        linked,
        unix_seconds: time::OffsetDateTime::now_utc().unix_timestamp(),
        cleared,
    };
    let dir = store.join("events");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    let event = dir.join(format!("{now:020}-{}.json", rand::random::<u64>()));
    replace(
        &event,
        &serde_json::to_vec(&entry).map_err(|e| e.to_string())?,
    )?;
    if install {
        if let Err(error) = replace(live, after) {
            let _ = fs::remove_file(event);
            return Err(error);
        }
    }
    Ok(())
}

// Rows are newest first. A swap can still be active when the main UPK is
// byte-identical to its baseline, so also check its state and attachments.
fn restored_original(
    cooked: &Path,
    history: &[&Entry],
    after: &str,
    swap: Option<&SwapTransition>,
) -> bool {
    let Some(first) = history.last() else {
        return false;
    };
    if first.before != after {
        return false;
    }
    let transitions = swap
        .into_iter()
        .chain(history.iter().filter_map(|row| row.swap.as_ref()));
    if transitions
        .clone()
        .next()
        .is_some_and(|swap| swap.after.active.is_some())
    {
        return false;
    }
    let mut original = HashMap::new();
    for (index, swap) in history
        .iter()
        .rev()
        .filter_map(|row| row.swap.as_ref())
        .chain(swap)
        .enumerate()
    {
        if index == 0 && swap.before.active.is_some() {
            return false;
        }
        for (name, digest) in &swap.before.sidecars {
            original.entry(name).or_insert(digest);
        }
    }
    original.into_iter().all(|(name, digest)| {
        valid_sidecar(name) && read(&cooked.join(name)).is_ok_and(|bytes| hash(&bytes) == *digest)
    })
}

impl Entry {
    fn active_swap(&self) -> Option<&super::swapper::ActiveSwap> {
        self.swap.as_ref()?.after.active.as_ref()
    }

    fn related_swap(&self) -> Option<&super::swapper::ActiveSwap> {
        let swap = self.swap.as_ref()?;
        swap.after.active.as_ref().or(swap.before.active.as_ref())
    }

    fn item_file(&self) -> &str {
        self.recipe
            .as_ref()
            .map(|recipe| recipe.donor.as_str())
            .filter(|file| !file.is_empty())
            .or_else(|| self.related_swap().map(|swap| swap.source_upk.as_str()))
            .unwrap_or(&self.file)
    }

    fn removed_swap(&self) -> bool {
        self.active_swap().is_none()
            && self
                .related_swap()
                .is_some_and(|swap| !swap.source_upk.eq_ignore_ascii_case(&swap.target_upk))
    }
}

#[derive(Clone, Copy)]
struct HistoryPoint<'a> {
    index: usize,
    entry: &'a Entry,
    current: &'a Entry,
    original: &'a Entry,
    has_linked: bool,
    swap: Option<&'a super::swapper::ActiveSwap>,
}

impl HistoryPoint<'_> {
    fn latest(self) -> bool {
        std::ptr::eq(self.entry, self.current)
    }

    fn priority(self) -> u8 {
        match (self.latest(), self.entry.removed_swap(), self.swap) {
            (true, false, Some(swap))
                if !swap.source_upk.eq_ignore_ascii_case(&swap.target_upk) =>
            {
                3
            }
            (true, false, _) => 2,
            (true, true, _) => 1,
            _ => 0,
        }
    }
}

fn rank_display_entry(entry: &Entry) -> bool {
    entry.linked && matches!(entry.label.as_str(), "Rank display" | "Restore Rank display")
}

fn group_entries(rows: &[Entry]) -> Vec<(&str, Vec<HistoryPoint<'_>>)> {
    let mut current = HashMap::new();
    let mut active_swaps = HashMap::new();
    let mut originals = HashMap::new();
    let mut linked = std::collections::HashSet::new();
    let mut groups = Vec::<(&str, Vec<HistoryPoint<'_>>)>::new();
    let mut group_indexes = HashMap::new();
    // Walk oldest first to associate recipe-only edits with their last swap state.
    let swaps: Vec<_> = rows
        .iter()
        .rev()
        .map(|entry| {
            let file = entry.file.to_ascii_lowercase();
            originals.entry(file.clone()).or_insert(entry);
            if entry.linked {
                linked.insert(file.clone());
            }
            if let Some(swap) = &entry.swap {
                active_swaps.insert(file.clone(), swap.after.active.as_ref());
            }
            active_swaps.get(&file).copied().flatten()
        })
        .collect();
    for (index, (entry, swap)) in rows.iter().zip(swaps.into_iter().rev()).enumerate() {
        let current = *current
            .entry(entry.file.to_ascii_lowercase())
            .or_insert(entry);
        // Keep legacy rank checkpoints in the chain for restore safety.
        if rank_display_entry(entry) {
            continue;
        }
        let item = swap.map_or_else(|| entry.item_file(), |swap| swap.source_upk.as_str());
        let group_index = *group_indexes
            .entry(item.to_ascii_lowercase())
            .or_insert_with(|| {
                groups.push((item, Vec::new()));
                groups.len() - 1
            });
        groups[group_index].1.push(HistoryPoint {
            index,
            entry,
            current,
            original: originals[&entry.file.to_ascii_lowercase()],
            has_linked: linked.contains(&entry.file.to_ascii_lowercase()),
            swap,
        });
    }
    groups
}

enum Action {
    Original,
    Before,
    After,
    Edit(Recipe),
}

#[derive(Default)]
pub(crate) struct History {
    cooked: PathBuf,
    entries: Vec<Entry>,
    filter: String,
    status: String,
    editors: std::collections::HashMap<usize, Recipe>,
    job: Option<mpsc::Receiver<Result<(), String>>>,
}

impl History {
    pub(crate) fn enter(&mut self) {
        self.cooked.clear();
    }

    fn refresh(&mut self, cooked: &Path) {
        self.status.clear();
        match root(cooked).and_then(|store| entries(&store)) {
            Ok(entries) => {
                let mut by_file = HashMap::<_, Vec<_>>::new();
                for row in &entries {
                    by_file
                        .entry(row.file.to_ascii_lowercase())
                        .or_default()
                        .push(row);
                }
                let restored: std::collections::HashSet<_> = by_file
                    .into_iter()
                    .filter_map(|(file, rows)| {
                        let latest = rows[0];
                        (latest.after == rows.last()?.before
                            && read(&cooked.join(&latest.file))
                                .is_ok_and(|bytes| hash(&bytes) == latest.after)
                            && restored_original(cooked, &rows, &latest.after, None))
                        .then_some(file)
                    })
                    .collect();
                self.entries = entries
                    .into_iter()
                    .filter(|row| !restored.contains(&row.file.to_ascii_lowercase()))
                    .collect();
            }
            Err(error) => {
                self.entries.clear();
                if cooked.is_dir() {
                    self.status = error;
                }
            }
        }
        self.editors.clear();
    }

    pub(crate) fn render(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        swapper: &super::swapper::SwapperState,
    ) {
        if self.cooked != cooked {
            self.cooked = cooked.to_owned();
            self.refresh(cooked);
        }
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.job = None;
                    self.refresh(cooked);
                    self.status = result
                        .err()
                        .unwrap_or_else(|| t("upk-history-saved").into());
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    self.status = t("upk-history-stopped").into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let running = hebnix_sdk::process::is_rocket_league_running();
        let enabled = !running && self.job.is_none();
        egui::ScrollArea::vertical()
            .id_salt("upk_history")
            .show(ui, |ui| {
                crate::i18n::layout::bordered_panel(ui, |ui| {
                    ui.heading(t("upk-history"));
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text(t("upk-history-search")),
                        );
                        if ui.button(t("upk-history-refresh")).clicked() {
                            self.refresh(cooked);
                        }
                        if self.job.is_some() {
                            ui.spinner();
                        }
                    });
                    if running {
                        ui.weak(t("upk-history-close-game"));
                    }
                    if !self.status.is_empty() {
                        ui.label(&self.status);
                    }
                    if self.entries.iter().all(rank_display_entry) {
                        ui.weak(t("upk-history-empty"));
                    }
                });
                let filter = self.filter.trim().to_lowercase();
                let mut action = None;
                for (item, points) in group_entries(&self.entries) {
                    let primary = points
                        .iter()
                        .copied()
                        .max_by_key(|point| (point.priority(), std::cmp::Reverse(point.index)))
                        .expect("each history group has at least one point");
                    let name = points
                        .iter()
                        .find_map(|point| {
                            point
                                .entry
                                .related_swap()
                                .map(|swap| swap.source_name.as_str())
                        })
                        .or_else(|| swapper.history_item_name(item))
                        .unwrap_or(&primary.entry.label);
                    if !name.to_lowercase().contains(&filter)
                        && !points.iter().any(|point| {
                            point.entry.file.to_lowercase().contains(&filter)
                                || point.entry.label.to_lowercase().contains(&filter)
                        })
                    {
                        continue;
                    }
                    ui.push_id(item.to_ascii_lowercase(), |ui| {
                        crate::i18n::layout::bordered_panel(ui, |ui| {
                            ui.label(egui::RichText::new(name.replace('\u{2192}', "-")).strong());
                            action = render_point(ui, primary, &mut self.editors, enabled, true)
                                .or(action.take());
                            if points.len() > 1 {
                                egui::CollapsingHeader::new(t("upk-history-checkpoints")).show(
                                    ui,
                                    |ui| {
                                        for point in points
                                            .iter()
                                            .copied()
                                            .filter(|point| point.index != primary.index)
                                        {
                                            ui.separator();
                                            action = render_point(
                                                ui,
                                                point,
                                                &mut self.editors,
                                                enabled,
                                                false,
                                            )
                                            .or(action.take());
                                        }
                                    },
                                );
                            }
                        });
                    });
                }
                if let Some((entry, current, action)) = action {
                    let cooked = cooked.to_owned();
                    let (tx, rx) = mpsc::channel();
                    self.job = Some(rx);
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(restore(&cooked, &entry, &current, action));
                        ctx.request_repaint();
                    });
                }
            });
    }
}

fn render_point(
    ui: &mut egui::Ui,
    point: HistoryPoint<'_>,
    editors: &mut HashMap<usize, Recipe>,
    enabled: bool,
    primary: bool,
) -> Option<(Entry, String, Action)> {
    let entry = point.entry;
    let latest = point.latest();
    let enabled = enabled && !point.current.linked;
    ui.push_id(point.index, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.weak(&entry.file);
            if let Ok(utc) = time::OffsetDateTime::from_unix_timestamp(entry.unix_seconds) {
                let date = time::UtcOffset::local_offset_at(utc)
                    .map_or(utc, |offset| utc.to_offset(offset));
                ui.weak(format!(
                    "{} {:02}:{:02}",
                    date.date(),
                    date.hour(),
                    date.minute()
                ));
            }
        });
        if entry.linked {
            ui.weak(t("upk-history-linked"));
            return None;
        }
        if let Some(swap) = point
            .swap
            .filter(|swap| !swap.source_upk.eq_ignore_ascii_case(&swap.target_upk))
        {
            ui.label(t_args(
                "upk-history-swapped-with",
                &[("item", swap.target_name.as_str().into())],
            ));
        } else if !primary || entry.recipe.is_none() && entry.swap.is_some() {
            ui.label(entry.label.replace('\u{2192}', "-"));
        }
        if let Some(recipe) = &entry.recipe {
            if recipe.paint != painted_swap::SwapPaint::Default {
                ui.label(t_args(
                    "upk-history-colour",
                    &[("value", recipe.paint.description().into())],
                ));
            }
            if let Some(alignment) = recipe
                .alignment
                .filter(|value| *value != wheel_alignment::Settings::default())
            {
                ui.label(t_args(
                    "upk-history-alignment",
                    &[("value", alignment.description().into())],
                ));
            }
        }
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            let restore_original = primary && latest && !point.has_linked;
            let revert = if restore_original {
                "upk-history-restore-original"
            } else if primary && latest {
                "upk-history-revert"
            } else {
                "upk-history-before"
            };
            if ui
                .add_enabled(enabled, egui::Button::new(t(revert)))
                .clicked()
            {
                action = Some(if restore_original {
                    Action::Original
                } else {
                    Action::Before
                });
            }
            if !latest
                && ui
                    .add_enabled(enabled, egui::Button::new(t("upk-history-after")))
                    .clicked()
            {
                action = Some(Action::After);
            }
        });
        if latest && let Some(recipe) = &entry.recipe {
            let edit_label = if recipe.alignment.is_some() {
                "upk-history-edit"
            } else {
                "upk-history-edit-colour"
            };
            egui::CollapsingHeader::new(t(edit_label)).show(ui, |ui| {
                let edit = editors.entry(point.index).or_insert_with(|| recipe.clone());
                painted_swap::controls(ui, &mut edit.paint);
                if let Some(alignment) = &mut edit.alignment {
                    alignment.history_controls(ui);
                    if ui.button(t("upk-history-reset-alignment")).clicked() {
                        *alignment = wheel_alignment::Settings::default();
                    }
                }
                if ui
                    .add_enabled(enabled, egui::Button::new(t("upk-history-apply")))
                    .clicked()
                {
                    action = Some(Action::Edit(edit.clone()));
                }
            });
        }
        action.map(|action| {
            let checkpoint = if matches!(action, Action::Original) {
                point.original
            } else {
                entry
            };
            (checkpoint.clone(), point.current.after.clone(), action)
        })
    })
    .inner
}

pub(super) fn edit_current(cooked: &Path, file: &str, recipe: Recipe) -> Result<(), String> {
    let row = entries(&root(cooked)?)?
        .into_iter()
        .find(|row| row.file.eq_ignore_ascii_case(file))
        .ok_or("No history for this package")?;
    restore(cooked, &row, &row.after, Action::Edit(recipe))
}

fn restore(cooked: &Path, entry: &Entry, expected: &str, action: Action) -> Result<(), String> {
    backup_guard::check_install(cooked, &cooked.join("Backups"))?;
    let _guard = INSTALL.lock().map_err(|e| e.to_string())?;
    let live = cooked.join(&entry.file);
    if location(cooked, &live).as_deref() != Some(entry.file.as_str())
        || Path::new(&entry.file)
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Invalid history package path".into());
    }
    let store = root(cooked)?;
    let before = read(&live)?;
    if hash(&before) != expected {
        return Err(t("upk-history-conflict").into());
    }
    let rows = entries(&store)?;
    let current_row = rows
        .iter()
        .find(|row| row.file.eq_ignore_ascii_case(&entry.file));
    if entry.linked || current_row.is_none_or(|row| row.linked || row.after != expected) {
        return Err(t("upk-history-conflict"));
    }
    if matches!(action, Action::Original)
        && rows
            .iter()
            .any(|row| row.file.eq_ignore_ascii_case(&entry.file) && row.linked)
    {
        return Err(t("upk-history-linked"));
    }
    let prior = rows
        .iter()
        .find(|row| row.file.eq_ignore_ascii_case(&entry.file) && row.after == entry.before);
    let last_swap = rows
        .iter()
        .filter(|row| row.file.eq_ignore_ascii_case(&entry.file))
        .find_map(|row| row.swap.as_ref());
    let current = last_swap.map(|swap| &swap.after);
    let (after, label, recipe, destination) = match action {
        Action::Edit(recipe) => (
            render_recipe(cooked, &recipe)?,
            t("upk-history-edited"),
            Some(recipe),
            current,
        ),
        Action::Original => (
            load_blob(&store, &entry.before)?,
            t("upk-history-restored"),
            None,
            rows.iter()
                .rev()
                .filter(|row| row.file.eq_ignore_ascii_case(&entry.file))
                .find_map(|row| row.swap.as_ref().map(|swap| &swap.before)),
        ),
        Action::Before => (
            load_blob(&store, &entry.before)?,
            t("upk-history-restored"),
            prior.and_then(|row| row.recipe.clone()),
            entry
                .swap
                .as_ref()
                .map(|swap| &swap.before)
                .or_else(|| {
                    prior
                        .and_then(|row| row.swap.as_ref())
                        .map(|swap| &swap.after)
                })
                .or_else(|| last_swap.map(|swap| &swap.before)),
        ),
        Action::After => (
            load_blob(&store, &entry.after)?,
            t("upk-history-restored"),
            entry.recipe.clone(),
            entry
                .swap
                .as_ref()
                .map(|swap| &swap.after)
                .or_else(|| last_swap.map(|swap| &swap.before)),
        ),
    };
    if hebnix_sdk::process::is_rocket_league_running() {
        return Err(t("upk-history-close-game"));
    }
    let caption = recipe
        .as_ref()
        .or(entry.recipe.as_ref())
        .map(|recipe| recipe.caption.as_str())
        .unwrap_or_default();
    let label = if caption.is_empty() {
        label
    } else {
        format!("{caption} · {label}")
    };
    let mut transition = None;
    if let (Some(current), Some(destination)) = (current, destination) {
        let mut destination = destination.clone();
        if let (Some(active), Some(recipe)) = (&mut destination.active, &recipe) {
            active.paint = recipe.paint;
        }
        let backups = cooked.join("Backups");
        let mut replacements = Vec::new();
        for (name, id) in &destination.sidecars {
            if !valid_sidecar(name) {
                return Err("Invalid swap attachment".into());
            }
            let path = cooked.join(name);
            let original = read(&path)?;
            let expected = current
                .sidecars
                .iter()
                .find(|(file, _)| file == name)
                .map(|(_, id)| id.as_str())
                .unwrap_or(id);
            if hash(&original) != expected {
                return Err(t("upk-history-conflict"));
            }
            let desired = load_blob(&store, id)?;
            if desired != original {
                replacements.push((path, Some(original), Some(desired)));
            }
        }
        for (name, id) in &destination.backups {
            if !valid_sidecar(name) {
                return Err("Invalid swap backup".into());
            }
            let path = backups.join(format!("{name}.bak"));
            let original = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.to_string()),
            };
            let desired = load_blob(&store, id)?;
            if original.as_deref() != Some(&desired) {
                replacements.push((path, original, Some(desired)));
            }
        }
        for (name, id) in &current.backups {
            if !valid_sidecar(name) {
                return Err("Invalid swap backup".into());
            }
            if destination.backups.iter().any(|(file, _)| file == name) {
                continue;
            }
            let path = backups.join(format!("{name}.bak"));
            if path.is_file() {
                let original = read(&path)?;
                if hash(&original) != *id {
                    return Err(t("upk-history-conflict"));
                }
                replacements.push((path, Some(original), None));
            }
        }
        let (path, old, new) =
            super::swapper::history_manifest(&backups, &entry.file, destination.active.clone())?;
        replacements.push((path, old, Some(new)));
        transition = Some(SwapTransition {
            before: current.clone(),
            after: destination,
        });
        for (index, (path, _, bytes)) in replacements.iter().enumerate() {
            if let Err(error) = replace_optional(path, bytes.as_deref()) {
                rollback(&replacements[..index]);
                return Err(error);
            }
        }
        let result = commit_locked(
            cooked, &live, &before, &after, &label, recipe, transition, true, false,
        );
        if result.is_err() {
            rollback(&replacements);
        }
        return result;
    }
    commit_locked(
        cooked, &live, &before, &after, &label, recipe, transition, true, false,
    )
}

fn valid_sidecar(name: &str) -> bool {
    valid_file(name)
        || (!name.contains(['/', '\\', ':']) && name.to_ascii_lowercase().ends_with(".bnk"))
}

type Replacement = (PathBuf, Option<Vec<u8>>, Option<Vec<u8>>);

fn replace_optional(path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
    match bytes {
        Some(bytes) => replace(path, bytes),
        None => fs::remove_file(path).map_err(|error| error.to_string()),
    }
}

fn rollback(replacements: &[Replacement]) {
    for (path, old, _) in replacements.iter().rev() {
        if let Err(error) = replace_optional(path, old.as_deref()) {
            tracing::error!("Could not roll back {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_rank_entries_are_hidden_but_protect_other_history_points() {
        let row = |file: &str, label: &str| Entry {
            file: file.into(),
            label: label.into(),
            before: String::new(),
            after: String::new(),
            recipe: None,
            swap: None,
            linked: true,
            unix_seconds: 0,
            cleared: false,
        };
        let shared = "GFX_PlayMenu_SF.upk";
        let rows = [
            row(shared, "Rank display"),
            row("GFX_Hud_SF.upk", "Restore Rank display"),
            row(shared, "Title overrides"),
        ];
        let groups = group_entries(&rows);
        assert_eq!(groups.len(), 1);
        let points = &groups[0].1;
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].entry.label, "Title overrides");
        assert_eq!(points[0].current.label, "Rank display");
        assert!(points[0].has_linked);
    }

    #[test]
    fn fennec_history_groups_alignment_and_swap_across_packages() {
        let source = "body_grain_SF.upk";
        let target = "Body_Octane_SF.upk";
        let active = super::super::swapper::ActiveSwap::deserialize(serde_json::json!({
            "category": "bodies",
            "source_name": "Fennec",
            "source_upk": "Body_Grain_SF.upk",
            "target_name": "Octane",
            "target_upk": target,
        }))
        .unwrap();
        let stock = SwapPoint {
            active: None,
            sidecars: Vec::new(),
            backups: Vec::new(),
        };
        let swapped = SwapPoint {
            active: Some(active),
            ..stock.clone()
        };
        let point = |file: &str, swap| Entry {
            file: file.to_owned(),
            label: String::new(),
            before: String::new(),
            after: String::new(),
            recipe: None,
            swap,
            linked: false,
            unix_seconds: 0,
            cleared: false,
        };
        let alignment = point(source, None);
        let swap = point(
            target,
            Some(SwapTransition {
                before: stock.clone(),
                after: swapped.clone(),
            }),
        );
        let target_edit = point(target, None);
        let unrelated = point("Body_Dominus_SF.upk", None);
        let rows = [target_edit, unrelated, swap, alignment];
        let groups = group_entries(&rows);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1.len(), 3);
        assert_eq!(groups[0].1[0].swap.unwrap().target_name, "Octane");
        assert_eq!(groups[0].1[0].priority(), 3);
        assert_eq!(groups[0].1[2].entry.file, source);
        assert!(groups[0].1[2].latest());

        let restored = point(
            target,
            Some(SwapTransition {
                before: swapped,
                after: stock,
            }),
        );
        let reverted_rows = [restored, rows[3].clone()];
        let reverted = group_entries(&reverted_rows);
        assert_eq!(reverted.len(), 1);
        assert!(reverted[0].1[0].swap.is_none());
        assert!(reverted[0].1[1].priority() > reverted[0].1[0].priority());
    }

    #[test]
    fn checkpoints_restore_swap_sidecars_and_refuse_unrecorded_changes() {
        let root = std::env::temp_dir().join(format!("hebnix-history-{}", rand::random::<u64>()));
        let cooked = root.join("TAGame/CookedPCConsole");
        let backups = cooked.join("Backups");
        fs::create_dir_all(&backups).unwrap();
        fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
        fs::write(root.join("Binaries/Win64/RocketLeague.exe"), b"test build").unwrap();

        backup_guard::check_install(&cooked, &backups).unwrap();
        let file = "Body_Octane_SF.upk";
        let live = cooked.join(file);
        let preview = "Body_Octane_T_SF.upk";
        let preview_live = cooked.join(preview);
        let sidecars = [preview.to_owned()];
        fs::write(&live, b"stock").unwrap();
        fs::write(&preview_live, b"stock preview").unwrap();
        let before = swap_point(&cooked, &backups, file, &sidecars, None).unwrap();
        fs::write(&live, b"swapped").unwrap();
        fs::write(&preview_live, b"swapped preview").unwrap();
        let active = serde_json::from_value(serde_json::json!({
            "category": "bodies",
            "source_name": "Fennec",
            "source_upk": "Body_Grain_SF.upk",
            "target_name": "Octane",
            "target_upk": file,
            "target_thumbnail": preview,
        }))
        .unwrap();
        let original_backup = backups.join(format!("{file}.bak"));
        fs::write(&original_backup, b"stock").unwrap();
        let after = swap_point(&cooked, &backups, file, &sidecars, Some(active)).unwrap();
        record_swap(
            &cooked,
            &live,
            b"stock",
            "Fennec → Octane",
            None,
            SwapTransition { before, after },
        )
        .unwrap();
        let store = root
            .join("TAGame/CookedPCConsole/Backups/UPKHistory")
            .join(backup_guard::build_id(&cooked).unwrap());
        let row = entries(&store).unwrap().remove(0);
        fs::write(&preview_live, b"separate preview edit").unwrap();
        assert!(restore(&cooked, &row, &row.after, Action::Original).is_err());
        assert_eq!(read(&live).unwrap(), b"swapped");
        assert_eq!(read(&preview_live).unwrap(), b"separate preview edit");

        fs::write(&preview_live, b"swapped preview").unwrap();
        restore(&cooked, &row, &row.after, Action::Original).unwrap();
        assert_eq!(read(&live).unwrap(), b"stock");
        assert_eq!(read(&preview_live).unwrap(), b"stock preview");
        assert!(!original_backup.exists());
        assert!(entries(&store).unwrap().is_empty());
        assert!(restore(&cooked, &row, &row.before, Action::After).is_err());

        install(&cooked, &live, b"first edit", "First edit", None).unwrap();
        install(&cooked, &live, b"second edit", "Second edit", None).unwrap();
        let rows = entries(&store).unwrap();
        restore(&cooked, &rows[0], &rows[0].after, Action::Before).unwrap();
        assert_eq!(read(&live).unwrap(), b"first edit");
        assert_eq!(entries(&store).unwrap().len(), 3);
        install(&cooked, &live, b"stock", "Restore in editor", None).unwrap();
        assert!(entries(&store).unwrap().is_empty());

        let excluded = cooked.join("TAGame.UPK");
        fs::write(&excluded, b"original").unwrap();
        install(&cooked, &excluded, b"modified", "Excluded", None).unwrap();
        assert!(entries(&store).unwrap().is_empty());
        assert_eq!(read(&excluded).unwrap(), b"modified");
        let _ = fs::remove_dir_all(root);
    }
}

/// Check a specific file again immediately before patching or restoring it.
pub(super) fn check_live_package(cooked: &Path, live: &Path) -> Result<(), String> {
    let backups = cooked.join("Backups");
    backup_guard::check_install(cooked, &backups)?;
    let Some(file) = location(cooked, live) else {
        return Ok(());
    };
    let store = root(cooked)?;
    let expected = package_expectations(&store)?
        .into_iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case(&file));
    let replaced = match expected {
        Some((_, hash, _)) => live_hash(cooked, &file)?.as_deref() != Some(&hash),
        None => backup_guard::different_version(&backups.join(format!("{file}.bak")), live)?,
    };
    if replaced {
        return Err(format!("{file} changed outside Hebnix. Close Rocket League and run repatch in the Hebnix Console."));
    }
    Ok(())
}

fn package_expectations(store: &Path) -> Result<Vec<(String, String, bool)>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut expected = Vec::new();
    for row in entries(store)? {
        if seen.insert(row.file.to_ascii_lowercase()) {
            let patched = row.before != row.after;
            expected.push((row.file, row.after, patched));
        }
        if let Some(swap) = row.swap {
            for (name, hash) in swap.after.sidecars {
                if valid_sidecar(&name) && seen.insert(name.to_ascii_lowercase()) {
                    let patched = swap.before.sidecars.iter().any(|(file, before)| file == &name && before != &hash);
                    expected.push((name, hash, patched));
                }
            }
        }
    }
    Ok(expected)
}

pub(super) fn tracked_items(cooked: &Path) -> Result<std::collections::HashSet<String>, String> {
    Ok(entries(&root(cooked)?)?.into_iter()
        .filter(|row| row.swap.is_some() || matches!(row.label.as_str(),
            "Ball textures" | "Custom decal" | "Decal variant"))
        .map(|row| row.file.to_ascii_lowercase())
        .collect())
}

/// Compare against the last bytes Hebnix wrote, since a vanilla backup
/// alone cannot distinguish a normal patch from a launcher replacement.
pub(super) fn replaced_packages(
    cooked: &Path,
    backups: &Path,
    build: &str,
) -> Result<(std::collections::HashSet<String>, usize), String> {
    let mut tracked = std::collections::HashSet::new();
    let mut changed = 0;
    for (file, expected, _) in package_expectations(&backups.join(DIR).join(build))? {
        tracked.insert(file.to_ascii_lowercase());
        if live_hash(cooked, &file)?.as_deref() != Some(&expected) {
            changed += 1;
        }
    }
    Ok((tracked, changed))
}

/// Packages from the active item history that still contain our bytes but
/// could not be reset to a verified input. Never promote these to new stock.
pub(super) fn remaining_repatch_item(
    cooked: &Path,
    backups: &Path,
    build: &str,
) -> Result<Option<String>, String> {
    let store = backups.join(DIR).join(build);
    let rows = entries(&store)?;
    for row in &rows {
        let direct_item = matches!(row.label.as_str(), "Ball textures" | "Custom decal" | "Decal variant");
        if direct_item && row.before != row.after && live_hash(cooked, &row.file)?.as_deref() == Some(&row.after) {
            return Ok(Some(row.file.clone()));
        }
        if let Some(swap) = &row.swap {
            if row.before != row.after && live_hash(cooked, &row.file)?.as_deref() == Some(&row.after) {
                return Ok(Some(row.file.clone()));
            }
            for (name, expected) in &swap.after.sidecars {
                if valid_sidecar(name)
                    && swap.before.sidecars.iter().any(|(file, before)| file == name && before != expected)
                    && live_hash(cooked, name)?.as_deref() == Some(expected)
                {
                    return Ok(Some(name.clone()));
                }
            }
        }
    }
    Ok(None)
}

fn live_hash(cooked: &Path, file: &str) -> Result<Option<String>, String> {
    let live = super::game_path::resolve(cooked, file);
    if Path::new(file).components().any(|part| !matches!(part, std::path::Component::Normal(_)))
        || (location(cooked, &live).is_none() && !valid_sidecar(file))
    {
        return Err("Invalid UPK history path".into());
    }
    let mut input = match fs::File::open(&live) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", live.display())),
    };
    type Stamp = (u64, std::time::SystemTime, Option<std::time::SystemTime>);
    static HASHES: std::sync::LazyLock<Mutex<HashMap<PathBuf, (Stamp, String)>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let stamp = input.metadata().ok().and_then(|meta| {
        Some((meta.len(), meta.modified().ok()?, meta.created().ok()))
    });
    if let Some(stamp) = stamp
        && let Ok(hashes) = HASHES.lock()
        && let Some((saved, hash)) = hashes.get(&live)
        && *saved == stamp
    {
        return Ok(Some(hash.clone()));
    }
    let mut digest = Sha256::new();
    std::io::copy(&mut input, &mut digest).map_err(|error| format!("{}: {error}", live.display()))?;
    let hash = hex::encode(digest.finalize());
    if let Some(stamp) = stamp && let Ok(mut hashes) = HASHES.lock() {
        hashes.insert(live, (stamp, hash.clone()));
    }
    Ok(Some(hash))
}

pub(super) fn unchanged_ui_packages(
    cooked: &Path,
    backups: &Path,
    build: &str,
) -> Result<std::collections::HashSet<String>, String> {
    let store = backups.join(DIR).join(build);
    let rows = entries(&store)?;
    let mut retained = std::collections::HashSet::new();
    for (file, expected, _) in package_expectations(&store)? {
        let hud = file.eq_ignore_ascii_case(super::title_patcher::BOOST_HUD);
        let title = rows.iter().find(|row| row.file.eq_ignore_ascii_case(&file))
            .is_some_and(|row| row.linked && matches!(row.label.as_str(),
                "Title overrides" | "Restore Title overrides" | "Rank display" | "Restore Rank display"));
        let workshop = matches!(file.to_ascii_lowercase().as_str(),
            "gfx_workshopmenu_sf.upk" | "gfx_serverbrowsermenu_sf.upk" | "gfx_winnermenu_sf.upk");
        if (hud || title || workshop)
            && !rows.iter().any(|row| row.file.eq_ignore_ascii_case(&file) && row.swap.is_some())
            && live_hash(cooked, &file)?.as_deref() == Some(&expected)
        {
            retained.insert(file.to_ascii_lowercase());
        }
    }
    Ok(retained)
}

/// Keep restore points for UI packages that item recovery will leave in place.
pub(super) fn carry_ui_history(
    source: &Path,
    destination: &Path,
    build: &str,
    retained: &std::collections::HashSet<String>,
) -> Result<(), String> {
    let source = source.join(DIR).join(build);
    let destination = destination.join(DIR).join(build);
    let events = match fs::read_dir(source.join("events")) {
        Ok(events) => events,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for event in events {
        let path = event.map_err(|error| error.to_string())?.path();
        if !path.extension().is_some_and(|extension| extension == "json") {
            continue;
        }
        let bytes = read(&path)?;
        let entry: Entry = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if !retained.contains(&entry.file.to_ascii_lowercase()) {
            continue;
        }
        for id in [&entry.before, &entry.after].into_iter()
            .chain(entry.recipe.iter().map(|recipe| &recipe.baseline))
        {
            if !id.is_empty() && !blob_path(&destination, id)?.is_file() {
                store_blob(&destination, &load_blob(&source, id)?)?;
            }
        }
        let events = destination.join("events");
        fs::create_dir_all(&events).map_err(|error| error.to_string())?;
        replace(&events.join(path.file_name().ok_or("Missing UI history filename")?), &bytes)?;
    }
    Ok(())
}

/// Roll back known item changes before archiving their restore points. The
/// package's bytes identify its history even after the executable changes.
/// A package replaced by Rocket League is already the new input.
pub(super) fn restore_repatch_baselines(
    cooked: &Path,
    backups: &Path,
    build: &str,
    retained: &std::collections::HashSet<String>,
) -> Result<(), String> {
    let _guard = INSTALL.lock().map_err(|error| error.to_string())?;
    let store = backups.join(DIR).join(build);
    let rows = entries(&store)?;
    let mut changes = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for latest in &rows {
        if !seen.insert(latest.file.to_ascii_lowercase())
            || retained.contains(&latest.file.to_ascii_lowercase())
            || !valid_file(&latest.file)
            || latest.file.contains(['/', '\\'])
        {
            continue;
        }
        let history: Vec<_> = rows.iter().filter(|row| row.file.eq_ignore_ascii_case(&latest.file)).collect();
        let first = history.last().unwrap();
        if history.windows(2).any(|pair| pair[0].before != pair[1].after)
            || history.iter().any(|row| row.linked && !matches!(
                row.label.as_str(), "Ball textures" | "Custom decal" | "Decal variant"
            ))
            || first.swap.as_ref().is_some_and(|swap| swap.before.active.is_some())
        {
            continue;
        }
        let installed = live_hash(cooked, &latest.file)?;
        let installed_patch = history.iter()
            .find(|row| installed.as_deref() == Some(row.after.as_str()))
            .map_or_else(|| latest.after.clone(), |row| row.after.clone());
        let mut files = HashMap::from([(latest.file.clone(), (first.before.clone(), installed_patch))]);
        for transition in history.iter().rev().filter_map(|row| row.swap.as_ref()) {
            for (name, before) in &transition.before.sidecars {
                if valid_sidecar(name) {
                    files.entry(name.clone()).or_insert_with(|| (before.clone(), before.clone()));
                }
            }
            for (name, after) in &transition.after.sidecars {
                if let Some((_, current)) = files.get_mut(name) {
                    current.clone_from(after);
                }
            }
        }
        // The backup must agree with the history's starting point. This
        // excludes histories that started on top of a pre-existing patch.
        let backup_name = format!("{}.bak", latest.file);
        let car_backups = backups.join("CarPatcher");
        let backup_dirs = [car_backups.as_path(), backups];
        let baseline = backup_dirs.into_iter().find_map(|dir| {
            let bytes = fs::read(super::game_path::resolve(dir, &backup_name)).ok()?;
            (hash(&bytes) == first.before).then_some(bytes)
        }).or_else(|| {
            let recorded_backup = history.iter().filter_map(|row| row.swap.as_ref())
                .flat_map(|swap| &swap.after.backups)
                .any(|(name, id)| name.eq_ignore_ascii_case(&latest.file) && id == &first.before);
            // The first swap records its input before it installs anything.
            // It is also a restore point when the standalone backup is gone.
            if recorded_backup || first.swap.as_ref().is_some_and(|swap| swap.before.active.is_none()) {
                load_blob(&store, &first.before).ok()
            } else {
                None
            }
        });
        let Some(baseline) = baseline else {
            continue;
        };
        for (name, (before, after)) in files {
            let live = super::game_path::resolve(cooked, &name);
            if before == after || !fs::read(&live).is_ok_and(|bytes| hash(&bytes) == after) {
                continue;
            }
            let bytes = if name == latest.file {
                baseline.clone()
            } else {
                load_blob(&store, &before).or_else(|_| {
                let saved = read(&backups.join(format!("{name}.bak")))?;
                if hash(&saved) == before { Ok(saved) } else { Err(format!("No verified backup for {name}")) }
                })?
            };
            let key = name.to_ascii_lowercase();
            if let Some((_, expected, original)) = changes.get(&key) {
                if expected != &after || original != &bytes {
                    return Err(format!("Conflicting UPK history for {name}"));
                }
            } else {
                changes.insert(key, (name, after, bytes));
            }
        }
    }
    if !changes.is_empty() && hebnix_sdk::process::is_rocket_league_running() {
        return Err("Close Rocket League before restoring item package baselines.".into());
    }
    for (_, (name, expected, bytes)) in changes {
        let path = super::game_path::resolve(cooked, &name);
        if hash(&read(&path)?) != expected {
            return Err(format!("{name} changed while preparing its recovery"));
        }
        replace(&path, &bytes)?;
    }
    Ok(())
}

pub(super) fn remaining_patch(cooked: &Path, backups: &Path, retained: &std::collections::HashSet<String>) -> Result<Option<String>, String> {
    let history = backups.join(DIR);
    let stores = match backup_guard::recorded_build(backups)? {
        Some(build) => vec![history.join(build)],
        None => {
            let entries = match fs::read_dir(&history) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            entries.map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        }
    };
    for store in stores {
        for (file, expected, patched) in package_expectations(&store)? {
            if patched && live_hash(cooked, &file)?.as_deref() == Some(&expected)
                && !retained.contains(&file.to_ascii_lowercase())
            {
                return Ok(Some(file));
            }
        }
    }
    Ok(None)
}
