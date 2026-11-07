//! The Delete backups button. It removes the whole Backups folder next to the
//! game packages, so the second click is the only way to delete it.
use crate::i18n::t;
use eframe::egui;
use std::path::Path;

#[derive(Default)]
pub struct State {
    confirm: bool,
    error: Option<String>,
}

impl State {
    pub fn render(&mut self, ui: &mut egui::Ui, cooked_pc: &Path) {
        ui.horizontal(|ui| {
            if self.confirm {
                if ui.button(t("delete-backups-confirm")).clicked() {
                    self.confirm = false;
                    self.error = super::backup_guard::delete_backups(cooked_pc).err();
                }
                if ui.button(t("delete-backups-cancel")).clicked() {
                    self.confirm = false;
                }
            } else if ui.button(t("delete-backups")).clicked() {
                self.confirm = true;
                self.error = None;
            }
        });
        ui.label(egui::RichText::new(t("delete-backups-note")).weak());
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
}
