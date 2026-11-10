//! Loadouts on the account save: import (BakkesMod or share codes), copy, rename, delete.
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::i18n::{t, t_args};
use crate::item_cleanup::{
    apply_loadout_edit, loadout_export, loadout_export_bakkes, loadout_order, loadout_owned_instances, loadout_preset, loadout_summaries,
};
use crate::patcher::colours::CarColours;
use crate::presets::PresetStore;
use crate::swapper::{SwapCategory, SwapPick, SwapperState, category_for_equip_slot};
use crate::ui::browser;
use hebnix_sdk::save_file::loadout_edit::Edit;
use hebnix_sdk::save_file::{self, SaveAccount, bakkes_code, loadout_code};
use hebnix_sdk::utils::constants::{SAVE_PATH_EPIC, SAVE_PATH_STEAM};
use serde_json::{Value, json};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SaveMode {
    #[default]
    New,
    Replace,
}

/// One slot of the code, as the preview shows it.
#[derive(Clone)]
struct Row {
    slot: u8,
    /// What other players see: the base preset's item
    others: String,
    /// What you see: the code's item, swapped in locally
    you: String,
    /// Other presets that equip the same stock item and so change locally too
    affects: Vec<String>,
    /// Why the slot gets no swap
    skipped: Option<String>,
}

#[derive(Clone)]
struct Plan {
    name: String,
    base: usize,
    replace: Option<usize>,
    rows: Vec<Row>,
    swaps: Vec<Value>,
    cars: CarColours,
}

type PlanKey = (usize, SaveMode, usize, String);
type Palettes = [Vec<[u8; 3]>; 3];

enum PaletteLoad {
    Loading(Receiver<Result<Palettes, String>>),
    Ready(Result<Palettes, String>),
}

#[derive(Default)]
pub struct LoadoutsPanel {
    accounts: Vec<SaveAccount>,
    save: Option<PathBuf>,
    names: Vec<String>,
    bodies: Vec<i64>,
    loaded: bool,
    input: String,
    search: String,
    body_filter: Option<i64>,
    page: usize,
    dialog: Option<(usize, LoadoutDialog)>,
    message: String,
    /// BakkesMod code waiting for a base preset, a name and a confirm
    code: Option<bakkes_code::Code>,
    base: usize,
    mode: SaveMode,
    target: usize,
    name: String,
    plan: Option<(PlanKey, Result<Plan, String>)>,
}

enum LoadoutDialog {
    Edit { name: String, preset: loadout_code::Preset, expected: loadout_code::Preset, owned: Vec<(i32, u64, u64, i64)>, team: usize, search: String, changes: Vec<(usize, usize, u64, u64)>, palettes: PaletteLoad },
    Delete,
    Move(usize, Vec<u32>),
}

impl LoadoutsPanel {
    fn reload(&mut self) {
        self.plan = None;
        match self.save.as_deref().map(loadout_summaries).transpose() {
            Ok(loadouts) => {
                (self.names, self.bodies) = loadouts.unwrap_or_default().into_iter().unzip();
            }
            Err(error) => {
                self.names.clear();
                self.bodies.clear();
                self.message = error;
            }
        }
        if self
            .body_filter
            .is_some_and(|body| !self.bodies.contains(&body))
        {
            self.body_filter = None;
            self.page = 0;
        }
    }

    fn finish(&mut self, result: Result<(), String>) {
        self.message = result.err().unwrap_or_default();
        self.reload();
    }

    fn refresh_accounts(&mut self) {
        let preferred = save_file::detect_save_data_path();
        let mut directories: Vec<_> = preferred.into_iter().collect();
        if let Some(documents) = hebnix_sdk::utils::paths::documents_dir() {
            for relative in [SAVE_PATH_STEAM, SAVE_PATH_EPIC] {
                let directory = documents.join(relative);
                if !directories.contains(&directory) {
                    directories.push(directory);
                }
            }
        }
        self.accounts = directories
            .iter()
            .flat_map(|directory| save_file::find_save_accounts(Some(directory)))
            .map(|mut account| {
                // Positions can differ in rotating copies. Prefer the account's main save.
                let main = main_save_path(&account.path);
                if main.is_file() {
                    account.path = main;
                }
                account
            })
            .collect();
        self.save = match self.save.as_deref() {
            Some(previous) => {
                let main = main_save_path(previous);
                self.accounts
                    .iter()
                    .find(|account| main_save_path(&account.path) == main)
            }
            None => self.accounts.first(),
        }
        .map(|account| account.path.clone());
    }

