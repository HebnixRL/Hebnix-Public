//! Read-only inventory snapshots from the selected local Rocket League save.
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;
use hebnix_sdk::save_file::{self, SaveAccount, SaveData};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};

pub type OwnedPaints = HashMap<i64, BTreeSet<i64>>;

#[derive(Default)]
struct OwnedProducts {
    ids: HashSet<i64>,
    paints: OwnedPaints,
}

fn owned_products(save: &SaveData) -> Result<OwnedProducts, String> {
    if !save
        .raw_properties
        .get("OnlineProducts")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err("This save does not contain a supported inventory.".into());
    }
    for reference in save.raw_properties["OnlineProducts"].as_array().unwrap() {
        let object = reference
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| save.objects.get(index))
            .ok_or("This save contains an invalid inventory reference.")?;
        if object["__type"] != "TAGame.OnlineProduct_TA" {
            return Err("This save contains an unsupported inventory entry.".into());
        }
    }
    let mut products = OwnedProducts::default();
    for product in save
        .inventory()
        .into_iter()
        .filter(|product| product.product_id > 0)
    {
        products.ids.insert(product.product_id);
        for attribute in product.attributes.as_array().into_iter().flatten() {
            let attribute = if attribute.is_object() {
                attribute
            } else {
                attribute
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| save.objects.get(index))
                    .ok_or("Invalid saved product attribute reference.")?
            };
            if attribute["__type"] == "TAGame.ProductAttribute_Painted_TA"
                && let Some(paint) = attribute["PaintID"].as_i64().filter(|&id| id > 0)
            {
                products
                    .paints
                    .entry(product.product_id)
                    .or_default()
                    .insert(paint);
            }
        }
    }
    Ok(products)
}

fn selected_account_path(accounts: &[SaveAccount], selected: Option<&Path>) -> Option<PathBuf> {
    let selected_id = selected
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.split('_').next());
    accounts
        .iter()
        .find(|account| selected_id.is_some_and(|id| account.account_id.eq_ignore_ascii_case(id)))
        // SDK discovery orders accounts by their newest save, including rotations.
        .or_else(|| accounts.first())
        .map(|account| account.path.clone())
}

#[derive(Default)]
pub struct OwnedInventory {
    pub ids: HashSet<i64>,
    pub paints: OwnedPaints,
    pub title_ids: HashSet<String>,
    accounts: Vec<SaveAccount>,
    selected: Option<PathBuf>,
    result: Option<
        Receiver<Result<(Vec<SaveAccount>, PathBuf, OwnedProducts, HashSet<String>), String>>,
    >,
    error: Option<String>,
    initial_refresh_attempted: bool,
}

