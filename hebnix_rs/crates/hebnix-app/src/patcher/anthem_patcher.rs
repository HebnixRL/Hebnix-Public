mod audio;
mod bank;
#[cfg(test)]
mod tests;

use self::{
    audio::{Audio, Player},
    bank::{Bank, Clip},
};
use super::{
    backup_guard,
    upk_package::{UpkPackage, strip},
};
use crate::{config::Config, messages::AppMsg};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
struct Region {
    label: String,
    event: String,
    bank: String,
    clip: Clip,
    preview_available: bool,
}
#[derive(Clone)]
struct Anthem {
    key: String,
    product_id: Option<i64>,
    name: String,
    upk: String,
    internal: String,
    thumbnail: String,
    regions: Vec<Region>,
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct SavedPatch {
    name: String,
    upk: String,
    internal: String,
    source: String,
    starts: BTreeMap<String, f64>,
}
type Saved = BTreeMap<String, SavedPatch>;
#[derive(Clone)]
struct Editor {
    anthem: Anthem,
    audio: Option<Audio>,
    source_name: String,
    start: f64,
    cursor: f64,
}
impl Editor {
    fn longest_region(&self) -> Option<&Region> {
        self.anthem
            .regions
            .iter()
            .max_by(|a, b| a.clip.duration.total_cmp(&b.clip.duration))
    }
    fn patch_starts(&self, audio: &Audio) -> Result<BTreeMap<String, f64>, String> {
        let longest = self
            .longest_region()
            .ok_or("No anthem audio regions were found")?;
        audio
            .section_bounds(self.start, longest.clip.duration)
            .map_err(|e| {
                format!(
                    "The longest region needs {:.3}s: {e}",
                    longest.clip.duration
                )
            })?;
        let mut starts = BTreeMap::new();
        for region in &self.anthem.regions {
            starts.insert(region.label.clone(), self.start);
        }
        Ok(starts)
    }
}
enum JobResult {
    Scan(Vec<Anthem>),
    Import(Audio, String, bool),
    Preview(Audio),
    Applied(HashMap<String, String>, Saved),
}

pub struct AnthemPatcherState {
    base: PathBuf,
    catalog: Option<Value>,
    entries: Vec<Anthem>,
    scanned_path: Option<PathBuf>,
    saved: Saved,
    pub active_anthems: HashMap<String, String>,
    search: String,
    patched_only: bool,
    owned_only: bool,
    page: usize,
    editor: Option<Editor>,
    player: Option<Player>,
    player_imported: bool,
    receiver: Option<Receiver<Result<JobResult, String>>>,
    status: String,
    error: Option<String>,
    thumbnails: HashMap<String, Option<Arc<[u8]>>>,
    thumbnail_jobs: Sender<(String, PathBuf, egui::Context)>,
    thumbnail_results: Receiver<(String, Option<Arc<[u8]>>)>,
}

fn file_name(name: &str, extension: &str) -> Result<String, String> {
    let p = Path::new(name);
    if p.components().count() != 1
        || !p
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case(extension))
        || name.contains(['/', '\\', ':'])
    {
        return Err("Invalid anthem asset filename".into());
    }
    Ok(name.to_string())
}
fn string_property(p: &UpkPackage, reference: i32, name: &str) -> Result<String, String> {
    let e = p
        .exports
        .get(reference.checked_sub(1).ok_or("Invalid anthem reference")? as usize)
        .ok_or("External anthem cue is unsupported")?;
    let prop = p
        .parse_props(e)
        .into_iter()
        .find(|v| v.name == name && v.tag_type == "StrProperty")
        .ok_or_else(|| format!("Missing anthem {name}"))?;
    let at = e.serial_offset + prop.value_offset;
    let n = p.read_int(at)?;
    if n <= 0 || n as usize + 4 > prop.size {
        return Err("Invalid anthem event string".into());
    }
    String::from_utf8(
        p.image
            .get(at + 4..at + 3 + n as usize)
            .ok_or("Truncated anthem event")?
            .to_vec(),
    )
    .map_err(|e| e.to_string())
}
fn reference_property(p: &UpkPackage, reference: i32, name: &str) -> Result<i32, String> {
    let e = p
        .exports
        .get(reference.checked_sub(1).ok_or("Invalid anthem reference")? as usize)
        .ok_or("External anthem reference is unsupported")?;
    let prop = p
        .parse_props(e)
        .into_iter()
        .find(|v| v.name == name && v.tag_type == "ObjectProperty")
        .ok_or_else(|| format!("Missing anthem {name}"))?;
    p.read_int(e.serial_offset + prop.value_offset)
}
fn baseline(cooked: &Path, bank: &str) -> PathBuf {
    let backup = cooked.join("Backups/Anthems").join(format!("{bank}.bak"));
    if backup.is_file() {
        backup
    } else {
        cooked.join(bank)
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension(format!("hbnx-anthem-{}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn inspect(p: &UpkPackage, internal: &str, cooked: &Path) -> Result<Vec<Region>, String> {
    let product = p
        .exports
        .iter()
        .find(|e| {
            strip(&p.name_of(e.object_name)).eq_ignore_ascii_case(internal)
                && strip(&p.class_of(e)) == "ProductAsset_MusicStingers_TA"
        })
        .ok_or("Anthem asset is missing from this package")?;
    let mut regions = Vec::new();
    for (label, property) in [
        ("Goal", "GoalScoredStinger"),
        ("MVP", "MVPAnthem"),
        ("Epic Save", "EpicSaveStinger"),
    ] {
        let cue = match reference_property(p, product.table_index as i32 + 1, property) {
            Ok(cue) if cue > 0 => cue,
            _ if label == "Epic Save" => continue,
            Err(e) => return Err(e),
            _ => return Err(format!("{label} has no anthem cue")),
        };
        let event = string_property(p, cue, "StartEvent")?;
        let bank_ref = reference_property(p, cue, "RequiredBank")?;
        let bank_export = p
            .exports
            .get((bank_ref - 1) as usize)
            .ok_or("Anthem bank is external")?;
        let bank_name = file_name(
            &format!("{}.bnk", strip(&p.name_of(bank_export.object_name))),
            "bnk",
        )?;
        let bank = Bank::parse(
            &fs::read(baseline(cooked, &bank_name))
                .map_err(|e| format!("Missing audio bank {bank_name}: {e}"))?,
        )?;
        let clips = bank.clips(&event)?;
        let count = clips.len();
        for (i, clip) in clips.into_iter().enumerate() {
            let preview_available = cooked.join(format!("{}.wem", clip.source)).is_file()
                || bank.embedded(clip.source).is_ok_and(|media| {
                    media.get(..4) == Some(b"RIFF")
                        && bank::u32_at(&media, 4)
                            .is_ok_and(|size| size as usize + 8 <= media.len())
                });
            let label = if count == 1 {
                label.into()
            } else {
                format!("{label} {}", i + 1)
            };
            regions.push(Region {
                label,
                event: event.clone(),
                bank: bank_name.clone(),
                clip,
                preview_available,
            });
        }
    }
    let mut tracks = HashSet::new();
    if regions
        .iter()
        .any(|r| !tracks.insert((r.bank.clone(), r.clip.track)))
    {
        return Err(
            "Anthem events share a music track; independent replacement is unsupported".into(),
        );
    }
    Ok(regions)
}
fn scan(catalog: Value, cooked: &Path) -> Result<Vec<Anthem>, String> {
    let values = catalog["anthems"]
        .as_array()
        .ok_or("Anthem catalog has no anthems list")?;
    let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for v in values {
        let Some(internal) = v["internal_name"].as_str() else {
            continue;
        };
        if internal.to_ascii_lowercase().starts_with("album_") {
            continue;
        }
        let Some(upk) = v["upk_path"].as_str() else {
            continue;
        };
        if file_name(upk, "upk").is_ok() && cooked.join(upk).is_file() {
            groups.entry(upk.to_string()).or_default().push(v);
        }
    }
    let mut entries = Vec::new();
    for (upk, values) in groups {
        let package = UpkPackage::load(&cooked.join(&upk));
        for v in values {
            let internal = v["internal_name"].as_str().unwrap().to_string();
            let result = package
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|p| inspect(p, &internal, cooked));
            let (regions, error) = match result {
                Ok(r) => (r, None),
                Err(e) => (Vec::new(), Some(e)),
            };
            let name = v["name"].as_str().unwrap_or(&internal).to_string();
            let key = format!("{upk}/{internal}").to_ascii_lowercase();
            entries.push(Anthem {
                key,
                product_id: v["id"]
                    .as_i64()
                    .or_else(|| v["id"].as_str().and_then(|id| id.parse().ok())),
                name,
                upk: upk.clone(),
                internal,
                thumbnail: v["thumbnail"]
                    .as_str()
                    .filter(|name| file_name(name, "upk").is_ok())
                    .unwrap_or(&upk)
                    .to_string(),
                regions,
                error,
            });
        }
    }
    entries.sort_by(|a, b| {
        a.error
            .is_some()
            .cmp(&b.error.is_some())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

// Rebuild each affected bank from its original, composing every enabled anthem.
// Stage all output first and roll back the entire bank set on a failed write.
fn apply(
    cooked: &Path,
    desired: HashMap<String, String>,
    saved: Saved,
) -> Result<JobResult, String> {
    let backups = cooked.join("Backups/Anthems");
    backup_guard::check_install(cooked, &backups)?;
    fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
    let mut banks: BTreeMap<String, Vec<(Region, Vec<u8>, String)>> = BTreeMap::new();
    // Include every existing anthem backup, so disabling a patch restores its bank.
    for entry in fs::read_dir(&backups)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
    {
        if let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.strip_suffix(".bnk.bak"))
        {
            banks.entry(format!("{name}.bnk")).or_default();
        }
    }
    for key in desired.keys() {
        let patch = saved.get(key).ok_or("Saved anthem patch is missing")?;
        let package = UpkPackage::load(&cooked.join(file_name(&patch.upk, "upk")?))?;
        let source = Audio::decode(Path::new(&patch.source))?;
        for region in inspect(&package, &patch.internal, cooked)? {
            let Some(&start) = patch.starts.get(&region.label) else {
                continue;
            };
            let wav = source.section(start, region.clip.duration)?.wem();
            banks
                .entry(region.bank.clone())
                .or_default()
                .push((region, wav, key.clone()));
        }
    }
    let mut staged = Vec::new();
    for (name, replacements) in banks {
        let live = cooked.join(file_name(&name, "bnk")?);
        let backup = backups.join(format!("{name}.bak"));
        let original =
            fs::read(if backup.is_file() { &backup } else { &live }).map_err(|e| e.to_string())?;
        let mut bank = Bank::parse(&original)?;
        let mut used = bank.media_ids();
        let mut tracks = HashSet::new();
        for (region, wav, key) in &replacements {
            if !tracks.insert(region.clip.track) {
                return Err("Enabled anthem patches overlap the same music track".into());
            }
            let mut id = bank::event_id(&format!("hebnix/{key}/{}", region.label)) | 0x80000000;
            while used.contains(&id) {
                id = id.wrapping_add(1);
            }
            used.insert(id);
            bank.replace_clip(&region.clip, id, wav)?;
        }
        let bytes = if replacements.is_empty() {
            original.clone()
        } else {
            bank.encode()
        };
        if bytes.len() > 256 * 1024 * 1024 {
            return Err("The patched sound bank exceeds 256 MB; import a lower sample rate".into());
        }
        let check = Bank::parse(&bytes)?;
        if !replacements.is_empty() {
            Bank::validate_media_alignment(&bytes)?;
        }
        for (region, _, _) in &replacements {
            let clip = check
                .clips(&region.event)?
                .into_iter()
                .find(|c| c.track == region.clip.track)
                .ok_or("Patched anthem has no audio")?;
            let embedded = check.embedded(clip.source)?;
            let decoded = Audio::from_pcm_wem(&embedded)?;
            if (decoded.duration() - clip.duration).abs() > 1.0 / decoded.rate as f64
                || (clip.duration - region.clip.duration).abs() > 0.00001
            {
                return Err("Patched anthem verification failed".into());
            }
        }
        staged.push((live, backup, original, bytes));
    }
    let previous: Vec<_> = staged
        .iter()
        .map(|(live, _, _, _)| fs::read(live).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    for (i, (live, backup, original, bytes)) in staged.iter().enumerate() {
        let result = (|| {
            if !backup.is_file() {
                atomic_write(backup, original)?;
            }
            atomic_write(live, bytes)
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (row, old) in staged.iter().take(i).zip(&previous) {
                if let Err(e) = atomic_write(&row.0, old) {
                    rollback_errors.push(e);
                }
            }
            return Err(format!(
                "Could not patch anthem: {error}. Rollback errors: {}",
                rollback_errors.join(", ")
            ));
        }
    }
    Ok(JobResult::Applied(desired, saved))
}

impl AnthemPatcherState {
    pub fn new(base: &Path, config: &Config) -> Self {
        let (thumbnail_jobs, jobs) =
            crossbeam_channel::unbounded::<(String, PathBuf, egui::Context)>();
        let (results, thumbnail_results) = crossbeam_channel::unbounded();
        std::thread::spawn(move || {
            while let Ok((key, path, ctx)) = jobs.recv() {
                let image = super::cosmetic_thumbnail::extract_raw_png(&path, "anthems")
                    .ok()
                    .map(Arc::<[u8]>::from);
                if results.send((key, image)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        let saved = fs::read(base.join("anthems/patches.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            base: base.into(),
            catalog: None,
            entries: Vec::new(),
            scanned_path: None,
            saved,
            active_anthems: config.patcher.active_anthems.clone(),
            search: String::new(),
            patched_only: false,
            owned_only: false,
            page: 0,
            editor: None,
            player: None,
            player_imported: false,
            receiver: None,
            status: String::new(),
            error: None,
            thumbnails: HashMap::new(),
            thumbnail_jobs,
            thumbnail_results,
        }
    }
    pub fn set_catalog(&mut self, catalog: Value) {
        self.catalog = Some(catalog);
        self.scanned_path = None;
    }
    fn job(
        &mut self,
        ctx: &egui::Context,
        status: &str,
        work: impl FnOnce() -> Result<JobResult, String> + Send + 'static,
    ) {
        if self.receiver.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.receiver = Some(rx);
        self.status = status.into();
        self.error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, config: &mut Config, tx: &Sender<AppMsg>) {
        let result = match self.receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                Err("The anthem background task stopped unexpectedly; try again".into())
            }
            _ => return,
        };
        self.receiver = None;
        self.status.clear();
        match result {
            Ok(JobResult::Scan(entries)) => self.entries = entries,
            Ok(JobResult::Import(audio, name, reset)) => {
                self.player = None;
                if let Some(e) = &mut self.editor {
                    e.audio = Some(audio);
                    e.source_name = name;
                    if reset {
                        e.start = 0.0;
                    }
                    e.cursor = e.start;
                }
            }
            Ok(JobResult::Preview(audio)) => {
                self.player_imported = false;
                match Player::play(&audio, 0.0, audio.duration()) {
                    Ok(p) => self.player = Some(p),
                    Err(e) => self.error = Some(e),
                }
            }
            Ok(JobResult::Applied(active, saved)) => {
                self.active_anthems = active;
                self.saved = saved;
                config.patcher.active_anthems = self.active_anthems.clone();
                if let Err(e) = config.save(&self.base) {
                    self.error = Some(format!(
                        "Audio patched, but settings could not be saved: {e}"
                    ));
                }
                self.status =
                    "Anthem patches updated. Restart Rocket League to hear the changes.".into();
                let _ = tx.send(AppMsg::Log("[Anthems] Updated anthem patches".into()));
            }
            Err(e) => {
                let _ = tx.send(AppMsg::Log(format!("[Anthems] {e}")));
                self.error = Some(e);
            }
        }
    }
    fn preview(&mut self, region: Region, cooked: &Path, ctx: &egui::Context) {
        self.player = None;
        let base = self.base.clone();
        let cooked = cooked.to_path_buf();
        self.job(
            ctx,
            "Loading original anthem audio (the decoder downloads on first use)…",
            move || {
                let path = cooked.join(format!("{}.wem", region.clip.source));
                let (audio, temporary) = if path.is_file() {
                    (audio::original_audio(&base, &path)?, None)
                } else {
                    let bank = Bank::parse(
                        &fs::read(baseline(&cooked, &region.bank)).map_err(|e| e.to_string())?,
                    )?;
                    let dir = base.join("anthems/preview");
                    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let path = dir.join(format!("{}.wem", region.clip.source));
                    fs::write(&path, bank.embedded(region.clip.source)?)
                        .map_err(|e| e.to_string())?;
                    let result = audio::original_audio(&base, &path);
                    let _ = fs::remove_file(&path);
                    (result?, Some(path))
                };
                let _ = temporary;
                Ok(JobResult::Preview(
                    audio.section(region.clip.start, region.clip.duration)?,
                ))
            },
        );
    }
    pub fn update_enabled(
        &mut self,
        key: String,
        enabled: bool,
        cooked: &Path,
        ctx: &egui::Context,
    ) {
        let mut active = self.active_anthems.clone();
        if enabled {
            if let Some(patch) = self.saved.get(&key) {
                active.insert(key, patch.name.clone());
            }
        } else {
            active.remove(&key);
        }
        let saved = self.saved.clone();
        let cooked = cooked.to_path_buf();
        self.player = None;
        self.job(ctx, "Updating anthem patches…", move || {
            apply(&cooked, active, saved)
        });
    }
    pub fn restore_all(&mut self, cooked: &Path, ctx: &egui::Context) {
        let saved = self.saved.clone();
        let cooked = cooked.to_path_buf();
        self.player = None;
        self.job(ctx, "Restoring original anthems…", move || {
            apply(&cooked, HashMap::new(), saved)
        });
    }
    pub fn leave_page(&mut self) {
        self.player = None;
    }
    pub fn render_tab(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        tx: &Sender<AppMsg>,
        config: &mut Config,
        owned_ids: &HashSet<i64>,
    ) {
        self.poll(config, tx);
        while let Ok((key, image)) = self.thumbnail_results.try_recv() {
            self.thumbnails.insert(key, image);
        }
        let ctx = ui.ctx().clone();
        if self.receiver.is_none()
            && self.scanned_path.as_deref() != Some(cooked)
            && let Some(catalog) = self.catalog.clone()
        {
            let cooked = cooked.to_path_buf();
            self.scanned_path = Some(cooked.clone());
            self.job(&ctx, "Checking installed anthem audio…", move || {
                scan(catalog, &cooked).map(JobResult::Scan)
            });
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if !self.status.is_empty() {
            ui.horizontal_wrapped(|ui| {
                if self.receiver.is_some() {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
        }
        if let Some(p) = &self.player {
            if !p.paused() && !p.finished() {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
        let busy = self.receiver.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            if self.editor.is_some() {
                egui::ScrollArea::vertical()
                    .id_salt("anthem_editor")
                    .show(ui, |ui| self.render_editor(ui, cooked));
            } else {
                self.render_list(ui, cooked, owned_ids);
            }
        });
    }
    fn thumbnail(&mut self, ui: &mut egui::Ui, cooked: &Path, anthem: &Anthem, height: f32) {
        let path = cooked.join(&anthem.thumbnail);
        let key = path.to_string_lossy().into_owned();
        if !self.thumbnails.contains_key(&key) {
            self.thumbnails.insert(key.clone(), None);
            let _ = self
                .thumbnail_jobs
                .send((key.clone(), path, ui.ctx().clone()));
        }
        let size = egui::vec2(ui.available_width(), height);
        ui.allocate_ui_with_layout(
            size,
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                if let Some(Some(bytes)) = self.thumbnails.get(&key) {
                    ui.add(
                        egui::Image::from_bytes(
                            format!("bytes://anthem/{key}/{:08x}", crc32fast::hash(bytes)),
                            bytes.clone(),
                        )
                        .fit_to_exact_size(size)
                        .maintain_aspect_ratio(true),
                    );
                } else {
                    ui.add(
                        egui::Image::from_bytes(
                            "bytes://anthem/fallback",
                            include_bytes!("../../assets/hebnix.png").as_slice(),
                        )
                        .fit_to_exact_size(size)
                        .maintain_aspect_ratio(true),
                    );
                }
            },
        );
    }
    fn select(&mut self, anthem: Anthem, ctx: &egui::Context) {
        self.player = None;
        self.error = None;
        self.status.clear();
        let saved = self.saved.get(&anthem.key);
        let source = saved.map(|s| s.source.clone());
        self.editor = Some(Editor {
            start: saved
                .and_then(|s| {
                    anthem
                        .regions
                        .iter()
                        .max_by(|a, b| a.clip.duration.total_cmp(&b.clip.duration))
                        .and_then(|r| s.starts.get(&r.label))
                        .or_else(|| s.starts.values().next())
                })
                .copied()
                .unwrap_or(0.0),
            anthem,
            audio: None,
            source_name: source.clone().unwrap_or_default(),
            cursor: 0.0,
        });
        if let Some(source) = source {
            self.job(ctx, "Loading saved anthem audio…", move || {
                Audio::decode(Path::new(&source)).map(|a| JobResult::Import(a, source, false))
            });
        }
    }
    fn render_list(&mut self, ui: &mut egui::Ui, cooked: &Path, owned_ids: &HashSet<i64>) {
        let ctx = ui.ctx().clone();
        crate::theme::page_heading(ui, "Anthem Patcher");
        ui.weak("Select an anthem to import audio and choose its playback sections.");
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut self.owned_only, "Show Only Owned Items")
                .changed()
            {
                self.page = 0;
            }
            if ui
                .checkbox(&mut self.patched_only, "Show patched anthems")
                .changed()
            {
                self.page = 0;
                self.error = None;
            }
            if ui.button("Refresh").clicked() {
                self.scanned_path = None;
                self.error = None;
            }
            if ui
                .add_enabled(
                    !self.active_anthems.is_empty(),
                    egui::Button::new("Restore all anthems"),
                )
                .clicked()
            {
                self.restore_all(cooked, &ctx);
            }
        });
        if crate::ui::browser::search(ui, &mut self.search, "Search anthems".into()) {
            self.page = 0;
        }
        let query = self.search.to_ascii_lowercase();
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|a| {
                (!self.patched_only || self.active_anthems.contains_key(&a.key))
                    && (!self.owned_only || a.product_id.is_some_and(|id| owned_ids.contains(&id)))
                    && (a.name.to_ascii_lowercase().contains(&query)
                        || a.upk.to_ascii_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        let columns = anthem_columns(
            ui.available_width() - ui.spacing().scroll.allocated_width(),
            ui.spacing().item_spacing.x,
        );
        let page_size = columns * 3;
        crate::ui::browser::pagination(ui, &mut self.page, entries.len(), page_size);
        if entries.is_empty() {
            ui.weak("No installed anthems match this filter.");
        }
        let row_height =
            ((ui.available_height() - ui.spacing().item_spacing.y * 2.0) / 3.0).max(128.0);
        let preview_height = (row_height - 92.0).clamp(40.0, 200.0);
        egui::ScrollArea::vertical()
            .id_salt("anthem_list")
            .show(ui, |ui| {
                for row in entries
                    .iter()
                    .skip(self.page * page_size)
                    .take(page_size)
                    .collect::<Vec<_>>()
                    .chunks(columns)
                {
                    let width = ((ui.available_width()
                        - ui.spacing().item_spacing.x * (columns - 1) as f32)
                        / columns as f32)
                        .max(1.0);
                    ui.horizontal_top(|ui| {
                        for anthem in row {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, row_height),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    crate::ui::browser::frame(
                                        ui,
                                        self.active_anthems.contains_key(&anthem.key),
                                    )
                                    .show(ui, |ui| {
                                        ui.set_width((width - 20.0).max(1.0));
                                        self.thumbnail(ui, cooked, anthem, preview_height);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&anthem.name).strong(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(format!("{}\n{}", anthem.name, anthem.upk));
                                        if ui.button("Select").clicked() {
                                            self.select((*anthem).clone(), &ctx);
                                        }
                                        if self.saved.contains_key(&anthem.key)
                                            || self.active_anthems.contains_key(&anthem.key)
                                        {
                                            let mut enabled =
                                                self.active_anthems.contains_key(&anthem.key);
                                            if ui.checkbox(&mut enabled, "Patch").changed() {
                                                self.update_enabled(
                                                    anthem.key.clone(),
                                                    enabled,
                                                    cooked,
                                                    &ctx,
                                                );
                                            }
                                        }
                                    });
                                },
                            );
                        }
                    });
                }
            });
    }
    fn render_editor(&mut self, ui: &mut egui::Ui, cooked: &Path) {
        let ctx = ui.ctx().clone();
        if ui.button("‹ All anthems").clicked() {
            self.editor = None;
            self.player = None;
            self.error = None;
            self.status.clear();
            return;
        }
        let mut editor = self.editor.take().unwrap();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(110.0, 100.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    self.thumbnail(ui, cooked, &editor.anthem, 100.0);
                },
            );
            ui.vertical(|ui| {
                ui.heading(&editor.anthem.name);
                ui.weak(&editor.anthem.upk);
            });
        });
        ui.horizontal(|ui| {
            if ui.button("Import Audio").clicked() {
                let dialog =
                    rfd::FileDialog::new().add_filter("Audio", &["wav", "mp3", "ogg", "flac"]);
                if let Some(path) = crate::winutil::parent_file_dialog(dialog).pick_file() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    self.job(&ctx, "Decoding imported audio…", move || {
                        Audio::decode(&path).map(|a| JobResult::Import(a, name, true))
                    });
                }
            }
            if !editor.source_name.is_empty() {
                ui.label(
                    Path::new(&editor.source_name)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
            }
        });
        ui.add_space(8.0);
        if editor.anthem.regions.is_empty() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                editor
                    .anthem
                    .error
                    .as_deref()
                    .unwrap_or("Anthem clip timings could not be read."),
            );
            self.editor = Some(editor);
            return;
        }
        let region = editor.longest_region().unwrap().clone();
        ui.label(format!(
            "All {} regions · longest: {} ({:.3}s)",
            editor.anthem.regions.len(),
            region.label,
            region.clip.duration
        ));
        ui.weak("One selection replaces every region. Shorter regions use the same start and stop automatically at their original length.");
        if ui
            .add_enabled(region.preview_available, egui::Button::new(format!("▶ Preview original {}", region.label)))
            .on_disabled_hover_text("The original audio is not installed. You can still import and patch your own audio.")
            .clicked()
        {
            self.preview(region.clone(), cooked, &ctx);
        }
        if !self.player_imported
            && let Some(player) = &mut self.player
        {
            let mut stop = false;
            ui.horizontal(|ui| {
                if ui
                    .button(if player.paused() {
                        "Resume original"
                    } else {
                        "Pause original"
                    })
                    .clicked()
                {
                    player.toggle();
                }
                stop = ui.button("Stop original").clicked();
            });
            if stop {
                self.player = None;
            }
        }
        if let Some(audio) = editor.audio.clone() {
            let mut start = editor.start;
            let length = region.clip.duration;
            ui.label(format!(
                "Imported audio: {:.2}s · {} selection: {:.3}s",
                audio.duration(),
                region.label,
                length
            ));
            if audio.max_start(length).is_none() {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                        "{} needs {:.3}s. Import audio long enough to cover the longest region.",
                        region.label, length
                    ),
                );
            } else {
                start = start.clamp(0.0, audio.max_start(length).unwrap());
                let old_start = start;
                if self.player_imported
                    && let Some(player) = &self.player
                {
                    editor.cursor = player.position();
                }
                let seeked = timeline(ui, &audio, &mut start, length, &mut editor.cursor);
                start = start.min(audio.max_start(length).unwrap());
                editor.start = start;
                if old_start != start {
                    self.player = None;
                    editor.cursor = start;
                }
                if seeked && self.player_imported && self.player.is_some() {
                    let paused = self.player.as_ref().is_some_and(Player::paused);
                    match Player::play(
                        &audio,
                        editor.cursor.clamp(start, start + length - 0.001),
                        start + length,
                    ) {
                        Ok(mut p) => {
                            if paused {
                                p.toggle();
                            }
                            self.player = Some(p);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(
                            if self.player_imported
                                && self
                                    .player
                                    .as_ref()
                                    .is_some_and(|p| !p.paused() && !p.finished())
                            {
                                "Pause"
                            } else {
                                "Play"
                            },
                        )
                        .clicked()
                    {
                        if !self.player_imported {
                            self.player = None;
                        }
                        if let Some(player) = &mut self.player {
                            if !player.finished() {
                                player.toggle();
                            } else {
                                self.player = None;
                            }
                        }
                        if self.player.is_none() {
                            let cursor = if editor.cursor >= start
                                && editor.cursor < start + length - 0.001
                            {
                                editor.cursor
                            } else {
                                start
                            };
                            match Player::play(&audio, cursor, start + length) {
                                Ok(p) => {
                                    self.player = Some(p);
                                    self.player_imported = true;
                                }
                                Err(e) => self.error = Some(e),
                            }
                        }
                    }
                    if ui.button("Stop").clicked() {
                        self.player = None;
                        editor.cursor = start;
                    }
                    ui.label("Start");
                    if ui
                        .add(
                            egui::DragValue::new(&mut start)
                                .speed(0.01)
                                .range(0.0..=audio.max_start(length).unwrap())
                                .suffix("s"),
                        )
                        .changed()
                    {
                        editor.start = start;
                        editor.cursor = start;
                        self.player = None;
                    }
                    ui.label(format!(
                        "Stop {:.3}s · fixed length {:.3}s",
                        start + length,
                        length
                    ));
                });
                ui.weak("Drag either handle or the highlighted window to move the fixed-length selection. Click the waveform to scrub.");
            }
            ui.weak("Goal, MVP, and every Epic Save variation will use this selection.");
            let selections = editor.patch_starts(&audio);
            let valid = selections.is_ok();
            if ui
                .add_enabled(valid, egui::Button::new("Save and patch anthem"))
                .on_disabled_hover_text(selections.as_ref().err().cloned().unwrap_or_default())
                .clicked()
            {
                let key = editor.anthem.key.clone();
                let mut saved = self.saved.clone();
                let base = self.base.clone();
                let cooked = cooked.to_path_buf();
                let mut desired = self.active_anthems.clone();
                desired.insert(key.clone(), editor.anthem.name.clone());
                let starts = selections.unwrap();
                let patch = SavedPatch {
                    name: editor.anthem.name.clone(),
                    upk: editor.anthem.upk.clone(),
                    internal: editor.anthem.internal.clone(),
                    source: String::new(),
                    starts,
                };
                self.player = None;
                self.job(&ctx, "Saving and patching anthem audio…", move || {
                    let dir = base.join("anthems/sources");
                    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    // Immutable source names preserve the previous patch if applying fails.
                    let path = dir.join(format!("{:016x}.wav", rand::random::<u64>()));
                    fs::write(&path, audio.wav()).map_err(|e| e.to_string())?;
                    let mut patch = patch;
                    patch.source = path.to_string_lossy().into_owned();
                    saved.insert(key, patch);
                    let manifest = base.join("anthems/patches.json");
                    let previous = fs::read(&manifest).ok();
                    atomic_write(
                        &manifest,
                        &serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?,
                    )?;
                    match apply(&cooked, desired, saved) {
                        Ok(result) => Ok(result),
                        Err(error) => {
                            let rollback = if let Some(previous) = previous {
                                atomic_write(&manifest, &previous)
                            } else {
                                fs::remove_file(&manifest).map_err(|e| e.to_string())
                            };
                            let _ = fs::remove_file(path);
                            rollback.map_err(|e| {
                                format!("{error}; could not restore anthem manifest: {e}")
                            })?;
                            Err(error)
                        }
                    }
                });
            }
        } else {
            ui.add_space(30.0);
            ui.weak("Import a song to choose one section for all anthem sounds.");
        }
        self.editor = Some(editor);
    }
}

fn anthem_columns(width: f32, gap: f32) -> usize {
    ((width + gap) / (150.0 + gap)).floor().max(1.0) as usize
}

fn timeline(
    ui: &mut egui::Ui,
    audio: &Audio,
    start: &mut f64,
    length: f64,
    cursor: &mut f64,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 150.0),
        egui::Sense::click_and_drag(),
    );
    let total = audio.duration();
    let x = |t: f64| rect.left() + (t / total) as f32 * rect.width();
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, ui.visuals().extreme_bg_color);
    let selected = egui::Rect::from_min_max(
        egui::pos2(x(*start), rect.top()),
        egui::pos2(x(*start + length), rect.bottom()),
    );
    painter.rect_filled(
        selected,
        0.0,
        ui.visuals().selection.bg_fill.gamma_multiply(0.6),
    );
    for (i, peak) in audio.peaks.iter().enumerate() {
        let px = rect.left() + i as f32 / audio.peaks.len() as f32 * rect.width();
        let h = peak * (rect.height() - 36.0) * 0.5;
        painter.line_segment(
            [
                egui::pos2(px, rect.center().y - h),
                egui::pos2(px, rect.center().y + h),
            ],
            egui::Stroke::new(1.0, ui.visuals().text_color().gamma_multiply(0.65)),
        );
    }
    let mut moved = false;
    let body = selected.shrink2(egui::vec2(7.0, 20.0));
    let drag = ui.interact(body, response.id.with("window"), egui::Sense::drag());
    if drag.dragged() {
        *start = (*start + ui.input(|i| i.pointer.delta().x) as f64 / rect.width() as f64 * total)
            .clamp(0.0, (total - length).max(0.0));
        moved = true;
    }
    for (name, time, end_handle) in [("start", *start, false), ("stop", *start + length, true)] {
        let handle = egui::Rect::from_min_max(
            egui::pos2(x(time) - 7.0, rect.top()),
            egui::pos2(x(time) + 7.0, rect.bottom()),
        );
        let r = ui.interact(handle, response.id.with(name), egui::Sense::drag());
        painter.line_segment(
            [
                egui::pos2(x(time), rect.top()),
                egui::pos2(x(time), rect.bottom()),
            ],
            egui::Stroke::new(3.0, ui.visuals().selection.stroke.color),
        );
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(x(time), rect.top() + 9.0),
                egui::vec2(10.0, 18.0),
            ),
            2.0,
            ui.visuals().selection.stroke.color,
        );
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let t = ((pos.x - rect.left()) / rect.width()) as f64 * total;
            *start =
                (t - if end_handle { length } else { 0.0 }).clamp(0.0, (total - length).max(0.0));
            moved = true;
        }
        r.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    }
    let mut seeked = false;
    let scrub_handle = egui::Rect::from_min_max(
        egui::pos2(x(*cursor) - 5.0, rect.top() + 22.0),
        egui::pos2(x(*cursor) + 5.0, rect.bottom() - 18.0),
    );
    let scrub = ui.interact(
        scrub_handle,
        response.id.with("scrubber"),
        egui::Sense::drag(),
    );
    if scrub.dragged()
        && let Some(pos) = scrub.interact_pointer_pos()
    {
        *cursor =
            (((pos.x - rect.left()) / rect.width()) as f64 * total).clamp(*start, *start + length);
        seeked = true;
    }
    if response.clicked()
        && !moved
        && let Some(pos) = response.interact_pointer_pos()
    {
        *cursor =
            (((pos.x - rect.left()) / rect.width()) as f64 * total).clamp(*start, *start + length);
        seeked = true;
    }
    painter.line_segment(
        [
            egui::pos2(x(*cursor), rect.top()),
            egui::pos2(x(*cursor), rect.bottom()),
        ],
        egui::Stroke::new(2.0, egui::Color32::WHITE),
    );
    for i in 0..=5 {
        let time = total * i as f64 / 5.0;
        painter.text(
            egui::pos2(x(time), rect.bottom() - 3.0),
            if i == 5 {
                egui::Align2::RIGHT_BOTTOM
            } else {
                egui::Align2::LEFT_BOTTOM
            },
            format!("{:.1}s", time),
            egui::FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );
    }
    seeked
}
