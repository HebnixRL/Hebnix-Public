use super::{
    HebnixApp, HebnixSettingsTab, ImageState, PluginsSubTab, SpooferSubTab, Tab, install_zip,
    spoofer, t, winutil,
};
use crate::modern_ui::{ContentPage, ManageAction, PresetAction};
use crate::ui::browser::image_url;
use eframe::egui;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};

#[derive(Default)]
pub(super) struct PluginSettingsState {
    selected: Option<String>,
    pending: VecDeque<(String, Value)>,
    expanded: HashSet<String>,
    inputs: HashSet<String>,
}

pub(super) const STUDIO_APPEARANCE: &[(super::ItemsMode, &str)] = &[
    (super::ItemsMode::GameColours, "nav-game-colours"),
    (super::ItemsMode::BallAppearance, "nav-ball-shape"),
    (super::ItemsMode::WheelAlignment, "app-wheel-alignment"),
    (super::ItemsMode::SpeedPatch, "app-speed-patch"),
    (super::ItemsMode::GoalExplosion, "goal-explosion-volume-title"),
    (
        super::ItemsMode::MenuTextures,
        "nav-menu-textures-experimental",
    ),
];

pub(super) const STUDIO_CUSTOM: &[(Option<super::PatcherSubTab>, &str)] = &[
    (Some(super::PatcherSubTab::Car), "nav-car-models"),
    (Some(super::PatcherSubTab::Decal), "nav-decals"),
    (Some(super::PatcherSubTab::Anthem), "studio-player-anthems"),
    (Some(super::PatcherSubTab::Ball), "nav-ball-textures"),
    (Some(super::PatcherSubTab::BoostMeter), "nav-boost-meter"),
    (Some(super::PatcherSubTab::Thumbnail), "nav-item-thumbnails"),
    (None, "nav-item-names"),
];

impl HebnixApp {
    pub(super) fn maintenance_selected(&self) -> bool {
        let options = &self.maintenance;
        options.application || options.settings || options.extensions || options.assets || options.selected_patches().next().is_some() || options.certificate
    }

    fn web_maintenance(&self) -> Value {
        let options = &self.maintenance;
        let mut paths = crate::maintenance::removal_paths(&self.base_dir, options);
        if options.application && let Ok(exe) = std::env::current_exe() {
            paths.push(exe);
        }
        json!({
            "page": "maintenance", "error": options.error, "confirm": options.confirm,
            "base": self.base_dir.to_string_lossy(), "selected": self.maintenance_selected(),
            "paths": paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>(),
            "options": [
                ("application", t("maintenance-app"), options.application),
                ("settings", t("maintenance-settings"), options.settings),
                ("extensions", t("maintenance-extensions"), options.extensions),
                ("assets", t("maintenance-assets"), options.assets),
                ("restore_swaps", t("maintenance-swaps"), options.patch_groups[0]),
                ("restore_game_mods", t("maintenance-game-mods"), options.patch_groups[1]),
                ("restore_workshop", t("maintenance-workshop"), options.patch_groups[2]),
                ("restore_custom", t("maintenance-custom-patches"), options.patch_groups[3]),
                ("certificate", t("maintenance-certificate"), options.certificate),
            ],
            "labels": {
                "back": t("maintenance-about"), "note": t("maintenance-note"), "files": t("maintenance-files"),
                "remove": t("maintenance-remove"), "confirm": t("maintenance-confirm"),
                "confirmButton": t("maintenance-confirm-button"), "cancel": t("maintenance-cancel"),
            }
        })
    }

    pub(super) fn has_modern_manager(&self) -> bool {
        match self.tab {
            Tab::About => self.maintenance.tab,
            Tab::Plugins => true,
            Tab::Spoofer => true,
            Tab::Settings => true,
            Tab::Freeplay => true,
            Tab::GameMods | Tab::LiveEditor => true,
            Tab::Replays => self.replays_subtab == super::ReplaysSubTab::ReplayTimeline,
            Tab::Patcher => matches!(
                self.items_mode,
                super::ItemsMode::SpeedPatch | super::ItemsMode::GoalExplosion | super::ItemsMode::Manage | super::ItemsMode::WheelAlignment
                    | super::ItemsMode::GameColours | super::ItemsMode::BallAppearance | super::ItemsMode::MenuTextures | super::ItemsMode::History
            ) || self.on_studio_asset_page(),
            Tab::Workshop => matches!(self.workshop.view, crate::ui::workshop::WorkshopView::InGameMenu | crate::ui::workshop::WorkshopView::Import | crate::ui::workshop::WorkshopView::Multiplayer | crate::ui::workshop::WorkshopView::MenuMap),
            Tab::Items => matches!(
                self.items_subtab,
                super::ItemsSubTab::Loadouts | super::ItemsSubTab::PresetCreator | super::ItemsSubTab::Presets
                    | super::ItemsSubTab::Colours
            ),
            _ => false,
        }
    }

