//! Loadout presets on the account save: share codes, rename, delete.
use eframe::egui;

use crate::i18n::t;
use crate::item_cleanup::{apply_loadout_edit, loadout_export, loadout_names};
use hebnix_sdk::save_file::{bakkes_code, loadout_code};
use hebnix_sdk::save_file::loadout_edit::Edit;

#[derive(Default)]
pub struct LoadoutsPanel {
    names: Vec<String>,
    loaded: bool,
    import_text: String,
    renaming: Option<(usize, String)>,
    message: String,
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

    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.loaded {
            self.loaded = true;
            self.reload();
        }
        ui.heading(t("loadouts-title"));
        ui.label(t("loadouts-description"));
        ui.separator();
        self.show_bakkes(ui);
        ui.separator();
        if ui.button(t("btn-refresh")).clicked() {
            self.reload();
        }
        ui.separator();

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
            Some(Pending::Copy(position)) => match loadout_export(position) {
                Ok(code) => {
                    ui.ctx().copy_text(code);
                    self.message = t("loadouts-copied");
                }
                Err(error) => self.message = error,
            },
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

    fn show_bakkes(&mut self, ui: &mut egui::Ui) {
        ui.label(t("loadouts-bakkes-hint"));
        ui.text_edit_singleline(&mut self.bakkes_text);
        if ui.button(t("loadouts-bakkes-read")).clicked() {
            self.bakkes = Some(bakkes_code::decode(&self.bakkes_text));
        }
        match &self.bakkes {
            Some(Ok(code)) => {
                ui.strong(t("loadouts-bakkes-blue"));
                show_team(ui, &code.blue);
                match &code.orange {
                    Some(orange) => {
                        ui.strong(t("loadouts-bakkes-orange"));
                        show_team(ui, orange);
                    }
                    None => {
                        ui.weak(t("loadouts-bakkes-same"));
                    }
                }
            }
            Some(Err(error)) => {
                ui.label(error);
            }
            None => {}
        }
    }
}

fn show_team(ui: &mut egui::Ui, team: &bakkes_code::Team) {
    for item in &team.items {
        ui.label(format!(
            "slot {}: product {} paint {}",
            item.slot, item.product_id, item.paint
        ));
    }
    if let Some((primary, secondary)) = team.colors {
        ui.label(format!("colors {primary:?} / {secondary:?}"));
    }
}
