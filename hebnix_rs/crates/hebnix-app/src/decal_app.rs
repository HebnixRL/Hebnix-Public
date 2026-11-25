use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::config::{self, Config};
use crate::decal_patcher::{DecalPatcherState, SkinInfo};
use crate::messages::AppMsg;
use crate::owned_inventory::OwnedInventory;
use crate::patcher::live_decals;
use crate::swapper::{SwapCategory, SwapperState};

const CATALOGS: &[&str] = &["skins", "bodies"];
pub(crate) const WINDOW_TITLE: &str = "Hebnix Decal Studio";

pub(crate) fn run() -> eframe::Result {
    let patch_command = match std::env::args().nth(1).as_deref() {
        Some("--install-changer") => Some(true),
        Some("--restore-changer") => Some(false),
        _ => None,
    };
    // The main app and this window share config, package backups and the decal library.
    let Some(_instance) = crate::winutil::acquire_single_instance() else {
        if patch_command.is_some() {
            eprintln!("Close Hebnix and Decal Studio before changing the package patch.");
            std::process::exit(1);
        }
        crate::winutil::focus_existing_instance();
        show_launch_message(
            "Another Hebnix or Decal Studio instance holds the shared package lock. Close that instance and reopen Decal Studio.",
        );
        return Ok(());
    };
    let base = config::base_dir();
    let config = Config::load(&base);
    let cooked = PathBuf::from(&config.settings.rl_path)
        .join("TAGame")
        .join("CookedPCConsole");
    let backups = cooked.join("Backups");
    if let Some(enable) = patch_command {
        match live_decals::set_enabled(&cooked, &backups, enable) {
            Ok(true) => println!(
                "Decal changer installed and verified in {}.",
                cooked.display()
            ),
            Ok(false) => println!("Decal changer removed from {}.", cooked.display()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    crate::i18n::init(&config.settings.language, Some(base.join("locales")));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 760.0])
            .with_min_inner_size([600.0, 500.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let decals = DecalPatcherState::new(&base, &config);
            let (tx, rx) = crossbeam_channel::unbounded();
            crate::app::fetch_catalogs(tx.clone(), cc.egui_ctx.clone(), CATALOGS);
            let mut app = DecalApp {
                config,
                decals,
                previews: SwapperState::new(&base),
                browsing: true,
                body: String::new(),
                cars: Vec::new(),
                targets: Vec::new(),
                selected: None,
                live_enabled: None,
                patch_job: None,
                page: 0,
                cooked,
                backups,
                inventory: OwnedInventory::default(),
                tx,
                rx,
                status: "Loading car and decal catalogs…".into(),
            };
            app.start_patch_job(None, &cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(error) = &result {
        show_launch_message(&format!(
            "Decal Studio could not open its window.\n\n{error}"
        ));
    }
    result
}

fn show_launch_message(message: &str) {
    rfd::MessageDialog::new()
        .set_title(WINDOW_TITLE)
        .set_level(rfd::MessageLevel::Error)
        .set_description(message)
        .show();
}

struct DecalApp {
    config: Config,
    decals: DecalPatcherState,
    previews: SwapperState,
    browsing: bool,
    body: String,
    cars: Vec<(String, String)>,
    targets: Vec<(String, SkinInfo)>,
    selected: Option<(i32, String)>,
    live_enabled: Option<bool>,
    patch_job: Option<Receiver<Result<bool, String>>>,
    page: usize,
    cooked: PathBuf,
    backups: PathBuf,
    inventory: OwnedInventory,
    tx: Sender<AppMsg>,
    rx: Receiver<AppMsg>,
    status: String,
}

impl eframe::App for DecalApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for message in self.rx.try_iter() {
            match message {
                AppMsg::CatalogsFetched { result } => {
                    let result = result.and_then(|mut catalogs| {
                        self.previews
                            .set_catalog_categories(&catalogs, &[SwapCategory::Skins])?;
                        let skins = catalogs.remove("skins").ok_or("Missing skin catalog")?;
                        let bodies = catalogs.remove("bodies").ok_or("Missing body catalog")?;
                        self.decals.set_catalogs(skins, bodies)
                    });
                    match result {
                        Ok(()) => {
                            self.cars = self.decals.catalog_cars();
                            self.targets = self.decals.decal_targets_for_body(&self.body);
                            self.status.clear();
                        }
                        Err(error) => self.status = error,
                    }
                }
                AppMsg::ReloadCatalogs => {
                    self.status = "Loading car and decal catalogs…".into();
                    crate::app::fetch_catalogs(self.tx.clone(), ctx.clone(), CATALOGS);
                }
                AppMsg::ItemActionBlocked => {
                    self.status = "Close Rocket League before changing decals.".into();
                }
                AppMsg::Log(message) => self.status = message,
                _ => {}
            }
        }
        if let Some(result) = self
            .patch_job
            .as_ref()
            .and_then(|job| match job.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err("Decal changer worker stopped.".into()))
                }
                Err(crossbeam_channel::TryRecvError::Empty) => None,
            })
        {
            self.patch_job = None;
            match result {
                Ok(enabled) => {
                    if self.live_enabled.is_some() {
                        self.status = if enabled {
                            "Decal changer installed. Start Rocket League to use it.".into()
                        } else {
                            "Decal changer removed from the game packages.".into()
                        };
                    }
                    self.live_enabled = Some(enabled);
                }
                Err(error) => self.status = error,
            }
        }
        if self.decals.processing_target.is_some() || self.patch_job.is_some() {
            if ctx.input(|input| input.viewport().close_requested()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::bottom("decal_status").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.browsing, true, "Cars and decals");
                ui.selectable_value(&mut self.browsing, false, "Custom decals");
                if ui.button("Reload catalogs").clicked() {
                    let _ = self.tx.send(AppMsg::ReloadCatalogs);
                }
            });
            ui.separator();
            if self.browsing {
                self.live_controls(ui);
                self.browse(ui);
                return;
            }
            if self.patch_job.is_some() {
                ui.spinner();
                return;
            }
            if !self.cooked.is_dir() {
                ui.label("Set the Rocket League installation path in Hebnix Settings.");
                return;
            }
            self.decals.render_tab(
                ui,
                &self.cooked,
                &self.backups,
                &self.tx,
                &ctx,
                &mut self.config,
                &mut self.inventory,
            );
        });
    }
}