    fn show_save_picker(&mut self, ui: &mut egui::Ui) {
        let before = self.save.clone();
        let selected = self
            .accounts
            .iter()
            .find(|account| Some(&account.path) == before.as_ref())
            .map(|account| account.display_name.as_str());
        let placeholder = t("loadouts-select-save");
        let refresh = ui
            .horizontal_wrapped(|ui| {
                ui.label(t("loadouts-save"));
                ui.add_enabled_ui(!self.accounts.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt("loadouts-save-account")
                        .selected_text(selected.unwrap_or(&placeholder))
                        .show_ui(ui, |ui| {
                            for account in &self.accounts {
                                ui.selectable_value(
                                    &mut self.save,
                                    Some(account.path.clone()),
                                    &account.display_name,
                                )
                                .on_hover_text(account.path.display().to_string());
                            }
                        });
                });
                ui.button(t("loadouts-refresh-saves")).clicked()
            })
            .inner;
        if refresh {
            self.refresh_accounts();
        }
        if before != self.save {
            self.body_filter = None;
            self.page = 0;
            self.dialog = None;
            self.code = None;
            self.mode = SaveMode::New;
            self.base = 0;
            self.target = 0;
        }
        if refresh || before != self.save {
            self.message.clear();
            self.reload();
        }
        if self.accounts.is_empty() {
            ui.weak(t("loadouts-no-save"));
        }
    }

    /// Share codes (`LDS...`) import at once. A BakkesMod code opens the import card.
    fn read_input(&mut self) {
        let Some(save) = self.save.as_deref() else {
            return;
        };
        // The one-line field flattens pasted line breaks to spaces.
        let codes: Vec<&str> = self.input.split_whitespace().collect();
        let Some(first) = codes.first() else {
            return;
        };
        if first.starts_with("LDS") {
            let presets = codes
                .iter()
                .enumerate()
                .map(|(i, code)| {
                    loadout_code::decode(code).map_err(|error| format!("code {}: {error}", i + 1))
                })
                .collect::<Result<Vec<_>, _>>();
            match presets {
                Ok(presets) => {
                    let result = apply_loadout_edit(save, &Edit::Import(presets));
                    if result.is_ok() {
                        self.input.clear();
                    }
                    self.finish(result);
                }
                Err(error) => self.message = error,
            }
        } else if codes.len() > 1 {
            self.message = t("loadouts-one-bakkes-code");
        } else {
            match bakkes_code::decode(first) {
                Ok(code) => {
                    self.code = Some(code);
                    self.plan = None;
                    self.name = self.free_name();
                    self.message.clear();
                }
                Err(error) => self.message = error,
            }
        }
    }

    fn free_name(&self) -> String {
        let taken = |name: &str| self.names.iter().any(|n| n.eq_ignore_ascii_case(name));
        let mut name = "BakkesMod".to_string();
        let mut n = 2;
        while taken(&name) {
            name = format!("BakkesMod {n}");
            n += 1;
        }
        name
    }

    /// Returns the name of a swap preset to apply, when the user chose Import and apply.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        swapper: &mut SwapperState,
        cooked_pc: &Path,
        presets: &mut PresetStore,
        catalog_loading: bool,
    ) -> Option<String> {
        if !self.loaded {
            self.loaded = true;
            self.refresh_accounts();
            self.reload();
        }
        crate::i18n::layout::bordered_panel(ui, |ui| {
            ui.heading(t("loadouts-title"));
            ui.separator();
            ui.weak(t("loadouts-description"));
            self.show_save_picker(ui);
        });
        ui.add_space(8.0);
        let mut apply = None;
        crate::i18n::layout::bordered_panel(ui, |ui| {
            apply = self.show_import(ui, swapper, presets);
            if !self.message.is_empty() {
                ui.label(&self.message);
            }
        });
        ui.add_space(8.0);
        self.show_tiles(ui, swapper, cooked_pc, catalog_loading);
        self.show_dialog(ui.ctx(), swapper, cooked_pc);
        apply
    }

    fn show_import(
        &mut self,
        ui: &mut egui::Ui,
        swapper: &SwapperState,
        presets: &mut PresetStore,
    ) -> Option<String> {
        ui.heading(t("loadouts-import-title"));
        ui.separator();
        // presets live in the save file, which the game rewrites while it runs
        let game_closed = !hebnix_sdk::process::is_rocket_league_running();
        if !game_closed {
            ui.weak(t("loadouts-close-game"));
        }
        let Some(code) = self.code.clone() else {
            ui.weak(t("loadouts-import-hint"));
            if import_entry(ui, &mut self.input, game_closed && self.save.is_some()) {
                self.read_input();
            }
            return None;
        };
        let save = self.save.as_deref()?;
        let mut summary = t_args(
            "loadouts-bakkes-summary",
            &[("count", code.blue.items.len().to_string().into())],
        );
        if code.blue.colors.is_some() {
            summary.push_str(&format!(" · {}", t("loadouts-bakkes-colours")));
        }
        ui.label(summary);
        if self.names.is_empty() {
            ui.weak(t("loadouts-empty"));
            if ui.button(t("loadouts-cancel")).clicked() {
                self.code = None;
            }
            return None;
        }
        let last = self.names.len() - 1;
        self.base = self.base.min(last);
        self.target = self.target.min(last);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(t("loadouts-bakkes-base"));
            egui::ComboBox::from_id_salt("bakkes-base")
                .selected_text(&self.names[self.base])
                .show_ui(ui, |ui| {
                    for (i, name) in self.names.iter().enumerate() {
                        ui.selectable_value(&mut self.base, i, name);
                    }
                });
        });
        ui.weak(t("loadouts-bakkes-base-hint"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.mode, SaveMode::New, t("loadouts-bakkes-mode-new"));
            ui.radio_value(&mut self.mode, SaveMode::Replace, t("loadouts-bakkes-mode-replace"));
        });
        match self.mode {
            SaveMode::New => {
                ui.horizontal(|ui| {
                    ui.label(t("loadouts-bakkes-name"));
                    ui.text_edit_singleline(&mut self.name);
                });
            }
            SaveMode::Replace => {
                ui.horizontal(|ui| {
                    ui.label(t("loadouts-bakkes-target"));
                    egui::ComboBox::from_id_salt("bakkes-target")
                        .selected_text(&self.names[self.target])
                        .show_ui(ui, |ui| {
                            for (i, name) in self.names.iter().enumerate() {
                                ui.selectable_value(&mut self.target, i, name);
                            }
                        });
                });
            }
        }
        let key = (self.base, self.mode, self.target, self.name.clone());
        if self.plan.as_ref().is_none_or(|(built, _)| *built != key) {
            let plan = build_plan(save, &code, swapper, &self.names, key.clone());
            self.plan = Some((key, plan));
        }
        ui.add_space(4.0);
        let mut ready = false;
        match &self.plan {
            Some((_, Ok(plan))) => {
                show_plan(ui, plan, &self.names);
                ready = true;
            }
            Some((_, Err(error))) => {
                ui.label(error);
            }
            None => {}
        }
        ui.add_space(6.0);
        let mut action = None;
        ui.horizontal(|ui| {
            let primary = egui::Button::new(t("loadouts-import-apply")).fill(ui.visuals().selection.bg_fill);
            if ui.add_enabled(ready && game_closed, primary).clicked() {
                action = Some(true);
            }
            if ui.add_enabled(ready && game_closed, egui::Button::new(t("loadouts-import-only"))).clicked() {
                action = Some(false);
            }
            if ui.button(t("loadouts-cancel")).clicked() {
                self.code = None;
                self.plan = None;
            }
        });
        let apply = action?;
        let Some((_, Ok(plan))) = self.plan.take() else {
            return None;
        };
        match save_plan(save, &plan, presets) {
            Ok(mut message) => {
                if !apply {
                    message.push(' ');
                    message.push_str(&t("loadouts-bakkes-apply-hint"));
                }
                self.message = message;
                self.reload();
                self.code = None;
                self.input.clear();
                apply.then_some(plan.name)
            }
            Err(error) => {
                self.message = error;
                None
            }
        }
    }

    fn show_tiles(&mut self, ui: &mut egui::Ui, swapper: &mut SwapperState, cooked_pc: &Path, catalog_loading: bool) {
        let mut cars = std::collections::BTreeMap::new();
        for &body in &self.bodies {
            let (_, count) = cars.entry(body).or_insert_with(|| {
                let label = swapper
                    .product_name(SwapCategory::Bodies, body)
                    .unwrap_or_else(|| {
                        if catalog_loading {
                            t("app-loading-item-catalogs")
                        } else {
                            t("loadouts-unknown-car")
                        }
                    });
                (label, 0)
            });
            *count += 1;
        }
        let mut filters: Vec<_> = cars.iter().collect();
        filters.sort_by(|(a_id, (a, _)), (b_id, (b, _))| a.cmp(b).then(a_id.cmp(b_id)));

        let height_id = ui.make_persistent_id("loadout-list-height");
        let panel_height = ui.ctx().data_mut(|data| data.get_temp::<f32>(height_id)).unwrap_or(320.0);
        let frame = egui::Frame::group(ui.style());
        let margin = frame.total_margin();
        let content_height = (panel_height - margin.top - margin.bottom).max(0.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                frame.show(ui, |ui| {
                    ui.set_width((ui.available_width() * 0.2).clamp(110.0, 165.0));
                    let top = ui.next_widget_position().y;
                    ui.set_min_height(content_height);
                    ui.heading(t("loadouts-cars"));
                    ui.separator();
                    let all = t_args("loadouts-all-cars", &[("count", self.names.len().into())]);
                    if ui
                        .selectable_label(self.body_filter.is_none(), all)
                        .clicked()
                    {
                        self.body_filter = None;
                        self.page = 0;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("loadout-cars")
                        .max_height((content_height - (ui.next_widget_position().y - top)).max(20.0))
                        .show(ui, |ui| {
                            for (&body, (name, count)) in filters {
                                let label = egui::Button::selectable(
                                    self.body_filter == Some(body),
                                    format!("{name} ({count})"),
                                );
                                if ui.add(label.truncate()).on_hover_text(name).clicked() {
                                    self.body_filter = Some(body);
                                    self.page = 0;
                                }
                            }
                        });
                });
            });

            ui.vertical(|ui| {
                let panel = frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    browser::header(ui, t("loadouts-list-title"), |ui| {
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.search)
                                    .desired_width(ui.available_width().min(200.0))
                                    .hint_text(t("loadouts-search")),
                            )
                            .changed()
                        {
                            self.page = 0;
                        }
                    });
                    ui.separator();

                    let query = self.search.trim().to_lowercase();
                    let filtered: Vec<_> = self
                        .names
                        .iter()
                        .zip(&self.bodies)
                        .enumerate()
                        .filter(|(_, (name, body))| {
                            self.body_filter.is_none_or(|filter| filter == **body)
                                && (query.is_empty()
                                    || name.to_lowercase().contains(&query)
                                    || cars[*body].0.to_lowercase().contains(&query))
                        })
                        .map(|(position, _)| position)
                        .collect();

                    let columns = browser::columns(ui, 275.0);
                    let page_size = columns * browser::ROWS;
                    self.page = self
                        .page
                        .min(filtered.len().div_ceil(page_size).saturating_sub(1));
                    let start = self.page * page_size;
                    let visible = &filtered[start..(start + page_size).min(filtered.len())];

                    ui.add_space(6.0);
                    if filtered.is_empty() {
                        ui.weak(t(if self.names.is_empty() {
                            "loadouts-empty"
                        } else {
                            "loadouts-no-matches"
                        }));
                    }
                    let grid_top = ui.next_widget_position().y;
                    for row in visible.chunks(columns) {
                        ui.columns(columns, |cells| {
                            for (&position, cell) in row.iter().zip(cells) {
                                cell.push_id(position, |ui| {
                                    self.show_tile(
                                        ui,
                                        swapper,
                                        cooked_pc,
                                        position,
                                        &cars[&self.bodies[position]].0,
                                        catalog_loading,
                                    );
                                });
                            }
                        });
                    }
                    let row_height = 86.0 + 16.0 + ui.spacing().item_spacing.y;
                    let grid_bottom = grid_top + browser::ROWS as f32 * row_height;
                    ui.add_space((grid_bottom - ui.next_widget_position().y).max(0.0));
                    ui.add_space(6.0);
                    browser::pagination(ui, &mut self.page, filtered.len(), page_size);
                });
                let measured = panel.response.rect.height();
                if (measured - panel_height).abs() > 0.5 {
                    ui.ctx().data_mut(|data| data.insert_temp(height_id, measured));
                    ui.ctx().request_repaint();
                }
            });
        });
    }

    fn show_tile(
        &mut self,
        ui: &mut egui::Ui,
        swapper: &mut SwapperState,
        cooked_pc: &Path,
        position: usize,
        car: &str,
        catalog_loading: bool,
    ) {
        browser::frame(ui, false).inner_margin(8).show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.horizontal(|ui| {
                let preview_width = (width * 0.32).min(105.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(preview_width, 86.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        if catalog_loading { ui.spinner(); }
                        else { swapper.show_body_preview(ui, cooked_pc, self.bodies[position]); }
                    },
                );
                let actions_width = 24.0;
                let text_width =
                    (ui.available_width() - actions_width - ui.spacing().item_spacing.x).max(1.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(text_width, 86.0),
                    egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
                    |ui| {
                        ui.add_space(18.0);
                        let name = &self.names[position];
                        ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate())
                            .on_hover_text(name);
                        ui.add(
                            egui::Label::new(egui::RichText::new(car).small().weak()).truncate(),
                        )
                        .on_hover_text(car);
                    },
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let copy = tile_action(ui, "copy", "loadouts-copy-code");
                    egui::Popup::menu(&copy).show(|ui| {
                        for (key, bakkes) in [
                            ("loadouts-format-lds", false),
                            ("loadouts-format-bakkes", true),
                        ] {
                            if ui.button(t(key)).clicked() {
                                self.copy_code(ui.ctx(), position, bakkes);
                                ui.close();
                            }
                        }
                    });
                    if tile_action(ui, "edit", "loadouts-edit").clicked()
                        && let Some(save) = self.save.as_deref()
                    {
                        match (loadout_preset(save, position), loadout_owned_instances(save)) {
                            (Ok(preset), Ok(owned)) => {
                                let (send, receive) = mpsc::channel();
                                let cooked = cooked_pc.to_owned();
                                let ctx = ui.ctx().clone();
                                std::thread::spawn(move || {
                                    let _ = send.send(crate::patcher::colours::loadout_palettes(&cooked));
                                    ctx.request_repaint();
                                });
                                self.dialog = Some((position, LoadoutDialog::Edit {
                                    name: self.names[position].clone(), expected: preset.clone(), preset, owned, team: 0,
                                    search: String::new(), changes: Vec::new(), palettes: PaletteLoad::Loading(receive),
                                }));
                            }
                            (Err(error), _) | (_, Err(error)) => self.message = error,
                        }
                    }
                    if tile_action(ui, "move", "loadouts-move").clicked()
                        && let Some(save) = self.save.as_deref()
                    {
                        match loadout_order(save) {
                            Ok(order) => self.dialog = Some((position, LoadoutDialog::Move(position + 1, order))),
                            Err(error) => self.message = error,
                        }
                    }
                    if tile_action(ui, "delete", "loadouts-delete").clicked() {
                        self.dialog = Some((position, LoadoutDialog::Delete));
                    }
                });
            });
        });
    }

    fn copy_code(&mut self, ctx: &egui::Context, position: usize, bakkes: bool) {
        let Some(save) = self.save.as_deref() else {
            return;
        };
        let code = if bakkes {
            loadout_export_bakkes(save, position)
        } else {
            loadout_export(save, position)
        };
        match code {
            Ok(code) => {
                ctx.copy_text(code);
                self.message = t("loadouts-copied");
            }
            Err(error) => self.message = error,
        }
    }

    fn show_dialog(&mut self, ctx: &egui::Context, swapper: &mut SwapperState, cooked_pc: &Path) {
        let Some((position, mut dialog)) = self.dialog.take() else {
            return;
        };
        let Some(save) = self.save.as_deref() else {
            return;
        };
        let Some(current_name) = self.names.get(position) else {
            return;
        };
        let mut edit = None;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("loadout-edit")).show(ctx, |ui| {
            ui.set_width(320.0);
            match &mut dialog {
                LoadoutDialog::Edit { name, preset, expected, owned, team, search, changes, palettes } => {
                    ui.set_width(480.0);
                    egui::ScrollArea::vertical()
                        .id_salt("loadout-editor-content")
                        .max_height((ctx.content_rect().height() - 48.0).max(1.0))
                        .show(ui, |ui| {
                            ui.heading(t("loadouts-edit"));
                            ui.text_edit_singleline(name);
                            ui.horizontal(|ui| {
                                ui.selectable_value(team, 0, t("loadouts-team-blue"));
                                ui.selectable_value(team, 1, t("loadouts-team-orange"));
                            });
                            if let PaletteLoad::Loading(receive) = palettes {
                                match receive.try_recv() {
                                    Ok(result) => *palettes = PaletteLoad::Ready(result),
                                    Err(TryRecvError::Disconnected) => *palettes = PaletteLoad::Ready(Err("Could not load the car colour palettes.".into())),
                                    Err(TryRecvError::Empty) => {}
                                }
                            }
                            match palettes {
                                PaletteLoad::Loading(_) => { ui.spinner(); }
                                PaletteLoad::Ready(Ok(palettes)) => {
                                    let paint = &mut preset.teams[*team];
                                    ui.horizontal_wrapped(|ui| {
                                        colour_picker(ui, "loadouts-primary-colour", &mut paint.team_color, &palettes[*team]);
                                        colour_picker(ui, "loadouts-secondary-colour", &mut paint.custom_color, &palettes[2]);
                                    });
                                }
                                PaletteLoad::Ready(Err(error)) => { ui.weak(error.as_str()); }
                            }
                            ui.weak(t("loadouts-owned-only"));
                            for slot in [0u8, 1, 2, 3, 4, 5, 13, 14, 15, 18] {
                                let Some(category) = (if slot == 0 { Some(SwapCategory::Bodies) } else { category_for_equip_slot(slot) }) else { continue; };
                                ui.push_id(slot, |ui| {
                                    ui.horizontal(|ui| {
                                        let product = preset.teams[*team].products[slot as usize];
                                        ui.allocate_ui(egui::vec2(64.0, 64.0), |ui| {
                                            swapper.show_loadout_preview(ui, cooked_pc, category, product as i64);
                                        });
                                        ui.vertical(|ui| {
                                            ui.strong(slot_name(slot));
                                            let selected = changes.iter().find(|(t,s,_,_)| *t == *team && *s == slot as usize)
                                                .map(|(_,_,upper,lower)| (*upper,*lower))
                                                .or_else(|| preset.teams[*team].items[slot as usize].and_then(|i| preset.items.get(i)).map(|item| (item.instance_upper, item.instance_lower)));
                                            let mut current = swapper.product_name(category, product as i64).unwrap_or_else(|| format!("#{product}"));
                                            if let Some((_,_,_,paint)) = owned.iter().find(|(_,upper,lower,_)| Some((*upper,*lower)) == selected) {
                                                if *paint > 0 { current.push_str(&format!(" ({})", crate::patcher::painted_swap::factory_paint_label(*paint))); }
                                            }
                                            let response = ui.button(current);
                                            egui::Popup::menu(&response)
                                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                                .show(|ui| {
                                                    ui.set_min_width(260.0);
                                                    ui.add(egui::TextEdit::singleline(search).hint_text(t("loadouts-search-items")));
                                                    let query = search.trim().to_lowercase();
                                                    egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                                                        let mut choices: Vec<_> = owned.iter().filter_map(|&(id, upper, lower, paint)| {
                                                            if !swapper.loadout_item_compatible(category, id as i64, preset.teams[*team].products[0] as i64) { return None; }
                                                            let mut label = swapper.product_name(category, id as i64)?;
                                                            if paint > 0 { label.push_str(&format!(" ({})", crate::patcher::painted_swap::factory_paint_label(paint))); }
                                                            (query.is_empty() || label.to_lowercase().contains(&query)).then_some((label, id, upper, lower))
                                                        }).collect();
                                                        choices.sort_by(|a,b| a.0.cmp(&b.0).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
                                                        for (label, id, upper, lower) in choices {
                                                            if ui.selectable_label(selected == Some((upper, lower)), label).clicked() {
                                                                preset.teams[*team].products[slot as usize] = id;
                                                                changes.retain(|(t,s,_,_)| *t != *team || *s != slot as usize);
                                                                changes.push((*team, slot as usize, upper, lower));
                                                            }
                                                        }
                                                    });
                                                });
                                        });
                                    });
                                });
                            }
                            let name = name.trim();
                            ui.horizontal(|ui| {
                                let save = ui.add_enabled(
                                    !name.is_empty(),
                                    egui::Button::new(t("loadouts-save-changes")),
                                );
                                if !name.is_empty() && save.clicked() {
                                    let colours = preset.teams.each_ref().map(|team| [team.team_color, team.custom_color]);
                                    edit = Some(Edit::Update { position, name: name.to_owned(), changes: changes.clone(), colours, expected: expected.clone() });
                                }
                                cancel = ui.button(t("loadouts-cancel")).clicked();
                            });
                        });
                }
                LoadoutDialog::Delete => {
                    ui.heading(t("loadouts-delete"));
                    ui.label(t_args(
                        "loadouts-delete-confirm",
                        &[("name", current_name.as_str().into())],
                    ));
                    ui.horizontal(|ui| {
                        cancel = ui.button(t("loadouts-cancel")).clicked();
                        if ui
                            .button(
                                egui::RichText::new(t("loadouts-delete"))
                                    .color(ui.visuals().error_fg_color),
                            )
                            .clicked()
                        {
                            edit = Some(Edit::Delete { position });
                        }
                    });
                }
                LoadoutDialog::Move(destination, expected) => {
                    ui.heading(t("loadouts-move"));
                    ui.label(current_name);
                    ui.add_space(6.0);
                    ui.label(t("loadouts-move-position"));
                    egui::ComboBox::from_id_salt("loadout-move-position")
                        .selected_text(format!("{} · {}", *destination, self.names[*destination - 1]))
                        .show_ui(ui, |ui| {
                            for (index, name) in self.names.iter().enumerate() {
                                ui.selectable_value(destination, index + 1, format!("{} · {}", index + 1, name));
                            }
                        });
                    ui.weak(t_args("loadouts-move-total", &[("count", self.names.len().into())]));
                    ui.horizontal(|ui| {
                        if ui.button(t("loadouts-save-changes")).clicked() {
                            edit = Some(Edit::Move { from: position, to: *destination - 1, expected: expected.clone() });
                        }
                        cancel = ui.button(t("loadouts-cancel")).clicked();
                    });
                }
            }
        });
        if let Some(edit) = edit {
            let result = match loadout_summaries(save) {
                Ok(current)
                    if current
                        .iter()
                        .map(|(name, body)| (name, body))
                        .eq(self.names.iter().zip(&self.bodies)) =>
                {
                    apply_loadout_edit(save, &edit)
                }
                Ok(_) => Err(t("loadouts-save-changed")),
                Err(error) => Err(error),
            };
            self.finish(result);
        } else if !cancel && !modal.should_close() {
            self.dialog = Some((position, dialog));
        }
    }
}