    pub(super) fn render_modern_manager(&mut self, ui: &mut egui::Ui) -> bool {
        if self.egui_ported_page() && !self.egui_manage_supported() {
            return false;
        }
        if !self.has_modern_manager() {
            return false;
        }
        self.config.patcher.decal_only_owned = self.config.settings.owned_items_only;
        let mut data = match self.tab {
            Tab::About => self.web_maintenance(),
            Tab::Spoofer if self.spoofer_subtab == SpooferSubTab::Titles => self.web_titles_data(ui.ctx()),
            Tab::Spoofer => self.web_player_data(),
            Tab::Settings if self.hebnix_settings_tab == HebnixSettingsTab::Discord => {
                self.web_discord_settings()
            }
            Tab::Settings if self.hebnix_settings_tab == HebnixSettingsTab::System => {
                self.web_system_settings()
            }
            Tab::Settings if self.hebnix_settings_tab == HebnixSettingsTab::Interface => {
                self.web_appearance()
            }
            Tab::Settings if self.hebnix_settings_tab == HebnixSettingsTab::Directories => {
                self.web_files()
            }
            Tab::Settings => self.web_action_button_data(),
            Tab::Patcher if self.items_mode == super::ItemsMode::Manage => self.web_active_patches(ui.ctx()),
            Tab::Patcher if self.items_mode == super::ItemsMode::History => {
                let cooked = self.swapper_cooked_path();
                self.upk_history.web_snapshot(&cooked, &self.swapper)
            }
            Tab::Patcher if self.on_studio_asset_page() => self.web_studio_asset(ui.ctx()),
            Tab::Patcher => {
                let mut model = if self.items_mode == super::ItemsMode::MenuTextures {
                    self.web_menu_textures(ui.ctx())
                } else if matches!(self.items_mode, super::ItemsMode::GameColours | super::ItemsMode::BallAppearance) {
                    let backups = self.swapper_cooked_path().join("Backups");
                    self.colours.web_snapshot(self.items_mode == super::ItemsMode::BallAppearance, &backups)
                } else if self.items_mode == super::ItemsMode::WheelAlignment {
                    let cooked = self.swapper_cooked_path();
                    let mut model = self.wheel_alignment.web_snapshot(&cooked, ui.ctx());
                model["catalogLoading"] = json!(self.catalogs_loading);
                    model["catalogError"] = json!(self.catalogs_error);
                    model
                } else if self.items_mode == super::ItemsMode::GoalExplosion {
                    let backups = self.swapper_cooked_path().join("Backups");
                    json!({ "page": "goal-explosion", "panels": [
                        self.goal_explosion_volume.web_snapshot(ui.ctx(), &backups),
                    ] })
                } else {
                    self.speed_patch
                        .web_snapshot(&self.config.speed_patch, ui.ctx(), &self.tx)
                };
                if self.items_mode == super::ItemsMode::GameColours {
                    model["liveManaged"] = json!(crate::patcher::live_cosmetics::managed());
                }
                let mut pages: Vec<_> = STUDIO_APPEARANCE
                    .iter()
                    .enumerate()
                    .map(|(index, (mode, label))| (index, t(label), *mode == self.items_mode))
                    .collect();
                pages.sort_by_cached_key(|(_, label, _)| label.to_lowercase());
                model["pages"] = json!(pages);
                model
            }
            Tab::Workshop if self.workshop.view == crate::ui::workshop::WorkshopView::Import => self.workshop.add_maps_web_snapshot(),
            Tab::Workshop if self.workshop.view == crate::ui::workshop::WorkshopView::Multiplayer => self.workshop.multiplayer_web_snapshot(),
            Tab::Workshop if self.workshop.view == crate::ui::workshop::WorkshopView::MenuMap => self.workshop.menu_map_web_snapshot(&self.config.settings.rl_path, ui.ctx()),
            Tab::Workshop => {
                let cooked = self.swapper_cooked_path();
                self.workshop.menu_web_snapshot(&cooked, ui.ctx())
            }
            Tab::LiveEditor => self.web_live_editor(self.live_editor_page),
            Tab::Freeplay if self.freeplay_page == super::FreeplayPage::Practice => self
                .freeplay
                .web_snapshot(&self.config.settings.rl_path, ui.ctx()),
            Tab::GameMods | Tab::Freeplay => self.web_game_mods(ui.ctx()),
            Tab::Replays => {
                let folder = self.configured_replay_folder();
                self.replay_timeline.web_snapshot(folder, &self.ws_stats, ui.ctx())
            }
            Tab::Items if self.items_subtab == super::ItemsSubTab::Colours => self.web_live_editor(super::LiveEditorPage::Colours),
            Tab::Items if self.items_subtab == super::ItemsSubTab::Presets => {
                let cooked = self.swapper_cooked_path();
                self.process_preset_operations(&cooked, &cooked.join("Backups"), ui.ctx());
                self.web_presets()
            }
            Tab::Items => {
                let cooked = self.swapper_cooked_path();
                let selected_save = self.owned_inventory.selected_save().map(std::path::Path::to_path_buf);
                self.loadouts.sync_global_save(selected_save.as_deref(), false);
                let mut model = self.loadouts.web_snapshot(
                    &mut self.swapper,
                    self.catalogs_loading,
                    &cooked,
                    ui.ctx(),
                );
                if self.items_subtab == super::ItemsSubTab::PresetCreator {
                    model["page"] = json!("preset-creator");
                    model["creator"] = self.loadouts.creator_snapshot(&mut self.swapper, &cooked, ui.ctx());
                }
                model["catalogLoading"] = json!(self.catalogs_loading);
                model["catalogError"] = json!(self.catalogs_error);
                model
            }
            Tab::Plugins if self.plugins_subtab == PluginsSubTab::Settings => self.web_plugin_settings(ui.ctx()),
            _ => self.modern_plugins_data(ui.ctx()),
        };
        data["globalOwnedOnly"] = json!(self.config.settings.owned_items_only);
        if self.egui_shell() {
            return self.show_manage_egui(ui, &data);
        }
        let Some(navigation) = self.modern_navigation.as_mut() else {
            return false;
        };
        match navigation.show_content(ui, ContentPage::Manage, &data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not display management page");
                navigation.keep_content(None);
                false
            }
        }
    }

    fn on_studio_asset_page(&self) -> bool {
        self.tab == Tab::Patcher && (self.items_mode == super::ItemsMode::ItemNames
            || self.items_mode == super::ItemsMode::Patching
                && matches!(self.patcher_subtab, super::PatcherSubTab::Ball | super::PatcherSubTab::BoostMeter | super::PatcherSubTab::Car
                    | super::PatcherSubTab::Anthem | super::PatcherSubTab::Thumbnail | super::PatcherSubTab::Decal))
    }

    fn web_studio_asset(&mut self, ctx: &egui::Context) -> Value {
        let mut model = if self.items_mode == super::ItemsMode::ItemNames || self.patcher_subtab == super::PatcherSubTab::Thumbnail {
            let cooked = self.swapper_cooked_path();
            self.owned_inventory.poll(ctx);
            let mut model = if self.items_mode == super::ItemsMode::ItemNames {
                self.swapper.web_item_names(&cooked, &self.owned_inventory.ids, &self.swapper_preview_ids, ctx)
            } else {
                self.swapper.web_item_thumbnails(&cooked, &self.owned_inventory.ids, &self.swapper_preview_ids, ctx)
            };
            model["inventory"] = self.owned_inventory.web_snapshot();
            model["closed"] = json!(!self.last_rl_open && !self.first_status);
            model
        } else if self.patcher_subtab == super::PatcherSubTab::Ball {
            self.patcher_ball.web_snapshot(ctx, &mut self.config, &self.tx, &self.swapper_preview_ids)
        } else if self.patcher_subtab == super::PatcherSubTab::Car {
            self.patcher_car.web_snapshot(ctx, &mut self.config, &self.tx, &self.swapper_preview_ids)
        } else if self.patcher_subtab == super::PatcherSubTab::Decal {
            self.owned_inventory.poll(ctx);
            let mut model = self.patcher_decal.web_snapshot(&mut self.config, &self.tx, &self.owned_inventory.ids, &self.swapper_preview_ids, ctx);
            model["inventory"] = self.owned_inventory.web_snapshot();
            model
        } else if self.patcher_subtab == super::PatcherSubTab::Anthem {
            let cooked = self.swapper_cooked_path();
            self.owned_inventory.poll(ctx);
            let mut model = self.patcher_anthem.web_snapshot(&cooked, &self.tx, &mut self.config, &self.owned_inventory.ids, &self.swapper_preview_ids, ctx);
            model["inventory"] = self.owned_inventory.web_snapshot();
            model["closed"] = json!(!self.last_rl_open && !self.first_status);
            model
        } else {
            self.patcher_boost.web_snapshot(ctx, &mut self.config, &self.tx, &self.swapper_preview_ids)
        };
        self.studio_custom_navigation(&mut model);
        model
    }

    fn studio_custom_navigation(&self, model: &mut Value) {
        let mut pages: Vec<_> = STUDIO_CUSTOM.iter().enumerate().map(|(index, (tab, label))| {
            let active = if let Some(tab) = tab { self.items_mode == super::ItemsMode::Patching && self.patcher_subtab == *tab }
                else { self.items_mode == super::ItemsMode::ItemNames };
            (index, t(label), active)
        }).collect();
        pages.sort_by_cached_key(|(_, label, _)| label.to_lowercase());
        model["pages"] = json!(pages);
        model["customPages"] = json!(true);
    }

    fn web_colour_action(&mut self, action: crate::patcher::colours::ColourAction, ctx: &egui::Context) {
        let cooked = self.swapper_cooked_path();
        if crate::messages::block_item_action_if_game_running(&self.tx) {
            self.console.write("[Colours] Close Rocket League before changing game files.");
        } else if !spoofer::is_admin() && crate::patcher::colours::needs_admin(&cooked) {
            self.colour_admin_prompt_open = true;
        } else {
            self.colours.begin(action, &cooked, &cooked.join("Backups"), &self.tx, ctx);
        }
    }

    fn web_plugin_settings(&mut self, ctx: &egui::Context) -> Value {
        let available: Vec<_> = self.plugin_mgr.plugins_by_name().filter(|plugin| plugin.enabled && plugin.has_settings())
            .map(|plugin| (plugin.slug.clone(), plugin.display_name().to_owned())).collect();
        if let Some((id, _)) = available.first() {
            if !available.iter().any(|(id, _)| self.selected_settings_plugin.as_ref() == Some(id)) {
                self.selected_settings_plugin = Some(id.clone());
            }
        } else {
            self.selected_settings_plugin = None;
        }
        if self.web_plugin_settings.selected != self.selected_settings_plugin {
            self.web_plugin_settings = PluginSettingsState {
                selected: self.selected_settings_plugin.clone(), ..Default::default()
            };
        }
        let mut widgets = Vec::new();
        let mut error = None;
        let mut title = String::new();
        if let Some((id, name)) = available.iter().find(|(id, _)| Some(id) == self.selected_settings_plugin.as_ref()) {
            title = super::t_args("plugin-settings-display-name-configuration", &[("display_name", name.as_str().into())]);
            let action = self.web_plugin_settings.pending.pop_front();
            if action.is_some() { ctx.request_repaint(); }
            match self.plugin_mgr.web_settings(id, ctx, action, self.web_plugin_settings.expanded.clone()) {
                Ok(frame) => { widgets = frame.widgets; self.web_plugin_settings.inputs = frame.inputs; }
                Err(message) => {
                    self.web_plugin_settings.inputs.clear();
                    error = Some(message);
                }
            }
        }
        json!({
            "page": "plugin-settings", "plugins": available, "selected": self.selected_settings_plugin,
            "widgets": widgets, "title": title, "error": error, "empty": t("plugin-settings-none"), "go": t("plugin-settings-go"),
        })
    }

    /// a Live Editor page; Items > Colours shows the Live Editor colours too
    fn web_live_editor(&mut self, page: super::LiveEditorPage) -> Value {
        let backups = self.swapper_cooked_path().join("Backups");
        let mut model = self.colours.web_snapshot(false, &backups);
        model["page"] = json!(match page {
            super::LiveEditorPage::Colours => "live-colours",
            super::LiveEditorPage::Wheels => "live-wheels",
            super::LiveEditorPage::GoalExplosion => "live-goal-explosion",
        });
        model["live"] = crate::patcher::live_cosmetics::snapshot();
        model
    }

    fn on_live_colours_page(&self) -> bool {
        self.tab == Tab::LiveEditor || self.tab == Tab::Items && self.items_subtab == super::ItemsSubTab::Colours
    }

    /// a loadout colour outside the stock palette: add it to the custom car colours,
    /// pick the cell it lands in, and patch the palette
    fn web_loadout_custom_colour(&mut self, team: usize, primary: bool, rgb: [u8; 3], ctx: &egui::Context) {
        if self.colours.is_busy() {
            self.loadouts.set_message("Colours are still being applied. Try again in a moment.".into());
            return;
        }
        if crate::messages::block_item_action_if_game_running(&self.tx) {
            self.loadouts.set_message("Close Rocket League before adding a custom colour.".into());
            return;
        }
        let cooked = self.swapper_cooked_path();
        self.colours.add_custom_colours(&cooked.join("Backups"), [rgb]);
        let palette = if primary { team } else { 2 };
        match crate::patcher::colours::custom_colour_cells(&cooked, &self.colours.settings.custom_colours, &[(palette, rgb)]) {
            Ok(cells) => {
                self.loadouts.use_palette_colour(team, primary, cells[0], rgb);
                self.web_colour_action(crate::patcher::colours::ColourAction::Apply, ctx);
            }
            Err(error) => self.loadouts.set_message(error),
        }
    }

    fn web_active_patches(&mut self, ctx: &egui::Context) -> Value {
        self.patcher_ball.poll_ops(&self.tx, ctx, &mut self.config);
        self.patcher_boost.poll_ops(&self.tx, &mut self.config);
        self.patcher_decal.handle_op_result(&self.tx, &mut self.config);
        let mut rows = Vec::new();
        let row = |kind, target, name: &String, description: String, images: Vec<String>| {
            json!({ "kind": kind, "target": target, "name": name, "description": description, "images": images })
        };
        if let Some(name) = &self.patcher_ball.active_ball {
            let images = self.patcher_ball.balls.iter().find(|item| &item.name == name)
                .and_then(|item| item.image_bytes.as_deref()).and_then(image_url).into_iter().collect();
            rows.push(row("ball", "", name, t("app-replaced-the-default-ball"), images));
        }
        if let Some(name) = &self.patcher_boost.active_boost {
            let images = self.patcher_boost.boosts.iter().find(|item| &item.name == name).into_iter()
                .flat_map(|item| [&item.background_image, &item.fill_image, &item.glow_image, &item.tint_image])
                .filter_map(|image| image.as_deref().and_then(image_url)).collect();
            rows.push(row("boost", "", name, t("app-replaced-the-default-boost-meter"), images));
        }
        for (target, name) in &self.patcher_decal.active_decals {
            let images = self.patcher_decal.decals.iter().find(|item| &item.name == name)
                .and_then(|item| item.preview_image.as_deref()).and_then(image_url).into_iter().collect();
            let description = super::t_args("app-replaced-target-label", &[("target_label", self.patcher_decal.target_display_name(target).into())]);
            rows.push(row("decal", target, name, description, images));
        }
        for (target, name) in &self.patcher_car.active_cars {
            let images = self.patcher_car.active_preview_bytes(target, name).and_then(image_url).into_iter().collect();
            let description = super::t_args("app-replaced-target-for-its-bodyid-car", &[("target", target.clone().into())]);
            rows.push(row("car", target, name, description, images));
        }
        for (target, name) in &self.patcher_anthem.active_anthems {
            rows.push(row("anthem", target, name, "Patched anthem audio".into(), Vec::new()));
        }
        rows.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()).then_with(|| left["description"].as_str().cmp(&right["description"].as_str())));
        json!({
            "page": "studio-active", "count": super::t_n("items-active-changes", rows.len() as i64),
            "rows": rows, "restore": t("app-restore"), "all": t("app-restore-all"),
            "empty": t("app-no-patches-or-swaps-are-active"), "missing": t("items-browser-preview-missing"),
        })
    }

    fn web_restore_patch(&mut self, kind: Option<&str>, target: Option<&str>, ctx: &egui::Context) {
        let cooked = self.swapper_cooked_path();
        let backups = cooked.join("Backups");
        if kind.is_none_or(|kind| kind == "ball") && self.patcher_ball.active_ball.is_some() {
            self.patcher_ball.begin_restore(&cooked, &backups, &self.tx, ctx);
        }
        if kind.is_none_or(|kind| kind == "boost") && self.patcher_boost.active_boost.is_some() {
            self.patcher_boost.begin_restore(&cooked, &backups, &self.tx, ctx);
        }
        if kind.is_none() {
            if !self.patcher_anthem.active_anthems.is_empty() { self.patcher_anthem.restore_all(&cooked, ctx); }
            if !self.patcher_decal.active_decals.is_empty() { let _ = self.patcher_decal.restore_all_decals(&cooked, &backups, &self.tx, ctx); }
            if !self.patcher_car.active_cars.is_empty() && let Err(error) = self.patcher_car.restore_all(&cooked, &backups, &self.tx, &mut self.config) {
                self.console.write(format!("[Cars] Restore failed: {error}"));
            }
        }
        match (kind, target) {
            (Some("decal"), Some(target)) if self.patcher_decal.active_decals.contains_key(target) => {
                if let Some((car, skin)) = target.split_once('|') {
                    let _ = self.patcher_decal.restore_decal_from_skin(car, skin, &cooked, &backups, &self.tx, ctx);
                }
            }
            (Some("car"), Some(target)) if self.patcher_car.active_cars.contains_key(target) => {
                if let Err(error) = self.patcher_car.restore(target, &cooked, &backups, &self.tx, &mut self.config) {
                    self.console.write(format!("[Cars] Restore failed: {error}"));
                }
            }
            (Some("anthem"), Some(target)) if self.patcher_anthem.active_anthems.contains_key(target) => {
                self.patcher_anthem.update_enabled(target.to_owned(), false, &cooked, ctx);
            }
            _ => {}
        }
    }

    fn web_game_mods(&mut self, ctx: &egui::Context) -> Value {
        use crate::patcher::forfeit_command;
        let (capturing, captured) = forfeit_command::web_capture_poll();
        if let Some((id, button)) = captured {
            self.web_game_mod(&id, "pad", json!(button), ctx);
        }
        let cooked = self.swapper_cooked_path();
        let backups = cooked.join("Backups");
        let cfg = &self.config.game_mods;
        let mods = [
            self.forfeit_command.web_snapshot(ctx, &backups, &cfg.forfeit_key, &cfg.forfeit_pad),
            self.chat_export.web_snapshot(ctx, &backups),
            self.lan_commands.web_snapshot(ctx, &backups, |error| self.console.write(format!("[LAN chat commands] {error}"))),
            self.chat_paste.web_snapshot(ctx, &backups, &cooked, |error| self.console.write(format!("[Chat Paste] {error}"))),
            self.instant_suite.web_snapshot(&cooked, cfg),
            self.celebration.web_snapshot(ctx, &backups, &cfg.celebration_key, &cfg.celebration_pad),
            self.bots_freeplay.web_snapshot(ctx, &backups, &cfg.bots_freeplay_key, &cfg.bots_freeplay_pad),
        ];
        let rows: Vec<_> = mods.into_iter()
            .filter(|row| row["id"].as_str().is_some_and(|id| self.game_mod_visible(id)))
            .collect();
        json!({
            "page": "game-mods", "rows": rows, "capturing": capturing,
            "labels": {
                "pad": t("game-mod-controller-button"), "press": t("game-mod-press-a-button"),
                "none": t("game-mod-no-controller-button"), "bind": t("game-mod-bind-controller"),
                "clear": t("game-mod-clear-controller"),
            },
        })
    }

    fn web_game_mod(&mut self, id: &str, key: &str, value: Value, ctx: &egui::Context) {
        use crate::patcher::{bots_freeplay, celebration, forfeit_command};
        let cooked = self.swapper_cooked_path();
        let backups = cooked.join("Backups");
        if id == "instant-suite" {
            if key == "enabled" {
                if let Some(enabled) = value.as_bool() { self.instant_suite.web_toggle(&cooked, enabled); }
            } else if self.instant_suite.web_option(key, &value, &mut self.config.game_mods) {
                self.save_config();
                if crate::patcher::live_cosmetics::managed() {
                    if let Err(error) = crate::patcher::live_cosmetics::sync_game_options(&cooked, &self.config.game_mods) { self.console.write(format!("[Live Game Mods] {error}")); }
                }
            }
            return;
        }
        if let ("enabled", Some(enabled)) = (key, value.as_bool()) {
            match id {
                "chat-export" => { self.chat_export.set_enabled(&cooked, &backups, enabled); return; }
                "lan-commands" => { self.lan_commands.set_enabled(&cooked, &backups, enabled); return; }
                "chat-paste" => {
                    let rules = self.title_rules();
                    self.chat_paste.set_enabled(&cooked, &backups, enabled, rules);
                    return;
                }
                _ => {}
            }
        }
        let cfg = &mut self.config.game_mods;
        let (keyboard, pad, default, on, busy) = match id {
            "forfeit" => (&mut cfg.forfeit_key, &mut cfg.forfeit_pad, forfeit_command::DEFAULT_KEY, forfeit_command::enabled(&backups), self.forfeit_command.busy()),
            "celebration" => (&mut cfg.celebration_key, &mut cfg.celebration_pad, celebration::DEFAULT_KEY, celebration::enabled(&backups), self.celebration.busy()),
            "bots-freeplay" => (&mut cfg.bots_freeplay_key, &mut cfg.bots_freeplay_pad, bots_freeplay::DEFAULT_KEY, bots_freeplay::enabled(&backups), self.bots_freeplay.busy()),
            _ => return,
        };
        if busy { return; }
        let enabled = match key {
            "bind" => { forfeit_command::web_capture_start(id, ctx); return; }
            "keyboard" => {
                let Some(value) = value.as_str().filter(|value| forfeit_command::KEYS.contains(value)) else { return };
                *keyboard = value.to_owned();
                on
            }
            "pad" => {
                let Some(value) = value.as_str().filter(|value| value.is_empty() || forfeit_command::PAD_BUTTONS.iter().any(|(name, _)| name == value)) else { return };
                *pad = value.to_owned();
                on
            }
            "enabled" => {
                let Some(enabled) = value.as_bool() else { return };
                enabled
            }
            _ => return,
        };
        if key == "enabled" || on {
            let key = if keyboard.is_empty() { default.to_owned() } else { keyboard.clone() };
            let pad = pad.clone();
            match id {
                "forfeit" => self.forfeit_command.set_enabled(&cooked, &backups, enabled, key, pad),
                "celebration" => self.celebration.set_enabled(&cooked, &backups, enabled, key, pad),
                _ => self.bots_freeplay.set_enabled(&cooked, &backups, enabled, key, pad),
            }
        }
        self.save_config();
    }

    fn configured_replay_folder(&self) -> Result<std::path::PathBuf, String> {
        crate::auto_upload_replays::configured_replay_directory(
            &self.config.replay_upload,
            std::path::Path::new(&self.config.settings.rl_path),
            &self.config.rl_launch,
        )
    }

    fn web_files(&self) -> Value {
        use crate::config::RlLaunchMode;
        let mode = t(match self.config.rl_launch.mode {
            RlLaunchMode::Unconfigured => "settings-rl-mode-unconfigured",
            RlLaunchMode::SteamNative => "settings-rl-mode-steam-native",
            RlLaunchMode::EpicDirect => "settings-rl-mode-epic-direct",
            RlLaunchMode::SteamShortcut => "settings-rl-mode-steam-shortcut",
        });
        let paths = [
            (
                "game",
                t("settings-rl-folder-label"),
                self.config.settings.rl_path.clone(),
                false,
            ),
            (
                "stats",
                t("settings-statsapi-ini-label"),
                self.config.settings.statsapi_path.clone(),
                false,
            ),
            (
                "replays",
                t("settings-replay-save-folder"),
                self.configured_replay_folder()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|error| error),
                !self.config.replay_upload.save_directory.is_empty(),
            ),
        ];
        let ini = [
            (
                "PacketSendRate",
                t("hebnix-settings-packetsendrate"),
                &self.packet_rate,
                "20",
            ),
            ("Port", t("hebnix-settings-port"), &self.port_value, "49123"),
            (
                "WebPort",
                t("hebnix-settings-webport"),
                &self.web_port_value,
                "49124",
            ),
        ];
        let labels = json!({
            "intro": t("settings-dirs-autodetected"), "browse": t("btn-browse"), "refresh": t("btn-refresh"),
            "reset": t("btn-reset"), "missing": t("settings-not-found"), "note": t("settings-ini-restart-note"),
            "title": t("settings-rl-launch-heading"), "description": t("settings-rl-launch-desc"),
            "status": super::t_args("settings-rl-current-setup", &[("mode", mode.as_str().into())]),
            "setup": t("settings-rl-launch-setup-button"), "setRate": super::t_args("settings-set-value", &[("value", "20".into())]),
            "setWebPort": super::t_args("settings-set-value", &[("value", "49124".into())]),
        });
        let setup = self.rl_launch_setup_open.then(|| {
            let labels = json!({
                "title": t("rl-launch-setup-rocket-league-launch-setup"),
                "hint": t("rl-launch-setup-how-do-you-actually-launch-rocket"),
                "warning": t("rl-launch-setup-non-steam-shortcuts-can-t-pass"),
                "steam": t("rl-launch-setup-steam-app-id-252950-is-rocket"),
                "shortcut": t("rl-launch-setup-steam-shortcut-id"),
                "scan": t("rl-launch-setup-scan-steam-shortcuts"),
                "empty": t("rl-launch-setup-no-candidates-found-yet-click-scan"),
                "save": t("rl-launch-setup-save"), "cancel": t("spawner-enable-prompt-cancel"),
            });
            let modes = [
                (RlLaunchMode::Unconfigured, t("rl-launch-setup-auto-detect-steam-or-epic-games")),
                (RlLaunchMode::SteamNative, t("settings-rl-mode-steam-native")),
                (RlLaunchMode::EpicDirect, t("settings-rl-mode-epic-direct")),
                (RlLaunchMode::SteamShortcut, t("settings-rl-mode-steam-shortcut")),
            ];
            let candidates: Vec<_> = self.rl_launch_shortcut_candidates.iter().map(|item| format!("{}  ({})", item.app_name, item.exe)).collect();
            json!({"draft": self.rl_launch_draft, "modes": modes, "labels": labels, "candidates": candidates})
        });
        json!({ "page": "files-settings", "paths": paths, "ini": ini, "labels": labels, "setup": setup })
    }

    fn web_files_action(&mut self, key: &str, value: Option<String>) {
        match key {
            "game" => {
                let dialog = rfd::FileDialog::new().set_directory(&self.config.settings.rl_path);
                if let Some(path) = winutil::parent_file_dialog(dialog).pick_folder() {
                    let folder = path.to_string_lossy().into_owned();
                    self.config.settings.rl_path.clone_from(&folder);
                    self.remember_patcher_state(&folder);
                    self.save_config();
                    self.refresh_stats_api_viewer();
                }
            }
            "stats" => {
                let directory = std::path::Path::new(&self.config.settings.statsapi_path)
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default();
                let dialog = rfd::FileDialog::new()
                    .set_directory(directory)
                    .add_filter(t("filter-ini-files"), &["ini"]);
                if let Some(path) = winutil::parent_file_dialog(dialog).pick_file() {
                    self.config.settings.statsapi_path = path.to_string_lossy().into_owned();
                    self.save_config();
                    self.refresh_stats_api_viewer();
                }
            }
            "replays" => {
                let mut dialog = rfd::FileDialog::new();
                if let Ok(path) = self.configured_replay_folder() {
                    dialog = dialog.set_directory(path);
                }
                if let Some(path) = winutil::parent_file_dialog(dialog).pick_folder() {
                    self.config.replay_upload.save_directory = path.to_string_lossy().into_owned();
                    self.save_config();
                }
            }
            "reset_replays" => {
                self.config.replay_upload.save_directory.clear();
                self.save_config();
            }
            "refresh" => self.refresh_stats_api_viewer(),
            "setup" => {
                self.rl_launch_draft = self.config.rl_launch.clone();
                self.rl_launch_shortcut_candidates.clear();
                self.rl_launch_setup_open = true;
            }
            "PacketSendRate" | "Port" | "WebPort" => {
                let Some(value) = value.filter(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                }) else {
                    return;
                };
                if key == "PacketSendRate" || value.parse::<u16>().is_ok_and(|value| value > 0) {
                    self.update_ini_setting(key, &value);
                }
            }
            _ => {}
        }
    }

    fn web_appearance(&self) -> Value {
        let cfg = &self.config.settings;
        let languages =
            std::iter::once((crate::i18n::AUTO.to_owned(), t("settings-language-auto")))
                .chain(
                    crate::i18n::available()
                        .iter()
                        .map(|item| (item.code.clone(), crate::i18n::display_name(item))),
                )
                .collect::<Vec<_>>();
        let general = json!({
            "hotkey": cfg.hotkey.to_uppercase(), "capturing": self.capturing_hotkey,
            "language": cfg.language, "languages": languages, "opacity": cfg.window_opacity,
            "toast": cfg.toast_position.label(),
            "toasts": crate::toast::ToastPos::ALL.map(|choice| choice.label()),
            "defaultTab": cfg.default_tab, "tabs": Tab::ALL.map(|tab| (tab.config_key(), tab.label())),
        });
        let panel = json!({
            "enabled": cfg.panel_enabled, "left": cfg.panel_left, "width": cfg.panel_width,
            "animate": cfg.panel_animate, "advanced": cfg.panel_advanced,
        });
        let labels = json!({
            "general": t("settings-appearance-general"), "theme": t("settings-appearance-theme"),
            "themeLabel": t("settings-theme-label"), "refresh": t("btn-refresh"),
            "import": t("settings-import-themes"), "folder": t("btn-open-folder"),
            "fonts": t("settings-open-fonts-folder"), "hotkey": t("settings-keybind-label"),
            "capture": t(if self.capturing_hotkey { "settings-keybind-listening" } else { "settings-keybind-set" }),
            "language": t("settings-language-label"), "opacity": t("settings-opacity-label"),
            "toast": t("settings-toast-position-label"), "defaultTab": t("system-default-tab"),
        });
        json!({
            "page": "appearance-settings", "themePage": self.appearance_subtab == super::AppearanceSubTab::Theme,
            "legacy": cfg.legacy_ui, "theme": cfg.theme, "themes": self.theme_options,
            "general": general, "panel": panel, "labels": labels,
        })
    }

    fn web_appearance_action(&mut self, key: &str, value: Value, ctx: &egui::Context) {
        let boolean = value.as_bool();
        let number = value.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
        let text = value.as_str().unwrap_or_default();
        let cfg = &mut self.config.settings;
        match key {
            "page" => {
                self.appearance_subtab = if boolean == Some(true) {
                    super::AppearanceSubTab::Theme
                } else {
                    super::AppearanceSubTab::General
                };
                return;
            }
            "capture" => {
                self.start_hotkey_capture(ctx);
                return;
            }
            "theme" if self.theme_options.iter().any(|name| name == text) => {
                self.change_theme(ctx, text);
                return;
            }
            "refresh_theme" | "import_theme" => {
                if key == "import_theme" {
                    let dialog = rfd::FileDialog::new()
                        .set_title(t("settings-import-themes"))
                        .add_filter("ZIP", &["zip"]);
                    let Some(path) = winutil::parent_file_dialog(dialog).pick_file() else {
                        return;
                    };
                    if let Err(error) =
                        crate::deep_link::import_theme_zip(&path, &self.themes_dir, &self.fonts_dir)
                    {
                        let dialog = rfd::MessageDialog::new()
                            .set_title(t("settings-import-themes"))
                            .set_level(rfd::MessageLevel::Error)
                            .set_description(error);
                        winutil::parent_message_dialog(dialog).show();
                        return;
                    }
                }
                self.theme_options = super::theme::list_themes(&self.themes_dir);
                let theme = self.config.settings.theme.clone();
                self.change_theme(ctx, &theme);
                if key == "refresh_theme" {
                    self.console
                        .write(format!("[Console] {}", t("console-themes-rescanned")));
                }
                return;
            }
            "theme_folder" => {
                let _ = open::that(&self.themes_dir);
                return;
            }
            "fonts_folder" => {
                let _ = open::that(&self.fonts_dir);
                return;
            }
            "language"
                if text == crate::i18n::AUTO
                    || crate::i18n::available()
                        .iter()
                        .any(|item| item.code == text) =>
            {
                self.change_language(ctx, text);
                return;
            }
            "legacy" => cfg.legacy_ui = boolean.unwrap_or(cfg.legacy_ui),
            "opacity" => {
                let Some(opacity) = number else { return };
                cfg.window_opacity = opacity.clamp(0.5, 1.0);
                let _ =
                    super::theme::apply_theme(ctx, &self.themes_dir, &self.fonts_dir, &cfg.theme);
                super::theme::apply_window_opacity(ctx, cfg.window_opacity);
            }
            "default_tab" if Tab::ALL.iter().any(|tab| tab.config_key() == text) => {
                cfg.default_tab = text.to_owned()
            }
            "toast" => {
                let Some(choice) = crate::toast::ToastPos::ALL
                    .into_iter()
                    .find(|choice| choice.label() == text)
                else {
                    return;
                };
                cfg.toast_position = choice;
            }
            "panel_enabled" => cfg.panel_enabled = boolean.unwrap_or(cfg.panel_enabled),
            "panel_left" if cfg.panel_enabled => cfg.panel_left = boolean.unwrap_or(cfg.panel_left),
            "panel_animate" if cfg.panel_enabled => {
                cfg.panel_animate = boolean.unwrap_or(cfg.panel_animate)
            }
            "panel_advanced" if cfg.panel_enabled => {
                cfg.panel_advanced = boolean.unwrap_or(cfg.panel_advanced)
            }
            "panel_width" if cfg.panel_enabled => {
                let Some(width) = number else { return };
                cfg.panel_width = width.clamp(0.2, 0.5);
            }
            _ => return,
        }
        self.save_config();
        if key == "panel_enabled" && !self.config.settings.panel_enabled && self.panel.active {
            self.set_hidden(ctx, true);
        }
    }

    fn web_system_settings(&self) -> Value {
        if self.maintenance.tab { return self.web_maintenance(); }
        let cfg = &self.config.settings;
        let rows: Vec<_> = [
            ("startup", "system-start-with-windows", self.startup_enabled, ""),
            ("tray", "system-start-in-tray", cfg.start_in_tray, ""),
            ("close", "system-close-to-tray", cfg.close_to_tray, ""),
            ("p2p", "system-p2p", cfg.p2p_file_sharing, "system-p2p-note"),
            ("left", "system-suppress-left", cfg.suppress_left_alerts, ""),
            ("fullscreen", "system-fullscreen-warning", !cfg.suppress_fullscreen_warning, "system-fullscreen-note"),
            ("focus", "system-allow-draw-focus", cfg.allow_draw_on_hebnix_focus, ""),
            ("hotkey", "system-limit-hotkey", cfg.restrict_hotkey_to_hebnix_or_rocket_league, ""),
            ("rate", "system-statsapi-rate", !cfg.suppress_statsapi_rate_warning, "system-statsapi-note"),
        ].into_iter().map(|(key, label, enabled, note)| json!({
            "key": key, "label": t(label), "enabled": enabled, "note": (!note.is_empty()).then(|| t(note)),
        })).collect();
        let proxy = if self.spoofer_mgr.http_running() {
            t("spoofer-running")
        } else if self.proxy_options().account && !self.spoofer_mgr.game_running() {
            t("spoofer-waiting-for-game")
        } else {
            t("spoofer-stopped")
        };
        let certificate = json!({
            "installed": self.spoofer_cert_installed, "admin": spoofer::is_admin(), "proxy": proxy,
            "proxyLabel": t("spoofer-http-proxy"), "statusLabel": t("spoofer-certificate-status"),
            "status": t(if self.spoofer_cert_installed { "spoofer-installed" } else { "spoofer-missing" }),
            "action": t(if self.spoofer_cert_installed { "spoofer-remove" } else { "spoofer-install-certificate" }),
            "refresh": t("spoofer-refresh-status"),
        });
        json!({
            "page": "system-settings", "rows": rows, "fps": cfg.overlay_refresh_fps,
            "repair": t("action-fix-epic-connection"), "repairBusy": self.epic_repair.running,
            "backups": self.delete_backups.web_snapshot(), "certificate": certificate,
        })
    }

    fn web_system_toggle(&mut self, key: &str, enabled: bool) {
        let cfg = &mut self.config.settings;
        match key {
            "startup" => {
                if let Err(error) = winutil::set_startup_enabled(enabled) {
                    self.console.write(format!(
                        "[Console] {}",
                        super::t_args(
                            "console-startup-failed",
                            &[("error", error.to_string().into())]
                        )
                    ));
                    self.startup_enabled = winutil::is_startup_enabled();
                } else {
                    self.startup_enabled = enabled;
                }
                return;
            }
            "tray" => cfg.start_in_tray = enabled,
            "close" => cfg.close_to_tray = enabled,
            "p2p" => {
                cfg.p2p_file_sharing = enabled;
                self.workshop.set_share_files(enabled);
            }
            "left" => cfg.suppress_left_alerts = enabled,
            "fullscreen" => {
                cfg.suppress_fullscreen_warning = !enabled;
                self.fullscreen_notice_dismissed = false;
                self.fullscreen_notice =
                    enabled && self.window_mode == Some(super::WindowMode::Fullscreen);
            }
            "focus" => cfg.allow_draw_on_hebnix_focus = enabled,
            "hotkey" => cfg.restrict_hotkey_to_hebnix_or_rocket_league = enabled,
            "rate" => cfg.suppress_statsapi_rate_warning = !enabled,
            _ => return,
        }
        self.save_config();
    }

    fn web_discord_settings(&mut self) -> Value {
        let settings = &self.config.settings;
        json!({
            "page": "discord-settings",
            "enabled": settings.discord_rich_presence, "only": settings.discord_rocket_league_only,
            "game": settings.discord_game_state, "score": settings.discord_show_score,
            "map": settings.discord_show_map, "mode": settings.discord_show_gamemode,
            "message": settings.discord_custom_message, "link": self.discord_link.web_snapshot(),
            "labels": {
                "enabled": t("discord-enable"), "only": t("discord-rl-only"), "message": t("discord-message-label"),
                "game": t("discord-game-state"), "score": t("discord-show-score"), "map": t("discord-show-map"),
                "mode": t("discord-show-gamemode"), "custom": t("discord-custom"),
                "placeholder": t("discord-custom-hint"), "disabled": t("discord-custom-disabled-note"),
            },
        })
    }

    fn web_discord_option(&mut self, key: &str, enabled: bool) {
        let settings = &mut self.config.settings;
        let count = [
            settings.discord_show_score,
            settings.discord_show_map,
            settings.discord_show_gamemode,
        ]
        .into_iter()
        .filter(|on| *on)
        .count();
        let field = match key {
            "enabled" => &mut settings.discord_rich_presence,
            "only" => &mut settings.discord_rocket_league_only,
            "game" => &mut settings.discord_game_state,
            "score"
                if settings.discord_game_state
                    && (enabled || !settings.discord_show_score || count > 1) =>
            {
                &mut settings.discord_show_score
            }
            "map"
                if settings.discord_game_state
                    && (enabled || !settings.discord_show_map || count > 1) =>
            {
                &mut settings.discord_show_map
            }
            "mode"
                if settings.discord_game_state
                    && (enabled || !settings.discord_show_gamemode || count > 1) =>
            {
                &mut settings.discord_show_gamemode
            }
            _ => return,
        };
        if *field == enabled {
            return;
        }
        *field = enabled;
        if key == "enabled" {
            self.discord_presence.configure(enabled);
        }
        self.save_config();
        self.refresh_discord_presence();
    }

    fn web_presets(&self) -> Value {
        let store = &self.presets;
        json!({
            "page": "presets", "selected": store.selected, "editing": store.editing,
            "presets": store.presets.iter().map(|preset| &preset.name).collect::<Vec<_>>(),
            "current": store.presets.get(store.selected), "name": store.name_edit,
            "patches": store.include_patches, "key": store.preset_key, "status": store.status,
            "missing": !store.missing.is_empty(),
            "labels": {
                "intro": t("presets-save-and-restore-named-collections-of"), "name": t("presets-name"),
                "patches": t("presets-include-patches"), "capture": t("presets-save-preset"),
                "refresh": t("btn-refresh"), "import": t("presets-import-json"), "empty": t("presets-no-presets-saved"),
                "delete": t("presets-delete"),
            },
        })
    }

    fn web_preset_action(&mut self, action: PresetAction, ctx: &egui::Context) {
        let cooked = self.swapper_cooked_path();
        let backups = cooked.join("Backups");
        let result = match action {
            PresetAction::Capture { name, patches } => {
                self.presets.name_edit = name;
                self.presets.include_patches = patches;
                let preset = self.capture_preset(&backups);
                self.presets.save(preset)
            }
            PresetAction::Refresh => {
                self.presets.refresh();
                Ok(())
            }
            PresetAction::ImportFile => {
                let dialog = rfd::FileDialog::new().add_filter(t("presets-json"), &["json"]);
                let Some(path) = winutil::parent_file_dialog(dialog).pick_file() else {
                    return;
                };
                std::fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
                    })
                    .and_then(|preset| self.presets.save(preset))
            }
            PresetAction::Select { index } if index < self.presets.presets.len() => {
                self.presets.selected = index;
                Ok(())
            }
            PresetAction::Apply => {
                self.apply_selected_preset(&cooked, &backups, ctx);
                Ok(())
            }
            PresetAction::Edit => {
                self.presets.begin_edit_selected();
                Ok(())
            }
            PresetAction::Delete => {
                self.presets.delete_selected();
                Ok(())
            }
            PresetAction::Draft { name, patches } => {
                if let Some(draft) = self.presets.editing.as_mut() {
                    draft.name = name;
                    draft.include_patches = patches;
                }
                Ok(())
            }
            PresetAction::RemoveSwap { index } => {
                if let Some(draft) = self.presets.editing.as_mut()
                    && index < draft.swaps.len()
                {
                    draft.swaps.remove(index);
                }
                Ok(())
            }
            PresetAction::RemovePatch { kind, key } => {
                if let Some(patches) = self
                    .presets
                    .editing
                    .as_mut()
                    .and_then(|draft| draft.patches.as_object_mut())
                {
                    if let Some(key) = key {
                        if let Some(entries) = patches.get_mut(&kind).and_then(Value::as_object_mut)
                        {
                            entries.remove(&key);
                        }
                    } else {
                        patches.remove(&kind);
                    }
                }
                Ok(())
            }
            PresetAction::Save => self.presets.save_edit(),
            PresetAction::Cancel => {
                self.presets.cancel_edit();
                Ok(())
            }
            PresetAction::RemoveMissing => self.presets.remove_missing_selected(),
            PresetAction::IgnoreMissing => {
                self.presets.missing.clear();
                self.presets.status.clear();
                Ok(())
            }
            PresetAction::ShowKey => self
                .presets
                .selected_key()
                .map(|key| self.presets.preset_key = key),
            PresetAction::ImportKey { key } => {
                self.presets.preset_key.clone_from(&key);
                self.presets.import_key(&key).map(|()| {
                    self.presets.status =
                        "Preset imported. Missing local items will be skipped when applied.".into();
                })
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.presets.status = error;
        }
    }

    fn web_player_data(&mut self) -> Value {
        match self.spoofer_subtab {
            SpooferSubTab::Username => json!({
                "page": "player-name", "name": self.spoofer_username, "enabled": self.spoofer_username_enabled,
                "history": self.spoofer_username_history, "max": spoofer::MAX_NAME_LENGTH,
            }),
            SpooferSubTab::Friends => {
                self.sync_friend_names();
                let mut rows: Vec<_> = self.spoofer_friends.iter().collect();
                rows.sort_by_cached_key(|(id, state)| {
                    (state.original_name.to_lowercase(), id.to_string())
                });
                let rows: Vec<_> = rows.into_iter().map(|(id, state)| json!({
                    "id": id, "name": state.original_name, "alias": state.spoofed_name, "enabled": state.enabled,
                })).collect();
                json!({
                    "page": "player-friends", "rows": rows, "enabled": self.spoofer_friends_enabled,
                    "toggle": t("spoofer-enable-friend-name-spoof"), "search": t("spoofer-search"),
                    "revert": t("spoofer-revert-all"), "placeholder": t("spoofer-spoofed-name"),
                    "empty": t("spoofer-start-rocket-league-to-populate-friends"),
                })
            }
            SpooferSubTab::Scoreboard => self.scoreboard.web_snapshot(&self.config.scoreboard),
            SpooferSubTab::ProfilePictures => self
                .profile_pictures
                .web_snapshot(std::path::Path::new(&self.config.settings.rl_path)),
            SpooferSubTab::TitleRank => {
                let rows: Vec<_> = super::RANK_SPOOF_PLAYLISTS.iter().enumerate().map(|(icon, &(id, name, _))| {
                    let rank = self.spoofer_ranks.entry(id).or_default();
                    json!({ "id": id, "name": name, "icon": icon, "enabled": rank.enabled, "rank": rank.rank, "division": rank.division, "mmr": rank.mmr })
                }).collect();
                json!({
                    "page": "player-rank", "rows": rows, "ranks": (0..=22).map(super::rank_name).collect::<Vec<_>>(),
                    "rankDefaults": (0..=22).map(|rank| (super::default_mu_for_tier(rank) * 20.0 + 100.0) as u32).collect::<Vec<_>>(),
                    "ready": self.web_rank_ready(), "busy": self.rank_patch.busy(), "status": self.rank_patch.status,
                    "apply": t("spoofer-apply-ranks"), "restore": t("spoofer-restore-ranks"), "mmr": t("spoofer-mmr"),
                })
            }
            _ => unreachable!(),
        }
    }

    fn web_rank_ready(&self) -> bool {
        !self.rank_patch.busy()
            && !self.title_patcher.busy()
            && !self.last_rl_open
            && !self.first_status
            && self.swapper_cooked_path().is_dir()
    }

    fn web_action_button_data(&self) -> Value {
        let rows = |entries: &[super::ActionButtonEntry]| -> Vec<Value> {
            entries
                .iter()
                .map(|entry| json!({ "label": entry.action.label(), "enabled": entry.enabled }))
                .collect()
        };
        json!({
            "showInHeader": self.config.action_button.show_in_header,
            "page": "action-button", "intro": t("action-settings-intro"), "hint": t("action-settings-hint"),
            "keepOne": t("action-keep-one"), "openLabel": t("action-state-open"), "closedLabel": t("action-state-closed"),
            "open": rows(&self.config.action_button.rocket_league_open),
            "closed": rows(&self.config.action_button.rocket_league_closed),
        })
    }

    fn web_action_entries(&mut self, running: bool) -> &mut Vec<super::ActionButtonEntry> {
        if running {
            &mut self.config.action_button.rocket_league_open
        } else {
            &mut self.config.action_button.rocket_league_closed
        }
    }

    fn web_overlay_order(&self) -> Vec<(String, String)> {
        self.plugin_mgr
            .overlay_layers(&self.config.overlay_order)
            .iter()
            .rev()
            .map(|(slug, _, _, _)| {
                let name = self
                    .plugin_mgr
                    .plugins
                    .iter()
                    .find(|plugin| &plugin.slug == slug)
                    .map(|plugin| plugin.display_name().to_string())
                    .unwrap_or_else(|| slug.clone());
                (slug.clone(), name)
            })
            .collect()
    }

    fn modern_plugins_data(&mut self, ctx: &egui::Context) -> Value {
        if self.plugins_subtab == PluginsSubTab::Installed {
            let rows: Vec<_> = self.plugin_mgr.plugins_by_name().map(|plugin| json!({
                "id": plugin.slug, "name": plugin.display_name(), "description": plugin.filename,
                "version": plugin.manifest.version, "author": plugin.manifest.author,
                "enabled": plugin.enabled, "settings": plugin.has_settings(), "error": plugin.load_error,
            })).collect();
            return json!({ "page": "installed", "rows": rows, "overlay": self.web_overlay_order() });
        }
        let banners: Vec<_> = self
            .install_modal
            .catalog
            .iter()
            .filter(|entry| {
                plugin_id(entry).is_some_and(|id| {
                    self.plugin_web_preview_ids
                        .iter()
                        .any(|requested| requested == id)
                })
            })
            .filter_map(|entry| entry["banner_path"].as_str().map(str::to_owned))
            .collect();
        for banner in banners {
            self.ensure_plugin_image(&banner, ctx);
        }
        let rows: Vec<_> = self.install_modal.catalog.iter().filter_map(|entry| {
            let id = plugin_id(entry)?;
            let existing = self.plugin_mgr.plugins.iter().find(|plugin| plugin.manifest.plugin_id.as_deref() == Some(id));
            let banner = if self.plugin_web_preview_ids.iter().any(|requested| requested == id) {
                entry["banner_path"].as_str().and_then(|path| match self.install_modal.images.get(path) {
                    Some(ImageState::Ready(bytes)) => image_url(bytes),
                    _ => None,
                })
            } else { None };
            Some(json!({
                "id": id, "name": entry["name"].as_str().unwrap_or(id), "author": entry["author"],
                "description": entry["short_description"], "version": entry["version_number"], "banner": banner,
                "installed": existing.map(|plugin| &plugin.slug),
                "enabled": existing.is_some_and(|plugin| plugin.enabled),
                "settings": existing.is_some_and(|plugin| plugin.has_settings()),
                "error": existing.and_then(|plugin| plugin.load_error.as_deref()),
            }))
        }).collect();
        json!({ "page": "browse", "rows": rows, "loading": self.install_modal.fetching,
            "downloading": self.install_modal.downloading_id, "error": self.install_modal.error })
    }

    pub(super) fn handle_modern_manager(&mut self, action: ManageAction, ctx: &egui::Context) {
        if !self.has_modern_manager() {
            return;
        }
        match action {
            ManageAction::Maintenance { key, enabled } if (self.tab == Tab::About || (self.tab == Tab::Settings && self.hebnix_settings_tab == HebnixSettingsTab::System)) && self.maintenance.tab => {
                match (key.as_str(), enabled) {
                    ("back", _) => {
                        self.maintenance.tab = false;
                        self.maintenance.confirm = false;
                    }
                    ("confirm", _) if self.maintenance_selected() => self.maintenance.confirm = true,
                    ("cancel", _) => self.maintenance.confirm = false,
                    ("remove", _) if self.maintenance.confirm && self.maintenance_selected() => {
                        self.maintenance.confirm = false;
                        self.run_maintenance(ctx);
                    }
                    (key, Some(value)) if !self.maintenance.confirm => {
                        let options = &mut self.maintenance;
                        let setting = match key {
                            "application" => &mut options.application,
                            "settings" => &mut options.settings,
                            "extensions" => &mut options.extensions,
                            "assets" => &mut options.assets,
                            "restore_swaps" => &mut options.patch_groups[0],
                            "restore_game_mods" => &mut options.patch_groups[1],
                            "restore_workshop" => &mut options.patch_groups[2],
                            "restore_custom" => &mut options.patch_groups[3],
                            "certificate" => &mut options.certificate,
                            _ => return,
                        };
                        *setting = value;
                        options.error = None;
                    }
                    _ => {}
                }
            }
            ManageAction::StudioDecal { key, id, value } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::Patching
                && self.patcher_subtab == super::PatcherSubTab::Decal => {
                if key == "inventory" {
                    self.owned_inventory.refresh_web(ctx, id.as_deref());
                } else {
                    let cooked = self.swapper_cooked_path();
                    self.patcher_decal.web_action(&key, id, value, &cooked, &self.owned_inventory.ids, &self.tx, &mut self.config, ctx);
                }
            }
            ManageAction::ItemArtwork { key, value } if self.tab == Tab::Patcher && (self.items_mode == super::ItemsMode::ItemNames
                || self.items_mode == super::ItemsMode::Patching && self.patcher_subtab == super::PatcherSubTab::Thumbnail) => {
                if key == "inventory" {
                    self.owned_inventory.refresh_web(ctx, value.as_str());
                } else {
                    let cooked = self.swapper_cooked_path();
                    let closed = !self.last_rl_open && !self.first_status;
                    if self.items_mode == super::ItemsMode::ItemNames {
                        self.swapper.web_item_name_action(&key, value, &cooked, closed, ctx);
                    } else {
                        self.swapper.web_item_thumbnail_action(&key, value, &cooked, closed, &self.owned_inventory.ids, ctx);
                    }
                }
            }
            ManageAction::Anthem { key, id, value } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::Patching && self.patcher_subtab == super::PatcherSubTab::Anthem => {
                if key == "inventory" {
                    self.owned_inventory.refresh_web(ctx, id.as_deref());
                } else if !matches!(key.as_str(), "save" | "patch" | "restore-all") || (!self.last_rl_open && !self.first_status) {
                    let cooked = self.swapper_cooked_path();
                    self.patcher_anthem.web_action(&key, id, value, &cooked, ctx);
                }
            }
            ManageAction::UpkHistory { key, index, value } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::History => {
                let cooked = self.swapper_cooked_path();
                self.upk_history.web_action(&cooked, &key, index, value, ctx);
            }
            ManageAction::MenuTextures { key, value } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::MenuTextures => {
                self.web_menu_texture_action(&key, value, ctx);
            }
            ManageAction::StudioCustomPage { index } if self.tab == Tab::Patcher => {
                if let Some(&(tab, _)) = STUDIO_CUSTOM.get(index) {
                    if let Some(tab) = tab {
                        self.items_mode = super::ItemsMode::Patching;
                        self.patcher_subtab = tab;
                    } else {
                        self.items_mode = super::ItemsMode::ItemNames;
                    }
                }
            }
            ManageAction::StudioAssetPreviews { ids } if self.on_studio_asset_page() => self.swapper_preview_ids = ids.into_iter().take(36).collect(),
            ManageAction::StudioAsset { key, id } if self.on_studio_asset_page() && self.items_mode == super::ItemsMode::Patching
                && matches!(self.patcher_subtab, super::PatcherSubTab::Ball | super::PatcherSubTab::BoostMeter | super::PatcherSubTab::Car) => {
                let cooked = self.swapper_cooked_path();
                if self.patcher_subtab == super::PatcherSubTab::Ball {
                    self.patcher_ball.web_action(&key, id, &cooked, &mut self.config, &self.tx, ctx);
                } else if self.patcher_subtab == super::PatcherSubTab::Car {
                    self.patcher_car.web_action(&key, id, &cooked, &mut self.config, &self.tx, ctx);
                } else {
                    self.patcher_boost.web_action(&key, id, &cooked, &mut self.config, &self.tx, ctx);
                }
            }
            ManageAction::GameColours { settings, restore }
                if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::GameColours || self.on_live_colours_page() => {
                if let Some(mut settings) = settings {
                    if crate::patcher::live_cosmetics::managed() { settings.stadium_colours = false; settings.hud_colours = false; settings.heatseeker_glow = false; }
                    self.colours.web_colour_settings(settings);
                }
                self.web_colour_action(if restore { crate::patcher::colours::ColourAction::Restore } else { crate::patcher::colours::ColourAction::Apply }, ctx);
            }
            ManageAction::BallAppearance { key, value } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::BallAppearance => {
                if let Some(action) = self.colours.web_ball_action(&key, &value) { self.web_colour_action(action, ctx); }
            }
            ManageAction::WheelSelect { index } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::WheelAlignment => {
                self.wheel_alignment.web_select(index);
            }
            ManageAction::LiveCosmetics { settings } if self.on_live_colours_page() => {
                let cooked = self.swapper_cooked_path();
                if let Err(error) = crate::patcher::live_cosmetics::update(&cooked, settings) {
                    self.console.write(format!("[Live Cosmetics] {error}"));
                }
            }
            ManageAction::WheelApply { settings, remove } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::WheelAlignment => {
                let cooked = self.swapper_cooked_path();
                self.wheel_alignment.web_run(&cooked, settings, remove, ctx);
            }
            ManageAction::MenuMap { action } if self.tab == Tab::Workshop && self.workshop.view == crate::ui::workshop::WorkshopView::MenuMap => {
                self.workshop.menu_map_web_action(action, &self.config.settings.rl_path, &self.tx, ctx);
            }
            ManageAction::Multiplayer { action } if self.tab == Tab::Workshop && self.workshop.view == crate::ui::workshop::WorkshopView::Multiplayer => {
                self.workshop.multiplayer_web_action(action, &self.config.settings.rl_path, &self.tx, ctx);
            }
            ManageAction::Titles { action } if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Titles => self.web_titles_action(action, ctx),
            ManageAction::PluginSettingsSelect { id } if self.tab == Tab::Plugins && self.plugins_subtab == PluginsSubTab::Settings => {
                if self.plugin_mgr.plugins.iter().any(|plugin| plugin.slug == id && plugin.enabled && plugin.has_settings()) {
                    self.selected_settings_plugin = Some(id);
                }
            }
            ManageAction::PluginSettingsInput { plugin, id, value } if self.tab == Tab::Plugins && self.plugins_subtab == PluginsSubTab::Settings
                && self.selected_settings_plugin.as_ref() == Some(&plugin) && self.web_plugin_settings.inputs.contains(&id) =>
            {
                if self.web_plugin_settings.pending.len() < 32 {
                    self.web_plugin_settings.pending.push_back((id, value));
                }
            }
            ManageAction::PluginSettingsExpand { plugin, id, open } if self.tab == Tab::Plugins && self.plugins_subtab == PluginsSubTab::Settings
                && self.selected_settings_plugin.as_ref() == Some(&plugin) && id.starts_with("collapsing:") && self.web_plugin_settings.inputs.contains(&id) =>
            {
                if open { self.web_plugin_settings.expanded.insert(id); }
                else { self.web_plugin_settings.expanded.remove(&id); }
            }
            ManageAction::PluginSettingsInstalled if self.tab == Tab::Plugins && self.plugins_subtab == PluginsSubTab::Settings => {
                self.plugins_subtab = PluginsSubTab::Installed;
            }
            ManageAction::ReplayLibrary { action } if self.tab == Tab::Replays => {
                self.replay_timeline.web_action(action, &self.ws_stats, ctx);
            }
            ManageAction::StudioRestore { kind, target } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::Manage => {
                self.web_restore_patch(kind.as_deref(), target.as_deref(), ctx);
            }
            ManageAction::GameMod { id, key, value } if self.game_mod_visible(&id) => self.web_game_mod(&id, &key, value, ctx),
            ManageAction::AddMaps { action } if self.tab == Tab::Workshop && self.workshop.view == crate::ui::workshop::WorkshopView::Import => {
                self.workshop.add_maps_web_action(action, &self.config.settings.rl_path, &self.tx, ctx);
            }
            ManageAction::LaunchSetup { key, draft, candidate } if self.tab == Tab::Settings
                && self.hebnix_settings_tab == HebnixSettingsTab::Directories && self.rl_launch_setup_open =>
            {
                if let Some(draft) = draft { self.rl_launch_draft = draft; }
                match key.as_str() {
                    "scan" => self.rl_launch_shortcut_candidates = crate::rl_launch::find_steam_shortcuts(),
                    "candidate" => {
                        if let Some(item) = candidate.and_then(|index| self.rl_launch_shortcut_candidates.get(index)) {
                            self.rl_launch_draft.steam_id = item.rungameid.to_string();
                            self.rl_launch_draft.shortcut_exe = item.exe.trim().trim_matches('"').to_string();
                        }
                    }
                    "save" => {
                        self.config.rl_launch = self.rl_launch_draft.clone();
                        self.save_config();
                        self.console.write("[Core] Rocket League launch setup saved.");
                        self.rl_launch_setup_open = false;
                    }
                    "cancel" => self.rl_launch_setup_open = false,
                    _ => {}
                }
            }
            ManageAction::Files { key, value }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::Directories =>
            {
                self.web_files_action(&key, value);
            }
            ManageAction::Appearance { key, value }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::Interface =>
            {
                self.web_appearance_action(&key, value, ctx);
            }
            ManageAction::SystemToggle { key, enabled }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::System =>
            {
                self.web_system_toggle(&key, enabled)
            }
            ManageAction::SystemFps { fps }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::System =>
            {
                self.config.settings.overlay_refresh_fps = fps.clamp(20, 60);
                self.save_config();
            }
            ManageAction::SystemBackups { key }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::System =>
            {
                let cooked = self.swapper_cooked_path();
                self.delete_backups.web_action(&key, &cooked);
            }
            ManageAction::SystemCommand { key }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::System =>
            {
                match key.as_str() {
                    "uninstall" => self.maintenance.tab = true,
                "repair" if !self.epic_repair.running => {
                        self.epic_repair.begin(ctx);
                        if self.epic_repair.running {
                            self.spoofer_mgr.shutdown();
                        }
                    }
                    "certificate" if self.spoofer_cert_installed => {
                        self.remove_spoofer_certificate()
                    }
                    "certificate" => self.install_spoofer_certificate(),
                    "refresh_certificate" => {
                        self.prompt_for_missing_spoofer_certificate();
                        self.evaluate_proxies();
                    }
                    _ => {}
                }
            }
            ManageAction::GoalExplosion {
                volume,
                value,
                restore,
            } if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::GoalExplosion => {
                let cooked = self.swapper_cooked_path();
                if volume {
                    self.goal_explosion_volume.web_run(&cooked, value, restore);
                }
                // Scale is exclusively file-backed now. Ignore stale one-time scale actions.
            }
            ManageAction::SpeedItems { enabled }
                if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::SpeedPatch =>
            {
                self.config.speed_patch.items_page = enabled;
                self.save_config();
            }
            ManageAction::SpeedChoose
                if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::SpeedPatch =>
            {
                let cooked = self.swapper_cooked_path();
                self.speed_patch.web_choose(&cooked);
            }
            ManageAction::SpeedRun { speed, restore }
                if self.tab == Tab::Patcher && self.items_mode == super::ItemsMode::SpeedPatch =>
            {
                let cooked = self.swapper_cooked_path();
                self.speed_patch.web_run(&cooked, speed, restore, &self.tx);
            }
            ManageAction::StudioAppearancePage { index } if self.tab == Tab::Patcher => {
                if let Some(&(mode, _)) = STUDIO_APPEARANCE.get(index) {
                    self.items_mode = mode;
                }
            }
            ManageAction::DiscordOption { key, enabled }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::Discord =>
            {
                self.web_discord_option(&key, enabled);
            }
            ManageAction::DiscordMessage { message }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::Discord
                    && !self.config.settings.discord_game_state =>
            {
                self.config.settings.discord_custom_message = message;
                self.save_config();
                self.refresh_discord_presence();
            }
            ManageAction::DiscordLink { code }
                if self.tab == Tab::Settings
                    && self.hebnix_settings_tab == HebnixSettingsTab::Discord =>
            {
                self.discord_link.web_link(code, ctx);
            }
            ManageAction::Preset { action }
                if self.tab == Tab::Items && self.items_subtab == super::ItemsSubTab::Presets =>
            {
                self.web_preset_action(action, ctx);
            }
            ManageAction::WorkshopMenu { action } if self.tab == Tab::Workshop && self.workshop.view == crate::ui::workshop::WorkshopView::InGameMenu => {
                let cooked = self.swapper_cooked_path();
                self.workshop
                    .menu_web_action(action, &cooked, &self.tx, ctx);
            }
            ManageAction::ProfilePicture { key, value }
                if self.tab == Tab::Spoofer
                    && self.spoofer_subtab == SpooferSubTab::ProfilePictures =>
            {
                self.profile_pictures.web_action(
                    std::path::Path::new(&self.config.settings.rl_path),
                    &key,
                    &value,
                    ctx,
                );
            }
            ManageAction::PlayerRank {
                id,
                enabled,
                rank,
                division,
                mmr,
            } if self.tab == Tab::Spoofer
                && self.spoofer_subtab == SpooferSubTab::TitleRank
                && !self.rank_patch.busy()
                && super::RANK_SPOOF_PLAYLISTS.iter().any(|row| row.0 == id) =>
            {
                let state = self.spoofer_ranks.entry(id).or_default();
                if let Some(enabled) = enabled {
                    state.enabled = enabled;
                }
                if let Some(rank) =
                    rank.filter(|rank| (0..=22).contains(rank) && state.rank != *rank)
                {
                    state.rank = rank;
                    state.mmr =
                        ((super::default_mu_for_tier(rank) * 20.0 + 100.0) as u32).to_string();
                }
                if let Some(division) =
                    division.filter(|division| *division < 4 && (1..22).contains(&state.rank))
                {
                    state.division = division;
                }
                if let Some(mmr) = mmr {
                    state.mmr = mmr;
                }
            }
            ManageAction::PlayerRankApply { restore }
                if self.tab == Tab::Spoofer
                    && self.spoofer_subtab == SpooferSubTab::TitleRank
                    && self.web_rank_ready() =>
            {
                self.update_rank_display(self.swapper_cooked_path(), restore, ctx);
            }
            ManageAction::LoadoutsReloadCatalogs
                if self.tab == Tab::Items
                    && self.items_subtab == super::ItemsSubTab::Loadouts
                    && self.catalogs_error.is_some() =>
            {
                self.reload_catalogs(ctx);
            }
            ManageAction::Loadouts { action: crate::ui::loadouts::web::WebAction::CustomColour { team, primary, rgb } }
                if self.tab == Tab::Items && matches!(self.items_subtab, super::ItemsSubTab::Loadouts | super::ItemsSubTab::PresetCreator) && team < 2 =>
            {
                self.web_loadout_custom_colour(team, primary, rgb, ctx);
            }
            ManageAction::Loadouts { action }
                if self.tab == Tab::Items && matches!(self.items_subtab, super::ItemsSubTab::Loadouts | super::ItemsSubTab::PresetCreator) =>
            {
                let refresh_inventory = action.refreshes_inventory();
                let cooked = self.swapper_cooked_path();
                if let Some(name) =
                    self.loadouts
                        .web_action(action, &mut self.swapper, &cooked, &mut self.presets, ctx)
                    && let Some(index) = self
                        .presets
                        .presets
                        .iter()
                        .position(|preset| preset.name == name)
                {
                    self.presets.selected = index;
                    self.apply_selected_preset(&cooked, &cooked.join("Backups"), ctx);
                }
                if refresh_inventory { self.owned_inventory.refresh_web(ctx, None); }
            }
            ManageAction::FreeplayToggle { enabled } if self.tab == Tab::Freeplay => {
                self.freeplay
                    .web_toggle(&self.config.settings.rl_path, enabled, ctx);
            }
            ManageAction::FreeplayPick { index, value } if self.tab == Tab::Freeplay => {
                self.freeplay
                    .web_pick(&self.config.settings.rl_path, Some((index, value)));
            }
            ManageAction::FreeplayReset if self.tab == Tab::Freeplay => {
                self.freeplay.web_pick(&self.config.settings.rl_path, None);
            }
            ManageAction::PlayerName { name, commit }
                if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Username =>
            {
                self.spoofer_username = name.chars().take(spoofer::MAX_NAME_LENGTH).collect();
                self.spoofer_mgr.set_username(
                    if self.http_spoofer_ready() && self.spoofer_username_enabled {
                        &self.spoofer_username
                    } else {
                        ""
                    },
                );
                if commit {
                    self.remember_username_spoof();
                    self.save_config();
                }
            }
            ManageAction::PlayerNameEnabled { enabled }
                if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Username =>
            {
                self.spoofer_username_enabled = enabled;
                if enabled {
                    self.remember_username_spoof();
                }
                self.save_config();
                self.evaluate_proxies();
            }
            ManageAction::FriendNamesEnabled { enabled }
                if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Friends =>
            {
                self.spoofer_friends_enabled = enabled;
                self.save_friends();
                self.evaluate_proxies();
            }
            ManageAction::FriendName { id, enabled, name }
                if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Friends =>
            {
                if let Some(state) = self.spoofer_friends.get_mut(&id) {
                    if let Some(enabled) = enabled {
                        state.enabled = enabled;
                    }
                    if let Some(name) = name {
                        state.spoofed_name = name;
                    }
                    self.save_friends();
                }
            }
            ManageAction::RevertFriendNames
                if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Friends =>
            {
                for state in self.spoofer_friends.values_mut() {
                    state.enabled = false;
                    state.spoofed_name.clear();
                }
                self.save_friends();
            }
            ManageAction::Scoreboard { key, value }
                if self.tab == Tab::Spoofer && self.spoofer_subtab == SpooferSubTab::Scoreboard =>
            {
                if self
                    .scoreboard
                    .web_change(&mut self.config.scoreboard, &key, &value, ctx)
                {
                    self.save_config();
                }
            }
            ManageAction::ActionButtonVisibility { enabled } if self.tab == Tab::Settings => {
                self.config.action_button.show_in_header = enabled;
                self.save_config();
            }
            ManageAction::ActionButtonToggle {
                running,
                index,
                enabled,
            } if self.tab == Tab::Settings => {
                let entries = self.web_action_entries(running);
                if index < entries.len()
                    && (enabled
                        || !entries[index].enabled
                        || entries.iter().filter(|entry| entry.enabled).count() > 1)
                {
                    entries[index].enabled = enabled;
                    self.save_config();
                }
            }
            ManageAction::ActionButtonMove { running, from, to } if self.tab == Tab::Settings => {
                let entries = self.web_action_entries(running);
                if from != to && from < entries.len() && to < entries.len() {
                    let entry = entries.remove(from);
                    entries.insert(to, entry);
                    self.save_config();
                }
            }
            _ if self.tab != Tab::Plugins => {}
            ManageAction::PluginPreviews { ids }
                if self.plugins_subtab == PluginsSubTab::Browse =>
            {
                self.plugin_web_preview_ids = ids.into_iter().take(24).collect();
            }
            ManageAction::OverlayMove { id, step }
                if self.plugins_subtab == PluginsSubTab::Installed && matches!(step, -1 | 1) =>
            {
                let mut rows = self.web_overlay_order();
                let Some(index) = rows.iter().position(|(slug, _)| slug == &id) else {
                    return;
                };
                let Some(target) = index
                    .checked_add_signed(isize::from(step))
                    .filter(|position| *position < rows.len())
                else {
                    return;
                };
                rows.swap(index, target);
                self.config.overlay_order = rows.into_iter().rev().map(|(slug, _)| slug).collect();
                self.save_config();
            }
            ManageAction::PluginToggle { id, enabled } => {
                if self
                    .plugin_mgr
                    .plugins
                    .iter()
                    .any(|plugin| plugin.slug == id && plugin.load_error.is_none())
                {
                    self.plugin_mgr.set_enabled(&id, enabled, &mut self.config);
                    self.save_config();
                }
            }
            ManageAction::PluginSettings { id } => {
                if self
                    .plugin_mgr
                    .plugins
                    .iter()
                    .any(|plugin| plugin.slug == id && plugin.enabled && plugin.has_settings())
                {
                    self.selected_settings_plugin = Some(id);
                    self.plugins_subtab = PluginsSubTab::Settings;
                }
            }
            ManageAction::PluginDelete { id }
                if self.plugins_subtab == PluginsSubTab::Installed =>
            {
                if self
                    .plugin_mgr
                    .plugins
                    .iter()
                    .any(|plugin| plugin.slug == id)
                {
                    if let Err(error) = self.plugin_mgr.delete_plugin(&id, &mut self.config) {
                        self.console.write(format!("[Plugins] {error}"));
                    }
                    self.save_config();
                }
            }
            ManageAction::PluginInstall { id } if self.plugins_subtab == PluginsSubTab::Browse => {
                if self.install_modal.downloading_id.is_none()
                    && self
                        .install_modal
                        .catalog
                        .iter()
                        .any(|entry| plugin_id(entry) == Some(id.as_str()))
                {
                    self.download_hebnix_plugin(&id);
                }
            }
            ManageAction::PluginCommand { key } => match key.as_str() {
                "folder" => {
                    let _ = open::that(&self.plugin_dir);
                }
                "reload" => {
                    self.plugin_mgr.reload_all(&mut self.config);
                    self.save_config();
                }
                "refresh" => self.fetch_hebnix_plugin(),
                "refresh_installed" => {
                    self.plugin_mgr.refresh(&mut self.config, true);
                    self.save_config();
                }
                "zip" => {
                    let dialog = rfd::FileDialog::new()
                        .add_filter(t("install-modal-plugin-archives"), &["zip"]);
                    if let Some(file) = winutil::parent_file_dialog(dialog).pick_file() {
                        match install_zip(&file, &self.plugin_dir) {
                            Ok(()) => {
                                self.plugin_mgr.refresh(&mut self.config, true);
                                self.save_config();
                            }
                            Err(error) => self.console.write(format!("[Plugins] {error}")),
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn plugin_id(entry: &Value) -> Option<&str> {
    entry
        .get("plugin_id")
        .or_else(|| entry.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

const RANK_PLAYLIST_ICONS: [egui::ImageSource<'static>; 10] = [
    egui::include_image!("../assets/ingame-rank/Playlists/0.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/1.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/2.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/3.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/4.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/5.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/6.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/7.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/8.png"),
    egui::include_image!("../assets/ingame-rank/Playlists/9.png"),
];
const RANK_TIER_ICONS: [egui::ImageSource<'static>; 23] = [
    egui::include_image!("../assets/ingame-rank/Tiers/0.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/1.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/2.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/3.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/4.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/5.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/6.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/7.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/8.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/9.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/10.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/11.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/12.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/13.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/14.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/15.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/16.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/17.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/18.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/19.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/20.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/21.png"),
    egui::include_image!("../assets/ingame-rank/Tiers/22.png"),
];

/// Picker state kept while the popup is open: playlist id, tier, division, MMR text.
type RankDraft = (i32, i32, u8, String);

impl HebnixApp {
    /// Rank spoofing as the web page draws it: a tile per playlist and a tier-picker popup.
    pub(super) fn render_rank_tiles(&mut self, ui: &mut egui::Ui) {
        let draft_id = egui::Id::new("rank_draft");
        let mut draft: Option<RankDraft> = ui.ctx().data(|d| d.get_temp(draft_id));
        let busy = self.rank_patch.busy();
        let line = ui.visuals().widgets.noninteractive.bg_stroke;
        let accent = ui.visuals().selection.stroke.color;
        let muted = ui.visuals().weak_text_color();
        ui.label("Choose the rank, division and MMR shown locally in Rocket League.");
        ui.label(egui::RichText::new("Close Rocket League before applying. Untick a playlist and apply to restore its display.").small().color(muted));
        ui.add_space(10.0);
        let columns = 5usize;
        let gap = 12.0;
        let width = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor();
        let numerals = ["I", "II", "III", "IV"];
        egui::Grid::new("rank_tiles").spacing([gap, gap]).show(ui, |ui| {
            for (index, &(id, name, _)) in super::RANK_SPOOF_PLAYLISTS.iter().enumerate() {
                let state = self.spoofer_ranks.entry(id).or_default();
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 242.0), egui::Sense::click());
                let hover = ui.ctx().animate_bool_with_time(response.id.with("h"), response.hovered() && !busy, 0.12);
                let border = if state.enabled { accent } else { line.color.lerp_to_gamma(accent, hover) };
                ui.painter().rect_stroke(rect, 12.0, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
                let icon = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 62.0), egui::Vec2::splat(76.0));
                egui::Image::new(RANK_PLAYLIST_ICONS[index.min(9)].clone()).fit_to_exact_size(icon.size()).paint_at(ui, icon);
                ui.painter().text(egui::pos2(rect.center().x, rect.top() + 128.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(14.0), ui.visuals().text_color());
                let division = if state.rank > 0 && state.rank < 22 { numerals[usize::from(state.division).min(3)].to_owned() } else { "—".to_owned() };
                let mmr = if state.mmr.trim().is_empty() { "—".to_owned() } else { state.mmr.clone() };
                for (row, (label, value)) in [("Rank", super::rank_name(state.rank).to_owned()), ("Div", division), ("MMR", mmr)].into_iter().enumerate() {
                    let y = rect.bottom() - 63.0 + row as f32 * 21.0 + 10.0;
                    ui.painter().text(egui::pos2(rect.left() + 10.0, y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.0), muted);
                    ui.painter().text(egui::pos2(rect.right() - 10.0, y), egui::Align2::RIGHT_CENTER, value, egui::FontId::proportional(12.0), ui.visuals().text_color());
                }
                let check = egui::Rect::from_min_size(egui::pos2(rect.right() - 30.0, rect.top() + 8.0), egui::Vec2::splat(22.0));
                let toggled = ui.put(check, egui::Checkbox::without_text(&mut state.enabled)).hovered();
                if response.clicked() && !busy && !toggled {
                    draft = Some((id, state.rank, state.division, state.mmr.clone()));
                }
                if (index + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
        ui.add_space(12.0);
        let ready = self.web_rank_ready();
        let mut action = None;
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 292.0) / 2.0).max(0.0));
            if ui.add_enabled(ready, egui::Button::new(t("spoofer-apply-ranks")).fill(accent.gamma_multiply(0.6)).min_size(egui::vec2(140.0, 30.0)))
                .on_disabled_hover_text("Set the Rocket League folder and close the game first.").clicked()
            {
                action = Some(false);
            }
            if ui.add_enabled(ready, egui::Button::new(t("spoofer-restore-ranks")).min_size(egui::vec2(140.0, 30.0)))
                .on_hover_text(t("spoofer-restore-ranks-help")).clicked()
            {
                action = Some(true);
            }
        });
        if busy {
            ui.horizontal(|ui| { ui.spinner(); ui.label(&self.rank_patch.status); });
        } else if !self.rank_patch.status.is_empty() {
            ui.label(egui::RichText::new(&self.rank_patch.status).small());
        }
        if let Some(restore) = action {
            self.update_rank_display(self.swapper_cooked_path(), restore, ui.ctx());
        }

        let mut apply = None;
        let mut close = false;
        if let Some((id, rank, division, mmr)) = draft.as_mut() {
            let ctx = ui.ctx().clone();
            let screen = ctx.viewport_rect();
            egui::Area::new(egui::Id::new("rank_scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(&ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));
                close |= response.clicked();
            });
            let title = super::RANK_SPOOF_PLAYLISTS.iter().find(|row| row.0 == *id).map_or("", |row| row.1);
            egui::Area::new(egui::Id::new("rank_picker")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
                    ui.set_width(760.0f32.min(screen.width() - 40.0));
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(title).size(18.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("×").clicked() { close = true; }
                        });
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().max_height((screen.height() * 0.5).min(380.0)).show(ui, |ui| {
                        let columns = 6usize;
                        let gap = 8.0;
                        let tile_w = ((ui.available_width() - gap * 5.0) / columns as f32).floor();
                        egui::Grid::new("rank_tiers").spacing([gap, gap]).show(ui, |ui| {
                            for tier in 0..=22i32 {
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(tile_w, 92.0), egui::Sense::click());
                                let selected = *rank == tier;
                                let hover = ui.ctx().animate_bool_with_time(response.id.with("h"), response.hovered(), 0.1);
                                let fill = if selected { ui.visuals().widgets.hovered.weak_bg_fill } else { ui.visuals().faint_bg_color.lerp_to_gamma(ui.visuals().widgets.hovered.weak_bg_fill, hover) };
                                let border = if selected { accent } else { line.color.lerp_to_gamma(accent, hover) };
                                ui.painter().rect(rect, 8.0, fill, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
                                let icon = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 37.0), egui::vec2(80.0f32.min(tile_w - 8.0), 58.0));
                                egui::Image::new(RANK_TIER_ICONS[tier as usize].clone()).fit_to_exact_size(icon.size()).paint_at(ui, icon);
                                ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - 14.0), egui::Align2::CENTER_CENTER, super::rank_name(tier), egui::FontId::proportional(12.0), ui.visuals().text_color());
                                if response.clicked() {
                                    *rank = tier;
                                    *mmr = ((super::default_mu_for_tier(tier) * 20.0 + 100.0) as u32).to_string();
                                    if tier == 0 || tier == 22 { *division = 0; }
                                }
                                if (tier as usize + 1) % columns == 0 { ui.end_row(); }
                            }
                        });
                    });
                    ui.add_space(10.0);
                    let names = ["Division I", "Division II", "Division III", "Division IV"];
                    ui.columns(2, |cols| {
                        cols[0].label("Division");
                        cols[0].add_enabled_ui(*rank != 0 && *rank != 22, |ui| {
                            egui::ComboBox::from_id_salt("rank_picker_division")
                                .selected_text(names[usize::from(*division).min(3)])
                                .width(ui.available_width())
                                .show_ui(ui, |ui| {
                                    for (value, name) in names.into_iter().enumerate() {
                                        ui.selectable_value(division, value as u8, name);
                                    }
                                });
                        });
                        cols[1].label("MMR");
                        cols[1].add(egui::TextEdit::singleline(mmr).desired_width(f32::INFINITY));
                    });
                    ui.add_space(4.0);
                    let recommended = (super::default_mu_for_tier(*rank) * 20.0 + 100.0) as u32;
                    ui.label(egui::RichText::new(format!("Recommended MMR: {recommended}. You can enter a custom value.")).small().color(muted));
                    ui.add_space(8.0);
                    let valid = mmr.trim().parse::<u32>().is_ok_and(|value| (100..=9999).contains(&value));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(valid && !busy, egui::Button::new("Save").fill(accent.gamma_multiply(0.6)).min_size(egui::vec2(90.0, 28.0))).clicked() {
                            apply = Some((*id, *rank, *division, mmr.trim().to_owned()));
                        }
                        if ui.add(egui::Button::new("Cancel").min_size(egui::vec2(90.0, 28.0))).clicked() { close = true; }
                    });
                });
            });
            if ctx.input(|input| input.key_pressed(egui::Key::Escape)) { close = true; }
        }
        if let Some((id, rank, division, mmr)) = apply {
            let state = self.spoofer_ranks.entry(id).or_default();
            state.rank = rank;
            state.division = if (1..22).contains(&rank) { division } else { 0 };
            state.mmr = mmr;
            draft = None;
        } else if close {
            draft = None;
        }
        ui.ctx().data_mut(|d| match draft {
            Some(value) => { d.insert_temp(draft_id, value); }
            None => d.remove::<RankDraft>(draft_id),
        });
    }
}