impl DecalApp {
    fn start_patch_job(&mut self, enable: Option<bool>, ctx: &egui::Context) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cooked = self.cooked.clone();
        let backups = self.backups.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = match enable {
                Some(enable) => live_decals::set_enabled(&cooked, &backups, enable),
                None => live_decals::status(&cooked),
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.patch_job = Some(rx);
    }

    fn live_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let busy = self.patch_job.is_some() || self.decals.processing_target.is_some();
            if ui
                .add_enabled(
                    !busy && self.live_enabled.is_some(),
                    egui::Button::new(if self.live_enabled == Some(true) {
                        "Update decal changer"
                    } else {
                        "Enable decal changer"
                    }),
                )
                .clicked()
            {
                self.start_patch_job(Some(true), ui.ctx());
            }
            if ui
                .add_enabled(
                    !busy && self.live_enabled == Some(true),
                    egui::Button::new("Restore original UPKs"),
                )
                .clicked()
            {
                self.start_patch_job(Some(false), ui.ctx());
            }
            if busy {
                ui.spinner();
            } else if self.live_enabled == Some(true) {
                ui.label("Open Customize Car and select the Decal tab to preview.");
            } else {
                ui.label("Close Rocket League before installing the changer.");
            }
        });
        ui.horizontal_wrapped(|ui| {
            let enabled = self.live_enabled == Some(true) && self.patch_job.is_none();
            let button = egui::Button::new("Set decal");
            if ui
                .add_enabled(enabled && self.selected.is_some(), button)
                .clicked()
                && let Some((id, name)) = &self.selected
            {
                self.status = match live_decals::preview(&self.cooked, Some(*id)) {
                    Ok(()) => format!("Queued garage preview for {name}."),
                    Err(error) => error,
                };
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Clear preview"))
                .clicked()
            {
                self.status = match live_decals::preview(&self.cooked, None) {
                    Ok(()) => "Queued clear preview for the garage.".into(),
                    Err(error) => error,
                };
            }
            if let Some((_, name)) = &self.selected {
                ui.label(name);
            }
        });
    }

    fn browse(&mut self, ui: &mut egui::Ui) {
        let cars = &self.cars;
        let selected = cars.iter().find(|(key, _)| key == &self.body);
        let label = selected.map_or("Select a car", |(_, name)| name.as_str());
        egui::ComboBox::from_label("Car")
            .selected_text(label)
            .show_ui(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (key, name) in cars {
                            if ui.selectable_label(&self.body == key, name).clicked()
                                && &self.body != key
                            {
                                self.body.clone_from(key);
                                self.targets = self.decals.decal_targets_for_body(key);
                                self.selected = None;
                                self.page = 0;
                            }
                        }
                    });
            });
        if self.body.is_empty() {
            return;
        }
        let targets = &self.targets;
        if targets.is_empty() {
            ui.label("No compatible decals in the catalog.");
            return;
        }
        let columns = crate::ui::browser::columns(ui, 150.0);
        let page_size = columns * crate::ui::browser::ROWS;
        crate::ui::browser::pagination(ui, &mut self.page, targets.len(), page_size);
        egui::ScrollArea::vertical().show(ui, |ui| {
            let width = (ui.available_width() - (columns - 1) as f32 * ui.spacing().item_spacing.x)
                / columns as f32;
            let start = self.page * page_size;
            let end = (start + page_size).min(targets.len());
            for row in targets.get(start..end).unwrap_or_default().chunks(columns) {
                ui.horizontal(|ui| {
                    for (_, skin) in row {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 94.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(width);
                                ui.set_min_width(width);
                                if let Ok(id) = skin.id.parse() {
                                    self.previews.show_loadout_preview(
                                        ui,
                                        &self.cooked,
                                        SwapCategory::Skins,
                                        id,
                                    );
                                    let selected = self
                                        .selected
                                        .as_ref()
                                        .is_some_and(|(selected, _)| i64::from(*selected) == id);
                                    if ui
                                        .add(
                                            egui::Button::selectable(selected, &skin.name)
                                                .truncate(),
                                        )
                                        .on_hover_text(&skin.name)
                                        .clicked()
                                        && let Ok(id) = i32::try_from(id)
                                    {
                                        self.selected = Some((id, skin.name.clone()));
                                    }
                                }
                            },
                        );
                    }
                });
            }
        });
    }
}