fn tile_action(ui: &mut egui::Ui, action: &str, label: &str) -> egui::Response {
    macro_rules! icon {
        ($path:literal) => {
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d=""#,
                $path,
                r#"" fill="none" stroke="white" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
            )
        };
    }
    let svg = match action {
        "copy" => icon!("M8 8h12v12H8z M16 8V4H4v12h4"),
        "edit" => icon!("m4 20 1-5L16 4l4 4L9 19z M14 6l4 4"),
        "move" => icon!("M7 7l3-3 3 3 M10 4v16 M17 17l-3 3-3-3 M14 20V4"),
        _ => icon!("M4 6h16 M9 6V3h6v3 M6 6l1 15h10l1-15 M10 10v7 M14 10v7"),
    };
    let tint = if action == "delete" {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().text_color()
    };
    let icon = egui::Image::from_bytes(format!("bytes://loadouts/{action}.svg"), svg.as_bytes())
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(tint);
    let label = t(label);
    let response = ui.add_sized([24.0, 20.0], egui::Button::image(icon).frame(false));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    response.on_hover_text(label)
}

fn import_entry(ui: &mut egui::Ui, input: &mut String, can_import: bool) -> bool {
    let field_id = ui.make_persistent_id("loadout-import-code");
    let label = t("loadouts-import");
    let label_width = ui
        .painter()
        .layout_no_wrap(
            label.clone(),
            egui::TextStyle::Button.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x;
    let button_width = (label_width.ceil() + ui.spacing().button_padding.x * 2.0 + 2.0)
        .max(ui.spacing().interact_size.x);
    ui.horizontal(|ui| {
        let field_width =
            (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(1.0);
        ui.add(
            egui::TextEdit::singleline(input)
                .id(field_id)
                .desired_width(field_width)
                .hint_text(t("loadouts-import-placeholder")),
        );
        ui.add_enabled(can_import && !input.trim().is_empty(), egui::Button::new(label))
            .clicked()
    })
    .inner
}

fn main_save_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let account = stem.split('_').next().unwrap_or(&stem);
    path.with_file_name(format!("{account}.save"))
}

/// Plain name for a BakkesMod equip slot.
fn slot_name(slot: u8) -> String {
    match slot {
        0 => t("loadouts-slot-body"),
        1 => t("loadouts-slot-decal"),
        2 => t("loadouts-slot-wheels"),
        3 => t("loadouts-slot-boost"),
        4 => t("loadouts-slot-antenna"),
        5 => t("loadouts-slot-topper"),
        13 => t("loadouts-slot-engine"),
        14 => t("loadouts-slot-trail"),
        15 => t("loadouts-slot-goal"),
        18 => t("loadouts-slot-anthem"),
        _ => t("loadouts-slot-other"),
    }
}

/// Previews the import. Others keep seeing the base preset's items. The code's items are
/// swapped over those items locally, so each swap source is the base preset's product.
fn build_plan(
    save: &Path,
    code: &bakkes_code::Code,
    swapper: &SwapperState,
    names: &[String],
    key: PlanKey,
) -> Result<Plan, String> {
    let (base, mode, target, typed) = key;
    let replace = (mode == SaveMode::Replace).then_some(target);
    let name = match replace {
        Some(target) => names.get(target).cloned().ok_or_else(|| t("loadouts-bakkes-no-target"))?,
        None => {
            let typed = typed.trim();
            if typed.is_empty() {
                return Err(t("loadouts-bakkes-need-name"));
            }
            if names.iter().any(|name| name.eq_ignore_ascii_case(typed)) {
                return Err(t("loadouts-bakkes-name-taken"));
            }
            typed.to_string()
        }
    };
    let base_products = loadout_preset(save, base)?.teams[0].products;
    // The base is the copy's source and the replaced preset goes away, so neither is listed.
    let others: Vec<(String, Vec<[i32; loadout_code::SLOTS]>)> = names
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != base && Some(*i) != replace)
        .filter_map(|(i, name)| {
            let preset = loadout_preset(save, i).ok()?;
            Some((name.clone(), preset.teams.iter().map(|team| team.products).collect()))
        })
        .collect();
    let label = |category, id: i64| swapper.product_name(category, id).unwrap_or_else(|| format!("#{id}"));
    let mut rows = Vec::new();
    let mut swaps = Vec::new();
    for item in &code.blue.items {
        let slot = usize::from(item.slot);
        let stock = i64::from(base_products[slot]);
        let mut row = Row {
            slot: item.slot,
            others: if stock == 0 { "-".into() } else { format!("#{stock}") },
            you: format!("#{}", item.product_id),
            affects: Vec::new(),
            skipped: None,
        };
        if let Some(category) = category_for_equip_slot(item.slot) {
            if stock != 0 {
                row.others = label(category, stock);
            }
            row.you = label(category, i64::from(item.product_id));
            if item.paint > 0 {
                row.you.push_str(&format!(" ({})", t("loadouts-bakkes-painted")));
            }
            if stock == 0 {
                row.skipped = Some(t("loadouts-bakkes-skip-empty"));
            } else {
                let pick = SwapPick {
                    category,
                    stock_product: stock,
                    product: i64::from(item.product_id),
                    paint: item.paint,
                };
                let (made, skipped) = swapper.bakkes_preset_swaps(&[pick]);
                if made.is_empty() {
                    row.skipped = Some(skipped.join("; "));
                } else {
                    swaps.extend(made);
                    for (other, teams) in &others {
                        if teams.iter().any(|products| i64::from(products[slot]) == stock) {
                            row.affects.push(other.clone());
                        }
                    }
                }
            }
        } else {
            row.skipped = Some(t("loadouts-bakkes-skip-unswappable"));
        }
        rows.push(row);
    }
    if swaps.is_empty() {
        return Err(t("loadouts-bakkes-no-swaps"));
    }
    // A team without its own colours uses the blue team's, the same as a shared loadout.
    let blue = code.blue.colors;
    let orange = code.orange.as_ref().map_or(blue, |team| team.colors);
    let cars = CarColours {
        blue_primary: blue.map(|(primary, _)| primary),
        blue_accent: blue.map(|(_, accent)| accent),
        orange_primary: orange.map(|(primary, _)| primary),
        orange_accent: orange.map(|(_, accent)| accent),
        ..CarColours::default()
    };
    Ok(Plan { name, base, replace, rows, swaps, cars })
}