/// Bordered section like the web `tracker-detail` panel.
fn web_card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    egui::Frame::NONE
        .stroke(line)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn web_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(16.0).strong());
}

fn web_muted(ui: &mut egui::Ui, text: &str) {
    let muted = ui.visuals().weak_text_color();
    ui.label(egui::RichText::new(text).size(13.0).color(muted));
}

/// Checkbox row; returns the new value when it changed.
fn web_toggle(ui: &mut egui::Ui, label: &str, value: bool, enabled: bool) -> Option<bool> {
    let mut checked = value;
    let changed = ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, label)).changed();
    changed.then_some(checked)
}

impl HebnixApp {
    /// Pages with an egui view driven by the web snapshot and the web action handler.
    pub(super) fn egui_manage_supported(&self) -> bool {
        match self.tab {
            Tab::Spoofer => matches!(
                self.spoofer_subtab,
                SpooferSubTab::Username | SpooferSubTab::Friends | SpooferSubTab::Scoreboard
            ),
            Tab::Settings => self.hebnix_settings_tab == HebnixSettingsTab::ActionButton,
            Tab::GameMods | Tab::Freeplay => true,
            Tab::Workshop => matches!(self.workshop.view, crate::ui::workshop::WorkshopView::Import | crate::ui::workshop::WorkshopView::MenuMap | crate::ui::workshop::WorkshopView::InGameMenu | crate::ui::workshop::WorkshopView::Multiplayer),
            _ => false,
        }
    }

