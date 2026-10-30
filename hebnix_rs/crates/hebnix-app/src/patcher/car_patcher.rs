use crate::config::{Config, PatchSource};
use crate::i18n::t;
use crate::messages::AppMsg;
use crate::patcher::catalog::PatchCatalog;
use crate::patcher::{backup_guard, patch_source_selector};
use crossbeam_channel::Sender;
use eframe::egui;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const UPK_MAGIC: [u8; 4] = 0x9E2A83C1u32.to_le_bytes();

#[derive(Clone, Debug)]
pub struct CarPatch {
    pub name: String,
    pub body_id: i32,
    pub mesh_path: String,
    pub json_path: PathBuf,
    pub upk_path: PathBuf,
    strategy: CarPatchStrategy,
    thumbnail: Option<Arc<[u8]>>,
    support: Result<(), String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CarPatchStrategy {
    WholePackage,
    PreparedBody,
    EmbedGeometry,
}

#[derive(Clone, Debug)]
struct BodyTarget {
    name: String,
    upk_path: String,
}

pub struct CarPatcherState {
    base_dir: PathBuf,
    pub cars_dir: PathBuf,
    pub cars: Vec<CarPatch>,
    pub active_cars: HashMap<String, String>,
    pub(crate) source: PatchSource,
    search: String,
    page: usize,
    bodies_catalog: Option<Value>,
    catalog_error: Option<String>,
    catalog: PatchCatalog,
    fallback_image: Arc<[u8]>,
}

#[derive(Clone, Copy)]
enum TileAction {
    Apply,
    Restore,
    Delete,
}

fn catalog_id(value: Option<&Value>) -> Option<i32> {
    value.and_then(|value| {
        value
            .as_i64()
            .and_then(|id| i32::try_from(id).ok())
            .or_else(|| value.as_str()?.parse().ok())
    })
}

fn normalized_json_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn manifest_field<'a>(data: &'a Value, wanted: &str) -> Option<&'a Value> {
    let wanted = normalized_json_key(wanted);
    data.as_object()?
        .iter()
        .find_map(|(key, value)| (normalized_json_key(key) == wanted).then_some(value))
}

fn upk_file_name(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "The bodies catalog returned an unsafe UPK path: {path:?}"
        ));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "The bodies catalog returned an invalid UPK filename".to_string())?;
    if !name.to_ascii_lowercase().ends_with(".upk") {
        return Err(format!("The BodyID resolved to a non-UPK file: {name}"));
    }
    Ok(name.to_string())
}

fn upk_versions(path: &Path) -> Result<(u16, u16), String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let mut summary = [0u8; 8];
    file.read_exact(&mut summary)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if summary[..4] != UPK_MAGIC {
        return Err(format!("{} is not a valid UPK file", path.display()));
    }
    Ok((
        u16::from_le_bytes([summary[4], summary[5]]),
        u16::from_le_bytes([summary[6], summary[7]]),
    ))
}

fn validate_replacement_upk(path: &Path, target: &BodyTarget) -> Result<(), String> {
    let (file_version, licensee_version) = upk_versions(path)?;
    if licensee_version < 33 {
        return Err(format!(
            "{} is a standalone CustomCar mesh package (UPK {file_version}, licensee {licensee_version}), not a complete current Rocket League body package. Replacing {} with it would remove the required {} body asset and can crash the game. This package needs mesh-reference injection; no files were changed.",
            path.file_name().unwrap_or_default().to_string_lossy(),
            target.upk_path,
            target.name,
        ));
    }
    Ok(())
}

fn files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            {
                found.push(path);
            }
        }
    }
    found
}

fn archive_files(archive: &mut zip::ZipArchive<fs::File>) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read ZIP entry: {error}"))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| format!("ZIP contains an unsafe path: {}", entry.name()))?;
        if !entry.is_dir() {
            files.push(enclosed.to_path_buf());
        }
    }
    Ok(files)
}

