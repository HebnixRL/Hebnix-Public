use crate::i18n::{t, t_args};
use crate::messages::AppMsg;
use crate::patcher::menu_map::{
    self, ActiveSwap, Arena, CameraTweak, CarSpot, CarTweak, MenuCamera, MenuTweaks,
};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The BakkesMod plugin that previews camera and car values live in game.
const MENUCAM_URL: &str = "https://github.com/xplodingeggo/menucam";

/// Where MenuCam saves its export, under %APPDATA% on Windows or a Windows
/// user's home inside the game's Wine prefix on Linux.
#[cfg(target_os = "windows")]
const MENUCAM_EXPORT: &str = "bakkesmod/bakkesmod/data/menucam/hebnix.json";
#[cfg(target_os = "linux")]
const MENUCAM_EXPORT: &str = "AppData/Roaming/bakkesmod/bakkesmod/data/menucam/hebnix.json";

/// Width of the plugin card beside the settings.
const PLUGIN_CARD_WIDTH: f32 = 260.0;

/// A downloaded or imported workshop map that can stand in for the stadium.
pub struct CustomMap {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

struct MenuMapScan {
    active: Result<Option<ActiveSwap>, String>,
    arenas: Result<Vec<Arena>, String>,
    cameras: Result<Vec<MenuCamera>, String>,
    car_spots: Result<Vec<CarSpot>, String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Apply,
    Restore,
    /// Restore, then apply the same map again with the current camera values.
    Reapply,
}

#[derive(Default)]
pub struct MenuMapState {
    arenas: Vec<Arena>,
    selected: String,
    /// Picking from workshop maps instead of the game's arenas.
    workshop: bool,
    selected_custom: String,
    filter: String,
    active: Option<ActiveSwap>,
    last_path: Option<String>,
    scan: Option<Receiver<MenuMapScan>>,
    refresh_needed: bool,
    busy: bool,
    state_error: bool,
    status: String,
    cameras: Vec<MenuCamera>,
    camera_error: Option<String>,
    camera_on: bool,
    /// Each camera's own change, by camera name. Untouched cameras have none.
    camera_tweaks: BTreeMap<String, CameraTweak>,
    /// The camera being edited.
    camera_selected: String,
    /// The camera last picked on the Other cameras tab.
    camera_other: String,
    car_spots: Vec<CarSpot>,
    car_on: bool,
    car: CarTweak,
}

impl MenuMapState {
    fn refresh(&mut self, rl_path: &str, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        if self.last_path.as_deref() != Some(rl_path) {
            self.arenas.clear();
            self.active = None;
            self.selected.clear();
            self.status.clear();
        }
        self.last_path = Some(rl_path.to_string());
        self.refresh_needed = false;
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.scan = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = MenuMapScan {
                active: menu_map::active(&cooked),
                arenas: menu_map::installed_arenas(&cooked),
                cameras: menu_map::menu_cameras(&cooked),
                car_spots: menu_map::car_spots(&cooked),
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn ensure_loaded(&mut self, rl_path: &str, ctx: &egui::Context) {
        if self.last_path.as_deref() != Some(rl_path) || self.refresh_needed {
            self.refresh(rl_path, ctx);
        }
    }

    fn poll_scan(&mut self) {
        let Some(scan) = &self.scan else { return };
        let result = match scan.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.scan = None;
                self.state_error = true;
                self.status = t("menu-map-scan-failed");
                return;
            }
        };
        self.scan = None;
        self.apply_scan(result);
    }

