use super::*;
use crate::patcher::thumbnail_patcher as patches;

pub(super) const ITEM_THUMBNAIL_HEIGHT: f32 = 120.0;
pub(super) const ITEM_THUMBNAIL_CARD_HEIGHT: f32 = 208.0;

#[derive(Clone)]
struct Selection {
    file: String,
    kind: String,
    name: String,
}

pub(super) struct State {
    install: PathBuf,
    category: SwapCategory,
    search: String,
    page: usize,
    patched_only: bool,
    owned_only: bool,
    selection: Option<Selection>,
    replacement: Option<Arc<[u8]>>,
    manifest: patches::Manifest,
    load_error: Option<String>,
    status: Option<Result<String, String>>,
    revision: u64,
    pending: Option<Receiver<Result<patches::Manifest, String>>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            install: PathBuf::new(),
            category: SwapCategory::Bodies,
            search: String::new(),
            page: 0,
            patched_only: false,
            owned_only: false,
            selection: None,
            replacement: None,
            manifest: patches::Manifest::default(),
            load_error: None,
            status: None,
            revision: 0,
            pending: None,
        }
    }
}

impl State {
    pub(super) fn busy(&self) -> bool { self.pending.is_some() }

    fn reload(&mut self, backups: &Path) {
        match patches::load(backups) {
            Ok(manifest) => {
                self.manifest = manifest;
                self.load_error = None;
            }
            Err(error) => {
                self.manifest = patches::Manifest::default();
                self.load_error = Some(error);
            }
        }
        self.revision += 1;
    }

    fn begin(
        &mut self,
        ctx: &egui::Context,
        action: impl FnOnce() -> Result<patches::Manifest, String> + Send + 'static,
    ) {
        let (sender, receiver) = unbounded();
        let ctx = ctx.clone();
        self.pending = Some(receiver);
        self.status = None;
        std::thread::spawn(move || {
            let _ = sender.send(action());
            ctx.request_repaint();
        });
    }
}

impl SwapperState {
    fn editor_preview(
        &mut self,
        ctx: &egui::Context,
        cooked: &Path,
        selection: &Selection,
    ) -> browser::Preview {
        let key = format!(
            "thumbnail-editor|{}|{}|{}|{}",
            cooked.display(),
            self.thumbnail_editor.revision,
            selection.kind,
            selection.file
        );
        if !self.thumbnails.contains_key(&key) {
            self.thumbnails.insert(key.clone(), None);
            let _ = self.thumbnail_jobs.send((
                key.clone(),
                cooked.join(&selection.file),
                format!("raw|{}", selection.kind),
                ctx.clone(),
            ));
        }
        if let Some(Some(png)) = self.thumbnails.get(&key) {
            browser::Preview::Ready(png.clone())
        } else if let Some(error) = self.failed_thumbnails.get(&key) {
            browser::Preview::Failed(error.clone())
        } else {
            browser::Preview::Loading
        }
    }

    pub fn render_thumbnail_patcher(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        owned_ids: &HashSet<i64>,
    ) {
        ui.heading(t("nav-item-thumbnails"));
        self.render_thumbnail_patcher_content(ui, cooked, backups, owned_ids);
    }

    pub fn render_owned_thumbnail_patcher(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        inventory: &mut crate::owned_inventory::OwnedInventory,
    ) {
        ui.heading(t("nav-item-thumbnails"));
        inventory.show(ui);
        self.render_thumbnail_patcher_content(ui, cooked, backups, &inventory.ids);
    }