fn archive_already_imported(cars_dir: &Path, files: &[PathBuf]) -> bool {
    if files.is_empty() {
        return false;
    }

    let matches = |root: &Path| files.iter().all(|path| root.join(path).is_file());
    if matches(cars_dir) {
        return true;
    }

    fs::read_dir(cars_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .any(|entry| matches(&entry.path()))
}

impl CarPatcherState {
    pub fn new(base_dir: &Path, config: &Config) -> Self {
        let cars_dir = base_dir.join("cars");
        let _ = fs::create_dir_all(&cars_dir);
        let mut state = Self {
            base_dir: base_dir.to_path_buf(),
            cars_dir,
            cars: Vec::new(),
            active_cars: config.patcher.active_cars.clone(),
            source: config.patcher.car_source,
            search: String::new(),
            page: 0,
            bodies_catalog: None,
            catalog_error: None,
            catalog: PatchCatalog::new(base_dir, "car"),
            fallback_image: crate::ui::browser::fallback_image(base_dir),
        };
        state.refresh_cars();
        state
    }

    pub fn set_catalog(&mut self, bodies: Value) -> Result<(), String> {
        if !bodies.get("bodies").is_some_and(Value::is_array) {
            return Err("The bodies catalog has no 'bodies' array".to_string());
        }
        self.bodies_catalog = Some(bodies);
        self.catalog_error = None;
        Ok(())
    }

    fn resolve_body(&self, body_id: i32) -> Result<BodyTarget, String> {
        let catalog = self
            .bodies_catalog
            .as_ref()
            .ok_or_else(|| "The bodies catalog has not been downloaded".to_string())?;
        let body = catalog["bodies"]
            .as_array()
            .and_then(|bodies| {
                bodies
                    .iter()
                    .find(|body| catalog_id(body.get("id")) == Some(body_id))
            })
            .ok_or_else(|| format!("BodyID {body_id} was not found in bodies.json"))?;
        let name = body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Unknown car")
            .to_string();
        let upk_path = body
            .get("upk_path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("BodyID {body_id} has no upk_path in bodies.json"))?;
        Ok(BodyTarget {
            name,
            upk_path: upk_file_name(upk_path)?,
        })
    }

    pub fn refresh_cars(&mut self) {
        self.cars.clear();
        for json_path in files_with_extension(&self.cars_dir, "json") {
            let Ok(bytes) = fs::read(&json_path) else {
                continue;
            };
            let Ok(root) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            let Some(entries) = root.as_object() else {
                continue;
            };
            let parent = json_path.parent().unwrap_or(&self.cars_dir);
            let upks = files_with_extension(parent, "upk");
            for (name, data) in entries {
                let Some(body_id) = catalog_id(manifest_field(data, "bodyid")) else {
                    continue;
                };
                let Some(mesh_path) = manifest_field(data, "meshpath").and_then(Value::as_str)
                else {
                    continue;
                };
                let package_name = mesh_path.split('.').next().unwrap_or(mesh_path);
                let matching = upks.iter().find(|path| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(|stem| stem.eq_ignore_ascii_case(package_name))
                });
                let upk_path = matching
                    .cloned()
                    .or_else(|| (upks.len() == 1).then(|| upks[0].clone()));
                let Some(upk_path) = upk_path else {
                    continue;
                };
                let declared_strategy = manifest_field(data, "Patch")
                    .and_then(|patch| manifest_field(patch, "Strategy"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let strategy = if declared_strategy.eq_ignore_ascii_case("embed_geometry") {
                    CarPatchStrategy::EmbedGeometry
                } else if declared_strategy.eq_ignore_ascii_case("prepared_body") {
                    CarPatchStrategy::PreparedBody
                } else {
                    CarPatchStrategy::WholePackage
                };
                let upk_path = if strategy == CarPatchStrategy::PreparedBody {
                    let Some(package) = manifest_field(data, "Patch")
                        .and_then(|patch| manifest_field(patch, "PreparedPackage"))
                        .and_then(Value::as_str)
                        .and_then(|name| upk_file_name(name).ok())
                    else {
                        continue;
                    };
                    parent.join(package)
                } else {
                    upk_path
                };
                let declared_status = manifest_field(data, "Status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let support = if declared_status.eq_ignore_ascii_case("unsupported") {
                    let reason = manifest_field(data, "UnsupportedReason")
                        .and_then(Value::as_str)
                        .unwrap_or("This patch profile is marked unsupported.");
                    Err(format!("Not supported: {reason}"))
                } else if strategy == CarPatchStrategy::EmbedGeometry {
                    Err(
                        "This car requires a prepared body package. Import a prepared car instead."
                            .to_string(),
                    )
                } else {
                    match upk_versions(&upk_path) {
                        Ok((_, licensee)) if licensee >= 33 => Ok(()),
                        Ok((file_version, licensee)) => Err(format!(
                            "Not supported: standalone CustomCar mesh package (UPK {file_version}, licensee {licensee}). It cannot safely replace a complete Rocket League body package."
                        )),
                        Err(error) => Err(format!("Not supported: {error}")),
                    }
                };
                let thumbnail = fs::read(parent.join("thumbnail.png"))
                    .ok()
                    .filter(|bytes| !bytes.is_empty())
                    .map(Arc::from);
                self.cars.push(CarPatch {
                    name: name.clone(),
                    body_id,
                    mesh_path: mesh_path.to_string(),
                    json_path: json_path.clone(),
                    upk_path,
                    strategy,
                    thumbnail,
                    support,
                });
            }
        }
        self.cars.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });
    }

    fn delete_local_patch(&mut self, car: &CarPatch) -> Result<(), String> {
        if !car.json_path.starts_with(&self.cars_dir) {
            return Err("Refusing to delete a patch outside Hebnix's cars folder".to_string());
        }
        let parent = car
            .json_path
            .parent()
            .ok_or_else(|| "Patch manifest has no parent folder".to_string())?;
        if parent == self.cars_dir || !parent.starts_with(&self.cars_dir) {
            return Err("Refusing to delete Hebnix's cars folder".to_string());
        }
        fs::remove_dir_all(parent)
            .map_err(|error| format!("Could not delete {}: {error}", parent.display()))?;

        // Catalog archives may contain their own top-level folder inside the
        // import folder. Remove now-empty wrapper folders without ever
        // crossing the cars directory boundary.
        let mut wrapper = parent.parent();
        while let Some(path) = wrapper {
            if path == self.cars_dir || !path.starts_with(&self.cars_dir) {
                break;
            }
            if fs::remove_dir(path).is_err() {
                break;
            }
            wrapper = path.parent();
        }
        self.refresh_cars();
        Ok(())
    }

    pub fn import_zip(&mut self, zip_path: &Path, tx: &Sender<AppMsg>) -> Result<usize, String> {
        let before = self.cars.len();
        let file = fs::File::open(zip_path)
            .map_err(|error| format!("Could not open {}: {error}", zip_path.display()))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|error| format!("Could not read ZIP: {error}"))?;
        let files = archive_files(&mut archive)?;
        if archive_already_imported(&self.cars_dir, &files) {
            self.refresh_cars();
            let _ = tx.send(AppMsg::Log(
                "[Cars] This catalog patch is already imported; showing Local patches.".to_string(),
            ));
            return Ok(0);
        }

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let destination = self
            .cars_dir
            .join(format!("{nonce:x}-{:x}", std::process::id()));
        fs::create_dir(&destination)
            .map_err(|error| format!("Could not create import folder: {error}"))?;

        let result = (|| {
            for index in 0..archive.len() {
                let mut entry = archive
                    .by_index(index)
                    .map_err(|error| format!("Could not read ZIP entry: {error}"))?;
                let enclosed = entry
                    .enclosed_name()
                    .ok_or_else(|| format!("ZIP contains an unsafe path: {}", entry.name()))?;
                let output = destination.join(enclosed);
                if entry.is_dir() {
                    fs::create_dir_all(&output).map_err(|error| error.to_string())?;
                    continue;
                }
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                let mut output_file = fs::File::create(&output)
                    .map_err(|error| format!("Could not create {}: {error}", output.display()))?;
                std::io::copy(&mut entry, &mut output_file)
                    .map_err(|error| format!("Could not extract {}: {error}", output.display()))?;
                output_file.flush().map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        })();

        if let Err(error) = result {
            let _ = fs::remove_dir_all(&destination);
            return Err(error);
        }

        self.refresh_cars();
        let imported = self.cars.len().saturating_sub(before);
        if imported == 0 {
            let _ = fs::remove_dir_all(&destination);
            self.refresh_cars();
            return Err(t("import-zip-no-usable-custom-cars-were-found").to_string());
        }
        let _ = tx.send(AppMsg::Log(format!(
            "[Cars] Imported {imported} custom car patch(es)."
        )));
        Ok(imported)
    }

    fn backup_dir(backups_dir: &Path) -> PathBuf {
        backups_dir.join("CarPatcher")
    }

    fn apply(
        &mut self,
        car: &CarPatch,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        config: &mut Config,
    ) -> Result<(), String> {
        if crate::messages::block_item_action_if_game_running(tx) {
            return Ok(());
        }
        if let Err(reason) = &car.support {
            return Err(reason.clone());
        }
        let target = self.resolve_body(car.body_id)?;
        if matches!(
            car.strategy,
            CarPatchStrategy::WholePackage | CarPatchStrategy::PreparedBody
        ) {
            validate_replacement_upk(&car.upk_path, &target)?;
        }
        if self.active_cars.contains_key(&target.upk_path) {
            return Err(format!(
                "{} is already patched. Restore it before applying another custom car.",
                target.name
            ));
        }
        let live_path = cooked_pc.join(&target.upk_path);
        if !live_path.is_file() {
            return Err(format!(
                "BodyID {} resolved to {}, but {} does not exist",
                car.body_id,
                target.upk_path,
                live_path.display()
            ));
        }

        let car_backups = Self::backup_dir(backups_dir);
        let backup_name = format!("{}.bak", target.upk_path);
        backup_guard::prepare(cooked_pc, &car_backups, "car-build.sha256", |name| {
            name.to_ascii_lowercase().ends_with(".upk.bak")
        })?;
        let backup_path = car_backups.join(&backup_name);
        if !backup_path.is_file() {
            fs::copy(&live_path, &backup_path)
                .map_err(|error| format!("Could not back up {}: {error}", live_path.display()))?;
        }

        let staged = live_path.with_extension("upk.hebnix-car.tmp");
        let _ = fs::remove_file(&staged);
        match car.strategy {
            CarPatchStrategy::WholePackage | CarPatchStrategy::PreparedBody => {
                fs::copy(&car.upk_path, &staged).map_err(|error| {
                    format!(
                        "Could not stage custom UPK {}: {error}",
                        car.upk_path.display()
                    )
                })?;
            }
            CarPatchStrategy::EmbedGeometry => {
                return Err("This car requires a prepared body package.".to_string());
            }
        }
        let replace_result = fs::copy(&staged, &live_path);
        let _ = fs::remove_file(&staged);
        if let Err(error) = replace_result {
            let _ = fs::copy(&backup_path, &live_path);
            return Err(format!(
                "Could not replace {}: {error}",
                live_path.display()
            ));
        }

        self.active_cars
            .insert(target.upk_path.clone(), car.name.clone());
        config.patcher.active_cars = self.active_cars.clone();
        config
            .save(&self.base_dir)
            .map_err(|error| error.to_string())?;
        let _ = tx.send(AppMsg::Log(format!(
            "[Cars] Applied '{}' to {} (BodyID {}) using {:?}.",
            car.name, target.name, car.body_id, car.strategy
        )));
        Ok(())
    }

    pub fn apply_preset(
        &mut self,
        patch_name: &str,
        expected_target: &str,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        config: &mut Config,
    ) -> Result<(), String> {
        let car = self
            .cars
            .iter()
            .find(|car| car.name == patch_name)
            .cloned()
            .ok_or_else(|| format!("Car patch '{patch_name}' is not installed locally"))?;
        let target = self.resolve_body(car.body_id)?;
        if !target.upk_path.eq_ignore_ascii_case(expected_target) {
            return Err(format!(
                "Car patch '{patch_name}' now targets {}, not {expected_target}",
                target.upk_path
            ));
        }
        if self.active_cars.contains_key(&target.upk_path) {
            self.restore(&target.upk_path, cooked_pc, backups_dir, tx, config)?;
        }
        self.apply(&car, cooked_pc, backups_dir, tx, config)
    }

    pub fn restore(
        &mut self,
        target_upk: &str,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        config: &mut Config,
    ) -> Result<(), String> {
        if crate::messages::block_item_action_if_game_running(tx) {
            return Ok(());
        }
        let target_upk = upk_file_name(target_upk)?;
        let car_backups = Self::backup_dir(backups_dir);
        if let Err(error) =
            backup_guard::check(cooked_pc, &car_backups, "car-build.sha256", |name| {
                name.to_ascii_lowercase().ends_with(".upk.bak")
            })
        {
            if error.starts_with("Rocket League was updated") {
                self.active_cars.clear();
                config.patcher.active_cars.clear();
                let _ = config.save(&self.base_dir);
            }
            return Err(error);
        }
        let backup = car_backups.join(format!("{target_upk}.bak"));
        if !backup.is_file() {
            return Err(format!("No original backup exists for {target_upk}"));
        }
        fs::copy(&backup, cooked_pc.join(&target_upk))
            .map_err(|error| format!("Could not restore {target_upk}: {error}"))?;
        let patch_name = self.active_cars.remove(&target_upk).unwrap_or_default();
        config.patcher.active_cars = self.active_cars.clone();
        config
            .save(&self.base_dir)
            .map_err(|error| error.to_string())?;
        let _ = tx.send(AppMsg::Log(format!(
            "[Cars] Restored {target_upk} from the original backup{}.",
            if patch_name.is_empty() {
                String::new()
            } else {
                format!(" (removed '{patch_name}')")
            }
        )));
        Ok(())
    }

    pub fn restore_all(
        &mut self,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        config: &mut Config,
    ) -> Result<usize, String> {
        let targets = self.active_cars.keys().cloned().collect::<Vec<_>>();
        let mut restored = 0;
        for target in targets {
            self.restore(&target, cooked_pc, backups_dir, tx, config)?;
            restored += 1;
        }
        Ok(restored)
    }

    pub fn render_tab(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        cooked_pc: &Path,
        backups_dir: &Path,
        tx: &Sender<AppMsg>,
        config: &mut Config,
    ) {
        crate::ui::browser::header(ui, t("nav-car-models"), |ui| {
            if ui
                .add_enabled(
                    !self.active_cars.is_empty(),
                    egui::Button::new(t("ball-restore-original"))
                        .fill(egui::Color32::from_rgb(180, 50, 50)),
                )
                .clicked()
            {
                if let Err(error) = self.restore_all(cooked_pc, backups_dir, tx, config) {
                    let _ = tx.send(AppMsg::Log(format!("[Cars] Restore failed: {error}")));
                }
            }
            if ui.button(t("ball-import-zip")).clicked() {
                let dialog = rfd::FileDialog::new().add_filter(t("ball-zip-archives"), &["zip"]);
                if let Some(file) = crate::winutil::parent_file_dialog(dialog).pick_file() {
                    if let Err(error) = self.import_zip(&file, tx) {
                        let _ = tx.send(AppMsg::Log(format!("[Cars] Import failed: {error}")));
                    }
                }
            }
            if ui.button(t("btn-refresh")).clicked() {
                if self.source == PatchSource::Catalog {
                    self.catalog.refresh(ctx);
                } else {
                    self.refresh_cars();
                }
            }
        });
        ui.add_space(8.0);
        let hint = t(if self.source == PatchSource::Catalog {
            "ball-name-or-author"
        } else {
            "tab-search-local-patches"
        });
        if crate::ui::browser::search(ui, &mut self.search, hint) {
            self.page = 0;
        }
        if patch_source_selector(ui, &mut self.source) {
            config.patcher.car_source = self.source;
            let _ = config.save(&self.base_dir);
        }
        ui.add_space(8.0);

        if self.source == PatchSource::Catalog {
            if let Some(zip) = self.catalog.render(ui, &self.search, ctx, tx) {
                match self.import_zip(&zip, tx) {
                    Ok(_) => {
                        self.source = PatchSource::Custom;
                        config.patcher.car_source = self.source;
                        let _ = config.save(&self.base_dir);
                    }
                    Err(error) => {
                        let _ = tx.send(AppMsg::Log(format!(
                            "[Cars] Catalog import failed: {error}"
                        )));
                    }
                }
            }
            return;
        }

        ui.separator();

        if let Some(error) = &self.catalog_error {
            ui.colored_label(egui::Color32::from_rgb(0xe7, 0x4c, 0x3c), error);
        }
        let query = self.search.trim().to_ascii_lowercase();
        let mut filtered = self
            .cars
            .iter()
            .filter(|car| query.is_empty() || car.name.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect::<Vec<_>>();
        // Keep usable patches first without adding section headings between tile rows.
        filtered.sort_by_key(|car| car.support.is_err());
        let columns = crate::ui::browser::columns(ui, 224.0);
        let page_size = columns * crate::ui::browser::ROWS;
        crate::ui::browser::pagination(ui, &mut self.page, filtered.len(), page_size);
        if filtered.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.weak(t("tab-no-local-car-patches-import-a"));
            });
            return;
        }

        let visible: Vec<_> = filtered
            .into_iter()
            .skip(self.page * page_size)
            .take(page_size)
            .collect();
        let row_height = (ui.available_height()
            - ui.spacing().item_spacing.y * (crate::ui::browser::ROWS - 1) as f32)
            / crate::ui::browser::ROWS as f32;
        let tile_height = crate::ui::browser::tile_height(ui).min(row_height);
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("car_patcher_local")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in visible.chunks(columns) {
                    ui.columns(columns, |uis| {
                        for (column, car) in row.iter().enumerate() {
                            if let Some(clicked) =
                                self.render_tile(&mut uis[column], car, tile_height)
                            {
                                action = Some((car.clone(), clicked));
                            }
                        }
                    });
                }
            });
        if let Some((car, action)) = action {
            let result = match action {
                TileAction::Apply => self.apply(&car, cooked_pc, backups_dir, tx, config),
                TileAction::Restore => self.resolve_body(car.body_id).and_then(|target| {
                    self.restore(&target.upk_path, cooked_pc, backups_dir, tx, config)
                }),
                TileAction::Delete => self.delete_local_patch(&car).map(|()| {
                    let _ = tx.send(AppMsg::Log(format!(
                        "[Cars] Deleted local patch '{}'.",
                        car.name
                    )));
                }),
            };
            if let Err(error) = result {
                let _ = tx.send(AppMsg::Log(format!("[Cars] Action failed: {error}")));
            }
        }
    }

    fn render_tile(&self, ui: &mut egui::Ui, car: &CarPatch, height: f32) -> Option<TileAction> {
        let resolved = self.resolve_body(car.body_id);
        let active = resolved
            .as_ref()
            .ok()
            .and_then(|target| self.active_cars.get(&target.upk_path));
        let applied = active.is_some_and(|name| name == &car.name);
        let style = crate::ui::browser::frame(ui, applied);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, style.corner_radius, style.fill);
        let content = rect.shrink(10.0);
        let body_font = egui::TextStyle::Body.resolve(ui.style());
        let subtitle_font = egui::FontId::new((body_font.size - 2.0).max(10.0), body_font.family);
        let subtitle_height = ui.fonts_mut(|fonts| fonts.row_height(&subtitle_font));
        let button_height = (ui.text_style_height(&egui::TextStyle::Button)
            + ui.spacing().button_padding.y * 2.0)
            .max(ui.spacing().interact_size.y);
        let footer_height = ui.text_style_height(&egui::TextStyle::Body)
            + subtitle_height
            + button_height
            + ui.spacing().item_spacing.y * 3.0;
        let image_rect = egui::Rect::from_min_max(
            content.min,
            egui::pos2(content.right(), content.bottom() - footer_height),
        );
        let bytes = car
            .thumbnail
            .clone()
            .unwrap_or_else(|| self.fallback_image.clone());
        ui.place(
            image_rect,
            egui::Image::from_bytes(
                format!(
                    "bytes://car-patch/{}/{:08x}",
                    car.json_path.display(),
                    crc32fast::hash(&bytes)
                ),
                bytes,
            )
            .fit_to_exact_size(image_rect.size())
            .maintain_aspect_ratio(true),
        );
        let footer = egui::Rect::from_min_max(
            egui::pos2(
                content.left(),
                image_rect.bottom() + ui.spacing().item_spacing.y,
            ),
            content.max,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 1.0, image_rect.bottom()),
                rect.max - egui::vec2(1.0, 1.0),
            ),
            style.corner_radius,
            ui.visuals().extreme_bg_color,
        );
        let mut action = None;
        let mut footer_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("car_patch_tile", &car.json_path, &car.name))
                .max_rect(footer)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        {
            let ui = &mut footer_ui;
            ui.shrink_clip_rect(footer);
            ui.add(egui::Label::new(egui::RichText::new(&car.name).strong()).truncate())
                .on_hover_text(&car.name);
            let target_name = resolved
                .as_ref()
                .map(|target| target.name.clone())
                .unwrap_or_else(|_| format!("BodyID {}", car.body_id));
            ui.add(
                egui::Label::new(egui::RichText::new(&target_name).font(subtitle_font).weak())
                    .truncate(),
            )
            .on_hover_text(&target_name);
            ui.horizontal(|ui| {
                if active.is_some() {
                    if ui.button(t("ball-restore-original")).clicked() {
                        action = Some(TileAction::Restore);
                    }
                } else {
                    let error = car
                        .support
                        .as_ref()
                        .err()
                        .or_else(|| resolved.as_ref().err());
                    let response = ui.add_enabled(
                        error.is_none(),
                        egui::Button::new(t(if error.is_none() {
                            "ball-apply"
                        } else {
                            "tab-not-supported"
                        })),
                    );
                    if response.clicked() {
                        action = Some(TileAction::Apply);
                    }
                    if let Some(error) = error {
                        response.on_disabled_hover_text(error);
                    }
                }
                if ui
                    .add_enabled(active.is_none(), egui::Button::new(t("presets-delete")))
                    .clicked()
                {
                    action = Some(TileAction::Delete);
                }
            });
        }
        if applied {
            ui.painter().text(
                image_rect.left_top(),
                egui::Align2::LEFT_TOP,
                t("items-browser-applied"),
                egui::TextStyle::Small.resolve(ui.style()),
                ui.visuals().selection.stroke.color,
            );
        }
        let focused = ui
            .memory(|memory| memory.focused())
            .and_then(|id| ui.ctx().read_response(id))
            .is_some_and(|response| rect.contains_rect(response.rect));
        let stroke = if focused || ui.rect_contains_pointer(rect) {
            egui::Stroke::new(
                if focused { 2.0 } else { 1.0 },
                ui.visuals().selection.stroke.color,
            )
        } else {
            style.stroke
        };
        ui.painter()
            .rect_stroke(rect, style.corner_radius, stroke, egui::StrokeKind::Inside);
        action
    }
}