fn colour_picker(ui: &mut egui::Ui, label: &str, selected: &mut u8, colours: &[[u8; 3]]) {
    ui.push_id(label, |ui| {
        let colour = |[r, g, b]: [u8; 3]| egui::Color32::from_rgb(r, g, b);
        let text = format!("{} ▾", t(label));
        let button = if let Some(rgb) = colours.get(usize::from(*selected)).copied() {
            egui::Button::image_and_text(
                egui::Image::new(egui::include_image!("../../assets/navigation/colours.svg"))
                    .fit_to_exact_size(egui::vec2(16.0, 16.0))
                    .tint(colour(rgb)),
                text,
            )
        } else {
            egui::Button::new(text)
        };
        let response = ui.add(button).on_hover_text(format!("{} #{}", t(label), selected));
        egui::Popup::menu(&response).show(|ui| {
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (row, swatches) in colours.chunks(10).enumerate() {
                    ui.horizontal(|ui| {
                        for (column, rgb) in swatches.iter().copied().enumerate() {
                            let index = (row * 10 + column) as u8;
                            let mut swatch = egui::Button::new("").fill(colour(rgb)).min_size(egui::vec2(20.0, 20.0));
                            if index == *selected {
                                swatch = swatch.stroke(egui::Stroke::new(2.0, ui.visuals().text_color()));
                            }
                            if ui.add(swatch).on_hover_text(format!("#{index}")).clicked() {
                                *selected = index;
                                ui.close();
                            }
                        }
                    });
                }
            });
        });
    });
}

