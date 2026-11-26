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
    #[cfg(windows)]
    pub(crate) fn web_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "confirming": self.confirm, "error": self.error, "label": t("delete-backups"),
            "confirm": t("delete-backups-confirm"), "cancel": t("delete-backups-cancel"), "note": t("delete-backups-note"),
        })
    }

    #[cfg(windows)]
    pub(crate) fn web_action(&mut self, action: &str, cooked: &Path) {
        match action {
            "open" => { self.confirm = true; self.error = None; }
            "cancel" => self.confirm = false,
            "confirm" if self.confirm => {
                self.confirm = false;
                self.error = super::backup_guard::delete_backups(cooked).err();
            }
            _ => {}
        }
    }
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

        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
}
