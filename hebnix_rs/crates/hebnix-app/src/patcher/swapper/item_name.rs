use super::*;
use crate::patcher::item_name as names;

type Loaded = (Vec<names::Label>, names::Manifest);

pub(super) struct State {
    install: PathBuf,
    file: String,
    category: SwapCategory,
    search: String,
    page: usize,
    owned_only: bool,
    patched_only: bool,
    show_names: bool,
    labels: Vec<names::Label>,
    originals: HashMap<(String, String), String>,
    manifest: names::Manifest,
    selection: Option<String>,
    name: String,
    artist: String,
    status: Option<Result<String, String>>,
    error: Option<String>,
    pending: Option<Receiver<Result<Loaded, String>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(section: &str, field: &str, name: &str) -> names::Label {
        names::Label {
            section: section.into(),
            field: field.into(),
            name: name.into(),
            start: 0,
            end: 0,
        }
    }

    fn setup() -> SwapperState {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/item-name-ui-fixture");
        let mut state = SwapperState::new(&root);
        state.name_editor.install = root;
        state.name_editor.labels = (0..60)
            .map(|i| label(&format!("Body_Test_{i}"), "Label", &format!("Body {i:02}")))
            .collect();
        state.name_editor.labels.extend([
            label("Anthem_Test", "Label", "Original Song"),
            label("Anthem_Test", "Artist", "Original Artist"),
        ]);
        state.name_editor.originals = state
            .name_editor
            .labels
            .iter()
            .map(|l| ((l.section.clone(), l.field.clone()), l.name.clone()))
            .collect();
        state.name_editor.manifest = serde_json::from_value(serde_json::json!({
            "original_hash":"", "patched_hash":"", "patches":[
                {"section":"Body_Test_2","field":"Label","original":"Body 02","name":"Renamed Car","aliases":["Body_Test_2"]},
                {"section":"Anthem_Test","field":"Label","original":"Original Song","name":"Custom Song","aliases":["Anthem_Test"]},
                {"section":"Anthem_Test","field":"Artist","original":"Original Artist","name":"Custom Artist","aliases":["Anthem_Test"]}
            ]
        })).unwrap();
        state.catalogs.insert(
            SwapCategory::Bodies,
            Arc::new(
                (0..60)
                    .map(|i| SwapItem {
                        name: format!("Body {i:02}"),
                        upk: format!("Body_Test_{i}_SF.upk"),
                        path: None,
                        thumbnail: None,
                        body_preview: None,
                        audio_bnk: None,
                        upk_type: "simple".into(),
                        product_id: Some(i),
                        car_key: None,
                        car_name: None,
                        car_product_id: None,
                        shared_package: false,
                    })
                    .collect(),
            ),
        );
        state
    }

