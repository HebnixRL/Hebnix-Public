//! Experimental: write catalog items into the account save as local inventory entries.
use eframe::egui;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::i18n::{t, t_args};
use crate::item_cleanup::{apply_loadout_edit, restore_before_added_items};
use crate::swapper::{CatalogEntry, SwapCategory, SwapperState};
use crate::ui::loadouts::discover_accounts;
use hebnix_sdk::save_file::{SaveAccount, loadout_edit::Edit};

/// In-game paint ids 1..=13, in order.
const PAINTS: [&str; 13] = [
    "Crimson", "Lime", "Black", "Sky Blue", "Cobalt", "Burnt Sienna", "Forest Green",
    "Purple", "Pink", "Orange", "Grey", "Titanium White", "Saffron",
];

#[derive(Default)]
pub struct AddItemsPanel {
    accounts: Vec<SaveAccount>,
    save: Option<PathBuf>,
    loaded: bool,
    entries: Vec<CatalogEntry>,
    /// Lowercased names, parallel to `entries`
    lower: Vec<String>,
    paintable_total: usize,
    /// Entry indices matching the filters, rebuilt only when they change
    shown: Vec<usize>,
    shown_key: Option<(String, Option<SwapCategory>)>,
    /// Reading the sync state loads the whole game package, so it is cached
    sync_active: Option<bool>,
    category: Option<SwapCategory>,
    search: String,
    picked: BTreeSet<i64>,
    /// 0 = unpainted, otherwise the paint id given to picked paintable items
    paint: u8,
    all_items: bool,
    all_paints: bool,
    understood: bool,
    /// Set by the app from settings before each frame, read back after it
    pub block_sync: bool,
    message: String,
    working: Option<Receiver<Result<String, String>>>,
}

impl AddItemsPanel {
    fn count(&self) -> usize {
        match (self.all_items, self.all_paints) {
            (true, true) => self.entries.len() + self.paintable_total * PAINTS.len(),
            (true, false) => self.entries.len(),
            _ => self.picked.len(),
        }
    }