    fn send_manage(&mut self, message: Value, ctx: &egui::Context) {
        match serde_json::from_value::<ManageAction>(message) {
            Ok(action) => self.handle_modern_manager(action, ctx),
            Err(error) => tracing::debug!(%error, "egui page produced an unknown action"),
        }
    }

    /// Draws the page named by `data["page"]`; false leaves it to the older egui view.
    fn show_manage_egui(&mut self, ui: &mut egui::Ui, data: &Value) -> bool {
        let mut actions: Vec<Value> = Vec::new();
        let page = data["page"].as_str().unwrap_or_default();
        let mut handled = true;
        egui::ScrollArea::vertical().id_salt(("manage_page", page)).auto_shrink([false, false]).show(ui, |ui| {
            match page {
                "player-name" => Self::egui_player_name(ui, data, &mut actions),
                "player-friends" => Self::egui_friends(ui, data, &mut actions),
                "player-scoreboard" => Self::egui_scoreboard(ui, data, &mut actions),
                "action-button" => Self::egui_action_buttons(ui, data, &mut actions),
                "game-mods" => Self::egui_game_mods(ui, data, &mut actions),
                "freeplay" => Self::egui_freeplay(ui, data, &mut actions),
                "add-maps" => Self::egui_add_maps(ui, data, &mut actions),
                "workshop-menu" => Self::egui_workshop_menu(ui, data, &mut actions),
                "multiplayer" => Self::egui_multiplayer(ui, data, &mut actions),
                "menu-map" => Self::egui_menu_map(ui, data, &mut actions),
                _ => handled = false,
            }
        });
        let ctx = ui.ctx().clone();
        for action in actions {
            self.send_manage(action, &ctx);
        }
        handled
    }