fn show_plan(ui: &mut egui::Ui, plan: &Plan, names: &[String]) {
    ui.strong(&plan.name);
    egui::Grid::new("bakkes-plan").striped(true).show(ui, |ui| {
        for heading in [
            "loadouts-bakkes-col-slot",
            "loadouts-bakkes-col-others",
            "loadouts-bakkes-col-you",
            "loadouts-bakkes-col-affects",
        ] {
            ui.strong(t(heading));
        }
        ui.end_row();
        for row in &plan.rows {
            ui.label(slot_name(row.slot));
            ui.label(&row.others);
            match &row.skipped {
                Some(reason) => {
                    ui.weak(format!("{} ({reason})", row.you));
                }
                None => {
                    ui.label(&row.you);
                }
            }
            ui.label(row.affects.join(", "));
            ui.end_row();
        }
    });
    ui.weak(t_args("loadouts-bakkes-plan-note", &[("base", names[plan.base].as_str().into())]));
}

/// Saves the garage copy of the base, then the swap preset with the same name.
/// Both names and the preset cap are checked before the save is touched.
fn save_plan(save: &Path, plan: &Plan, presets: &mut PresetStore) -> Result<String, String> {
    let existing = presets.presets.iter().find(|p| p.name == plan.name).cloned();
    if plan.replace.is_none() && existing.is_some() {
        return Err(t("loadouts-bakkes-name-taken"));
    }
    if existing.is_none() && presets.presets.len() >= 10 {
        return Err(t("loadouts-bakkes-preset-cap"));
    }
    let mut copy = loadout_preset(save, plan.base)?;
    copy.name = plan.name.clone();
    apply_loadout_edit(save, &Edit::Import(vec![copy]))?;
    // The copy is appended, so the replaced preset keeps its position until it is removed.
    if let Some(position) = plan.replace {
        apply_loadout_edit(save, &Edit::Delete { position })?;
    }
    let has_cars = plan.cars != CarColours::default();
    let mut preset = existing.unwrap_or_default();
    preset.name = plan.name.clone();
    preset.swaps = plan.swaps.clone();
    let mut patches = preset.patches.as_object().cloned().unwrap_or_default();
    if has_cars {
        patches.insert("car_colours".into(), json!(plan.cars));
        preset.include_patches = true;
    } else {
        patches.remove("car_colours");
    }
    preset.patches = if patches.is_empty() { Value::Null } else { Value::Object(patches) };
    let count = plan.swaps.len();
    presets.save(preset)?;
    Ok(t_args(
        "loadouts-bakkes-saved",
        &[("name", plan.name.as_str().into()), ("count", count.to_string().into())],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_row_stays_compact_inside_a_scroll_area() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        for (frame, events) in [
            vec![],
            vec![egui::Event::Paste("LDS-first\r\nLDS-second".into())],
        ]
        .into_iter()
        .enumerate()
        {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let top = ui.next_widget_position().y;
                        import_entry(ui, &mut text, true);
                        assert!(ui.next_widget_position().y - top < 30.0);
                        assert!(ui.min_rect().right() <= 360.0);
                        if frame == 0 {
                            let id = ui.make_persistent_id("loadout-import-code");
                            ui.memory_mut(|memory| memory.request_focus(id));
                        }
                    });
                });
            });
        }
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>(),
            ["LDS-first", "LDS-second"]
        );
    }

    #[test]
    fn loadout_tiles_fit_the_panel_and_paginate_in_three_rows() {
        for width in [600.0, 1200.0] {
            let ctx = egui::Context::default();
            let mut panel = LoadoutsPanel {
                names: (0..30).map(|i| format!("Preset {i:02}")).collect(),
                bodies: vec![23; 30],
                ..Default::default()
            };
            let mut swapper = SwapperState::new(Path::new(""));
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.0),
                )),
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    panel.show_tiles(ui, &mut swapper, Path::new(""), false);
                    assert!(ui.min_rect().right() <= width + 1.0, "Loadouts overflow at {width}: {:?}", ui.min_rect());
                });
            });

            let mut tiles = Vec::new();
            fn collect_tiles(shape: &egui::Shape, tiles: &mut Vec<egui::Pos2>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with("Preset ") => {
                        tiles.push(text.pos);
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect_tiles(shape, tiles);
                        }
                    }
                    _ => {}
                }
            }
            for shape in &output.shapes {
                collect_tiles(&shape.shape, &mut tiles);
            }
            assert_eq!(tiles.len(), if width == 600.0 { 3 } else { 9 });
            tiles.dedup_by(|a, b| (a.y - b.y).abs() < 1.0);
            assert_eq!(tiles.len(), browser::ROWS);
        }
    }
}