    fn apply_scan(&mut self, result: MenuMapScan) {
        self.state_error = false;
        match result.active {
            Ok(active) => self.active = active,
            Err(error) => {
                self.active = None;
                self.state_error = true;
                self.status = error;
            }
        }
        match result.arenas {
            Ok(arenas) => self.arenas = arenas,
            Err(error) => {
                self.arenas.clear();
                self.status = error;
            }
        }
        match result.cameras {
            Ok(cameras) => {
                self.cameras = cameras;
                self.camera_error = None;
            }
            Err(error) => {
                self.cameras.clear();
                self.camera_error = Some(error);
            }
        }
        self.car_spots = result.car_spots.unwrap_or_default();
        if let Some(tweak) = self.active.as_ref().and_then(|a| a.car.clone()) {
            self.car_on = true;
            self.car = tweak;
        }
        let applied = self
            .active
            .as_ref()
            .map(ActiveSwap::camera_tweaks)
            .unwrap_or_default();
        if !applied.is_empty() {
            self.camera_on = true;
            self.camera_tweaks = applied
                .into_iter()
                .map(|tweak| (tweak.camera.clone(), tweak))
                .collect();
        }
        let play = self.play_camera().map(|c| c.name.clone());
        if !self.cameras.iter().any(|c| c.name == self.camera_selected) {
            self.camera_selected = play
                .clone()
                .or_else(|| self.cameras.first().map(|c| c.name.clone()))
                .unwrap_or_default();
        }
        if !self
            .cameras
            .iter()
            .any(|c| c.name == self.camera_other && Some(&c.name) != play.as_ref())
        {
            self.camera_other = self
                .cameras
                .iter()
                .find(|c| Some(&c.name) != play.as_ref())
                .map(|c| c.name.clone())
                .unwrap_or_default();
        }
        if let Some(active) = &self.active {
            if active.label.is_some() {
                self.workshop = true;
                self.selected_custom = active.arena.clone();
            } else {
                self.workshop = false;
                self.selected = active.arena.clone();
            }
        } else if !self
            .arenas
            .iter()
            .any(|arena| arena.package == self.selected)
        {
            self.selected = self
                .arenas
                .first()
                .map(|arena| arena.package.clone())
                .unwrap_or_default();
        }
    }