    fn egui_player_name(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let max = data["max"].as_u64().unwrap_or(16) as usize;
        let id = egui::Id::new("player_name_buffer");
        let mut name: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| data["name"].as_str().unwrap_or_default().to_owned());
        let width = ui.available_width().min(760.0);
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            web_card(ui, |ui| {
                ui.set_width(width - 34.0);
                ui.vertical(|ui| {
                    web_muted(ui, "Choose your display name");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if let Some(enabled) = web_toggle(ui, "Display Name", data["enabled"].as_bool().unwrap_or(false), true) {
                            actions.push(json!({ "type": "player_name_enabled", "enabled": enabled }));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            web_muted(ui, &format!("{} / {max}", name.chars().count()));
                        });
                    });
                    let response = ui.add(egui::TextEdit::singleline(&mut name).horizontal_align(egui::Align::Center).desired_width(f32::INFINITY).margin(egui::vec2(10.0, 8.0)));
                    if response.changed() {
                        name = name.chars().take(max).collect();
                        actions.push(json!({ "type": "player_name", "name": name, "commit": false }));
                    }
                    if response.lost_focus() {
                        actions.push(json!({ "type": "player_name", "name": name, "commit": true }));
                    }
                    ui.add_space(8.0);
                    web_heading(ui, "Previous names");
                    ui.add_space(4.0);
                    let history = data["history"].as_array().cloned().unwrap_or_default();
                    if history.is_empty() {
                        web_muted(ui, "No previous names yet.");
                    } else {
                        let gap = 8.0;
                        let cell = ((ui.available_width() - gap * 2.0) / 3.0).floor();
                        egui::Grid::new("player_name_history").spacing([gap, gap]).show(ui, |ui| {
                            for (index, entry) in history.iter().enumerate() {
                                let entry = entry.as_str().unwrap_or_default();
                                let accent = ui.visuals().selection.stroke.color;
                                let mut button = egui::Button::new(egui::RichText::new(entry)).truncate().min_size(egui::vec2(cell, 30.0));
                                if entry == name {
                                    button = button.fill(accent.gamma_multiply(0.5));
                                }
                                if ui.add_sized([cell, 30.0], button).on_hover_text(entry).clicked() {
                                    name = entry.to_owned();
                                    actions.push(json!({ "type": "player_name", "name": entry, "commit": true }));
                                }
                                if (index + 1) % 3 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                    }
                    ui.add_space(10.0);
                    web_muted(ui, "Names are saved automatically.");
                });
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, name));
    }

    fn egui_friends(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let id = egui::Id::new("friends_search");
        let mut term: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
        ui.horizontal(|ui| {
            if let Some(enabled) = web_toggle(ui, data["toggle"].as_str().unwrap_or_default(), data["enabled"].as_bool().unwrap_or(false), true) {
                actions.push(json!({ "type": "friend_names_enabled", "enabled": enabled }));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(data["revert"].as_str().unwrap_or_default()).clicked() {
                    actions.push(json!({ "type": "revert_friend_names" }));
                }
                ui.add(egui::TextEdit::singleline(&mut term).hint_text(data["search"].as_str().unwrap_or_default()).desired_width(220.0).margin(egui::vec2(8.0, 6.0)));
            });
        });
        ui.add_space(6.0);
        let lower = term.trim().to_lowercase();
        web_card(ui, |ui| {
            let rows = data["rows"].as_array().cloned().unwrap_or_default();
            if rows.is_empty() {
                web_muted(ui, data["empty"].as_str().unwrap_or_default());
            }
            for friend in rows.iter().filter(|friend| {
                ["name", "alias"].iter().any(|key| friend[key].as_str().unwrap_or_default().to_lowercase().contains(&lower))
            }) {
                let friend_id = friend["id"].clone();
                let mut enabled = friend["enabled"].as_bool().unwrap_or(false);
                let mut alias = friend["alias"].as_str().unwrap_or_default().to_owned();
                let mut original = friend["name"].as_str().unwrap_or_default().to_owned();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut enabled, "").changed() {
                        actions.push(json!({ "type": "friend_name", "id": friend_id, "enabled": enabled }));
                    }
                    ui.add(egui::TextEdit::singleline(&mut original).desired_width(160.0).interactive(false).margin(egui::vec2(8.0, 5.0)));
                    let edit = ui.add(egui::TextEdit::singleline(&mut alias).hint_text(data["placeholder"].as_str().unwrap_or_default()).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 5.0)));
                    if edit.lost_focus() && edit.changed() {
                        actions.push(json!({ "type": "friend_name", "id": friend_id, "name": alias }));
                    }
                });
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, term));
    }

    fn egui_scoreboard(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let ready = data["ready"].as_bool().unwrap_or(false);
        let mut set = |key: &str, value: Value| actions.push(json!({ "type": "scoreboard", "key": key, "value": value }));
        web_card(ui, |ui| {
            web_heading(ui, &label("names"));
            if let Some(value) = web_toggle(ui, &label("enableNames"), data["names"].as_bool().unwrap_or(false), ready) {
                set("names", json!(value));
            }
            if let Some(value) = web_toggle(ui, &label("ignore"), data["ignore"].as_bool().unwrap_or(false), true) {
                set("ignore", json!(value));
            }
        });
        ui.add_space(12.0);
        web_card(ui, |ui| {
            web_heading(ui, &label("mmr"));
            if let Some(value) = web_toggle(ui, &label("enableMmr"), data["mmr"].as_bool().unwrap_or(false), ready) {
                set("mmr", json!(value));
            }
            let source = data["source"].as_str().unwrap_or_default();
            for mode in ["playlist", "gamemode", "manual"] {
                if ui.radio(source == mode, label(mode)).clicked() && source != mode {
                    set("source", json!(mode));
                }
            }
            if source == "manual" {
                let current = data["playlist"].as_i64();
                let playlists = data["playlists"].as_array().cloned().unwrap_or_default();
                let selected = playlists
                    .iter()
                    .find(|entry| entry[0].as_i64() == current)
                    .and_then(|entry| entry[1].as_str())
                    .map_or_else(|| label("choose"), str::to_owned);
                egui::ComboBox::from_id_salt("scoreboard_playlist").selected_text(selected).show_ui(ui, |ui| {
                    for entry in &playlists {
                        let id = entry[0].as_i64();
                        if ui.selectable_label(id == current, entry[1].as_str().unwrap_or_default()).clicked() {
                            set("playlist", json!(id));
                        }
                    }
                });
            }
        });
        let status = data["status"].as_str().filter(|status| !status.is_empty())
            .or_else(|| data["loading"].as_bool().unwrap_or(false).then_some("Loading…"));
        if let Some(status) = status {
            ui.add_space(8.0);
            web_muted(ui, status);
        }
    }

    fn egui_action_buttons(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        web_card(ui, |ui| {
            if let Some(enabled) = web_toggle(ui, "Show action button in header", data["showInHeader"].as_bool().unwrap_or(false), true) {
                actions.push(json!({ "type": "action_button_visibility", "enabled": enabled }));
            }
            ui.label(data["intro"].as_str().unwrap_or_default());
            web_muted(ui, data["hint"].as_str().unwrap_or_default());
            ui.add_space(6.0);
            let line = ui.visuals().widgets.noninteractive.bg_stroke;
            for running in [false, true] {
                let entries = data[if running { "open" } else { "closed" }].as_array().cloned().unwrap_or_default();
                let active = entries.iter().filter(|entry| entry["enabled"].as_bool().unwrap_or(false)).count();
                web_heading(ui, data[if running { "openLabel" } else { "closedLabel" }].as_str().unwrap_or_default());
                ui.add_space(4.0);
                for (index, entry) in entries.iter().enumerate() {
                    let enabled = entry["enabled"].as_bool().unwrap_or(false);
                    let blocked = enabled && active == 1;
                    egui::Frame::NONE.stroke(line).corner_radius(6.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}.", index + 1)).color(ui.visuals().weak_text_color()));
                            let toggle = web_toggle(ui, entry["label"].as_str().unwrap_or_default(), enabled, !blocked);
                            if let Some(enabled) = toggle {
                                actions.push(json!({ "type": "action_button_toggle", "running": running, "index": index, "enabled": enabled }));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(index + 1 < entries.len(), egui::Button::new("↓")).clicked() {
                                    actions.push(json!({ "type": "action_button_move", "running": running, "from": index, "to": index + 1 }));
                                }
                                if ui.add_enabled(index > 0, egui::Button::new("↑")).clicked() {
                                    actions.push(json!({ "type": "action_button_move", "running": running, "from": index, "to": index - 1 }));
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
                ui.add_space(8.0);
            }
        });
    }
}

/// Selection, search and confirmation state of the arena editor while its page is open.
#[derive(Clone, Default)]
struct ArenaView {
    full: bool,
    donor: String,
    host: String,
    donor_search: String,
    host_search: String,
    bulk_confirm: bool,
    requested: String,
}

type ArenaImages = std::collections::HashMap<String, std::sync::Arc<[u8]>>;

impl HebnixApp {
    /// The arena editor / map swapper as the web page lays it out: two searchable tile grids,
    /// action buttons, a bulk confirmation and the active changes list.
    pub(super) fn show_arena_egui(&mut self, ui: &mut egui::Ui, full: bool) {
        use crate::ui::workshop::background_changer::ArenaAction;
        let ctx = ui.ctx().clone();
        let data = self.workshop.background_changer.web_snapshot(&self.config.settings.rl_path, full, &ctx);
        let view_id = egui::Id::new("arena_view");
        let images_id = egui::Id::new("arena_images");
        let mut view: ArenaView = ctx.data(|d| d.get_temp(view_id)).unwrap_or_default();
        if view.full != full {
            view = ArenaView { full, ..Default::default() };
        }
        let mut images: ArenaImages = ctx.data(|d| d.get_temp(images_id)).unwrap_or_default();
        if let Some(previews) = data["previews"].as_object() {
            for (id, value) in previews {
                if images.contains_key(id) { continue; }
                let Some(uri) = value.as_str() else { continue };
                let Some((_, encoded)) = uri.split_once(",") else { continue };
                if let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded) {
                    images.insert(id.clone(), bytes.into());
                }
            }
        }
        let busy = data["busy"].as_bool().unwrap_or(false);
        let running = data["running"].as_bool().unwrap_or(false);
        let no_background = "__HBNX_NO_BACKGROUND__";
        let muted = ui.visuals().weak_text_color();
        let mut actions: Vec<Value> = Vec::new();
        let mut visible: Vec<String> = Vec::new();

        egui::ScrollArea::vertical().id_salt("arena_page").auto_shrink([false, false]).show(ui, |ui| {
            ui.label(egui::RichText::new(if full {
                "Choose the map you want to play, then choose the arena it will replace entirely."
            } else {
                "Choose the arena you want to play, then choose the fog, sky, and background to borrow."
            }).color(muted));
            ui.add_space(10.0);
            let gap = 16.0;
            let column = ((ui.available_width() - gap) / 2.0).floor();
            let grid_height = (ui.ctx().content_rect().height() * 0.53).clamp(240.0, 512.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for side in ["donor", "host"] {
                    let (title, placeholder) = if side == "donor" {
                        if full { ("Map you want", "Search maps") } else { ("Background you want", "Search backgrounds") }
                    } else {
                        ("Map to apply it to", "Search maps")
                    };
                    ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        web_card(ui, |ui| {
                            ui.set_width(column - 34.0);
                            web_heading(ui, title);
                            ui.add_space(4.0);
                            let search = if side == "donor" { &mut view.donor_search } else { &mut view.host_search };
                            ui.add(egui::TextEdit::singleline(search).hint_text(placeholder).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 5.0)));
                            ui.add_space(6.0);
                            let term = search.trim().to_lowercase();
                            let rows: Vec<&Value> = data[format!("{side}s")].as_array().into_iter().flatten()
                                .filter(|row| row["name"].as_str().unwrap_or_default().to_lowercase().contains(&term))
                                .collect();
                            let selected = if side == "donor" { &mut view.donor } else { &mut view.host };
                            let width = ui.available_width() - 10.0;
                            let columns = (((width + 8.0) / 128.0).floor() as usize).clamp(2, 5);
                            let tile_w = ((width - 8.0 * (columns as f32 - 1.0)) / columns as f32).floor();
                            egui::ScrollArea::vertical().id_salt(("arena_grid", side)).max_height(grid_height).min_scrolled_height(grid_height).auto_shrink([false, false]).show(ui, |ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                                egui::Grid::new(("arena_tiles", side)).spacing([8.0, 8.0]).show(ui, |ui| {
                                    for (index, row) in rows.iter().enumerate() {
                                        let id = row["id"].as_str().unwrap_or_default();
                                        let name = row["name"].as_str().unwrap_or_default();
                                        let (rect, response) = ui.allocate_exact_size(egui::vec2(tile_w, 116.0), egui::Sense::click());
                                        let on = *selected == id;
                                        let hover = ui.ctx().animate_bool_with_time(response.id.with("h"), response.hovered(), 0.1);
                                        let base = ui.visuals().widgets.inactive.weak_bg_fill;
                                        let fill = base.lerp_to_gamma(ui.visuals().widgets.hovered.weak_bg_fill, hover);
                                        let accent = ui.visuals().selection.stroke.color;
                                        let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
                                        ui.painter().rect(rect, 6.0, fill, egui::Stroke::new(1.0, if on { accent } else { line.lerp_to_gamma(accent, hover * 0.6) }), egui::StrokeKind::Inside);
                                        let caption_h = 30.0;
                                        let art = egui::Rect::from_min_max(rect.min + egui::vec2(4.0, 4.0), egui::pos2(rect.right() - 4.0, rect.bottom() - caption_h - 6.0));
                                        if ui.is_rect_visible(rect) && id != no_background { visible.push(id.to_owned()); }
                                        if id == no_background {
                                            let c = art.center();
                                            ui.painter().circle_stroke(c, 14.0, egui::Stroke::new(1.5, muted));
                                            ui.painter().line_segment([c + egui::vec2(-9.0, -9.0), c + egui::vec2(9.0, 9.0)], egui::Stroke::new(1.5, muted));
                                        } else if let Some(bytes) = images.get(id) {
                                            let source = egui::ImageSource::Bytes { uri: format!("bytes://arena/{id}").into(), bytes: egui::load::Bytes::Shared(bytes.clone()) };
                                            egui::Image::new(source).fit_to_exact_size(art.size()).corner_radius(3.0).paint_at(ui, art);
                                        } else if data["previews"].get(id).is_some_and(Value::is_null) {
                                            ui.painter().rect_stroke(egui::Rect::from_center_size(art.center(), egui::vec2(28.0, 22.0)), 3.0, egui::Stroke::new(1.5, muted), egui::StrokeKind::Inside);
                                        } else {
                                            ui.painter().text(art.center(), egui::Align2::CENTER_CENTER, "Loading preview…", egui::FontId::proportional(12.0), muted);
                                        }
                                        let cap = egui::Rect::from_min_max(egui::pos2(rect.left() + 4.0, rect.bottom() - caption_h - 4.0), rect.max - egui::vec2(4.0, 4.0));
                                        ui.painter().rect_filled(cap, 3.0, ui.visuals().extreme_bg_color.gamma_multiply(0.8));
                                        let galley = ui.painter().layout(name.to_owned(), egui::FontId::proportional(12.0), ui.visuals().text_color(), cap.width() - 8.0);
                                        let pos = egui::pos2(cap.center().x - galley.size().x / 2.0, cap.center().y - galley.size().y / 2.0);
                                        ui.painter().with_clip_rect(cap).galley(pos, galley, ui.visuals().text_color());
                                        response.clone().on_hover_text(name);
                                        if response.clicked() {
                                            *selected = id.to_owned();
                                        }
                                        if (index + 1) % columns == 0 { ui.end_row(); }
                                    }
                                });
                            });
                        });
                    });
                }
            });
            // selection must still exist after a refresh
            for (side, selected) in [("donors", &mut view.donor), ("hosts", &mut view.host)] {
                if !data[side].as_array().into_iter().flatten().any(|row| row["id"].as_str() == Some(selected.as_str())) {
                    selected.clear();
                }
            }
            let removal = view.donor == no_background;
            let can_apply = !busy && !running && !view.donor.is_empty() && !view.host.is_empty()
                && view.donor.trim_start_matches("arena:").trim_start_matches("full-map:") != view.host;
            let can_bulk = !busy && !running && !view.donor.is_empty() && !removal && data["bulk_targets"].as_u64().unwrap_or(0) > 0;
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let buttons = if full { 2.0 } else { 4.0 };
                ui.add_space(((ui.available_width() - buttons * 150.0) / 2.0).max(0.0));
                if ui.add(egui::Button::new("Refresh").min_size(egui::vec2(140.0, 28.0))).clicked() {
                    images.clear();
                    view.requested.clear();
                    actions.push(json!({ "type": "refresh" }));
                }
                if !full {
                    if ui.add(egui::Button::new("Background folder").min_size(egui::vec2(140.0, 28.0))).clicked() {
                        actions.push(json!({ "type": "folder" }));
                    }
                    if ui.add_enabled(can_bulk, egui::Button::new("Apply to all unchanged maps").min_size(egui::vec2(140.0, 28.0))).clicked() {
                        view.bulk_confirm = true;
                    }
                }
                let accent = ui.visuals().selection.stroke.color;
                let label = if removal { "Remove background" } else { "Apply" };
                if ui.add_enabled(can_apply, egui::Button::new(label).fill(accent.gamma_multiply(0.6)).min_size(egui::vec2(140.0, 28.0))).clicked() {
                    actions.push(json!({ "type": "apply", "host": view.host, "donor": view.donor }));
                }
            });
            if view.bulk_confirm && !full {
                ui.add_space(10.0);
                web_card(ui, |ui| {
                    ui.label(data["bulk_warning"].as_str().unwrap_or_default());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(can_bulk, egui::Button::new("Apply to all unchanged maps")).clicked() {
                            actions.push(json!({ "type": "apply_all", "donor": view.donor }));
                            view.bulk_confirm = false;
                        }
                        if ui.button("Cancel").clicked() { view.bulk_confirm = false; }
                    });
                });
            }
            if busy {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(data["status"].as_str().unwrap_or("Updating arena patch"));
                });
            }
            let status = if running {
                "Close Rocket League before applying or restoring changes."
            } else if !data["changed"].as_bool().unwrap_or(false) || data["status"].as_str().is_some_and(|s| s.starts_with("Restored ") && s.ends_with("original background.")) {
                ""
            } else {
                data["status"].as_str().unwrap_or_default()
            };
            if !status.is_empty() && !busy {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(status).color(muted));
            }
            let active = data["active"].as_array().cloned().unwrap_or_default();
            if !active.is_empty() {
                ui.add_space(12.0);
                web_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        web_heading(ui, "Active changes");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(!running && !busy, egui::Button::new("Restore all")).clicked() {
                                actions.push(json!({ "type": "restore_all" }));
                            }
                        });
                    });
                    for entry in &active {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(format!("{} → {}", entry[2].as_str().unwrap_or_default(), entry[1].as_str().unwrap_or_default()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(!running && !busy, egui::Button::new("Restore")).clicked() {
                                    actions.push(json!({ "type": "restore", "host": entry[0] }));
                                }
                            });
                        });
                    }
                });
            }
        });

        visible.sort();
        visible.dedup();
        visible.retain(|id| !images.contains_key(id) && !data["previews"].as_object().is_some_and(|map| map.contains_key(id)));
        let request = visible.join("\n");
        if request != view.requested {
            view.requested = request;
            if !visible.is_empty() {
                actions.push(json!({ "type": "preview", "ids": visible }));
            }
        }
        for action in actions {
            if let Ok(action) = serde_json::from_value::<ArenaAction>(action) {
                self.workshop.background_changer.web_action(action, &self.config.settings.rl_path, full, &self.tx, &ctx);
            }
        }
        ctx.data_mut(|d| {
            d.insert_temp(view_id, view);
            d.insert_temp(images_id, images);
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn val_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `[[id, label], ...]` or `["a", "b"]` as (value, label) pairs.
fn web_pairs(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| match entry.as_array() {
            Some(pair) if pair.len() >= 2 => (val_str(&pair[0]), val_str(&pair[1])),
            _ => (val_str(entry), val_str(entry)),
        })
        .collect()
}

/// Drop-down over `(value, label)` pairs; returns the picked value when it changed.
fn web_select(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[(String, String)], enabled: bool) -> Option<String> {
    let shown = options
        .iter()
        .find(|(value, _)| value == current)
        .map_or_else(|| current.to_owned(), |(_, label)| label.clone());
    let mut picked = None;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(egui::Id::new(id)).selected_text(shown).show_ui(ui, |ui| {
            for (value, label) in options {
                if ui.selectable_label(value == current, label).clicked() {
                    picked = Some(value.clone());
                }
            }
        });
    });
    picked
}

