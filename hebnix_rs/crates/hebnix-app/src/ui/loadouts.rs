//! Loadouts on the account save: import (BakkesMod or share codes), copy, rename, delete.
use eframe::egui;

use crate::i18n::{t, t_args};
use crate::item_cleanup::{
    apply_loadout_edit, loadout_export, loadout_export_bakkes, loadout_names, loadout_preset,
};
use crate::patcher::colours::CarColours;
use crate::presets::PresetStore;
use crate::swapper::{SwapPick, SwapperState, category_for_equip_slot};
use hebnix_sdk::save_file::loadout_edit::Edit;
use hebnix_sdk::save_file::{bakkes_code, loadout_code};
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

#[derive(Default)]
pub struct LoadoutsPanel {
    names: Vec<String>,
    loaded: bool,
    input: String,
    renaming: Option<(usize, String)>,
    message: String,
    /// BakkesMod code waiting for a base preset, a name and a confirm
    code: Option<bakkes_code::Code>,
    base: usize,
    mode: SaveMode,
    target: usize,
    name: String,
    plan: Option<(PlanKey, Result<Plan, String>)>,
}

enum Pending {
    Copy(usize, bool),
    Delete(usize),
    Rename(usize, String),
}

impl LoadoutsPanel {
    fn reload(&mut self) {
        match loadout_names() {
            Ok(names) => self.names = names,
            Err(error) => {
                self.names.clear();
                self.message = error;
            }
        }
    }

    fn finish(&mut self, result: Result<String, String>) {
        self.message = result.unwrap_or_else(|error| error);
        self.reload();
    }

    /// Share codes (`LDS...`) import at once. A BakkesMod code opens the import card.
    fn read_input(&mut self) {
        let lines: Vec<&str> = self
            .input
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let Some(first) = lines.first() else {
            return;
        };
        if first.starts_with("LDS") {
            let presets = lines
                .iter()
                .enumerate()
                .map(|(i, code)| loadout_code::decode(code).map_err(|e| format!("line {}: {e}", i + 1)))
                .collect::<Result<Vec<_>, _>>();
            match presets {
                Ok(presets) => {
                    let result = apply_loadout_edit(&Edit::Import(presets));
                    if result.is_ok() {
                        self.input.clear();
                    }
                    self.finish(result);
                }
                Err(error) => self.message = error,
            }
        } else if lines.len() > 1 {
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
        swapper: &SwapperState,
        presets: &mut PresetStore,
    ) -> Option<String> {
        if !self.loaded {
            self.loaded = true;
            self.reload();
        }
        ui.heading(t("loadouts-title"));
        ui.label(t("loadouts-description"));
        ui.add_space(6.0);
        let apply = egui::Frame::group(ui.style())
            .show(ui, |ui| self.show_import(ui, swapper, presets))
            .inner;
        ui.add_space(8.0);
        self.show_list(ui);
        if !self.message.is_empty() {
            ui.add_space(6.0);
            ui.label(&self.message);
        }
        apply
    }

    fn show_import(
        &mut self,
        ui: &mut egui::Ui,
        swapper: &SwapperState,
        presets: &mut PresetStore,
    ) -> Option<String> {
        ui.strong(t("loadouts-import-title"));
        let Some(code) = self.code.clone() else {
            ui.label(t("loadouts-import-hint"));
            ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(t("loadouts-import-placeholder")),
            );
            let filled = !self.input.trim().is_empty();
            if ui.add_enabled(filled, egui::Button::new(t("loadouts-import"))).clicked() {
                self.read_input();
            }
            return None;
        };
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
            let plan = build_plan(&code, swapper, &self.names, key.clone());
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
            if ui.add_enabled(ready, primary).clicked() {
                action = Some(true);
            }
            if ui.add_enabled(ready, egui::Button::new(t("loadouts-import-only"))).clicked() {
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
        match save_plan(&plan, presets) {
            Ok(mut message) => {
                if !apply {
                    message.push(' ');
                    message.push_str(&t("loadouts-bakkes-apply-hint"));
                }
                self.finish(Ok(message));
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

    fn show_list(&mut self, ui: &mut egui::Ui) {
        ui.strong(t("loadouts-list-title"));
        if self.names.is_empty() {
            ui.weak(t("loadouts-empty"));
        }
        let mut pending = None;
        for (position, name) in self.names.clone().into_iter().enumerate() {
            ui.horizontal(|ui| {
                if let Some((editing, buffer)) =
                    self.renaming.as_mut().filter(|(p, _)| *p == position)
                {
                    ui.text_edit_singleline(buffer);
                    if ui.button(t("loadouts-save-name")).clicked() {
                        pending = Some(Pending::Rename(*editing, buffer.trim().to_string()));
                    }
                } else {
                    ui.label(&name);
                    ui.menu_button(t("loadouts-copy-code"), |ui| {
                        if ui.button(t("loadouts-format-lds")).clicked() {
                            pending = Some(Pending::Copy(position, false));
                            ui.close();
                        }
                        if ui.button(t("loadouts-format-bakkes")).clicked() {
                            pending = Some(Pending::Copy(position, true));
                            ui.close();
                        }
                    });
                    if ui.button(t("loadouts-rename")).clicked() {
                        self.renaming = Some((position, name.clone()));
                    }
                    if ui.button(t("loadouts-delete")).clicked() {
                        pending = Some(Pending::Delete(position));
                    }
                }
            });
        }
        match pending {
            Some(Pending::Copy(position, bakkes)) => {
                let code = if bakkes {
                    loadout_export_bakkes(position)
                } else {
                    loadout_export(position)
                };
                match code {
                    Ok(code) => {
                        ui.ctx().copy_text(code);
                        self.message = t("loadouts-copied");
                    }
                    Err(error) => self.message = error,
                }
            }
            Some(Pending::Delete(position)) => {
                let result = apply_loadout_edit(&Edit::Delete { position });
                self.finish(result);
            }
            Some(Pending::Rename(position, name)) => {
                self.renaming = None;
                let result = apply_loadout_edit(&Edit::Rename { position, name });
                self.finish(result);
            }
            None => {}
        }
    }
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
    let base_products = loadout_preset(base)?.teams[0].products;
    // The base is the copy's source and the replaced preset goes away, so neither is listed.
    let others: Vec<(String, Vec<[i32; loadout_code::SLOTS]>)> = names
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != base && Some(*i) != replace)
        .filter_map(|(i, name)| {
            let preset = loadout_preset(i).ok()?;
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
fn save_plan(plan: &Plan, presets: &mut PresetStore) -> Result<String, String> {
    let existing = presets.presets.iter().find(|p| p.name == plan.name).cloned();
    if plan.replace.is_none() && existing.is_some() {
        return Err(t("loadouts-bakkes-name-taken"));
    }
    if existing.is_none() && presets.presets.len() >= 10 {
        return Err(t("loadouts-bakkes-preset-cap"));
    }
    let mut copy = loadout_preset(plan.base)?;
    copy.name = plan.name.clone();
    apply_loadout_edit(&Edit::Import(vec![copy]))?;
    // The copy is appended, so the replaced preset keeps its position until it is removed.
    if let Some(position) = plan.replace {
        apply_loadout_edit(&Edit::Delete { position })?;
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
