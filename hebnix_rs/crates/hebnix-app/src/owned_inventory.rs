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
        let blueprint=product.attributes.as_array().into_iter().flatten().any(|attribute| {
            let attribute=attribute.as_u64().and_then(|index|save.objects.get(index as usize)).unwrap_or(attribute);
            attribute["__type"]=="TAGame.ProductAttribute_Blueprint_TA"
        });
        if blueprint { continue; }
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

fn primary_account_from_log_line(line:&str)->Option<String> {
    if !line.contains("SaveGame: Load Player.ControllerId=(0)") {return None}
    let path=line.split_once("SaveFileName=(")?.1.split(')').next()?;
    let file=path.rsplit(['\\','/']).next()?;
    let stem=file.strip_suffix(".save")?.split('_').next()?;
    (!stem.is_empty()).then(||stem.to_owned())
}
fn last_launched_account()->Option<String> {
    use std::io::{BufRead,BufReader,Read};
    let log=hebnix_sdk::utils::paths::documents_dir()?.join("My Games/Rocket League/TAGame/Logs/Launch.log");
    let file=std::fs::File::open(log).ok()?;
    BufReader::new(file.take(4*1024*1024)).lines().map_while(Result::ok)
        .filter_map(|line|primary_account_from_log_line(&line)).last()
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
    game_was_running: bool,
    launch_watch_until: Option<std::time::Instant>,
    next_account_probe: Option<std::time::Instant>,
    account_probe: Option<Receiver<Option<String>>>,
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
                let logged=selected.is_none().then(last_launched_account).flatten();
                let preferred=logged.as_ref().and_then(|id|accounts.iter().find(|account|account.account_id.eq_ignore_ascii_case(id))).map(|account|account.path.as_path());
                let path = selected_account_path(&accounts, selected.as_deref().or(preferred))
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

    pub(crate) fn poll(&mut self, ctx: &egui::Context) {
        if !self.initial_refresh_attempted {
            self.refresh(ctx);
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
    }

    #[cfg(windows)]
    pub(crate) fn web_snapshot(&self) -> serde_json::Value {
        let accounts: Vec<_> = self.accounts.iter().map(|account| {
            serde_json::json!({
                "id": account.account_id,
                "label": account.display_name,
                "selected": self.selected.as_ref() == Some(&account.path),
            })
        }).collect();
        serde_json::json!({
            "accounts": accounts,
            "loading": self.result.is_some(),
            "error": self.error,
        })
    }

    pub(crate) fn show_account_picker(&mut self, ui: &mut egui::Ui) -> Option<String> {
        self.poll(ui.ctx());
        let selected = self.accounts.iter().find(|account| Some(&account.path) == self.selected.as_ref())
            .map(|account| account.display_name.clone()).unwrap_or_else(|| "Latest local save".to_owned());
        let mut next = None;
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label("Account:");
            egui::ComboBox::from_id_salt("settings_save_account")
                .selected_text(selected)
                .width(190.0)
                .show_ui(ui, |ui| {
                    for account in &self.accounts {
                        if ui.selectable_label(Some(&account.path) == self.selected.as_ref(), &account.display_name).clicked() {
                            next = Some(account.account_id.clone());
                        }
                    }
                });
            if ui.add_enabled(self.result.is_none(), egui::Button::new("↻")).on_hover_text("Refresh save").clicked() {
                refresh = true;
            }
        });
        if refresh { self.refresh(ui.ctx()); }
        if let Some(id) = next.as_ref() {
            self.refresh_web(ui.ctx(), Some(id));
        }
        next
    }

    pub(crate) fn follow_game_account(&mut self,ctx:&egui::Context,running:bool) {
        use std::time::{Instant,Duration};
        let now=Instant::now();
        if running && !self.game_was_running {self.launch_watch_until=Some(now+Duration::from_secs(45));self.next_account_probe=Some(now);}
        self.game_was_running=running;
        if !running {self.launch_watch_until=None;}
        if let Some(result)=self.account_probe.as_ref().and_then(|rx|rx.try_recv().ok()) {
            self.account_probe=None;
            if let Some(id)=result && self.result.is_none() && self.launch_watch_until.is_some() {
                if let Some(path)=self.accounts.iter().find(|account|account.account_id.eq_ignore_ascii_case(&id)).map(|account|account.path.clone()) {
                    if self.selected.as_ref()!=Some(&path) {self.selected=Some(path);self.ids.clear();self.paints.clear();self.title_ids.clear();self.refresh(ctx);}
                }
            }
        }
        if self.launch_watch_until.is_some_and(|until|now<until) && self.account_probe.is_none() && self.next_account_probe.is_none_or(|next|now>=next) {
            self.next_account_probe=Some(now+Duration::from_secs(2));
            let(tx,rx)=unbounded();self.account_probe=Some(rx);let ctx=ctx.clone();
            std::thread::spawn(move||{let _=tx.send(last_launched_account());ctx.request_repaint();});
        }
    }

    pub(crate) fn selected_save(&self) -> Option<&std::path::Path> { self.selected.as_deref() }

    #[cfg(windows)]
    pub(crate) fn refresh_web(&mut self, ctx: &egui::Context, account: Option<&str>) {
        if self.result.is_some() {
            return;
        }
        if let Some(id) = account {
            self.launch_watch_until=None;
            let Some(account) = self.accounts.iter().find(|account| account.account_id == id) else {
                return;
            };
            self.selected = Some(account.path.clone());
            self.ids.clear();
            self.paints.clear();
            self.title_ids.clear();
        }
        self.refresh(ctx);
    }

    pub fn show_with_actions(&mut self, ui: &mut egui::Ui, actions: impl FnOnce(&mut egui::Ui)) {
        self.poll(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            ui.label("Owned items from the active account");
            let name = self
                .accounts
                .iter()
                .find(|account| Some(&account.path) == self.selected.as_ref())
                .map(|account| account.display_name.as_str())
                .unwrap_or("Latest local account");
            ui.weak(name);
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

#[cfg(test)]
mod launch_account_tests {
    use super::primary_account_from_log_line;
    #[test]
    fn chooses_only_the_primary_save_load_not_other_player_ids() {
        assert_eq!(primary_account_from_log_line("[0024.22] SaveGame: Load Player.ControllerId=(0) SaveFileName=(../../TAGame/SaveDataEpic/DBE_Production/0123456789abcdef0123456789abcdef_2.save)"),Some("0123456789abcdef0123456789abcdef".into()));
        assert_eq!(primary_account_from_log_line("SaveGame: Load Player.ControllerId=(1) SaveFileName=(other.save)"),None);
        assert_eq!(primary_account_from_log_line("Party: OtherPlayerID=0123456789abcdef0123456789abcdef"),None);
        assert_eq!(primary_account_from_log_line("SaveGame: Load Player.ControllerId=(0) SaveFileName=(C:\\Games\\SaveData\\76561198000000000.save)"),Some("76561198000000000".into()));
    }
}