/// Bar for a running patch: a real fraction when one is known, otherwise indeterminate.
fn web_progress(ui: &mut egui::Ui, busy: bool, progress: &Value) {
    if !busy {
        return;
    }
    let fraction = progress.as_f64().or_else(|| {
        let total = progress["total"].as_f64()?;
        (total > 0.0).then(|| progress["done"].as_f64().unwrap_or(0.0) / total)
    });
    let bar = match fraction {
        Some(fraction) => egui::ProgressBar::new(fraction.clamp(0.0, 1.0) as f32),
        None => egui::ProgressBar::new(0.0).animate(true),
    };
    ui.add(bar.desired_height(8.0));
    ui.ctx().request_repaint();
}

fn web_primary(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let accent = ui.visuals().selection.stroke.color;
    ui.add_enabled(enabled, egui::Button::new(label).fill(accent.gamma_multiply(0.6))).clicked()
}

impl HebnixApp {
    fn egui_game_mods(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let capturing = data["capturing"].as_str().map(str::to_owned);
        let muted = ui.visuals().weak_text_color();
        for module in data["rows"].as_array().into_iter().flatten() {
            let id = module["id"].as_str().unwrap_or_default().to_owned();
            let busy = module["busy"].as_bool().unwrap_or(false);
            let update = |key: &str, value: Value| json!({ "type": "game_mod", "id": id, "key": key, "value": value });
            web_card(ui, |ui| {
                if let Some(enabled) = web_toggle(ui, module["label"].as_str().unwrap_or_default(), module["enabled"].as_bool().unwrap_or(false), !busy) {
                    actions.push(update("enabled", json!(enabled)));
                }
                web_muted(ui, module["description"].as_str().unwrap_or_default());
                if let Some(output) = module["output"].as_str().filter(|text| !text.is_empty()) {
                    web_muted(ui, output);
                }
                if module["keys"].is_array() {
                    let keyboard_label = module["keyboardLabel"].as_str().unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(keyboard_label);
                        let keys = web_pairs(&module["keys"]);
                        if let Some(key) = web_select(ui, ("game_mod_key", &id), module["keyboard"].as_str().unwrap_or_default(), &keys, !busy) {
                            actions.push(update("keyboard", json!(key)));
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let waiting = capturing.as_deref() == Some(id.as_str());
                        ui.label(egui::RichText::new(labels["pad"].as_str().unwrap_or_default()).strong());
                        let pad = module["pad"].as_str().unwrap_or_default();
                        let text = if waiting {
                            labels["press"].as_str().unwrap_or_default()
                        } else if pad.is_empty() {
                            labels["none"].as_str().unwrap_or_default()
                        } else {
                            pad
                        };
                        ui.label(egui::RichText::new(text).small().color(muted));
                        if ui.add_enabled(!busy && capturing.is_none(), egui::Button::new(labels["bind"].as_str().unwrap_or_default())).clicked() {
                            actions.push(update("bind", Value::Null));
                        }
                        if !pad.is_empty() && ui.add_enabled(!busy && !waiting, egui::Button::new(labels["clear"].as_str().unwrap_or_default())).clicked() {
                            actions.push(update("pad", json!("")));
                        }
                    });
                }
                if id == "instant-suite" {
                    let names = &module["labels"];
                    ui.horizontal_wrapped(|ui| {
                        if let Some(value) = web_toggle(ui, names["private"].as_str().unwrap_or_default(), module["private"].as_bool().unwrap_or(false), true) {
                            actions.push(update("private", json!(value)));
                        }
                        if let Some(value) = web_toggle(ui, names["tournaments"].as_str().unwrap_or_default(), module["tournaments"].as_bool().unwrap_or(false), true) {
                            actions.push(update("tournaments", json!(value)));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(names["mode"].as_str().unwrap_or_default());
                        let modes = web_pairs(&module["modes"]);
                        if let Some(value) = web_select(ui, "instant_mode", &val_str(&module["mode"]), &modes, true) {
                            actions.push(update("mode", json!(value)));
                        }
                    });
                    if module["maps"].is_object() {
                        if !module["maps"]["enabled"].as_bool().unwrap_or(false) {
                            ui.colored_label(ui.visuals().warn_fg_color, names["needsMenu"].as_str().unwrap_or_default());
                        }
                        ui.horizontal(|ui| {
                            ui.label(names["map"].as_str().unwrap_or_default());
                            let mut maps = vec![(String::new(), names["pick"].as_str().unwrap_or_default().to_owned())];
                            maps.extend(web_pairs(&module["maps"]["items"]));
                            if let Some(value) = web_select(ui, "instant_map", &val_str(&module["map"]), &maps, true).filter(|value| !value.is_empty()) {
                                actions.push(update("map", json!(value)));
                            }
                            if ui.button(names["reload"].as_str().unwrap_or_default()).clicked() {
                                actions.push(update("reload", Value::Null));
                            }
                        });
                    }
                }
                let status = module["status"].as_str().unwrap_or_default();
                if busy || !status.is_empty() {
                    web_muted(ui, if status.is_empty() { "Working…" } else { status });
                }
                web_progress(ui, busy, &module["progress"]);
            });
            ui.add_space(12.0);
        }
    }

