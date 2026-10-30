//! Loadout presets on the account save: share codes, rename, delete.
use eframe::egui;

use crate::i18n::t;
use crate::item_cleanup::{
    apply_loadout_edit, loadout_export, loadout_export_bakkes, loadout_names, loadout_preset,
};
use crate::patcher::colours::CarColours;
use crate::presets::{Preset, PresetStore};
use crate::swapper::{SwapPick, SwapperState, category_for_equip_slot};
use hebnix_sdk::save_file::{bakkes_code, loadout_code};
use hebnix_sdk::save_file::loadout_edit::Edit;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    #[default]
    Lds,
    Bakkes,
}

impl ExportFormat {
    fn label(self) -> String {
        match self {
            Self::Lds => t("loadouts-format-lds"),
            Self::Bakkes => t("loadouts-format-bakkes"),
        }
    }
}

#[derive(Default)]
pub struct LoadoutsPanel {
    names: Vec<String>,
    loaded: bool,
    import_text: String,
    renaming: Option<(usize, String)>,
    message: String,
    format: ExportFormat,
    bakkes_text: String,
    bakkes: Option<Result<bakkes_code::Code, String>>,
}

enum Pending {
    Copy(usize),
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

    fn import(&mut self) {
        let codes: Vec<&str> = self
            .import_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if codes.is_empty() {
            return;
        }
        let presets = codes
            .iter()
            .enumerate()
            .map(|(i, code)| loadout_code::decode(code).map_err(|e| format!("line {}: {e}", i + 1)))
            .collect::<Result<Vec<_>, _>>();
        match presets {
            Ok(presets) => {
                let result = apply_loadout_edit(&Edit::Import(presets));
                if result.is_ok() {
                    self.import_text.clear();
                }
                self.finish(result);
            }
            Err(error) => self.message = error,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, swapper: &SwapperState, presets: &mut PresetStore) {
        if !self.loaded {
            self.loaded = true;
            self.reload();
        }
        ui.heading(t("loadouts-title"));
        ui.label(t("loadouts-description"));
        ui.separator();
        self.show_bakkes(ui, swapper, presets);
        ui.separator();
        if ui.button(t("btn-refresh")).clicked() {
            self.reload();
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(t("loadouts-copy-format"));
            egui::ComboBox::from_id_salt("loadout-copy-format")
                .selected_text(self.format.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.format, ExportFormat::Lds, ExportFormat::Lds.label());
                    ui.selectable_value(&mut self.format, ExportFormat::Bakkes, ExportFormat::Bakkes.label());
                });
        });

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
                    if ui.button(t("loadouts-copy-code")).clicked() {
                        pending = Some(Pending::Copy(position));
                    }
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
            Some(Pending::Copy(position)) => {
                let code = match self.format {
                    ExportFormat::Lds => loadout_export(position),
                    ExportFormat::Bakkes => loadout_export_bakkes(position),
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

        ui.separator();
        ui.label(t("loadouts-import-hint"));
        ui.add(
            egui::TextEdit::multiline(&mut self.import_text)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if ui.button(t("loadouts-import")).clicked() {
            self.import();
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }

    }

    fn show_bakkes(&mut self, ui: &mut egui::Ui, swapper: &SwapperState, presets: &mut PresetStore) {
        ui.heading(t("loadouts-bakkes-title"));
        ui.label(t("loadouts-bakkes-hint"));
        ui.text_edit_singleline(&mut self.bakkes_text);
        if ui.button(t("loadouts-bakkes-read")).clicked() {
            self.bakkes = Some(bakkes_code::decode(&self.bakkes_text));
        }
        match &self.bakkes {
            Some(Ok(code)) => {
                match &code.orange {
                    Some(orange) => {
                        ui.strong(t("loadouts-bakkes-blue"));
                        show_team(ui, &code.blue);
                        ui.strong(t("loadouts-bakkes-orange"));
                        show_team(ui, orange);
                    }
                    None => {
                        ui.strong(t("loadouts-bakkes-same"));
                        show_team(ui, &code.blue);
                    }
                }
                if ui.button(t("loadouts-bakkes-make-swaps")).clicked() {
                    self.message = match swap_preset(code, swapper, presets) {
                        Ok(message) => message,
                        Err(error) => error,
                    };
                }
            }
            Some(Err(error)) => {
                ui.label(error);
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

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Saves a swap preset that shows the code's blue team in place of the stock loadout.
/// Stock is the first preset in the save: each swap's source is what it equips per slot.
fn swap_preset(
    code: &bakkes_code::Code,
    swapper: &SwapperState,
    presets: &mut PresetStore,
) -> Result<String, String> {
    let stock = loadout_preset(0)?;
    let products = &stock.teams[0].products;
    let mut picks = Vec::new();
    let mut skipped = Vec::new();
    for item in &code.blue.items {
        let Some(category) = category_for_equip_slot(item.slot) else {
            skipped.push(format!("{}: can't be swapped", slot_name(item.slot)));
            continue;
        };
        let stock_product = i64::from(products[usize::from(item.slot)]);
        if stock_product == 0 {
            skipped.push(format!("{}: you have none equipped", slot_name(item.slot)));
            continue;
        }
        picks.push(SwapPick {
            category,
            stock_product,
            product: i64::from(item.product_id),
            paint: item.paint,
        });
    }
    let (swaps, catalog_skipped) = swapper.bakkes_preset_swaps(&picks);
    skipped.extend(catalog_skipped);
    if swaps.is_empty() {
        return Err(format!("No swaps made. {}", skipped.join("; ")));
    }
    // A team without its own colours uses the blue team's, the same as a shared loadout.
    let blue = code.blue.colors;
    let orange = code.orange.as_ref().map_or(blue, |team| team.colors);
    let cars = CarColours {
        blue_primary: blue.map(|(primary, _)| primary),
        blue_accent: blue.map(|(_, accent)| accent),
        orange_primary: orange.map(|(primary, _)| primary),
        orange_accent: orange.map(|(_, accent)| accent),
    };
    let has_cars = cars != CarColours::default();
    let name = "BakkesMod code".to_string();
    let count = swaps.len();
    presets.save(Preset {
        name: name.clone(),
        include_patches: has_cars,
        patches: if has_cars {
            serde_json::json!({ "car_colours": cars })
        } else {
            serde_json::Value::Null
        },
        swaps,
    })?;
    let colours = if has_cars { ", plus custom colours" } else { "" };
    let mut message = format!(
        "Saved preset \"{name}\": {count} items{colours}. Select it in Presets and apply it to use it."
    );
    if !skipped.is_empty() {
        message.push_str(&format!(" Not included: {}.", skipped.join("; ")));
    }
    Ok(message)
}

fn show_team(ui: &mut egui::Ui, team: &bakkes_code::Team) {
    for item in &team.items {
        let painted = if item.paint > 0 {
            format!(" ({})", t("loadouts-bakkes-painted"))
        } else {
            String::new()
        };
        ui.label(format!("{}{painted}", slot_name(item.slot)));
    }
    if let Some((primary, secondary)) = team.colors {
        ui.label(format!(
            "{}: {} / {}",
            t("loadouts-bakkes-colours"),
            hex(primary),
            hex(secondary)
        ));
    }
}