impl OwnedInventory {
    #[cfg(test)]
    pub(crate) fn empty_for_ui_tests() -> Self {
        Self {
            initial_refresh_attempted: true,
            ..Default::default()
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        // Consume the one automatic load even if it fails or returns no items.
        self.initial_refresh_attempted = true;
        if self.result.is_some() {
            return;
        }
        let selected = self.selected.clone();
        let (tx, rx) = unbounded();
        self.result = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let accounts = save_file::find_save_accounts(None);
                let path = selected_account_path(&accounts, selected.as_deref())
                    .ok_or("No Rocket League account save was found in the detected save folder.")?;
                let save = save_file::load(&path, true)
                    .map_err(|e| format!("Could not read inventory: {e}"))?;
                let products = owned_products(&save)?;
                let titles = save
                    .owned_title_ids()?
                    .into_iter()
                    .map(|id| id.to_ascii_lowercase())
                    .collect();
                Ok((accounts, path, products, titles))
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.show_with_actions(ui, |_| {});
    }

    pub fn show_with_actions(&mut self, ui: &mut egui::Ui, actions: impl FnOnce(&mut egui::Ui)) {
        if !self.initial_refresh_attempted {
            self.refresh(ui.ctx());
        }
        if let Some(result) = self.result.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.result = None;
            match result {
                Ok((accounts, path, products, titles)) => {
                    self.accounts = accounts;
                    self.selected = Some(path);
                    self.ids = products.ids;
                    self.paints = products.paints;
                    self.title_ids = titles;
                    self.error = None;
                }
                Err(error) => {
                    self.accounts.clear();
                    self.selected = None;
                    self.ids.clear();
                    self.paints.clear();
                    self.title_ids.clear();
                    self.error = Some(error);
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Owned items from save:");
            let before = self.selected.clone();
            let name = self
                .accounts
                .iter()
                .find(|account| Some(&account.path) == self.selected.as_ref())
                .map(|account| account.display_name.as_str())
                .unwrap_or("Latest local account");
            ui.add_enabled_ui(self.result.is_none(), |ui| {
                egui::ComboBox::from_id_salt("owned_save_account")
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for account in &self.accounts {
                            ui.selectable_value(
                                &mut self.selected,
                                Some(account.path.clone()),
                                &account.display_name,
                            );
                        }
                    });
            });
            let changed = before != self.selected;
            if changed {
                self.ids.clear();
                self.paints.clear();
                self.title_ids.clear();
            }
            let refresh = ui
                .add_enabled(
                    self.result.is_none(),
                    egui::Button::new("Refresh owned items"),
                )
                .clicked();
            if refresh {
                self.refresh(ui.ctx());
            }
            if self.result.is_some() {
                ui.spinner();
            }
            actions(ui);
        });
        if let Some(error) = &self.error {
            ui.weak(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_to_latest_account_but_preserves_manual_account_across_rotations() {
        let latest_id = "f514e015f0504d8e9031c262f3d089ac";
        let chosen_id = "0123456789abcdef0123456789abcdef";
        let latest = PathBuf::from(format!("{latest_id}_2.save"));
        let chosen = PathBuf::from(format!("{chosen_id}_2.save"));
        let accounts = vec![
            SaveAccount {
                account_id: latest_id.into(),
                display_name: format!("Account ({latest_id})"),
                path: latest.clone(),
            },
            SaveAccount {
                account_id: chosen_id.into(),
                display_name: format!("Account ({chosen_id})"),
                path: chosen.clone(),
            },
        ];
        assert_eq!(selected_account_path(&accounts, None), Some(latest));
        let previous_rotation = PathBuf::from(format!("{chosen_id}_1.save"));
        assert_eq!(selected_account_path(&accounts, Some(&previous_rotation)), Some(chosen));
        assert!(selected_account_path(&[], None).is_none());
    }

    fn frame(inventory: &mut OwnedInventory) {
        let _ = egui::Context::default().run_ui(Default::default(), |ui| {
            inventory.show(ui);
        });
    }

    #[test]
    fn first_tab_load_is_not_repeated_after_empty_inventory_or_account_change() {
        let (tx, rx) = unbounded();
        let mut inventory = OwnedInventory {
            result: Some(rx),
            ..Default::default()
        };
        // Stand in for the background read so this test never accesses local saves.
        frame(&mut inventory);
        assert!(inventory.initial_refresh_attempted);
        assert!(inventory.result.is_some());
        tx.send(Ok((
            Vec::new(),
            PathBuf::from("first.save"),
            OwnedProducts::default(),
            HashSet::new(),
        )))
        .unwrap();
        frame(&mut inventory);
        assert!(inventory.result.is_none());
        inventory.selected = Some(PathBuf::from("second.save"));
        for _ in 0..3 {
            frame(&mut inventory);
            assert!(inventory.result.is_none());
        }
    }

    #[test]
    fn failed_initial_load_is_not_automatically_retried() {
        let (tx, rx) = unbounded();
        let mut inventory = OwnedInventory {
            result: Some(rx),
            ..Default::default()
        };
        frame(&mut inventory);
        tx.send(Err("No local save".into())).unwrap();
        for _ in 0..3 {
            frame(&mut inventory);
            assert!(inventory.result.is_none());
            assert_eq!(inventory.error.as_deref(), Some("No local save"));
        }
    }

    fn empty_save() -> SaveData {
        SaveData {
            source_file: PathBuf::new(),
            header: Default::default(),
            raw_properties: serde_json::Value::Null,
            objects: Vec::new(),
        }
    }
    #[test]
    fn only_root_inventory_members_are_owned() {
        let save = SaveData {
            raw_properties: json!({"OnlineProducts":[0,1,3]}),
            objects: vec![
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":4659}),
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":4659}),
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":4685}),
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":4681}),
            ],
            ..empty_save()
        };
        assert_eq!(
            owned_products(&save).unwrap().ids,
            HashSet::from([4659, 4681])
        );
    }
    #[test]
    fn reads_factory_paints_from_owned_instances_only() {
        let save = SaveData {
            raw_properties: json!({"OnlineProducts":[0,1,2]}),
            objects: vec![
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":1,"Attributes":[4]}),
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":1,"Attributes":[
                    {"__type":"TAGame.ProductAttribute_Painted_TA","PaintID":12}
                ]}),
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":2}),
                json!({"__type":"TAGame.OnlineProduct_TA","ProductID":3,"Attributes":[4]}),
                json!({"__type":"TAGame.ProductAttribute_Painted_TA","PaintID":8}),
            ],
            ..empty_save()
        };
        let owned = owned_products(&save).unwrap();
        assert_eq!(owned.ids, HashSet::from([1, 2]));
        assert_eq!(owned.paints, [(1, BTreeSet::from([8, 12]))].into());
    }
    #[test]
    fn missing_inventory_is_reported_while_empty_inventory_is_valid() {
        assert!(owned_products(&empty_save()).is_err());
        assert!(
            owned_products(&SaveData {
                raw_properties: json!({"OnlineProducts":[]}),
                ..empty_save()
            })
            .unwrap()
            .ids
            .is_empty()
        );
    }
    #[test]
    fn invalid_references_do_not_silently_present_a_partial_inventory() {
        let save = SaveData {
            raw_properties: json!({"OnlineProducts":[99]}),
            ..empty_save()
        };
        assert!(owned_products(&save).is_err());
    }
}