    fn requested(&self) -> Vec<(i32, u8)> {
        let mut out = Vec::new();
        for entry in &self.entries {
            let Ok(id) = i32::try_from(entry.product_id) else { continue };
            if self.all_items {
                out.push((id, 0));
                if self.all_paints && entry.paintable {
                    out.extend((1..=PAINTS.len() as u8).map(|paint| (id, paint)));
                }
            } else if self.picked.contains(&entry.product_id) {
                out.push((id, if entry.paintable { self.paint } else { 0 }));
            }
        }
        out
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.working else { return };
        match receiver.try_recv() {
            Ok(result) => {
                self.message = result.unwrap_or_else(|error| error);
                self.working = None;
                self.sync_active = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            Err(TryRecvError::Disconnected) => {
                self.message = t("add-items-failed");
                self.working = None;
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, swapper: &SwapperState, cooked: &Path, catalog_loading: bool) {
        self.poll(ui.ctx());
        if !self.loaded {
            self.accounts = discover_accounts();
            self.save = self.accounts.first().map(|account| account.path.clone());
            self.loaded = true;
        }
        if self.entries.is_empty() && !catalog_loading {
            self.entries = SwapCategory::ALL.into_iter().flat_map(|category| swapper.catalog_entries(category)).collect();
            self.entries.sort_by_key(|entry| entry.name.to_lowercase());
            self.lower = self.entries.iter().map(|entry| entry.name.to_lowercase()).collect();
            self.paintable_total = self.entries.iter().filter(|entry| entry.paintable).count();
            self.shown_key = None;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.colored_label(ui.visuals().warn_fg_color, egui::RichText::new(t("add-items-warning-title")).strong());
            ui.label(t("add-items-warning-body"));
            ui.checkbox(&mut self.understood, t("add-items-understood"));
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(t("loadouts-save"));
            let selected = self.accounts.iter()
                .find(|account| Some(&account.path) == self.save.as_ref())
                .map_or_else(|| t("loadouts-select-save"), |account| account.display_name.clone());
            egui::ComboBox::from_id_salt("add-items-save").selected_text(selected).show_ui(ui, |ui| {
                for account in &self.accounts {
                    ui.selectable_value(&mut self.save, Some(account.path.clone()), &account.display_name)
                        .on_hover_text(account.path.display().to_string());
                }
            });
            if ui.button(t("loadouts-refresh-saves")).clicked() {
                self.accounts = discover_accounts();
                self.save = self.accounts.first().map(|account| account.path.clone());
            }
        });
        ui.checkbox(&mut self.all_items, t("add-items-all"));
        ui.add_enabled_ui(self.all_items, |ui| {
            ui.checkbox(&mut self.all_paints, t("add-items-all-paints"));
        });
        if !self.all_items {
            self.all_paints = false;
            self.show_browser(ui, catalog_loading);
        } else if self.all_paints {
            ui.weak(t("add-items-size-note"));
        }
        ui.add_space(6.0);
        ui.checkbox(&mut self.block_sync, t("sync-block-checkbox")).on_hover_text(t("sync-block-hint"));
        let sync_active = *self.sync_active.get_or_insert_with(|| crate::patcher::sync_block::active(cooked));
        if sync_active {
            ui.horizontal(|ui| {
                ui.weak(t("sync-block-active"));
                if ui.add_enabled(self.working.is_none(), egui::Button::new(t("sync-block-remove"))).clicked() {
                    self.message = crate::patcher::sync_block::disable(cooked, &cooked.join("Backups"))
                        .map_or_else(|error| error, |()| t("sync-block-removed"));
                    self.sync_active = None;
                }
            });
        }
        if ui.add_enabled(self.save.is_some() && self.working.is_none(), egui::Button::new(t("add-items-restore"))).on_hover_text(t("add-items-restore-hint")).clicked() {
            self.restore(cooked);
        }
        let count = self.count();
        let ready = self.understood && self.save.is_some() && count > 0 && self.working.is_none();
        ui.horizontal(|ui| {
            let label = t_args("add-items-apply", &[("count", count.to_string().into())]);
            if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                self.apply(self.requested(), cooked);
            }
            if self.working.is_some() {
                ui.spinner();
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
    }

    fn show_browser(&mut self, ui: &mut egui::Ui, catalog_loading: bool) {
        if catalog_loading || self.entries.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("app-loading-item-catalogs"));
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(t("add-items-search")));
            egui::ComboBox::from_id_salt("add-items-category")
                .selected_text(self.category.map_or_else(|| t("add-items-every-category"), SwapCategory::label))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, None, t("add-items-every-category"));
                    for category in SwapCategory::ALL {
                        ui.selectable_value(&mut self.category, Some(category), category.label());
                    }
                });
            egui::ComboBox::from_id_salt("add-items-paint")
                .selected_text(match self.paint {
                    0 => t("add-items-unpainted"),
                    paint => PAINTS[paint as usize - 1].to_string(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.paint, 0, t("add-items-unpainted"));
                    for (index, name) in PAINTS.iter().enumerate() {
                        ui.selectable_value(&mut self.paint, index as u8 + 1, *name);
                    }
                });
            if ui.button(t("add-items-clear")).clicked() {
                self.picked.clear();
            }
        });
        let key = (self.search.to_lowercase(), self.category);
        if self.shown_key.as_ref() != Some(&key) {
            self.shown = (0..self.entries.len())
                .filter(|&i| key.1.is_none_or(|category| self.entries[i].category == category))
                .filter(|&i| key.0.is_empty() || self.lower[i].contains(&key.0))
                .collect();
            self.shown_key = Some(key);
        }
        ui.weak(t_args("add-items-picked", &[("count", self.picked.len().to_string().into())]));
        let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical().id_salt("add-items-list").max_height(320.0).auto_shrink([false, true])
            .show_rows(ui, row, self.shown.len(), |ui, range| {
                for &index in &self.shown[range] {
                    let entry = &self.entries[index];
                    let mut on = self.picked.contains(&entry.product_id);
                    let name = format!("{}  ({})", entry.name, entry.category.label());
                    if ui.checkbox(&mut on, name).changed() {
                        if on { self.picked.insert(entry.product_id); } else { self.picked.remove(&entry.product_id); }
                    }
                }
            });
    }

    fn restore(&mut self, cooked: &Path) {
        let Some(path) = self.save.clone() else { return };
        let cooked = cooked.to_path_buf();
        self.message.clear();
        let (tx, rx) = mpsc::channel();
        self.working = Some(rx);
        std::thread::spawn(move || {
            let result = restore_before_added_items(&path).and_then(|_| {
                if crate::patcher::sync_block::active(&cooked) {
                    crate::patcher::sync_block::disable(&cooked, &cooked.join("Backups"))
                        .map_err(|error| format!("{} {error}", t("add-items-restored-block-failed")))?;
                }
                Ok(t("add-items-restored"))
            });
            let _ = tx.send(result);
        });
    }

    fn apply(&mut self, items: Vec<(i32, u8)>, cooked: &Path) {
        let Some(path) = self.save.clone() else { return };
        let (cooked, block) = (cooked.to_path_buf(), self.block_sync);
        self.message.clear();
        let (tx, rx) = mpsc::channel();
        self.working = Some(rx);
        std::thread::spawn(move || {
            let result = if block { crate::patcher::sync_block::enable(&cooked, &cooked.join("Backups")) } else { Ok(()) }
                .and_then(|()| std::fs::read(&path).map_err(|e| e.to_string()))
                .and_then(|raw| hebnix_sdk::save_file::loadout_edit::new_item_count(&raw, &items))
                .and_then(|count| apply_loadout_edit(&path, &Edit::AddItems(items)).map(|()| count))
                .map(|count| match count {
                    0 => t("add-items-nothing-new"),
                    count => t_args("add-items-done", &[("count", count.to_string().into())]),
                });
            let _ = tx.send(result);
        });
    }
}