    fn update_thumbnail_editor(&mut self, cooked: &Path, backups: &Path) {
        self.collect_thumbnails();
        // Finish an operation before changing installs, so its result cannot leak into another game.
        if let Some(receiver) = &self.thumbnail_editor.pending {
            match receiver.try_recv() {
                Ok(result) => {
                    self.thumbnail_editor.pending = None;
                    self.thumbnail_editor.status =
                        Some(result.map(|_| t("thumbnail-patcher-saved")));
                    self.thumbnail_editor.reload(backups);
                    self.thumbnails
                        .retain(|key, _| !key.starts_with("thumbnail-editor|"));
                    self.failed_thumbnails
                        .retain(|key, _| !key.starts_with("thumbnail-editor|"));
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.thumbnail_editor.pending = None;
                    self.thumbnail_editor.status = Some(Err(t("thumbnail-patcher-worker-failed")));
                    self.thumbnail_editor.reload(backups);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if self.thumbnail_editor.install != cooked && self.thumbnail_editor.pending.is_none() {
            self.thumbnail_editor.install = cooked.to_path_buf();
            self.thumbnail_editor.selection = None;
            self.thumbnail_editor.status = None;
            self.thumbnail_editor.reload(backups);
        }
    }

    #[cfg(windows)]
    fn web_thumbnail_candidates(&self, cooked: &Path, category: SwapCategory, owned_ids: &HashSet<i64>) -> Vec<(Selection, bool)> {
        let mut seen = HashSet::new();
        self.catalogs.get(&category).into_iter().flat_map(|items| items.iter()).filter_map(|item| {
            let file = inferred_thumbnail(category, item, cooked)?;
            let kind = thumbnail_kind(category, item);
            if file.contains(['/', '\\', ':']) || !seen.insert((file.to_lowercase(), kind.clone())) { return None; }
            let owned = item.product_id.is_some_and(|id| owned_ids.contains(&id));
            Some((Selection { file, kind, name: item_label(category, item) }, owned))
        }).collect()
    }

    #[cfg(windows)]
    pub(crate) fn web_item_thumbnails(&mut self, cooked: &Path, owned_ids: &HashSet<i64>, visible: &[String], ctx: &egui::Context) -> serde_json::Value {
        use serde_json::json;
        self.update_thumbnail_editor(cooked, &cooked.join("Backups"));
        let cat = self.thumbnail_editor.category;
        let entries = self.web_thumbnail_candidates(cooked, cat, owned_ids);
        let owned_only = self.thumbnail_editor.owned_only;
        let owned_keys = if self.thumbnail_editor.patched_only && owned_only {
            self.catalogs.iter().flat_map(|(category, items)| {
                items.iter().filter(|item| item.product_id.is_some_and(|id| owned_ids.contains(&id))).filter_map(|item| {
                    Some((inferred_thumbnail(*category, item, cooked)?.to_lowercase(), thumbnail_kind(*category, item)))
                }).collect::<Vec<_>>()
            }).collect::<HashSet<_>>()
        } else { HashSet::new() };
        let patches = self.thumbnail_editor.manifest.packages.clone();
        let status = self.thumbnail_editor.status.as_ref().and_then(|status| status.as_ref().ok()).cloned();
        let error = self.thumbnail_editor.load_error.clone().or_else(|| self.thumbnail_editor.status.as_ref().and_then(|status| status.as_ref().err()).cloned());
        let busy = self.thumbnail_editor.busy();
        let patched_only = self.thumbnail_editor.patched_only;
        let mut rows = Vec::new();
        let to_image = |preview| match preview {
            browser::Preview::Ready(bytes) => crate::ui::browser::image_url(&bytes),
            _ => None,
        };
        if patched_only {
            for package in &patches {
                for patch in &package.patches {
                    if owned_only && !owned_keys.contains(&(package.file.to_lowercase(), patch.kind.clone())) { continue; }
                    let id = format!("{}|{}", package.file, patch.export);
                    let selection = Selection { file: package.file.clone(), kind: patch.kind.clone(), name: patch.name.clone() };
                    let image = visible.contains(&id).then(|| to_image(self.editor_preview(ctx, cooked, &selection))).flatten();
                    rows.push(json!({"id": id, "name": patch.name, "file": package.file, "image": image, "patched": true}));
                }
            }
        } else {
            for (selection, owned) in entries {
                if owned_only && !owned { continue; }
                let id = format!("{}|{}", selection.file, selection.kind);
                let preview = visible.contains(&id).then(|| self.editor_preview(ctx, cooked, &selection));
                let failed = preview.as_ref().and_then(|preview| match preview { browser::Preview::Failed(error) => Some(error.clone()), _ => None });
                let image = preview.and_then(to_image);
                rows.push(json!({"id": id, "name": selection.name, "file": selection.file, "image": image, "error": failed}));
            }
        }
        let selection = self.thumbnail_editor.selection.clone().map(|selection| {
            let image = to_image(self.editor_preview(ctx, cooked, &selection));
            let export = patches.iter().filter(|package| package.file.eq_ignore_ascii_case(&selection.file))
                .flat_map(|package| &package.patches).find(|patch| patch.kind == selection.kind).map(|patch| patch.export);
            json!({
                "id": format!("{}|{}", selection.file, selection.kind), "name": selection.name, "image": image,
                "replacement": self.thumbnail_editor.replacement.as_deref().and_then(crate::ui::browser::image_url),
                "export": export,
            })
        });
        let categories = SwapCategory::ALL.iter().map(|category| (category.slug(), category.label())).collect::<Vec<_>>();
        json!({
            "page": "item-artwork", "kind": "thumbnails", "categories": categories, "category": cat.slug(),
            "rows": rows, "selection": selection, "busy": busy, "active": patches.iter().any(|p| !p.patches.is_empty()),
            "error": error, "status": status, "owned": owned_only, "patched": patched_only,
            "labels": {
                "description": t("thumbnail-patcher-description"), "owned": "Show Only Owned Items", "patched": t("thumbnail-patcher-show-patched"),
                "search": t("thumbnail-patcher-search"), "edit": t("thumbnail-patcher-select"), "restoreAll": t("thumbnail-patcher-restore-all"),
                "apply": t("thumbnail-patcher-apply"), "restore": t("thumbnail-patcher-restore"), "current": t("thumbnail-patcher-current"),
                "choose": t("thumbnail-patcher-choose-image"), "replacement": t("thumbnail-patcher-replacement"),
                "empty": t(if patched_only { "thumbnail-patcher-no-patches" } else { "thumbnail-patcher-empty" }), "cancel": t("item-name-cancel"),
            },
        })
    }

    #[cfg(windows)]
    pub(crate) fn web_item_thumbnail_action(&mut self, key: &str, value: serde_json::Value, cooked: &Path, closed: bool, owned_ids: &HashSet<i64>, ctx: &egui::Context) {
        if self.thumbnail_editor.busy() { return; }
        let backups = cooked.join("Backups");
        let text = value.as_str().unwrap_or_default();
        match key {
            "category" => {
                if let Some(category) = SwapCategory::ALL.into_iter().find(|category| category.slug() == text) {
                    self.thumbnail_editor.category = category;
                    self.thumbnail_editor.selection = None;
                }
            }
            "owned" | "patched" => {
                let enabled = value.as_bool().unwrap_or(false);
                if key == "owned" { self.thumbnail_editor.owned_only = enabled; }
                else { self.thumbnail_editor.patched_only = enabled; }
                self.thumbnail_editor.selection = None;
            }
            "refresh" => self.thumbnail_editor.reload(&backups),
            "select" if !self.thumbnail_editor.patched_only => {
                let candidates = self.web_thumbnail_candidates(cooked, self.thumbnail_editor.category, owned_ids);
                if let Some((selection, _)) = candidates.into_iter().find(|(item, owned)| {
                    (!self.thumbnail_editor.owned_only || *owned) && format!("{}|{}", item.file, item.kind) == text
                }) && matches!(self.editor_preview(ctx, cooked, &selection), browser::Preview::Ready(_)) {
                    self.thumbnail_editor.selection = Some(selection);
                }
            }
            "cancel" => self.thumbnail_editor.selection = None,
            "choose" if self.thumbnail_editor.selection.is_some() => {
                let dialog = rfd::FileDialog::new().add_filter(t("thumbnail-patcher-images"), &["png", "jpg", "jpeg", "webp", "bmp", "gif", "ico"]);
                if let Some(path) = crate::winutil::parent_file_dialog(dialog).pick_file() {
                    match patches::import_image(&path) {
                        Ok(png) => {
                            self.thumbnail_editor.replacement = Some(png.into());
                            self.thumbnail_editor.status = None;
                        }
                        Err(error) => self.thumbnail_editor.status = Some(Err(error)),
                    }
                }
            }
            "save" if closed => {
                let (Some(selection), Some(png)) = (self.thumbnail_editor.selection.clone(), self.thumbnail_editor.replacement.clone()) else { return; };
                if matches!(self.editor_preview(ctx, cooked, &selection), browser::Preview::Ready(_)) {
                    let cooked = cooked.to_owned();
                    self.thumbnail_editor.begin(ctx, move || patches::apply(&cooked, &backups, &selection.file, &selection.kind, &selection.name, &png));
                }
            }
            "restore-all" if closed && self.thumbnail_editor.manifest.count() > 0 => {
                let cooked = cooked.to_owned();
                self.thumbnail_editor.begin(ctx, move || patches::restore_all(&cooked, &backups));
            }
            "restore" if closed => {
                let pair = self.thumbnail_editor.manifest.packages.iter().find_map(|package| {
                    package.patches.iter().find(|patch| format!("{}|{}", package.file, patch.export) == text)
                        .map(|patch| (package.file.clone(), patch.export))
                });
                if let Some((file, export)) = pair {
                    let cooked = cooked.to_owned();
                    self.thumbnail_editor.begin(ctx, move || patches::restore(&cooked, &backups, &file, export));
                }
            }
            _ => {}
        }
    }

    fn render_thumbnail_patcher_content(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        owned_ids: &HashSet<i64>,
    ) {
        self.update_thumbnail_editor(cooked, backups);
        ui.weak(t("thumbnail-patcher-description"));
        if let Some(status) = &self.thumbnail_editor.status {
            match status {
                Ok(message) => {
                    ui.label(message);
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            }
        }
        if self.thumbnail_editor.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("thumbnail-patcher-working"));
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(
                    &mut self.thumbnail_editor.owned_only,
                    "Show Only Owned Items",
                )
                .changed()
            {
                self.thumbnail_editor.page = 0;
                self.thumbnail_editor.selection = None;
            }
            let before = self.thumbnail_editor.category;
            egui::ComboBox::from_id_salt("thumbnail-patcher-category")
                .selected_text(before.label())
                .show_ui(ui, |ui| {
                    for category in SwapCategory::ALL {
                        ui.selectable_value(
                            &mut self.thumbnail_editor.category,
                            category,
                            category.label(),
                        );
                    }
                });
            if before != self.thumbnail_editor.category {
                self.thumbnail_editor.page = 0;
                self.thumbnail_editor.selection = None;
            }
            if ui
                .checkbox(
                    &mut self.thumbnail_editor.patched_only,
                    t("thumbnail-patcher-show-patched"),
                )
                .changed()
            {
                self.thumbnail_editor.page = 0;
                self.thumbnail_editor.selection = None;
            }
            if ui.button(t("thumbnail-patcher-refresh")).clicked() {
                self.thumbnail_editor.reload(backups);
            }
            if ui
                .add_enabled(
                    self.thumbnail_editor.manifest.count() > 0,
                    egui::Button::new(t("thumbnail-patcher-restore-all")),
                )
                .clicked()
            {
                let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
                self.thumbnail_editor
                    .begin(ui.ctx(), move || patches::restore_all(&cooked, &backups));
            }
        });
        if let Some(error) = &self.thumbnail_editor.load_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        // Active patches remain accessible even if a catalogue entry disappears.
        if self.thumbnail_editor.patched_only {
            self.render_patched_thumbnails(ui, cooked, backups, owned_ids);
            return;
        }
        if let Some(selection) = self.thumbnail_editor.selection.clone() {
            self.render_thumbnail_selection(ui, cooked, backups, &selection);
        }
        if browser::search(
            ui,
            &mut self.thumbnail_editor.search,
            t("thumbnail-patcher-search"),
        ) {
            self.thumbnail_editor.page = 0;
        }
        let category = self.thumbnail_editor.category;
        let Some(items) = self.catalogs.get(&category).cloned() else {
            return;
        };
        let search = self.thumbnail_editor.search.to_lowercase();
        let mut seen = HashSet::new();
        let entries: Vec<_> = items
            .iter()
            .filter_map(|item| {
                if self.thumbnail_editor.owned_only
                    && !item.product_id.is_some_and(|id| owned_ids.contains(&id))
                {
                    return None;
                }
                if !item_label(category, item).to_lowercase().contains(&search) {
                    return None;
                }
                // An engine's body fallback is a preview only, never an edit destination.
                let file = inferred_thumbnail(category, item, cooked)?;
                let kind = thumbnail_kind(category, item);
                if file.contains(['/', '\\', ':'])
                    || !seen.insert((file.to_lowercase(), kind.clone()))
                {
                    return None;
                }
                Some(Selection {
                    file,
                    kind,
                    name: item_label(category, item),
                })
            })
            .collect();
        let columns = browser::columns(ui, false);
        let page_size = columns * 2;
        browser::pagination(
            ui,
            &mut self.thumbnail_editor.page,
            entries.len(),
            page_size,
        );
        if entries.is_empty() {
            ui.weak(t("thumbnail-patcher-empty"));
        }
        let start = self.thumbnail_editor.page * page_size;
        egui::ScrollArea::vertical()
            .id_salt("thumbnail-patcher-items")
            .show(ui, |ui| crate::panel::followed(ui, |ui| {
                for row in entries
                    .iter()
                    .skip(start)
                    .take(page_size)
                    .collect::<Vec<_>>()
                    .chunks(columns)
                {
                    let width = ((ui.available_width()
                        - ui.spacing().item_spacing.x * (columns - 1) as f32)
                        / columns as f32)
                        .max(1.0);
                    ui.horizontal_top(|ui| {
                        for selection in row {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, ITEM_THUMBNAIL_CARD_HEIGHT),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    browser::frame(ui, false).show(ui, |ui| {
                                        ui.set_width((width - 20.0).max(1.0));
                                        let preview = self.editor_preview(ui.ctx(), cooked, selection);
                                        let loaded = matches!(&preview, browser::Preview::Ready(_));
                                        browser::preview(
                                            ui,
                                            &format!(
                                                "editor/{}|{}",
                                                selection.file, selection.kind
                                            ),
                                            preview,
                                            ITEM_THUMBNAIL_HEIGHT,
                                            true,
                                        );
                                        browser::name(ui, &selection.name, &selection.file);
                                        if ui
                                            .add_enabled(
                                                loaded,
                                                egui::Button::new(t("thumbnail-patcher-select")),
                                            )
                                            .clicked()
                                        {
                                            self.thumbnail_editor.selection =
                                                Some((*selection).clone());
                                        }
                                    });
                                },
                            );
                        }
                    });
                }
            }));
    }

    fn render_thumbnail_selection(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        selection: &Selection,
    ) {
        ui.separator();
        ui.strong(&selection.name);
        let preview = self.editor_preview(ui.ctx(), cooked, selection);
        let loaded = matches!(&preview, browser::Preview::Ready(_));
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.set_width(110.0);
                ui.label(t("thumbnail-patcher-current"));
                browser::preview(ui, "editor-current", preview, 100.0, false);
            });
            if let Some(png) = self.thumbnail_editor.replacement.clone() {
                ui.vertical(|ui| {
                    ui.set_width(110.0);
                    ui.label(t("thumbnail-patcher-replacement"));
                    browser::preview(
                        ui,
                        "editor-replacement",
                        browser::Preview::Ready(png),
                        100.0,
                        false,
                    );
                });
            }
            ui.vertical(|ui| {
                if ui.button(t("thumbnail-patcher-choose-image")).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter(
                            t("thumbnail-patcher-images"),
                            &["png", "jpg", "jpeg", "webp", "bmp", "gif", "ico"],
                        )
                        .pick_file()
                    {
                        match patches::import_image(&path) {
                            Ok(png) => {
                                self.thumbnail_editor.replacement = Some(png.into());
                                self.thumbnail_editor.status = None;
                            }
                            Err(error) => self.thumbnail_editor.status = Some(Err(error)),
                        }
                    }
                }
                if ui
                    .add_enabled(
                        loaded && self.thumbnail_editor.replacement.is_some(),
                        egui::Button::new(t("thumbnail-patcher-apply")),
                    )
                    .clicked()
                {
                    let png = self.thumbnail_editor.replacement.clone().unwrap();
                    let selection = selection.clone();
                    let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
                    self.thumbnail_editor.begin(ui.ctx(), move || {
                        patches::apply(
                            &cooked,
                            &backups,
                            &selection.file,
                            &selection.kind,
                            &selection.name,
                            &png,
                        )
                    });
                }
                let active = self
                    .thumbnail_editor
                    .manifest
                    .packages
                    .iter()
                    .find(|p| p.file.eq_ignore_ascii_case(&selection.file))
                    .and_then(|p| p.patches.iter().find(|p| p.kind == selection.kind))
                    .map(|p| p.export);
                if let Some(export) = active {
                    if ui.button(t("thumbnail-patcher-restore")).clicked() {
                        let (cooked, backups, file) = (
                            cooked.to_path_buf(),
                            backups.to_path_buf(),
                            selection.file.clone(),
                        );
                        self.thumbnail_editor.begin(ui.ctx(), move || {
                            patches::restore(&cooked, &backups, &file, export)
                        });
                    }
                }
            });
        });
        ui.separator();
    }

    fn render_patched_thumbnails(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        owned_ids: &HashSet<i64>,
    ) {
        let mut packages = self.thumbnail_editor.manifest.packages.clone();
        if self.thumbnail_editor.owned_only {
            let owned_keys: HashSet<_> = self
                .catalogs
                .iter()
                .flat_map(|(category, items)| {
                    items
                        .iter()
                        .filter(|item| item.product_id.is_some_and(|id| owned_ids.contains(&id)))
                        .filter_map(|item| {
                            Some((
                                inferred_thumbnail(*category, item, cooked)?.to_ascii_lowercase(),
                                thumbnail_kind(*category, item),
                            ))
                        })
                })
                .collect();
            for package in &mut packages {
                package.patches.retain(|patch| {
                    owned_keys.contains(&(package.file.to_ascii_lowercase(), patch.kind.clone()))
                });
            }
            packages.retain(|package| !package.patches.is_empty());
        }
        if packages.is_empty() {
            ui.weak(t("thumbnail-patcher-no-patches"));
        }
        egui::ScrollArea::vertical()
            .id_salt("thumbnail-patcher-active")
            .show(ui, |ui| crate::panel::followed(ui, |ui| {
                for package in packages {
                    for patch in package.patches {
                        ui.push_id((&package.file, patch.export), |ui| {
                            browser::frame(ui, false).show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width(100.0);
                                        let preview = self.editor_preview(ui.ctx(),
                                            cooked,
                                            &Selection {
                                                file: package.file.clone(),
                                                kind: patch.kind.clone(),
                                                name: patch.name.clone(),
                                            },
                                        );
                                        browser::preview(
                                            ui,
                                            &format!("patched/{}|{}", package.file, patch.export),
                                            preview,
                                            100.0,
                                            false,
                                        );
                                    });
                                    ui.vertical(|ui| {
                                        ui.strong(&patch.name);
                                        ui.weak(&package.file);
                                        if ui.button(t("thumbnail-patcher-restore")).clicked() {
                                            let (cooked, backups, file) = (
                                                cooked.to_path_buf(),
                                                backups.to_path_buf(),
                                                package.file.clone(),
                                            );
                                            let export = patch.export;
                                            self.thumbnail_editor.begin(ui.ctx(), move || {
                                                patches::restore(&cooked, &backups, &file, export)
                                            });
                                        }
                                    });
                                });
                            });
                            ui.add_space(6.0);
                        });
                    }
                }
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn visit(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| visit(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| visit(&shape.shape, label))
            .unwrap()
    }

    #[test]
    fn thumbnail_patcher_only_selects_loaded_images_and_renders_editor() {
        let root =
            std::env::temp_dir().join(format!("hebnix-thumbnail-ui-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("preview.upk"), b"fixture").unwrap();
        for ready in [false, true] {
            let ctx = egui::Context::default();
            egui_extras::install_image_loaders(&ctx);
            let mut state = SwapperState::new(&root);
            state.thumbnail_editor.install = root.clone();
            state.catalogs.insert(
                SwapCategory::Bodies,
                Arc::new(vec![SwapItem {
                    name: "Fixture body".into(),
                    upk: "body.upk".into(),
                    path: None,
                    thumbnail: Some("preview.upk".into()),
                    body_preview: None,
                    audio_bnk: None,
                    upk_type: "simple".into(),
                    product_id: None,
                    car_key: None,
                    car_name: None,
                    car_product_id: None,
                    shared_package: false,
                }]),
            );
            let png: Arc<[u8]> = crate::cosmetic_thumbnail::preview_png(
                image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 128])),
            )
            .unwrap()
            .into();
            let key = format!("thumbnail-editor|{}|0|bodies|preview.upk", root.display());
            state
                .thumbnails
                .insert(key.clone(), ready.then(|| png.clone()));
            if !ready {
                state
                    .failed_thumbnails
                    .insert(key, "Unreadable fixture".into());
            }
            let frame = |state: &mut SwapperState, events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 700.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            state.render_thumbnail_patcher(
                                ui,
                                &root,
                                &root.join("Backups"),
                                &HashSet::new(),
                            );
                        });
                    },
                )
            };
            frame(&mut state, vec![]);
            let output = frame(&mut state, vec![]);
            let pos = button_position(&output, &t("thumbnail-patcher-select"));
            frame(
                &mut state,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            frame(
                &mut state,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            assert_eq!(state.thumbnail_editor.selection.is_some(), ready);
            if ready {
                state.thumbnail_editor.replacement = Some(png);
                let output = frame(&mut state, vec![]);
                button_position(&output, &t("thumbnail-patcher-current"));
                button_position(&output, &t("thumbnail-patcher-replacement"));
                button_position(&output, &t("thumbnail-patcher-apply"));
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