    fn egui_freeplay(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let enabled = data["enabled"].as_bool().unwrap_or(false);
        let mut settings: Vec<&Value> = data["rows"].as_array().into_iter().flatten().collect();
        settings.sort_by_key(|setting| setting["label"].as_str().unwrap_or_default().to_lowercase());
        web_card(ui, |ui| {
            if let Some(checked) = web_toggle(ui, "Enable Freeplay Mutators", data["checked"].as_bool().unwrap_or(false), data["canToggle"].as_bool().unwrap_or(false)) {
                actions.push(json!({ "type": "freeplay_toggle", "enabled": checked }));
            }
            if data["patchMissing"].as_bool().unwrap_or(false) {
                ui.colored_label(ui.visuals().warn_fg_color, "TAGame.upk no longer contains the current Freeplay patch. Close Rocket League and enable Freeplay Mutators again.");
            }
            for setting in settings.iter().filter(|setting| setting["group"] == "mode") {
                Self::egui_freeplay_row(ui, setting, enabled, actions);
            }
            if data["heatseeker"].as_bool().unwrap_or(false) {
                web_muted(ui, "Use a Soccar arena and pick a team when the Heatseeker practice lobby opens.");
            }
            if data["lanSession"].as_bool().unwrap_or(false) {
                web_muted(ui, "This session uses Maps > In-game menu controls.");
            }
            if ui.add_enabled(enabled, egui::Button::new("Reset to defaults")).clicked() {
                actions.push(json!({ "type": "freeplay_reset" }));
            }
            if let Some(status) = data["status"].as_str().filter(|text| !text.is_empty()) {
                web_muted(ui, status);
            }
        });
        ui.add_space(12.0);
        let gap = 16.0;
        let column = ((ui.available_width() - gap) / 2.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (group, title) in [("ball", "Ball"), ("rules", "Cars and Rules")] {
                ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    web_card(ui, |ui| {
                        ui.set_width(column - 34.0);
                        web_heading(ui, title);
                        ui.add_space(4.0);
                        for setting in settings.iter().filter(|setting| setting["group"] == group) {
                            Self::egui_freeplay_row(ui, setting, enabled, actions);
                        }
                    });
                });
            }
        });
    }

    fn egui_freeplay_row(ui: &mut egui::Ui, setting: &Value, enabled: bool, actions: &mut Vec<Value>) {
        let options: Vec<(String, String)> = setting["options"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, name)| (index.to_string(), val_str(name)))
            .collect();
        let index = setting["index"].as_u64().unwrap_or(0);
        ui.horizontal(|ui| {
            ui.label(setting["label"].as_str().unwrap_or_default());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(value) = web_select(ui, ("freeplay", index), &val_str(&setting["pick"]), &options, enabled) {
                    if let Ok(value) = value.parse::<u64>() {
                        actions.push(json!({ "type": "freeplay_pick", "index": index, "value": value }));
                    }
                }
            });
        });
    }
}

/// Dimmed backdrop plus a centred popup card; `add` fills the card.
fn web_modal(ctx: &egui::Context, id: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let screen = ctx.viewport_rect();
    egui::Area::new(egui::Id::new((id, "scrim"))).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));
    });
    egui::Area::new(egui::Id::new((id, "card"))).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
            ui.set_width(width.min(screen.width() - 40.0));
            add(ui);
        });
    });
}