    fn frame(state: &mut SwapperState, width: f32) -> Vec<(egui::Pos2, String)> {
        let ctx = egui::Context::default();
        let cooked = state.name_editor.install.clone();
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 1000.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        state.render_item_names(
                            ui,
                            &cooked,
                            &cooked.join("Backups"),
                            &HashSet::from([2]),
                        );
                        assert!(
                            ui.min_rect().right() <= width + 1.0,
                            "content overflow: {:?}",
                            ui.min_rect()
                        );
                    });
                },
            ));
        }
        fn collect(shape: &egui::epaint::Shape, out: &mut Vec<(egui::Pos2, String)>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push((t.pos, t.galley.text().to_string())),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in &output.unwrap().shapes {
            collect(&shape.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn item_name_grid_has_three_rows_and_expands_beyond_six_columns() {
        for width in [360.0, 800.0, 1600.0] {
            let mut state = setup();
            state.name_editor.show_names = false;
            let texts = frame(&mut state, width);
            let cards: Vec<_> = texts
                .iter()
                .filter(|(_, s)| s.starts_with("Body "))
                .collect();
            let rows: HashSet<_> = cards.iter().map(|(p, _)| p.y.round() as i32).collect();
            assert_eq!(rows.len(), 3, "width {width}: {cards:?}");
            if width == 1600.0 {
                assert!(cards.len() > 18);
            }
        }
    }

    #[test]
    fn item_name_filters_search_both_names_and_show_song_artist_editor() {
        let mut state = setup();
        state.name_editor.owned_only = true;
        state.name_editor.patched_only = true;
        state.name_editor.search = "Body 02".into();
        let texts = frame(&mut state, 800.0);
        assert!(texts.iter().any(|(_, s)| s == "Renamed Car"));
        assert!(!texts.iter().any(|(_, s)| s == "Body 01"));
        state.name_editor.show_names = false;
        state.name_editor.search = "Renamed Car".into();
        assert!(frame(&mut state, 800.0).iter().any(|(_, s)| s == "Body 02"));
        state.name_editor.category = SwapCategory::Anthems;
        state.name_editor.owned_only = false;
        state.name_editor.search = "Original Artist".into();
        state.name_editor.selection = Some("Anthem_Test".into());
        state.name_editor.name = state.name_editor.current("Anthem_Test", "Label");
        state.name_editor.artist = state.name_editor.current("Anthem_Test", "Artist");
        let texts = frame(&mut state, 800.0);
        for expected in [
            "Song name",
            "Artist name",
            "Custom Song",
            "Custom Artist",
            "Restore Song Name",
            "Restore Artist Name",
        ] {
            assert!(
                texts.iter().any(|(_, s)| s == expected),
                "Missing {expected}"
            );
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            install: PathBuf::new(),
            file: "Coalesced_INT.bin".into(),
            category: SwapCategory::Bodies,
            search: String::new(),
            page: 0,
            owned_only: false,
            patched_only: false,
            show_names: true,
            labels: Vec::new(),
            originals: HashMap::new(),
            manifest: names::Manifest::default(),
            selection: None,
            name: String::new(),
            artist: String::new(),
            status: None,
            error: None,
            pending: None,
        }
    }
}

fn category(section: &str) -> Option<SwapCategory> {
    match section.split('_').next()?.to_ascii_lowercase().as_str() {
        "antenna" | "countryflag" | "flag" | "streamerflag" | "tourneyflag" | "pennant" => {
            Some(SwapCategory::Antennas)
        }
        "anthem" => Some(SwapCategory::Anthems),
        "avatarborder" => Some(SwapCategory::Borders),
        "body" => Some(SwapCategory::Bodies),
        "boost" => Some(SwapCategory::Boosts),
        "skin" | "skins" => Some(SwapCategory::Skins),
        "engineaudio" => Some(SwapCategory::Engines),
        "explosion" => Some(SwapCategory::Goals),
        "paintfinish" => Some(SwapCategory::Finishes),
        "playerbanner" => Some(SwapCategory::Banners),
        "hat" => Some(SwapCategory::Toppers),
        "ss" => Some(SwapCategory::Trails),
        "wheel" => Some(SwapCategory::Wheels),
        _ => None,
    }
}

impl State {
    fn original(&self, section: &str, field: &str) -> String {
        self.originals
            .get(&(section.into(), field.into()))
            .cloned()
            .unwrap_or_default()
    }

    fn current(&self, section: &str, field: &str) -> String {
        self.manifest
            .patches
            .iter()
            .find(|p| p.section == section && p.field == field)
            .map(|p| p.name.clone())
            .or_else(|| self.originals.get(&(section.into(), field.into())).cloned())
            .unwrap_or_default()
    }

    fn begin(
        &mut self,
        ui: &egui::Ui,
        cooked: &Path,
        backups: &Path,
        edit: Option<(String, Option<Vec<(String, String)>>)>,
    ) {
        let (cooked, backups, file) = (
            cooked.to_path_buf(),
            backups.to_path_buf(),
            self.file.clone(),
        );
        let (tx, rx) = unbounded();
        self.pending = Some(rx);
        self.error = None;
        self.status = None;
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let result = (|| {
                if let Some((section, changes)) = edit {
                    let refs = changes.as_ref().map(|c| {
                        c.iter()
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect::<Vec<_>>()
                    });
                    names::rename(&cooked, &backups, &file, &section, refs.as_deref())?;
                }
                names::catalog(&cooked, &backups, &file)
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
}

/// Prefer the internal product identity. A name fallback is only safe when unique
/// (many decals on different cars share a display name).
fn matching_item<'a>(label: &names::Label, items: &'a [SwapItem]) -> Option<&'a SwapItem> {
    if let Some(item) = items.iter().find(|item| {
        item.path
            .as_deref()
            .and_then(|p| p.rsplit('.').next())
            .is_some_and(|p| p.eq_ignore_ascii_case(&label.section))
            || item
                .upk
                .strip_suffix("_SF.upk")
                .is_some_and(|p| p.eq_ignore_ascii_case(&label.section))
    }) {
        return Some(item);
    }
    let mut matches = items
        .iter()
        .filter(|item| item.name.eq_ignore_ascii_case(&label.name));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

impl SwapperState {
    pub fn render_item_names(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        owned_ids: &HashSet<i64>,
    ) {
        self.collect_thumbnails();
        if let Some(rx) = &self.name_editor.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.name_editor.pending = None;
                    match result {
                        Ok((labels, manifest)) => {
                            self.name_editor.originals = labels
                                .iter()
                                .map(|l| ((l.section.clone(), l.field.clone()), l.name.clone()))
                                .collect();
                            self.name_editor.labels = labels;
                            self.name_editor.manifest = manifest;
                            self.name_editor.status = Some(Ok(t("item-name-loaded")));
                            if let Some(section) = self.name_editor.selection.clone() {
                                self.name_editor.name = self.name_editor.current(&section, "Label");
                                self.name_editor.artist =
                                    self.name_editor.current(&section, "Artist");
                            }
                        }
                        Err(error) => self.name_editor.error = Some(error),
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.name_editor.pending = None;
                    self.name_editor.error = Some(t("thumbnail-patcher-worker-failed"));
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if self.name_editor.install != cooked && self.name_editor.pending.is_none() {
            self.name_editor.install = cooked.into();
            self.name_editor.labels.clear();
            self.name_editor.originals.clear();
            self.name_editor.manifest = names::Manifest::default();
            self.name_editor.selection = None;
            self.name_editor.page = 0;
            self.name_editor.begin(ui, cooked, backups, None);
        }
        crate::theme::page_heading(ui, t("item-name-title"));
        ui.weak(t("item-name-description"));
        if self.name_editor.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("thumbnail-patcher-working"));
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let before = self.name_editor.file.clone();
            egui::ComboBox::from_id_salt("item-name-language")
                .selected_text(
                    self.name_editor
                        .file
                        .trim_start_matches("Coalesced_")
                        .trim_end_matches(".bin"),
                )
                .show_ui(ui, |ui| {
                    for file in names::languages(cooked) {
                        let label = file
                            .trim_start_matches("Coalesced_")
                            .trim_end_matches(".bin")
                            .to_string();
                        ui.selectable_value(&mut self.name_editor.file, file, label);
                    }
                });
            if before != self.name_editor.file {
                self.name_editor.labels.clear();
                self.name_editor.originals.clear();
                self.name_editor.manifest = names::Manifest::default();
                self.name_editor.selection = None;
                self.name_editor.page = 0;
                self.name_editor.begin(ui, cooked, backups, None);
            }
            let before = self.name_editor.category;
            egui::ComboBox::from_id_salt("item-name-category")
                .selected_text(before.label())
                .show_ui(ui, |ui| {
                    for cat in SwapCategory::ALL {
                        ui.selectable_value(&mut self.name_editor.category, cat, cat.label());
                    }
                });
            let mut changed = before != self.name_editor.category;
            changed |= ui
                .checkbox(&mut self.name_editor.owned_only, t("item-name-owned"))
                .changed();
            changed |= ui
                .checkbox(
                    &mut self.name_editor.patched_only,
                    t("item-name-patched-only"),
                )
                .changed();
            ui.checkbox(
                &mut self.name_editor.show_names,
                t("item-name-show-patched"),
            );
            if changed {
                self.name_editor.page = 0;
                self.name_editor.selection = None;
            }
            if ui.button(t("thumbnail-patcher-refresh")).clicked() {
                self.thumbnails
                    .retain(|key, _| !key.starts_with("item-name|"));
                self.failed_thumbnails
                    .retain(|key, _| !key.starts_with("item-name|"));
                self.name_editor.begin(ui, cooked, backups, None);
            }
            if ui
                .add_enabled(
                    !self.name_editor.manifest.patches.is_empty(),
                    egui::Button::new(t("item-name-restore-all")),
                )
                .clicked()
            {
                self.name_editor
                    .begin(ui, cooked, backups, Some((String::new(), None)));
            }
        });
        if let Some(error) = &self.name_editor.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            if self.name_editor.labels.is_empty() {
                return;
            }
        }
        if let Some(Ok(status)) = &self.name_editor.status {
            ui.weak(status);
        }
        if self.name_editor.pending.is_some() {
            return;
        }
        if let Some(section) = self.name_editor.selection.clone() {
            ui.separator();
            let original = self
                .name_editor
                .labels
                .iter()
                .find(|l| l.section == section && l.field == "Label")
                .map(|l| l.name.clone())
                .unwrap_or_default();
            ui.strong(original);
            let anthem = category(&section) == Some(SwapCategory::Anthems);
            ui.horizontal_wrapped(|ui| {
                ui.label(t(if anthem {
                    "item-name-song"
                } else {
                    "item-name-new"
                }));
                ui.add(
                    egui::TextEdit::singleline(&mut self.name_editor.name)
                        .char_limit(128)
                        .desired_width(240.0),
                );
            });
            let has_artist = anthem
                && self
                    .name_editor
                    .labels
                    .iter()
                    .any(|l| l.section == section && l.field == "Artist");
            if has_artist {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("item-name-artist"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.name_editor.artist)
                            .char_limit(128)
                            .desired_width(240.0),
                    );
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.name_editor.name.trim().is_empty(),
                        egui::Button::new(t("item-name-apply")),
                    )
                    .clicked()
                {
                    let mut changes = vec![("Label".into(), self.name_editor.name.clone())];
                    if has_artist {
                        changes.push(("Artist".into(), self.name_editor.artist.clone()));
                    }
                    self.name_editor.begin(
                        ui,
                        cooked,
                        backups,
                        Some((section.clone(), Some(changes))),
                    );
                }
                if self
                    .name_editor
                    .manifest
                    .patches
                    .iter()
                    .any(|p| p.section == section)
                    && ui.button(t("item-name-restore")).clicked()
                {
                    self.name_editor
                        .begin(ui, cooked, backups, Some((section.clone(), None)));
                }
                if has_artist {
                    for field in ["Label", "Artist"] {
                        if let Some(patch) = self
                            .name_editor
                            .manifest
                            .patches
                            .iter()
                            .find(|p| p.section == section && p.field == field)
                            .cloned()
                        {
                            if ui
                                .button(t(if field == "Label" {
                                    "item-name-restore-song"
                                } else {
                                    "item-name-restore-artist"
                                }))
                                .clicked()
                            {
                                self.name_editor.begin(
                                    ui,
                                    cooked,
                                    backups,
                                    Some((
                                        section.clone(),
                                        Some(vec![(field.into(), patch.original)]),
                                    )),
                                );
                            }
                        }
                    }
                }
                if ui.button(t("item-name-cancel")).clicked() {
                    self.name_editor.selection = None;
                }
            });
            ui.separator();
        }
        if self.name_editor.pending.is_some() {
            return;
        }
        if browser::search(ui, &mut self.name_editor.search, t("item-name-search")) {
            self.name_editor.page = 0;
        }
        let cat = self.name_editor.category;
        let items = self.catalogs.get(&cat).cloned().unwrap_or_default();
        let search = self.name_editor.search.trim().to_lowercase();
        let mut entries: Vec<_> = self
            .name_editor
            .labels
            .iter()
            .filter(|l| {
                if l.field != "Label" || category(&l.section) != Some(cat) {
                    return false;
                }
                if let Some(base) = l.section.strip_suffix("_Online") {
                    if self
                        .name_editor
                        .labels
                        .iter()
                        .any(|b| b.section == base && b.field == "Label" && b.name == l.name)
                    {
                        return false;
                    }
                }
                if self.name_editor.owned_only
                    && !matching_item(l, &items)
                        .and_then(|i| i.product_id)
                        .is_some_and(|id| owned_ids.contains(&id))
                {
                    return false;
                }
                if self.name_editor.patched_only
                    && !self
                        .name_editor
                        .manifest
                        .patches
                        .iter()
                        .any(|p| p.section == l.section)
                {
                    return false;
                }
                format!(
                    "{} {} {} {} {}",
                    l.name,
                    self.name_editor.current(&l.section, "Label"),
                    self.name_editor.current(&l.section, "Artist"),
                    self.name_editor.original(&l.section, "Artist"),
                    l.section
                )
                .to_lowercase()
                .contains(&search)
            })
            .cloned()
            .collect();
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.section.cmp(&b.section))
        });
        // Match the artwork cards, with no artificial upper limit on columns.
        let gap = ui.spacing().item_spacing.x;
        let columns = ((ui.available_width() - ui.spacing().scroll.allocated_width() + gap)
            / (150.0 + gap))
            .floor()
            .max(1.0) as usize;
        let page_size = columns * browser::ROWS;
        browser::pagination(ui, &mut self.name_editor.page, entries.len(), page_size);
        if entries.is_empty() {
            ui.weak(t("item-name-empty"));
        }
        let start = self.name_editor.page * page_size;
        let height = browser::tile_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("item-name-grid")
            .show(ui, |ui| {
                for row in entries
                    .iter()
                    .skip(start)
                    .take(page_size)
                    .collect::<Vec<_>>()
                    .chunks(columns)
                {
                    let width = ((ui.available_width() - gap * (columns - 1) as f32)
                        / columns as f32)
                        .max(1.0);
                    ui.horizontal_top(|ui| {
                        for label in row {
                            ui.push_id(&label.section, |ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(width, height),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        let frame = browser::frame(
                                            ui,
                                            self.name_editor.selection.as_deref()
                                                == Some(&label.section),
                                        );
                                        let margin = frame.total_margin().sum();
                                        frame.show(ui, |ui| {
                                            ui.set_width((width - margin.x).max(1.0));
                                            ui.set_min_height((height - margin.y).max(1.0));
                                            let file = matching_item(label, &items)
                                                .and_then(|i| preview_thumbnail(cat, i, cooked))
                                                .or_else(|| {
                                                    let file =
                                                        format!("{}_T_SF.upk", label.section);
                                                    cooked.join(&file).is_file().then_some(file)
                                                });
                                            let preview = if let Some(file) = file {
                                                let kind = matching_item(label, &items)
                                                    .map(|i| thumbnail_kind(cat, i))
                                                    .unwrap_or_else(|| cat.slug().into());
                                                let key = format!(
                                                    "item-name|{}|{}|{}",
                                                    cooked.display(),
                                                    file,
                                                    kind
                                                );
                                                if !self.thumbnails.contains_key(&key) {
                                                    self.thumbnails.insert(key.clone(), None);
                                                    let _ = self.thumbnail_jobs.send((
                                                        key.clone(),
                                                        cooked.join(file),
                                                        format!("raw|{kind}"),
                                                        ui.ctx().clone(),
                                                    ));
                                                }
                                                if let Some(Some(bytes)) = self.thumbnails.get(&key)
                                                {
                                                    browser::Preview::Ready(bytes.clone())
                                                } else if let Some(error) =
                                                    self.failed_thumbnails.get(&key)
                                                {
                                                    browser::Preview::Failed(error.clone())
                                                } else {
                                                    browser::Preview::Loading
                                                }
                                            } else {
                                                browser::Preview::Missing
                                            };
                                            browser::preview(
                                                ui,
                                                &format!("name/{}", label.section),
                                                preview,
                                                (height - 120.0).max(40.0),
                                            );
                                            let current =
                                                self.name_editor.current(&label.section, "Label");
                                            browser::name(
                                                ui,
                                                if self.name_editor.show_names {
                                                    &current
                                                } else {
                                                    &label.name
                                                },
                                                &label.section,
                                            );
                                            if cat == SwapCategory::Anthems {
                                                let artist = if self.name_editor.show_names {
                                                    self.name_editor
                                                        .current(&label.section, "Artist")
                                                } else {
                                                    self.name_editor
                                                        .original(&label.section, "Artist")
                                                };
                                                ui.add(egui::Label::new(&artist).truncate())
                                                    .on_hover_text(artist);
                                            }
                                            if self
                                                .name_editor
                                                .manifest
                                                .patches
                                                .iter()
                                                .any(|p| p.section == label.section)
                                            {
                                                browser::applied(ui);
                                            }
                                            if ui.button(t("item-name-edit")).clicked() {
                                                self.name_editor.selection =
                                                    Some(label.section.clone());
                                                self.name_editor.name = current;
                                                self.name_editor.artist = self
                                                    .name_editor
                                                    .current(&label.section, "Artist");
                                            }
                                        });
                                    },
                                );
                            });
                        }
                    });
                }
            });
    }
}