    fn launch(
        &mut self,
        action: Action,
        custom: Option<&CustomMap>,
        rl_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &egui::Context,
    ) {
        if self.busy
            || self.scan.is_some()
            || self.refresh_needed
            || self.state_error
            || self.last_path.as_deref() != Some(rl_path)
            || (action == Action::Apply) == self.active.is_some()
        {
            return;
        }
        if crate::messages::block_item_action_if_game_running(tx) {
            return;
        }
        self.busy = true;
        self.status = t(if action == Action::Restore {
            "menu-map-restoring"
        } else {
            "menu-map-applying"
        });
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        let arena = match (&self.active, action) {
            (Some(active), Action::Reapply) => active.arena.clone(),
            _ => self.selected.clone(),
        };
        let custom = custom.map(|map| (map.path.clone(), map.id.clone(), map.name.clone()));
        let tweaks = MenuTweaks {
            cameras: if self.camera_on {
                self.camera_tweaks.values().cloned().collect()
            } else {
                Vec::new()
            },
            car: self.car_on.then(|| self.car.clone()),
        };
        let tx = tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let apply = || match custom {
                Some((path, id, name)) => {
                    menu_map::apply_custom(&cooked, &path, &id, &name, tweaks)
                }
                None => menu_map::apply(&cooked, &arena, tweaks),
            };
            let result = match action {
                Action::Restore => menu_map::restore(&cooked),
                Action::Apply => apply(),
                Action::Reapply => menu_map::restore(&cooked).and_then(|_| {
                    apply().map_err(|e| format!("{e} The original menu map is back."))
                }),
            };
            let _ = tx.send(AppMsg::MenuMapDone(result));
            ctx.request_repaint();
        });
    }

    pub fn finish(&mut self, result: Result<String, String>) -> String {
        self.busy = false;
        self.refresh_needed = true;
        let message = match result {
            Ok(message) => message,
            Err(error) => error,
        };
        self.status = message.clone();
        format!("[Maps] {message}")
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        rl_path: &str,
        custom: &[CustomMap],
        tx: &Sender<AppMsg>,
    ) {
        let spacing = ui.spacing().item_spacing.x * 2.0;
        let main_width = ui.available_width() - PLUGIN_CARD_WIDTH - spacing;
        // Narrow windows get the card on top instead of beside the settings.
        if main_width < 420.0 {
            self.render_plugin_card(ui);
            ui.add_space(12.0);
            self.render_settings(ui, rl_path, custom, tx);
            return;
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(main_width);
                self.render_settings(ui, rl_path, custom, tx);
            });
            ui.vertical(|ui| {
                ui.set_width(PLUGIN_CARD_WIDTH);
                self.render_plugin_card(ui);
            });
        });
    }

    fn render_settings(
        &mut self,
        ui: &mut egui::Ui,
        rl_path: &str,
        custom: &[CustomMap],
        tx: &Sender<AppMsg>,
    ) {
        self.ensure_loaded(rl_path, ui.ctx());
        self.poll_scan();
        let loading = self.scan.is_some();
        crate::theme::page_heading(ui, t("menu-map-title"));
        ui.label(t("menu-map-description"));
        ui.small(t("menu-map-close-game"));
        ui.add_space(12.0);
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("menu-map-loading"));
            });
        }
        let ready = !self.busy
            && !loading
            && !self.refresh_needed
            && !self.state_error
            && self.last_path.as_deref() == Some(rl_path);
        let editable = ready && self.active.is_none();
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.workshop, false, t("menu-map-source-arenas"));
                ui.selectable_value(&mut self.workshop, true, t("menu-map-source-workshop"));
            });
        });
        if self.workshop && !custom.iter().any(|map| map.id == self.selected_custom) {
            self.selected_custom = custom.first().map(|map| map.id.clone()).unwrap_or_default();
        }
        let chosen_custom = custom.iter().find(|map| map.id == self.selected_custom);
        ui.add_enabled_ui(editable, |ui| {
            if self.workshop {
                egui::ComboBox::from_id_salt("menu_map_workshop")
                    .selected_text(match chosen_custom {
                        Some(map) => map.name.clone(),
                        None => t("menu-map-choose"),
                    })
                    .width(360.0)
                    .height(320.0)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text(t("render-search-maps")),
                        );
                        ui.separator();
                        let filter = self.filter.trim().to_lowercase();
                        for map in custom {
                            if map.name.to_lowercase().contains(&filter)
                                && ui
                                    .selectable_value(
                                        &mut self.selected_custom,
                                        map.id.clone(),
                                        &map.name,
                                    )
                                    .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
                return;
            }
            egui::ComboBox::from_id_salt("menu_map_arena")
                .selected_text(if self.selected.is_empty() {
                    t("menu-map-choose")
                } else {
                    crate::discord_presence::display_map_name(&self.selected)
                })
                .width(360.0)
                .height(320.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text(t("render-search-maps")),
                    );
                    ui.separator();
                    let filter = self.filter.trim().to_lowercase();
                    for arena in &self.arenas {
                        if arena.label.to_lowercase().contains(&filter)
                            || arena.package.to_lowercase().contains(&filter)
                        {
                            if ui
                                .selectable_value(
                                    &mut self.selected,
                                    arena.package.clone(),
                                    &arena.label,
                                )
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    }
                });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    editable
                        && if self.workshop {
                            chosen_custom.is_some()
                        } else {
                            !self.selected.is_empty()
                        },
                    egui::Button::new(t("menu-map-apply")),
                )
                .clicked()
            {
                let custom = if self.workshop { chosen_custom } else { None };
                self.launch(Action::Apply, custom, rl_path, tx, ui.ctx());
            }
            if ui
                .add_enabled(
                    ready && self.active.is_some(),
                    egui::Button::new(t("menu-map-restore")),
                )
                .clicked()
            {
                self.launch(Action::Restore, None, rl_path, tx, ui.ctx());
            }
            if self.busy {
                ui.spinner();
            }
        });
        if let Some(active) = &self.active {
            ui.label(t_args(
                "menu-map-active",
                &[("map", active.display_name().into())],
            ));
            ui.small(t("menu-map-restore-first"));
        }
        if self.workshop {
            if custom.is_empty() {
                ui.label(t("menu-map-workshop-empty"));
            } else {
                ui.small(t("menu-map-workshop-note"));
            }
        } else if !loading && self.arenas.is_empty() {
            ui.label(t("menu-map-empty"));
        }
        ui.add_space(12.0);
        self.render_car(ui, ready);
        self.render_camera(ui, ready);
        self.render_reapply(ui, ready, custom, rl_path, tx);
        if !self.status.is_empty() {
            ui.add_space(8.0);
            ui.label(&self.status);
        }
        if ui
            .add_enabled(
                !self.busy && !loading,
                egui::Button::new(t("render-scan-again")),
            )
            .clicked()
        {
            self.status.clear();
            self.refresh(rl_path, ui.ctx());
        }
    }

    /// The camera used after pressing Play, which gets its own tab.
    fn play_camera(&self) -> Option<&MenuCamera> {
        self.cameras.iter().find(|c| c.tag == "MainMenuPlay")
    }

    /// Experimental: move the menu level's fixed cameras. Each camera keeps
    /// its own values; the Play menu camera has a tab, the rest share one.
    fn render_camera(&mut self, ui: &mut egui::Ui, ready: bool) {
        ui.collapsing(t("menu-map-camera-title"), |ui| {
            ui.small(t("menu-map-camera-description"));
            if let Some(error) = &self.camera_error {
                ui.label(error);
                return;
            }
            if self.cameras.is_empty() {
                ui.label(t("menu-map-camera-none"));
                return;
            }
            ui.checkbox(&mut self.camera_on, t("menu-map-camera-enable"));
            ui.add_enabled_ui(self.camera_on && ready, |ui| {
                let label = |camera: &MenuCamera| match camera_purpose(&camera.tag) {
                    Some(purpose) => format!("{purpose} ({})", camera.name),
                    None if camera.name == PRESS_ANY_BUTTON_CAMERA => {
                        format!("PressAnyButtonToStart ({})", camera.name)
                    }
                    None if camera.tag.is_empty() || camera.tag == "CameraActor" => {
                        camera.name.clone()
                    }
                    None => format!("{} ({})", camera.tag, camera.name),
                };
                let changed = |name: &str| {
                    self.camera_tweaks
                        .get(name)
                        .is_some_and(|tweak| !tweak.is_noop())
                };
                let play = self.play_camera().map(|c| c.name.clone());
                let on_play = play.as_ref() == Some(&self.camera_selected);
                ui.horizontal(|ui| {
                    if let Some(play) = &play {
                        let title = t("menu-map-camera-play");
                        let title = if changed(play) {
                            format!("{title} •")
                        } else {
                            title
                        };
                        if ui.selectable_label(on_play, title).clicked() {
                            self.camera_selected = play.clone();
                        }
                    }
                    let title = t("menu-map-camera-other");
                    let other_changed = self
                        .cameras
                        .iter()
                        .any(|c| Some(&c.name) != play.as_ref() && changed(&c.name));
                    let title = if other_changed {
                        format!("{title} •")
                    } else {
                        title
                    };
                    if ui.selectable_label(!on_play, title).clicked() {
                        self.camera_selected = self.camera_other.clone();
                    }
                });
                if !on_play {
                    let current = self
                        .cameras
                        .iter()
                        .find(|c| c.name == self.camera_selected)
                        .map(label)
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("menu_map_camera")
                        .selected_text(current)
                        .width(360.0)
                        .show_ui(ui, |ui| {
                            for camera in &self.cameras {
                                if Some(&camera.name) == play.as_ref() {
                                    continue;
                                }
                                let text = if changed(&camera.name) {
                                    format!("{} •", label(camera))
                                } else {
                                    label(camera)
                                };
                                if ui
                                    .selectable_label(camera.name == self.camera_selected, text)
                                    .clicked()
                                {
                                    self.camera_selected = camera.name.clone();
                                    self.camera_other = camera.name.clone();
                                }
                            }
                        });
                }
                let Some(base) = self.cameras.iter().find(|c| c.name == self.camera_selected)
                else {
                    return;
                };
                ui.small(format!(
                    "{}: X {:.0}  Y {:.0}  Z {:.0}  ·  pitch {:.1}°  yaw {:.1}°{}",
                    t("menu-map-camera-original"),
                    base.location[0],
                    base.location[1],
                    base.location[2],
                    base.rotation[0],
                    base.rotation[1].rem_euclid(360.0),
                    base.fov
                        .map(|fov| format!("  ·  FOV {fov:.0}"))
                        .unwrap_or_default(),
                ));
                let original_fov = base.fov;
                let tweak = self
                    .camera_tweaks
                    .entry(self.camera_selected.clone())
                    .or_insert_with(|| CameraTweak {
                        camera: self.camera_selected.clone(),
                        ..CameraTweak::default()
                    });
                egui::Grid::new("menu_map_camera_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (axis, value) in ["X", "Y", "Z"].iter().zip(&mut tweak.offset) {
                            ui.label(format!("{} {axis}", t("menu-map-camera-move")));
                            ui.add(egui::DragValue::new(value).speed(5.0).suffix(" uu"));
                            ui.end_row();
                        }
                        ui.label(t("menu-map-camera-pitch"));
                        ui.add(
                            egui::DragValue::new(&mut tweak.pitch)
                                .speed(0.5)
                                .range(-89.0..=89.0)
                                .suffix("°"),
                        );
                        ui.end_row();
                        ui.label(t("menu-map-camera-yaw"));
                        ui.add(
                            egui::DragValue::new(&mut tweak.yaw)
                                .speed(0.5)
                                .range(-180.0..=180.0)
                                .suffix("°"),
                        );
                        ui.end_row();
                        if let Some(original) = original_fov {
                            let mut custom_fov = tweak.fov.is_some();
                            ui.checkbox(&mut custom_fov, t("menu-map-camera-fov"));
                            if custom_fov {
                                let fov = tweak.fov.get_or_insert(original);
                                ui.add(egui::DragValue::new(fov).speed(0.5).range(10.0..=170.0));
                            } else {
                                tweak.fov = None;
                            }
                            ui.end_row();
                        }
                    });
                if ui.button(t("menu-map-camera-reset")).clicked() {
                    *tweak = CameraTweak {
                        camera: self.camera_selected.clone(),
                        ..CameraTweak::default()
                    };
                }
                ui.small(t("menu-map-camera-each"));
            });
        });
    }

    /// Experimental: move the home screen's car spots. The home camera orbits
    /// the player's car, so it moves along with it.
    fn render_car(&mut self, ui: &mut egui::Ui, ready: bool) {
        ui.collapsing(t("menu-map-car-title"), |ui| {
            ui.small(t("menu-map-car-description"));
            let Some(player) = self.car_spots.first() else {
                ui.label(t("menu-map-car-none"));
                return;
            };
            ui.checkbox(&mut self.car_on, t("menu-map-car-enable"));
            ui.add_enabled_ui(self.car_on && ready, |ui| {
                ui.small(format!(
                    "{}: X {:.0}  Y {:.0}  Z {:.0}  ·  facing {:.1}°",
                    t("menu-map-camera-original"),
                    player.location[0],
                    player.location[1],
                    player.location[2],
                    player.yaw.rem_euclid(360.0),
                ));
                egui::Grid::new("menu_map_car_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (axis, value) in ["X", "Y", "Z"].iter().zip(&mut self.car.offset) {
                            ui.label(format!("{} {axis}", t("menu-map-camera-move")));
                            ui.add(egui::DragValue::new(value).speed(5.0).suffix(" uu"));
                            ui.end_row();
                        }
                        ui.label(t("menu-map-camera-yaw"));
                        ui.add(
                            egui::DragValue::new(&mut self.car.yaw)
                                .speed(0.5)
                                .range(-180.0..=180.0)
                                .suffix("°"),
                        );
                        ui.end_row();
                    });
                if ui.button(t("menu-map-camera-reset")).clicked() {
                    self.car = CarTweak::default();
                }
            });
        });
    }

    /// Points to the MenuCam BakkesMod plugin, which moves the menu camera and
    /// cars live so values can be found without restarting the game.
    fn render_plugin_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong(t("menu-map-plugin-title"));
            ui.add_space(4.0);
            ui.label(t("menu-map-plugin-description"));
            ui.add_space(8.0);
            if ui
                .button(t("menu-map-plugin-button"))
                .on_hover_text(MENUCAM_URL)
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(MENUCAM_URL));
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(t("menu-map-plugin-import")))
                .on_hover_text(t("menu-map-plugin-import-hint"))
                .clicked()
            {
                self.import_menucam();
            }
        });
    }

    /// Fills Car position and Camera from MenuCam's export. Looks in the
    /// game's prefixes first, then asks for the file.
    fn import_menucam(&mut self) {
        let path = find_menucam_export().or_else(|| {
            crate::winutil::parent_file_dialog(
                rfd::FileDialog::new()
                    .set_title(t("menu-map-import-pick"))
                    .add_filter("MenuCam export", &["json"]),
            )
            .pick_file()
        });
        let Some(path) = path else { return };
        let export = match menu_map::read_menucam_export(&path) {
            Ok(export) => export,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let mut imported = false;
        if let Some(car) = export.car {
            self.car_on = true;
            self.car = car;
            imported = true;
        }
        for camera in export.cameras {
            if self.cameras.iter().any(|c| c.name == camera.camera) {
                self.camera_on = true;
                self.camera_selected = camera.camera.clone();
                self.camera_tweaks.insert(camera.camera.clone(), camera);
                imported = true;
            }
        }
        self.status = if imported {
            t_args(
                "menu-map-import-done",
                &[("path", path.display().to_string().into())],
            )
        } else {
            t("menu-map-import-empty")
        };
    }

    fn render_reapply(
        &mut self,
        ui: &mut egui::Ui,
        ready: bool,
        custom: &[CustomMap],
        rl_path: &str,
        tx: &Sender<AppMsg>,
    ) {
        let Some(active) = &self.active else { return };
        let active_custom = active
            .label
            .is_some()
            .then(|| custom.iter().find(|map| map.id == active.arena))
            .flatten();
        let can_reapply = ready && (active.label.is_none() || active_custom.is_some());
        if ui
            .add_enabled(can_reapply, egui::Button::new(t("menu-map-camera-reapply")))
            .on_hover_text(t("menu-map-camera-reapply-hint"))
            .clicked()
        {
            self.launch(Action::Reapply, active_custom, rl_path, tx, ui.ctx());
        }
    }
}

/// The camera on the press any button screen. The menu level gives it no tag.
const PRESS_ANY_BUTTON_CAMERA: &str = "CameraActor_8";

/// What a tagged menu camera shows, where it has been checked in game.
fn camera_purpose(tag: &str) -> Option<String> {
    match tag {
        "MainMenuPlay" => Some(t("menu-map-camera-play")),
        _ => None,
    }
}

/// MenuCam's export in BakkesMod's data folder, if it has made one.
#[cfg(target_os = "windows")]
fn find_menucam_export() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("APPDATA")?).join(MENUCAM_EXPORT);
    path.is_file().then_some(path)
}

/// The newest MenuCam export in any Rocket League prefix.
#[cfg(target_os = "linux")]
fn find_menucam_export() -> Option<PathBuf> {
    hebnix_sdk::process::wine_prefix::rocket_league_prefixes()
        .iter()
        .filter_map(|prefix| std::fs::read_dir(prefix.join("drive_c/users")).ok())
        .flatten()
        .flatten()
        .map(|user| user.path().join(MENUCAM_EXPORT))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}