impl HebnixApp {
    fn egui_add_maps(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let local = &data["local"];
        let labels = &local["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let menu_busy = data["menu_busy"].as_bool().unwrap_or(false);
        let locked = data["menu_disabled"].as_bool().unwrap_or(false) || menu_busy;
        let action = |kind: &str, key: &str, fields: Option<&Vec<String>>| {
            let mut value = json!({ "type": "add_maps", "action": { "type": kind, "key": key } });
            if let Some(fields) = fields {
                value["action"]["fields"] = json!(fields);
            }
            value
        };
        let buffer_id = egui::Id::new("add_maps_fields");
        let file = local["file"].as_str().unwrap_or_default().to_owned();
        let mut buffer: (String, Vec<String>) = ui.ctx().data(|d| d.get_temp(buffer_id)).unwrap_or_default();
        if buffer.0 != file || buffer.1.len() != 3 {
            buffer = (file.clone(), local["fields"].as_array().into_iter().flatten().map(val_str).collect());
            buffer.1.resize(3, String::new());
        }
        let has_details = !(local["details"].is_null() || local["details"] == false);
        ui.add_enabled_ui(!locked, |ui| {
            web_card(ui, |ui| {
                web_heading(ui, &label("title"));
                ui.label(label("intro"));
                ui.add_space(6.0);
                if !has_details {
                    web_heading(ui, &label("file"));
                    web_muted(ui, &label("types"));
                    if let Some(done) = local["done"].as_str().filter(|text| !text.is_empty()) {
                        ui.colored_label(ui.visuals().hyperlink_color, done);
                    }
                    if web_primary(ui, &label("choose"), true) {
                        actions.push(action("local", "file", None));
                    }
                } else {
                    web_heading(ui, &label("details"));
                    ui.label(&file);
                    ui.add_space(4.0);
                    for (index, key) in ["name", "author", "description"].into_iter().enumerate() {
                        ui.label(label(key));
                        let edit = if index == 2 {
                            egui::TextEdit::multiline(&mut buffer.1[index]).desired_rows(3)
                        } else {
                            egui::TextEdit::singleline(&mut buffer.1[index])
                        };
                        ui.add(edit.desired_width(f32::INFINITY).margin(egui::vec2(8.0, 6.0)));
                        ui.add_space(4.0);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(label("vdf")).clicked() {
                            actions.push(action("local", "vdf", Some(&buffer.1)));
                        }
                        if ui.button(label("image")).clicked() {
                            actions.push(action("local", "image", Some(&buffer.1)));
                        }
                        let banner = local["banner"].as_str().unwrap_or_default();
                        ui.label(egui::RichText::new(if banner.is_empty() { label("optional") } else { banner.to_owned() }).small());
                        if !banner.is_empty() && ui.button(label("clear")).clicked() {
                            actions.push(action("local", "clear", Some(&buffer.1)));
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(label("back")).clicked() {
                            actions.push(action("local", "back", Some(&buffer.1)));
                        }
                        if web_primary(ui, &label("import"), !buffer.1[0].trim().is_empty()) {
                            actions.push(action("local", "import", Some(&buffer.1)));
                        }
                    });
                }
                if let Some(notice) = local["notice"].as_str().filter(|text| !text.is_empty()) {
                    web_muted(ui, notice);
                }
                if let Some(error) = local["error"].as_str().filter(|text| !text.is_empty()) {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        });
        if locked {
            let lock = &data["menu_lock"];
            web_modal(ui.ctx(), "add_maps_lock", 420.0, |ui| {
                web_heading(ui, lock["title"].as_str().unwrap_or_default());
                ui.add_space(6.0);
                ui.label(lock["body"].as_str().unwrap_or_default());
                ui.add_space(8.0);
                let text = if menu_busy { lock["busy"].as_str() } else { lock["enable"].as_str() }.unwrap_or_default();
                if web_primary(ui, text, !menu_busy) {
                    actions.push(json!({ "type": "add_maps", "action": { "type": "enable_menu" } }));
                }
            });
        }
        ui.ctx().data_mut(|d| d.insert_temp(buffer_id, buffer));
    }
}

/// `{type: outer, action: {type: kind, ...extra}}`, the shape the web pages post.
fn web_act(outer: &str, kind: &str, extra: Value) -> Value {
    let mut action = json!({ "type": kind });
    if let Some(map) = extra.as_object() {
        for (key, value) in map {
            action[key] = value.clone();
        }
    }
    json!({ "type": outer, "action": action })
}

/// Number box that commits on release / Enter, like the web number inputs.
fn web_number(ui: &mut egui::Ui, value: f64, range: Option<(f64, f64)>, step: f64, enabled: bool) -> Option<f64> {
    let mut edited = value;
    let mut drag = egui::DragValue::new(&mut edited).speed(step);
    if let Some((low, high)) = range {
        drag = drag.range(low..=high);
    }
    let response = ui.add_enabled(enabled, drag);
    ((response.drag_stopped() || response.lost_focus()) && edited != value).then_some(edited)
}

fn web_row<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.label(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add).inner
    })
    .inner
}

impl HebnixApp {
    fn egui_workshop_menu(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let send = |kind: &str, extra: Value| web_act("workshop_menu", kind, extra);
        let busy = data["busy"].as_bool().unwrap_or(false);
        let lan_only = data["lanOnly"].as_bool().unwrap_or(false);
        if data["freeplay"].as_bool().unwrap_or(false) {
            web_muted(ui, "The active practice session uses the Freeplay tab. LAN settings below apply to Workshop and LAN matches.");
        }
        let gap = 16.0;
        let column = ((ui.available_width() - gap) / 2.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(column);
                if !lan_only {
                    web_card(ui, |ui| {
                        web_heading(ui, "In-game Workshop menu");
                        web_muted(ui, "Adds Workshop to Rocket League’s offline Play menu. Close Rocket League before changing it.");
                        let enabled = data["enabled"].as_bool();
                        if let Some(value) = web_toggle(ui, "Enable in-game menu", enabled == Some(true), !busy && enabled.is_some()) {
                            actions.push(send("enable", json!({ "enabled": value })));
                        }
                        if ui.add_enabled(!busy && enabled == Some(true), egui::Button::new("Update Workshop patch")).clicked() {
                            actions.push(send("update", Value::Null));
                        }
                        if let Some(error) = data["error"].as_str().filter(|text| !text.is_empty()) {
                            ui.colored_label(ui.visuals().warn_fg_color, error);
                        }
                        let status = data["status"].as_str().unwrap_or_default();
                        if busy || !status.is_empty() {
                            ui.label(if status.is_empty() { "Working…" } else { status });
                        }
                        web_progress(ui, busy, &data["progress"]);
                    });
                    ui.add_space(8.0);
                    web_card(ui, |ui| {
                        web_heading(ui, "Map downloader");
                        let enabled = data["enabled"].as_bool() == Some(true);
                        if let Some(value) = web_toggle(ui, "Enable map downloader", data["downloader"].as_bool().unwrap_or(false), !busy && enabled) {
                            actions.push(send("downloader", json!({ "enabled": value })));
                        }
                        web_muted(ui, "With the downloader enabled, press L3 in the Workshop menu to browse maps.");
                    });
                    ui.add_space(8.0);
                    web_card(ui, |ui| {
                        web_heading(ui, "Local maps");
                        ui.label(egui::RichText::new(data["folder"].as_str().unwrap_or_default()).small());
                        if ui.button("Rescan").clicked() {
                            actions.push(send("rescan", Value::Null));
                        }
                        ui.label(egui::RichText::new(format!("{} maps found.", data["count"])).small());
                    });
                    ui.add_space(8.0);
                }
                web_card(ui, |ui| {
                    web_heading(ui, "LAN match");
                    web_muted(ui, "Used when you host a map from the in-game Workshop menu.");
                    let sizes: Vec<(String, String)> = (1..=4).map(|size| (size.to_string(), format!("{size}v{size}"))).collect();
                    web_row(ui, "Team size", |ui| {
                        if let Some(value) = web_select(ui, "lan_team", &val_str(&data["team"]), &sizes, true).and_then(|value| value.parse::<u64>().ok()) {
                            actions.push(send("team_size", json!({ "value": value })));
                        }
                    });
                    let bots: Vec<(String, String)> = data["botChoices"].as_array().into_iter().flatten().enumerate().map(|(index, name)| (index.to_string(), val_str(name))).collect();
                    web_row(ui, "Bots", |ui| {
                        if let Some(value) = web_select(ui, "lan_bots", &val_str(&data["bots"]), &bots, true).and_then(|value| value.parse::<u64>().ok()) {
                            actions.push(send("bots", json!({ "value": value })));
                        }
                    });
                    for status in data["offers"].as_array().into_iter().flatten() {
                        ui.label(egui::RichText::new(format!("Peer map: {}", val_str(status))).small());
                    }
                });
                ui.add_space(8.0);
                web_card(ui, |ui| {
                    let id = egui::Id::new("lan_access");
                    let mut form: (bool, String, String) = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| (
                        data["custom"].as_bool().unwrap_or(false),
                        data["name"].as_str().unwrap_or_default().to_owned(),
                        data["password"].as_str().unwrap_or_default().to_owned(),
                    ));
                    web_heading(ui, "LAN match access");
                    ui.checkbox(&mut form.0, "Use custom LAN credentials");
                    ui.add_enabled(form.0, egui::TextEdit::singleline(&mut form.1).hint_text("Game default").char_limit(32).desired_width(f32::INFINITY));
                    ui.add_enabled(form.0, egui::TextEdit::singleline(&mut form.2).password(true).hint_text("No password").char_limit(32).desired_width(f32::INFINITY));
                    if ui.button("Save LAN match access").clicked() {
                        actions.push(send("access", json!({ "custom": form.0, "name": form.1, "password": form.2 })));
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(id, form));
                });
            });
            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(column);
                web_card(ui, |ui| {
                    web_heading(ui, "LAN Mutators");
                    web_muted(ui, "These settings are used when hosting a LAN match. Heatseeker needs two Soccar goals.");
                    if lan_only {
                        if let Some(status) = data["status"].as_str().filter(|text| !text.is_empty()) {
                            ui.label(status);
                        }
                    }
                    for setting in data["rows"].as_array().into_iter().flatten() {
                        let options: Vec<(String, String)> = setting["options"].as_array().into_iter().flatten().enumerate().map(|(index, name)| (index.to_string(), val_str(name))).collect();
                        let index = setting["index"].as_u64().unwrap_or(0);
                        web_row(ui, setting["label"].as_str().unwrap_or_default(), |ui| {
                            if let Some(value) = web_select(ui, ("mutator", index), &val_str(&setting["pick"]), &options, true).and_then(|value| value.parse::<u64>().ok()) {
                                actions.push(send("mutator", json!({ "index": index, "value": value })));
                            }
                        });
                    }
                    if let Some(extra) = data["extra"].as_str().filter(|text| !text.is_empty()) {
                        web_muted(ui, &format!("Other tags kept: {extra}"));
                    }
                    if ui.button("Reset to defaults").clicked() {
                        actions.push(send("reset", Value::Null));
                    }
                });
            });
        });
    }

    fn egui_multiplayer(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let send = |kind: &str, extra: Value| web_act("multiplayer", kind, extra);
        let connection = &data["connection"];
        let options = &data["options"];
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let muted = ui.visuals().weak_text_color();
        let started = connection["started"].as_bool().unwrap_or(false);
        let preference = |ui: &mut egui::Ui, actions: &mut Vec<Value>, key: &str, text: &str| {
            if let Some(enabled) = web_toggle(ui, text, options[key].as_bool().unwrap_or(false), true) {
                actions.push(send("preference", json!({ "key": key, "enabled": enabled })));
            }
        };
        if !started {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(560.0);
                web_card(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Workshop Multiplayer").size(22.0).strong());
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(label("intro")).color(muted));
                        ui.add_space(6.0);
                        preference(ui, actions, "relay", &label("relay"));
                        ui.horizontal(|ui| {
                            ui.add_space(((ui.available_width() - 220.0) / 2.0).max(0.0));
                            if web_primary(ui, &label("connect"), !connection["stopping"].as_bool().unwrap_or(false)) {
                                actions.push(send("connect", Value::Null));
                            }
                            if ui.button(label("help")).clicked() {
                                actions.push(send("help", Value::Null));
                            }
                        });
                        preference(ui, actions, "close", "Disconnect when Hebnix closes");
                        preference(ui, actions, "toasts", &label("toasts"));
                        if let Some(status) = connection["status"].as_str().filter(|text| !text.is_empty()) {
                            ui.label(egui::RichText::new(status).small());
                        }
                    });
                });
            });
            return;
        }
        let gap = 16.0;
        let left = 320.0f32.min(ui.available_width() * 0.35).floor();
        let right = (ui.available_width() - left - gap).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(egui::vec2(left, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(left - 34.0);
                    web_heading(ui, &label("setup"));
                    ui.label(connection["setup"].as_str().unwrap_or_default());
                    if connection["checking"].as_bool().unwrap_or(false) {
                        web_muted(ui, &label("checking"));
                    }
                    if connection["close"].as_bool().unwrap_or(false) && ui.button(label("closeGame")).clicked() {
                        actions.push(send("close_game", Value::Null));
                    }
                    if connection["host"].as_bool().unwrap_or(false) {
                        ui.label(egui::RichText::new(label("password")).small());
                    }
                    if let Some(detected) = connection["detected"].as_str().filter(|text| !text.is_empty()) {
                        ui.label(egui::RichText::new(detected).small());
                    }
                    ui.add_space(6.0);
                    if ui.add_enabled(connection["ready"].as_bool().unwrap_or(false), egui::Button::new("Sync Map Slots"))
                        .on_hover_text("Prepare your map slots now and load the maps other players are hosting.").clicked()
                    {
                        actions.push(send("sync", Value::Null));
                    }
                    if ui.button(label("stop")).clicked() {
                        actions.push(send("disconnect", Value::Null));
                    }
                    if ui.button(label("help")).clicked() {
                        actions.push(send("help", Value::Null));
                    }
                    ui.add_space(6.0);
                    preference(ui, actions, "close", "Disconnect when Hebnix closes");
                    preference(ui, actions, "toasts", &label("toasts"));
                });
            });
            ui.allocate_ui_with_layout(egui::vec2(right, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(right - 34.0);
                    let rows = data["rows"].as_array().cloned().unwrap_or_default();
                    let ready = connection["ready"].as_bool().unwrap_or(false);
                    ui.vertical_centered(|ui| {
                        let heading = if !ready {
                            connection["setup"].as_str().unwrap_or_default().to_owned()
                        } else if rows.is_empty() {
                            label("empty")
                        } else {
                            data["heading"].as_str().unwrap_or_default().to_owned()
                        };
                        ui.label(egui::RichText::new(heading).size(18.0).strong());
                        if ready && rows.is_empty() {
                            ui.label(egui::RichText::new("Invite another Hebnix player to connect to Workshop Multiplayer.").color(muted));
                        }
                    });
                    let search_id = egui::Id::new("multiplayer_search");
                    let mut term: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
                    if data["search"].as_bool().unwrap_or(false) {
                        ui.add(egui::TextEdit::singleline(&mut term).hint_text(label("search")).desired_width(f32::INFINITY));
                    } else {
                        term.clear();
                    }
                    let lower = term.trim().to_lowercase();
                    let mut shown = 0;
                    for peer in rows.iter().filter(|peer| peer["name"].as_str().unwrap_or_default().to_lowercase().contains(&lower)) {
                        shown += 1;
                        web_card(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(peer["name"].as_str().unwrap_or_default()).strong());
                                    if let Some(latency) = peer["latency"].as_f64() {
                                        ui.label(egui::RichText::new(format!("{} ms", latency.round())).small());
                                    }
                                });
                                if peer["map"].is_object() {
                                    ui.horizontal(|ui| {
                                        ui.label(peer["map"]["label"].as_str().unwrap_or_default());
                                        if peer["map"]["have"].as_bool().unwrap_or(false) {
                                            ui.colored_label(ui.visuals().hyperlink_color, label("have"));
                                        } else if ui.add_enabled(!data["busy"].as_bool().unwrap_or(false), egui::Button::new(label("download"))).clicked() {
                                            actions.push(send("download", json!({ "peer": peer["peer"], "map": peer["map"]["id"] })));
                                        }
                                    });
                                }
                                if !peer["peer"].is_null() && ui.button(label("block")).clicked() {
                                    actions.push(send("block", json!({ "peer": peer["peer"] })));
                                }
                            });
                        });
                        ui.add_space(6.0);
                    }
                    if shown == 0 && !rows.is_empty() {
                        web_muted(ui, &label("noMatch"));
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(search_id, term));
                    let own = data["own"].as_array().cloned().unwrap_or_default();
                    if !own.is_empty() {
                        egui::CollapsingHeader::new("Your map slots").show(ui, |ui| {
                            for slot in &own {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(egui::RichText::new(slot["label"].as_str().unwrap_or_default()).small());
                                    if let Some(status) = slot["status"].as_str().filter(|text| !text.is_empty()) {
                                        let issues = slot["issues"].as_array().map_or(0, Vec::len);
                                        let colour = if issues > 0 { ui.visuals().warn_fg_color } else { ui.visuals().hyperlink_color };
                                        ui.label(egui::RichText::new(format!("· {status}")).small().color(colour));
                                    }
                                });
                            }
                        });
                    }
                    let blocked = data["blocked"].as_array().cloned().unwrap_or_default();
                    if !blocked.is_empty() {
                        egui::CollapsingHeader::new(format!("{} blocked players", blocked.len())).show(ui, |ui| {
                            web_muted(ui, &label("blockHint"));
                            for entry in &blocked {
                                ui.horizontal(|ui| {
                                    ui.label(val_str(&entry[1]));
                                    if ui.button(label("unblock")).clicked() {
                                        actions.push(send("unblock", json!({ "peer": entry[0] })));
                                    }
                                });
                            }
                        });
                    }
                    if data["progress"].is_object() {
                        let total = data["progress"]["total"].as_f64().unwrap_or(0.0);
                        let done = data["progress"]["done"].as_f64().unwrap_or(0.0);
                        ui.label(egui::RichText::new(format!("Downloading from a player: {} / {} MB", (done / 1e6).floor(), (total / 1e6).floor())).small());
                        if total > 0.0 {
                            ui.add(egui::ProgressBar::new((done / total).clamp(0.0, 1.0) as f32));
                        }
                    } else if let Some(transfer) = data["transfer"].as_str().filter(|text| !text.is_empty()) {
                        ui.label(egui::RichText::new(transfer).small());
                    }
                    let status = connection["progress"].as_str().filter(|text| !text.is_empty()).or_else(|| connection["status"].as_str().filter(|text| !text.is_empty()));
                    if let Some(status) = status {
                        ui.label(egui::RichText::new(status).small());
                    }
                });
            });
        });
        if let Some(warning) = data["warning"].as_str().filter(|text| !text.is_empty()) {
            web_modal(ui.ctx(), "peer_install", 460.0, |ui| {
                web_heading(ui, "Install from another player?");
                ui.add_space(6.0);
                ui.label(warning);
                ui.label(label("warning"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(label("cancel")).clicked() {
                        actions.push(send("confirm", json!({ "accept": false })));
                    }
                    if web_primary(ui, &label("confirm"), true) {
                        actions.push(send("confirm", json!({ "accept": true })));
                    }
                });
            });
        }
    }

    fn egui_menu_map(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let call = |kind: &str, extra: Value| web_act("menu_map", kind, extra);
        let maps = &data["maps"];
        let editable = maps["editable"].as_bool().unwrap_or(false);
        let ready = data["ready"].as_bool().unwrap_or(false);
        let busy = data["busy"].as_bool().unwrap_or(false);
        let muted = ui.visuals().weak_text_color();
        let gap = 16.0;
        let column = ((ui.available_width() - gap) / 2.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(column);
                web_card(ui, |ui| {
                    ui.label(label("description"));
                    web_muted(ui, &label("close"));
                    if data["loading"].as_bool().unwrap_or(false) {
                        web_muted(ui, &label("loading"));
                    }
                    let workshop = maps["workshop"].as_bool().unwrap_or(false);
                    ui.horizontal(|ui| {
                        let accent = ui.visuals().selection.stroke.color;
                        if ui.add_enabled(editable, egui::Button::new(label("arenas")).fill(if workshop { egui::Color32::TRANSPARENT } else { accent.gamma_multiply(0.6) })).clicked() {
                            actions.push(call("source", json!({ "workshop": false })));
                        }
                        if ui.add_enabled(editable, egui::Button::new(label("workshop")).fill(if workshop { accent.gamma_multiply(0.6) } else { egui::Color32::TRANSPARENT })).clicked() {
                            actions.push(call("source", json!({ "workshop": true })));
                        }
                    });
                    let id = egui::Id::new("menu_map_search");
                    let mut term: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
                    ui.add_enabled(editable, egui::TextEdit::singleline(&mut term).hint_text(label("search")).desired_width(f32::INFINITY));
                    let lower = term.trim().to_lowercase();
                    ui.ctx().data_mut(|d| d.insert_temp(id, term));
                    egui::ScrollArea::vertical().id_salt("menu_map_choices").max_height(260.0).show(ui, |ui| {
                        let accent = ui.visuals().selection.stroke.color;
                        for choice in maps["choices"].as_array().into_iter().flatten() {
                            let (key, name) = (val_str(&choice[0]), val_str(&choice[1]));
                            if !name.to_lowercase().contains(&lower) {
                                continue;
                            }
                            let on = maps["selected"].as_str() == Some(key.as_str());
                            let button = egui::Button::new(&name).fill(if on { accent.gamma_multiply(0.6) } else { egui::Color32::TRANSPARENT });
                            if ui.add_enabled(editable, button.min_size(egui::vec2(ui.available_width(), 28.0))).clicked() {
                                actions.push(call("select", json!({ "id": key })));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if web_primary(ui, &label("apply"), editable && !maps["selected"].is_null()) {
                            actions.push(call("apply", Value::Null));
                        }
                        if ui.add_enabled(ready && !maps["active"].is_null(), egui::Button::new(label("restore"))).clicked() {
                            actions.push(call("restore", Value::Null));
                        }
                    });
                    if let Some(active) = maps["active"].as_str() {
                        ui.label(active);
                        web_muted(ui, &label("restoreFirst"));
                    }
                    let choices = maps["choices"].as_array().map_or(0, Vec::len);
                    if workshop || (!data["loading"].as_bool().unwrap_or(false) && choices == 0) {
                        let note = if workshop { if choices > 0 { label("workshopNote") } else { label("workshopEmpty") } } else { label("empty") };
                        web_muted(ui, &note);
                    }
                });
                ui.add_space(8.0);
                let car = &data["car"];
                egui::CollapsingHeader::new(label("carTitle")).show(ui, |ui| {
                    web_muted(ui, &label("carDescription"));
                    if !car.is_object() {
                        web_muted(ui, &label("carNone"));
                        return;
                    }
                    let disabled = !car["on"].as_bool().unwrap_or(false) || !ready;
                    if let Some(enabled) = web_toggle(ui, &label("carEnable"), car["on"].as_bool().unwrap_or(false), true) {
                        actions.push(call("car_toggle", json!({ "enabled": enabled })));
                    }
                    let coords = car["position"].as_array().map(|values| values.iter().enumerate().map(|(i, v)| format!("{} {:.0}", ["X", "Y", "Z"][i.min(2)], v.as_f64().unwrap_or(0.0))).collect::<Vec<_>>().join("  ")).unwrap_or_default();
                    ui.label(egui::RichText::new(format!("{}: {} · {} {:.1}°", label("original"), coords, label("yaw"), car["yaw"].as_f64().unwrap_or(0.0))).small().color(muted));
                    let tweak = car["tweak"].clone();
                    for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                        let current = tweak["offset"][index].as_f64().unwrap_or(0.0);
                        web_row(ui, &format!("{} {axis}", label("move")), |ui| {
                            if let Some(value) = web_number(ui, current, None, 1.0, !disabled) {
                                let mut next = tweak.clone();
                                next["offset"][index] = json!(value);
                                actions.push(call("car", json!({ "tweak": next })));
                            }
                        });
                    }
                    web_row(ui, &label("yaw"), |ui| {
                        if let Some(value) = web_number(ui, tweak["yaw"].as_f64().unwrap_or(0.0), Some((-180.0, 180.0)), 0.5, !disabled) {
                            let mut next = tweak.clone();
                            next["yaw"] = json!(value);
                            actions.push(call("car", json!({ "tweak": next })));
                        }
                    });
                    if ui.add_enabled(!disabled, egui::Button::new(label("reset"))).clicked() {
                        actions.push(call("car", json!({ "tweak": { "offset": [0, 0, 0], "yaw": 0 } })));
                    }
                });
                let camera = &data["camera"];
                egui::CollapsingHeader::new(label("cameraTitle")).show(ui, |ui| {
                    web_muted(ui, &label("cameraDescription"));
                    let rows = camera["rows"].as_array().cloned().unwrap_or_default();
                    if camera["error"].as_str().is_some_and(|text| !text.is_empty()) || rows.is_empty() {
                        web_muted(ui, camera["error"].as_str().filter(|text| !text.is_empty()).unwrap_or(&label("cameraNone")));
                        return;
                    }
                    let disabled = !camera["on"].as_bool().unwrap_or(false) || !ready;
                    if let Some(enabled) = web_toggle(ui, &label("cameraEnable"), camera["on"].as_bool().unwrap_or(false), true) {
                        actions.push(call("camera_toggle", json!({ "enabled": enabled })));
                    }
                    let others: Vec<&Value> = rows.iter().filter(|item| !item["play"].as_bool().unwrap_or(false)).collect();
                    let selected = rows.iter().find(|item| item["id"] == camera["selected"]);
                    let on_play = selected.is_some_and(|item| item["play"].as_bool().unwrap_or(false));
                    ui.horizontal(|ui| {
                        let accent = ui.visuals().selection.stroke.color;
                        if let Some(play) = rows.iter().find(|item| item["play"].as_bool().unwrap_or(false))
                            && ui.add_enabled(!disabled, egui::Button::new(label("play")).fill(if on_play { accent.gamma_multiply(0.6) } else { egui::Color32::TRANSPARENT })).clicked()
                        {
                            actions.push(call("camera_select", json!({ "id": play["id"] })));
                        }
                        let other = if camera["other"].is_null() { others.first().map(|item| item["id"].clone()).unwrap_or(Value::Null) } else { camera["other"].clone() };
                        if ui.add_enabled(!disabled && !others.is_empty(), egui::Button::new(label("other")).fill(if on_play { egui::Color32::TRANSPARENT } else { accent.gamma_multiply(0.6) })).clicked() {
                            actions.push(call("camera_select", json!({ "id": other })));
                        }
                    });
                    if !on_play && !others.is_empty() {
                        let choices: Vec<(String, String)> = others.iter().map(|item| (val_str(&item["id"]), val_str(&item["label"]))).collect();
                        if let Some(id) = web_select(ui, "menu_camera", &val_str(&camera["selected"]), &choices, !disabled) {
                            actions.push(call("camera_select", json!({ "id": id })));
                        }
                    }
                    if let Some(selected) = selected {
                        let mut tweak = camera["tweak"].clone();
                        if !tweak.is_object() {
                            tweak = json!({ "camera": selected["id"], "offset": [0, 0, 0], "pitch": 0, "yaw": 0, "fov": null });
                        }
                        let coords = selected["position"].as_array().map(|values| values.iter().enumerate().map(|(i, v)| format!("{} {:.0}", ["X", "Y", "Z"][i.min(2)], v.as_f64().unwrap_or(0.0))).collect::<Vec<_>>().join("  ")).unwrap_or_default();
                        ui.label(egui::RichText::new(format!("{}: {} · {} {:.1}° · {} {:.1}°", label("original"), coords, label("pitch"), selected["rotation"][0].as_f64().unwrap_or(0.0), label("yaw"), selected["rotation"][1].as_f64().unwrap_or(0.0))).small().color(muted));
                        for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                            web_row(ui, &format!("{} {axis}", label("move")), |ui| {
                                if let Some(value) = web_number(ui, tweak["offset"][index].as_f64().unwrap_or(0.0), None, 1.0, !disabled) {
                                    let mut next = tweak.clone();
                                    next["offset"][index] = json!(value);
                                    actions.push(call("camera", json!({ "tweak": next })));
                                }
                            });
                        }
                        for (key, name, low, high) in [("pitch", label("pitch"), -89.0, 89.0), ("yaw", label("yaw"), -180.0, 180.0)] {
                            web_row(ui, &name, |ui| {
                                if let Some(value) = web_number(ui, tweak[key].as_f64().unwrap_or(0.0), Some((low, high)), 0.5, !disabled) {
                                    let mut next = tweak.clone();
                                    next[key] = json!(value);
                                    actions.push(call("camera", json!({ "tweak": next })));
                                }
                            });
                        }
                        if let Some(default_fov) = selected["fov"].as_f64() {
                            let override_on = !tweak["fov"].is_null();
                            if let Some(enabled) = web_toggle(ui, &label("fov"), override_on, !disabled) {
                                let mut next = tweak.clone();
                                next["fov"] = if enabled { json!(default_fov) } else { Value::Null };
                                actions.push(call("camera", json!({ "tweak": next })));
                            }
                            web_row(ui, &label("fov"), |ui| {
                                if let Some(value) = web_number(ui, tweak["fov"].as_f64().unwrap_or(default_fov), Some((10.0, 170.0)), 0.5, !disabled && override_on) {
                                    let mut next = tweak.clone();
                                    next["fov"] = json!(value);
                                    actions.push(call("camera", json!({ "tweak": next })));
                                }
                            });
                        }
                        if ui.add_enabled(!disabled, egui::Button::new(label("reset"))).clicked() {
                            actions.push(call("camera", json!({ "tweak": { "camera": selected["id"], "offset": [0, 0, 0], "pitch": 0, "yaw": 0, "fov": null } })));
                        }
                        web_muted(ui, &label("each"));
                    }
                });
                ui.add_space(8.0);
                web_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        if !maps["active"].is_null() && ui.add_enabled(maps["canReapply"].as_bool().unwrap_or(false), egui::Button::new(label("reapply"))).on_hover_text(label("reapplyHint")).clicked() {
                            actions.push(call("reapply", Value::Null));
                        }
                        if ui.add_enabled(!busy && !data["loading"].as_bool().unwrap_or(false), egui::Button::new(label("scan"))).clicked() {
                            actions.push(call("scan", Value::Null));
                        }
                    });
                    if let Some(status) = data["status"].as_str().filter(|text| !text.is_empty()) {
                        ui.label(status);
                    }
                });
            });
            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(column - 34.0);
                    web_heading(ui, &label("pluginTitle"));
                    web_muted(ui, &label("pluginDescription"));
                    ui.horizontal(|ui| {
                        if ui.button(label("plugin")).clicked() {
                            actions.push(call("plugin", Value::Null));
                        }
                        if ui.add_enabled(!busy, egui::Button::new(label("import"))).on_hover_text(label("importHint")).clicked() {
                            actions.push(call("import", Value::Null));
                        }
                    });
                });
            });
        });
    }
}
