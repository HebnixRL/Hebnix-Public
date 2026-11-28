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
    (super::ItemsMode::GoalExplosionScale, "nav-goal-explosion-scale"),
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
            Tab::Settings => !self.on_active_garage_page(),
            Tab::Freeplay => true,
            Tab::GameMods | Tab::LiveEditor => true,
            Tab::Replays => self.replays_subtab == super::ReplaysSubTab::ReplayTimeline,
            Tab::Patcher => matches!(
                self.items_mode,
                super::ItemsMode::SpeedPatch | super::ItemsMode::GoalExplosion | super::ItemsMode::Manage | super::ItemsMode::WheelAlignment | super::ItemsMode::GoalExplosionScale
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
            Tab::Settings if self.on_active_cosmetics_page() => self.web_active_patches(ui.ctx()),
            Tab::Settings if self.on_active_history_page() => {
                let cooked = self.swapper_cooked_path();
                self.upk_history.web_snapshot(&cooked, &self.swapper)
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
                    self.web_live_editor(super::LiveEditorPage::Wheels)
                } else if self.items_mode == super::ItemsMode::GoalExplosionScale {
                    self.web_live_editor(super::LiveEditorPage::GoalExplosion)
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
            Tab::Workshop if self.workshop.view == crate::ui::workshop::WorkshopView::MenuMap => self.workshop.menu_map_web_snapshot(&self.config.settings.rl_path, &self.tx, ui.ctx()),
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
                if self.items_subtab == super::ItemsSubTab::PresetCreator {
                    self.loadouts.creator_page_snapshot(&mut self.swapper, &cooked, ui.ctx())
                } else {
                let mut model = self.loadouts.web_snapshot(
                    &mut self.swapper,
                    self.catalogs_loading,
                    &cooked,
                    ui.ctx(),
                );
                model["catalogLoading"] = json!(self.catalogs_loading);
                model["catalogError"] = json!(self.catalogs_error);
                model
                }
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
        self.tab == Tab::Patcher && matches!(self.items_mode, super::ItemsMode::WheelAlignment | super::ItemsMode::GoalExplosionScale)
            || self.tab == Tab::Items && self.items_subtab == super::ItemsSubTab::Colours
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
            "wry": "Use Wry UI", "wryHint": "Disable to use the current egui modern UI. Changes immediately.",
            "legacy": "Legacy UI", "legacyHint": "Use the original themed tabs and layout. Changes immediately.",
            "capture": t(if self.capturing_hotkey { "settings-keybind-listening" } else { "settings-keybind-set" }),
            "language": t("settings-language-label"), "opacity": t("settings-opacity-label"),
            "toast": t("settings-toast-position-label"), "defaultTab": t("system-default-tab"),
        });
        json!({
            "page": "appearance-settings", "themePage": self.appearance_subtab == super::AppearanceSubTab::Theme,
            "theme": cfg.theme, "themes": self.theme_options,
            "wry": cfg.wry_ui, "legacy": cfg.legacy_ui,
            "general": general, "panel": panel, "labels": labels,
        })
    }

    fn web_appearance_action(&mut self, key: &str, value: Value, ctx: &egui::Context) {
        let boolean = value.as_bool();
        let number = value.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
        let text = value.as_str().unwrap_or_default();
        if key == "legacy" {
            let enabled = boolean.unwrap_or(self.config.settings.legacy_ui);
            self.config.settings.legacy_ui = enabled;
            if enabled {
                #[cfg(windows)]
                if let Some(navigation) = &mut self.modern_navigation { navigation.hide(); }
            } else {
                #[cfg(windows)]
                if let Some(navigation) = &mut self.modern_navigation
                    && let Err(error) = navigation.set_egui_shell(!self.config.settings.wry_ui)
                {
                    self.console.write(format!("[UI] Could not switch renderer: {error}"));
                    return;
                }
            }
            self.save_config();
            ctx.request_repaint();
            return;
        }
        if key == "wry" {
            let enabled = boolean.unwrap_or(self.config.settings.wry_ui);
            #[cfg(windows)]
            if let Some(navigation) = &mut self.modern_navigation {
                if let Err(error) = navigation.set_egui_shell(!enabled) {
                    self.console.write(format!("[UI] Could not switch renderer: {error}"));
                    return;
                }
            }
            self.config.settings.wry_ui = enabled;
            self.config.settings.legacy_ui = false;
            self.save_config();
            ctx.request_repaint();
            return;
        }
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
                        .add_filter("Theme", &["toml", "zip"]);
                    let Some(path) = winutil::parent_file_dialog(dialog).pick_file() else {
                        return;
                    };
                    if let Err(error) =
                        crate::deep_link::import_theme_file(&path, &self.themes_dir, &self.fonts_dir)
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
            "theme_store" => {
                self.open_theme_store();
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
                let rows: Vec<_> = super::RANK_SPOOF_PLAYLISTS.iter().map(|&(id, name, _)| {
                    let icon = [10, 11, 13, 27, 28, 29, 30, 34, 61, 63].iter().position(|key| *key == id).unwrap_or(0);
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
            ManageAction::UpkHistory { key, index, value } if self.on_active_history_page() => {
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
            ManageAction::LiveCosmetics { settings } if self.on_live_colours_page() => {
                let cooked = self.swapper_cooked_path();
                if let Err(error) = crate::patcher::live_cosmetics::update(&cooked, settings) {
                    self.console.write(format!("[Live Cosmetics] {error}"));
                }
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
            ManageAction::StudioRestore { kind, target } if self.on_active_cosmetics_page() => {
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
        // two rows plus the buttons below fit the visible height; under 200 the labels overlap
        let height = ((ui.clip_rect().bottom() - ui.cursor().top() - gap - 84.0) / 2.0).clamp(200.0, 230.0);
        let numerals = ["I", "II", "III", "IV"];
        // plain rows, since a Grid keeps the column widths of a wider window
        for (chunk_index, chunk) in super::RANK_SPOOF_PLAYLISTS.chunks(columns).enumerate() {
            if chunk_index > 0 {
                ui.add_space(gap);
            }
            ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (offset, &(id, name, _)) in chunk.iter().enumerate() {
                let index = chunk_index * columns + offset;
                let state = self.spoofer_ranks.entry(id).or_default();
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
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
            }
            });
        }
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
    // filled from the active theme so text stays readable over a theme background image
    let [r, g, b, a] = ui.visuals().panel_fill.to_srgba_unmultiplied();
    // in a row the rest of the row is "available", so row cards size themselves instead
    let fill = !ui.layout().is_horizontal();
    egui::Frame::NONE
        .fill(egui::Color32::from_rgba_unmultiplied(r, g, b, a.max(205)))
        .stroke(line)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            if fill {
                ui.set_width(ui.available_width());
            }
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
                SpooferSubTab::Username | SpooferSubTab::Friends | SpooferSubTab::Scoreboard | SpooferSubTab::ProfilePictures
            ),
            Tab::Settings => matches!(
                self.hebnix_settings_tab,
                HebnixSettingsTab::ActionButton | HebnixSettingsTab::Interface | HebnixSettingsTab::Directories | HebnixSettingsTab::Discord | HebnixSettingsTab::System
            ),
            Tab::GameMods | Tab::Freeplay | Tab::LiveEditor => true,
            Tab::About => self.maintenance.tab,
            Tab::Replays => self.replays_subtab == super::ReplaysSubTab::ReplayTimeline,
            Tab::Items => matches!(self.items_subtab, super::ItemsSubTab::Colours | super::ItemsSubTab::Presets | super::ItemsSubTab::Loadouts | super::ItemsSubTab::PresetCreator),
            Tab::Patcher => matches!(
                self.items_mode,
                super::ItemsMode::SpeedPatch | super::ItemsMode::GoalExplosion | super::ItemsMode::Manage | super::ItemsMode::WheelAlignment
                    | super::ItemsMode::GoalExplosionScale | super::ItemsMode::GameColours | super::ItemsMode::BallAppearance
                    | super::ItemsMode::MenuTextures | super::ItemsMode::History
            ) || self.on_studio_asset_page(),
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
                "files-settings" => Self::egui_files_settings(ui, data, &mut actions),
                "appearance-settings" => {
                    Self::egui_appearance_settings(ui, data, &mut actions);
                    if data["themePage"].as_bool().unwrap_or(false) {
                        ui.add_space(12.0);
                        self.theme_editor_ui(ui);
                    }
                }
                "system-settings" => Self::egui_system_settings(ui, data, &mut actions),
                "discord-settings" => Self::egui_discord_settings(ui, data, &mut actions),
                "ball-appearance" => Self::egui_ball_appearance(ui, data, &mut actions),
                "goal-explosion" => Self::egui_goal_explosion(ui, data, &mut actions),
                "speed-patch" => Self::egui_speed_patch(ui, data, &mut actions),
                "wheel-alignment" => Self::egui_wheel_alignment(ui, data, &mut actions),
                "game-colours" => Self::egui_game_colours(ui, data, &mut actions),
                "live-colours" | "live-wheels" | "live-goal-explosion" => Self::egui_live_editor(ui, data, &mut actions),
                "studio-active" => Self::egui_studio_active(ui, data, &mut actions),
                "maintenance" => Self::egui_maintenance(ui, data, &mut actions),
                "menu-textures" => Self::egui_menu_textures(ui, data, &mut actions),
                "upk-history" => Self::egui_upk_history(ui, data, &mut actions),
                "studio-asset" => Self::egui_studio_asset(ui, data, &mut actions),
                "item-artwork" => Self::egui_item_artwork(ui, data, &mut actions),
                "anthems" => Self::egui_anthems(ui, data, &mut actions),
                "profile-pictures" => Self::egui_profile_pictures(ui, data, &mut actions),
                "presets" => Self::egui_presets(ui, data, &mut actions),
                "loadouts" => Self::egui_loadouts(ui, data, &mut actions),
                "preset-creator" => Self::egui_preset_creator(ui, data, &mut actions),
                "replay-library" => Self::egui_replay_library(ui, data, &mut actions),
                _ => handled = false,
            }
        });
        let ctx = ui.ctx().clone();
        if page == "menu-map" {
            self.menu_map_previews(&ctx);
        }
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
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(label).strong())).clicked()
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

    /// The two-player outline the web page drew (64x48 viewBox), back player dimmed.
    fn multiplayer_icon(ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 48.0), egui::Sense::hover());
        let text = ui.visuals().text_color();
        let at = |x: f32, y: f32| rect.min + egui::vec2(x, y);
        for (head, base, r, alpha) in [((43.0, 14.0), (43.0, 40.0), 14.0f32, 0.55f32), ((24.0, 11.0), (24.0, 37.0), 14.0, 1.0)] {
            let stroke = egui::Stroke::new(2.0, text.gamma_multiply(alpha));
            ui.painter().circle_stroke(at(head.0, head.1), 8.0, stroke);
            // half disc: arc over the base line, closed by it
            let mut body: Vec<egui::Pos2> = (0..=24)
                .map(|step| {
                    let angle = std::f32::consts::PI * (1.0 + step as f32 / 24.0);
                    at(base.0 + r * angle.cos(), base.1 + r * angle.sin())
                })
                .collect();
            body.push(body[0]);
            ui.painter().add(egui::Shape::line(body, stroke));
        }
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
                ui.set_max_width(680.0);
                web_card(ui, |ui| {
                    ui.add_space(14.0);
                    ui.vertical_centered(|ui| {
                        Self::multiplayer_icon(ui);
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Workshop Multiplayer").size(26.0).strong());
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(label("intro")).color(muted));
                        ui.add_space(14.0);
                        // toggles share one left-aligned column so they line up
                        let column = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
                            ui.allocate_ui_with_layout(egui::vec2(320.0f32.min(ui.available_width()), 0.0), egui::Layout::top_down(egui::Align::Min), |ui| add(ui));
                        };
                        column(ui, &mut |ui| preference(ui, actions, "relay", &label("relay")));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let (connect, help) = (egui::vec2(150.0, 36.0), egui::vec2(300.0, 36.0));
                            ui.add_space(((ui.available_width() - connect.x - help.x - ui.spacing().item_spacing.x) / 2.0).max(0.0));
                            let enabled = !connection["stopping"].as_bool().unwrap_or(false);
                            if ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(label("connect")).strong()).min_size(connect)).clicked() {
                                actions.push(send("connect", Value::Null));
                            }
                            if ui.add(egui::Button::new(label("help")).min_size(help)).clicked() {
                                actions.push(send("help", Value::Null));
                            }
                        });
                        ui.add_space(8.0);
                        column(ui, &mut |ui| {
                            preference(ui, actions, "close", "Disconnect when Hebnix closes");
                            preference(ui, actions, "toasts", &label("toasts"));
                        });
                        if let Some(status) = connection["status"].as_str().filter(|text| !text.is_empty()) {
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(status).small());
                        }
                    });
                    ui.add_space(14.0);
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
                    let mut images: std::collections::HashMap<String, Option<std::sync::Arc<[u8]>>> = ui.ctx().data(|d| d.get_temp(egui::Id::new("menu_map_images"))).unwrap_or_default();
                    if workshop {
                        // downloaded maps bring their own thumbnail
                        for (id, uri) in maps["previews"].as_object().into_iter().flatten() {
                            if images.get(id).is_some_and(Option::is_some) {
                                continue;
                            }
                            let bytes = uri.as_str().and_then(|uri| uri.split_once(',')).and_then(|(_, encoded)| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok());
                            images.insert(id.clone(), bytes.map(Into::into));
                        }
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("menu_map_images"), images.clone()));
                    }
                    let mut visible: Vec<String> = Vec::new();
                    let grid_height = (ui.ctx().content_rect().height() * 0.5).clamp(240.0, 480.0);
                    let width = ui.available_width() - 10.0;
                    let columns = (((width + 8.0) / 128.0).floor() as usize).clamp(2, 5);
                    let tile_w = ((width - 8.0 * (columns as f32 - 1.0)) / columns as f32).floor();
                    egui::ScrollArea::vertical().id_salt("menu_map_choices").max_height(grid_height).min_scrolled_height(grid_height).auto_shrink([false, false]).show(ui, |ui| {
                        egui::Grid::new("menu_map_tiles").spacing([8.0, 8.0]).show(ui, |ui| {
                            let mut shown = 0;
                            for choice in maps["choices"].as_array().into_iter().flatten() {
                                let (key, name) = (val_str(&choice[0]), val_str(&choice[1]));
                                if !name.to_lowercase().contains(&lower) {
                                    continue;
                                }
                                let on = maps["selected"].as_str() == Some(key.as_str());
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(tile_w, 116.0), egui::Sense::click());
                                let hover = ui.ctx().animate_bool_with_time(response.id.with("h"), response.hovered() && editable, 0.1);
                                let fill = ui.visuals().widgets.inactive.weak_bg_fill.lerp_to_gamma(ui.visuals().widgets.hovered.weak_bg_fill, hover);
                                let accent = ui.visuals().selection.stroke.color;
                                let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
                                ui.painter().rect(rect, 6.0, fill, egui::Stroke::new(1.0, if on { accent } else { line.lerp_to_gamma(accent, hover * 0.6) }), egui::StrokeKind::Inside);
                                let caption_h = 30.0;
                                let art = egui::Rect::from_min_max(rect.min + egui::vec2(4.0, 4.0), egui::pos2(rect.right() - 4.0, rect.bottom() - caption_h - 6.0));
                                if !workshop && ui.is_rect_visible(rect) {
                                    visible.push(key.clone());
                                }
                                match images.get(&key) {
                                    Some(Some(bytes)) => {
                                        let source = egui::ImageSource::Bytes { uri: format!("bytes://menumap/{key}").into(), bytes: egui::load::Bytes::Shared(bytes.clone()) };
                                        egui::Image::new(source).fit_to_exact_size(art.size()).corner_radius(3.0).paint_at(ui, art);
                                    }
                                    Some(None) => {
                                        ui.painter().rect_stroke(egui::Rect::from_center_size(art.center(), egui::vec2(28.0, 22.0)), 3.0, egui::Stroke::new(1.5, muted), egui::StrokeKind::Inside);
                                    }
                                    None if workshop => {
                                        ui.painter().text(art.center(), egui::Align2::CENTER_CENTER, "No preview", egui::FontId::proportional(12.0), muted);
                                    }
                                    None => {
                                        ui.painter().text(art.center(), egui::Align2::CENTER_CENTER, "Loading preview…", egui::FontId::proportional(12.0), muted);
                                    }
                                }
                                let cap = egui::Rect::from_min_max(egui::pos2(rect.left() + 4.0, rect.bottom() - caption_h - 4.0), rect.max - egui::vec2(4.0, 4.0));
                                ui.painter().rect_filled(cap, 3.0, ui.visuals().extreme_bg_color.gamma_multiply(0.8));
                                let galley = ui.painter().layout(name.clone(), egui::FontId::proportional(12.0), ui.visuals().text_color(), cap.width() - 8.0);
                                let pos = egui::pos2(cap.center().x - galley.size().x / 2.0, cap.center().y - galley.size().y / 2.0);
                                ui.painter().with_clip_rect(cap).galley(pos, galley, ui.visuals().text_color());
                                response.clone().on_hover_text(&name);
                                if editable && response.clicked() {
                                    actions.push(call("select", json!({ "id": key })));
                                }
                                shown += 1;
                                if shown % columns == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("menu_map_visible"), visible));
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

/// Slider that reports its value once released, like the web range inputs.
fn web_slider(ui: &mut egui::Ui, title: &str, value: f64, range: std::ops::RangeInclusive<f64>, enabled: bool, show: impl Fn(f64) -> String) -> Option<f64> {
    let id = egui::Id::new(("web_slider", title));
    let mut edited: f64 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(value);
    let mut committed = None;
    let dragging = ui.ctx().data(|d| d.get_temp::<bool>(id.with("drag"))).unwrap_or(false);
    if !dragging {
        edited = value;
    }
    ui.horizontal(|ui| {
        ui.label(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(show(edited)).small());
            let response = ui.add_enabled(enabled, egui::Slider::new(&mut edited, range).show_value(false));
            if response.dragged() || response.has_focus() {
                ui.ctx().data_mut(|d| d.insert_temp(id.with("drag"), true));
            }
            if response.drag_stopped() || response.lost_focus() {
                ui.ctx().data_mut(|d| d.insert_temp(id.with("drag"), false));
                committed = Some(edited);
            }
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, edited));
    committed
}

fn web_path_row(ui: &mut egui::Ui, title: &str, value: &str, browse: &str, extra: Option<&str>) -> (bool, bool) {
    let (mut picked, mut extra_clicked) = (false, false);
    ui.horizontal(|ui| {
        ui.label(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            picked = ui.button(browse).clicked();
            if let Some(extra) = extra {
                extra_clicked = ui.button(extra).clicked();
            }
            let mut shown = value.to_owned();
            ui.add(egui::TextEdit::singleline(&mut shown).desired_width(ui.available_width()).interactive(false));
        });
    });
    (picked, extra_clicked)
}

impl HebnixApp {
    fn egui_files_settings(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let update = |key: &str, value: Value| json!({ "type": "files", "key": key, "value": value });
        let paths = data["paths"].as_array().cloned().unwrap_or_default();
        web_card(ui, |ui| {
            web_muted(ui, &label("intro"));
            for entry in paths.iter().take(2) {
                let key = val_str(&entry[0]);
                if web_path_row(ui, &val_str(&entry[1]), &val_str(&entry[2]), &label("browse"), None).0 {
                    actions.push(update(&key, Value::Null));
                }
            }
            for entry in data["ini"].as_array().into_iter().flatten() {
                let (key, title, value, suggested) = (val_str(&entry[0]), val_str(&entry[1]), val_str(&entry[2]), val_str(&entry[3]));
                let id = egui::Id::new(("files_ini", &key));
                let mut text: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| value.clone());
                ui.horizontal(|ui| {
                    ui.label(&title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if key == "Port" && ui.button(label("refresh")).clicked() {
                            actions.push(update("refresh", Value::Null));
                        }
                        if value != suggested {
                            let name = match key.as_str() {
                                "PacketSendRate" => label("setRate"),
                                "Port" => label("reset"),
                                _ => label("setWebPort"),
                            };
                            if ui.button(name).clicked() {
                                actions.push(update(&key, json!(suggested)));
                                text = suggested.clone();
                            }
                        }
                        let response = ui.add(egui::TextEdit::singleline(&mut text).hint_text(label("missing")).desired_width(90.0));
                        text.retain(|c| c.is_ascii_digit());
                        if response.lost_focus() && text != value {
                            let port: u32 = text.parse().unwrap_or(0);
                            if !text.is_empty() && (key == "PacketSendRate" || (1..=65535).contains(&port)) {
                                actions.push(update(&key, json!(text)));
                            } else {
                                text = value.clone();
                            }
                        }
                    });
                });
                ui.ctx().data_mut(|d| d.insert_temp(id, text));
            }
            web_muted(ui, &label("note"));
            if let Some(entry) = paths.get(2) {
                let (key, reset) = (val_str(&entry[0]), entry[3].as_bool().unwrap_or(false));
                let (picked, cleared) = web_path_row(ui, &val_str(&entry[1]), &val_str(&entry[2]), &label("browse"), reset.then(|| label("reset")).as_deref());
                if picked {
                    actions.push(update(&key, Value::Null));
                }
                if cleared {
                    actions.push(update("reset_replays", Value::Null));
                }
            }
        });
        ui.add_space(12.0);
        web_card(ui, |ui| {
            web_heading(ui, &label("title"));
            web_muted(ui, &label("description"));
            ui.label(label("status"));
            if ui.button(label("setup")).clicked() {
                actions.push(update("setup", Value::Null));
            }
        });
        if data["setup"].is_object() {
            Self::egui_launch_setup(ui.ctx(), &data["setup"], actions);
        }
    }

    fn egui_launch_setup(ctx: &egui::Context, model: &Value, actions: &mut Vec<Value>) {
        let labels = &model["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let id = egui::Id::new("launch_setup_draft");
        let mut draft: Value = ctx.data(|d| d.get_temp(id)).unwrap_or_else(|| model["draft"].clone());
        let send = |key: &str, draft: &Value, candidate: Value| json!({ "type": "launch_setup", "key": key, "draft": draft, "candidate": candidate });
        let mut closed = false;
        web_modal(ctx, "launch_setup", 460.0, |ui| {
            web_heading(ui, &label("title"));
            web_muted(ui, &label("hint"));
            let modes = web_pairs(&model["modes"]);
            if let Some(mode) = web_select(ui, "launch_mode", &val_str(&draft["mode"]), &modes, true) {
                draft["mode"] = json!(mode);
            }
            let text_field = |ui: &mut egui::Ui, draft: &mut Value, key: &str, title: String| {
                ui.label(title);
                let mut text = val_str(&draft[key]);
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY)).changed() {
                    draft[key] = json!(text);
                }
            };
            match draft["mode"].as_str().unwrap_or_default() {
                "steam_native" => text_field(ui, &mut draft, "steam_id", label("steam")),
                "steam_shortcut" => {
                    ui.colored_label(ui.visuals().warn_fg_color, label("warning"));
                    if ui.button(label("scan")).clicked() {
                        actions.push(send("scan", &draft, Value::Null));
                    }
                    let candidates = model["candidates"].as_array().cloned().unwrap_or_default();
                    if candidates.is_empty() {
                        web_muted(ui, &label("empty"));
                    }
                    for (index, name) in candidates.iter().enumerate() {
                        if ui.button(val_str(name)).clicked() {
                            actions.push(send("candidate", &draft, json!(index)));
                        }
                    }
                    text_field(ui, &mut draft, "steam_id", label("shortcut"));
                    if let Some(exe) = draft["shortcut_exe"].as_str().filter(|text| !text.is_empty()) {
                        ui.label(egui::RichText::new(exe).small());
                    }
                }
                _ => {}
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(label("cancel")).clicked() {
                    actions.push(send("cancel", &draft, Value::Null));
                    closed = true;
                }
                if web_primary(ui, &label("save"), true) {
                    actions.push(send("save", &draft, Value::Null));
                    closed = true;
                }
            });
        });
        ctx.data_mut(|d| {
            if closed {
                d.remove::<Value>(id);
            } else {
                d.insert_temp(id, draft);
            }
        });
    }

    fn egui_appearance_settings(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let update = |key: &str, value: Value| json!({ "type": "appearance", "key": key, "value": value });
        web_card(ui, |ui| {
            if data["themePage"].as_bool().unwrap_or(false) {
                let legacy = data["legacy"].as_bool().unwrap_or(false);
                if let Some(value) = web_toggle(ui, &label("wry"), data["wry"].as_bool().unwrap_or(true) && !legacy, true) {
                    actions.push(update("wry", json!(value)));
                }
                ui.label(egui::RichText::new(label("wryHint")).small().weak());
                if let Some(value) = web_toggle(ui, &label("legacy"), legacy, true) {
                    actions.push(update("legacy", json!(value)));
                }
                ui.label(egui::RichText::new(label("legacyHint")).small().weak());
                let themes: Vec<(String, String)> = data["themes"].as_array().into_iter().flatten().map(|name| (val_str(name), val_str(name))).collect();
                web_row(ui, &label("themeLabel"), |ui| {
                    if let Some(value) = web_select(ui, "appearance_theme", &val_str(&data["theme"]), &themes, true) {
                        actions.push(update("theme", json!(value)));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(label("refresh")).clicked() {
                        actions.push(update("refresh_theme", Value::Null));
                    }
                    ui.menu_button("Install Theme ⏷", |ui| {
                        if ui.button("Local file (.toml / .zip)…").clicked() {
                            actions.push(update("import_theme", Value::Null));
                        }
                        if ui.button("From Hebnix…").clicked() {
                            actions.push(update("theme_store", Value::Null));
                        }
                    });
                    for (key, text) in [("theme_folder", "folder"), ("fonts_folder", "fonts")] {
                        if ui.button(label(text)).clicked() {
                            actions.push(update(key, Value::Null));
                        }
                    }
                });
                return;
            }
            let general = &data["general"];
            let capturing = general["capturing"].as_bool().unwrap_or(false);
            web_row(ui, &label("hotkey"), |ui| {
                if ui.add_enabled(!capturing, egui::Button::new(label("capture"))).clicked() {
                    actions.push(update("capture", Value::Null));
                }
                let mut shown = if capturing { "…".to_owned() } else { val_str(&general["hotkey"]) };
                ui.add(egui::TextEdit::singleline(&mut shown).desired_width(120.0).interactive(false));
            });
            web_row(ui, &label("language"), |ui| {
                if let Some(value) = web_select(ui, "appearance_language", &val_str(&general["language"]), &web_pairs(&general["languages"]), true) {
                    actions.push(update("language", json!(value)));
                }
            });
            if let Some(value) = web_slider(ui, &label("opacity"), general["opacity"].as_f64().unwrap_or(1.0), 0.5..=1.0, true, |value| format!("{value:.2}")) {
                actions.push(update("opacity", json!(value)));
            }
            let toasts: Vec<(String, String)> = general["toasts"].as_array().into_iter().flatten().map(|name| (val_str(name), val_str(name))).collect();
            web_row(ui, &label("toast"), |ui| {
                if let Some(value) = web_select(ui, "appearance_toast", &val_str(&general["toast"]), &toasts, true) {
                    actions.push(update("toast", json!(value)));
                }
            });
            web_row(ui, &label("defaultTab"), |ui| {
                if let Some(value) = web_select(ui, "appearance_default_tab", &val_str(&general["defaultTab"]), &web_pairs(&general["tabs"]), true) {
                    actions.push(update("default_tab", json!(value)));
                }
            });
        });
        if data["themePage"].as_bool().unwrap_or(false) {
            return;
        }
        ui.add_space(12.0);
        let panel = &data["panel"];
        let enabled = panel["enabled"].as_bool().unwrap_or(false);
        web_card(ui, |ui| {
            web_heading(ui, "Controller panel");
            if let Some(value) = web_toggle(ui, "Enable controller panel", enabled, true) {
                actions.push(update("panel_enabled", json!(value)));
            }
            web_muted(ui, "Open the panel beside the game with L3 + R3.");
            let sides = vec![("left".to_owned(), "Left".to_owned()), ("right".to_owned(), "Right".to_owned())];
            let current = if panel["left"].as_bool().unwrap_or(false) { "left" } else { "right" };
            web_row(ui, "Opens from", |ui| {
                if let Some(value) = web_select(ui, "panel_side", current, &sides, enabled) {
                    actions.push(update("panel_left", json!(value == "left")));
                }
            });
            if let Some(value) = web_slider(ui, "Width", panel["width"].as_f64().unwrap_or(0.3), 0.2..=0.5, enabled, |value| format!("{}%", (value * 100.0).round())) {
                actions.push(update("panel_width", json!(value)));
            }
            if let Some(value) = web_toggle(ui, "Slide in like a drawer", panel["animate"].as_bool().unwrap_or(false), enabled) {
                actions.push(update("panel_animate", json!(value)));
            }
            if let Some(value) = web_toggle(ui, "Show every page (Advanced mode)", panel["advanced"].as_bool().unwrap_or(false), enabled) {
                actions.push(update("panel_advanced", json!(value)));
            }
        });
    }

    fn egui_system_settings(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let backups = &data["backups"];
        let certificate = &data["certificate"];
        let command = |key: &str| json!({ "type": "system_command", "key": key });
        let backup = |key: &str| json!({ "type": "system_backups", "key": key });
        if let Some(error) = backups["error"].as_str().filter(|text| !text.is_empty()) {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
        web_card(ui, |ui| {
            for item in data["rows"].as_array().into_iter().flatten() {
                if let Some(enabled) = web_toggle(ui, item["label"].as_str().unwrap_or_default(), item["enabled"].as_bool().unwrap_or(false), true) {
                    actions.push(json!({ "type": "system_toggle", "key": item["key"], "enabled": enabled }));
                }
                if let Some(note) = item["note"].as_str().filter(|text| !text.is_empty()) {
                    web_muted(ui, note);
                }
            }
            if let Some(value) = web_slider(ui, "Overlay refresh rate", data["fps"].as_f64().unwrap_or(30.0), 20.0..=60.0, true, |value| format!("{} fps", value.round())) {
                actions.push(json!({ "type": "system_fps", "fps": value.round() }));
            }
            let busy = data["repairBusy"].as_bool().unwrap_or(false);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(data["repair"].as_str().unwrap_or_default())).clicked() {
                    actions.push(command("repair"));
                }
                if ui.button("Uninstall").clicked() {
                    actions.push(command("uninstall"));
                }
                if ui.button(backups["label"].as_str().unwrap_or_default()).clicked() {
                    actions.push(backup("open"));
                }
            });
            web_progress(ui, busy, &Value::Null);
        });
        ui.add_space(12.0);
        web_card(ui, |ui| {
            web_heading(ui, "Account HTTPS proxy certificate");
            web_muted(ui, "Name and Friends use the account HTTPS proxy while Rocket League is running. It routes only api.epicgames.dev and leaves Windows proxy settings unchanged.");
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} {}", val_str(&certificate["statusLabel"]), val_str(&certificate["status"])));
                if ui.button(val_str(&certificate["action"])).clicked() {
                    actions.push(command("certificate"));
                }
                if ui.button("↻").on_hover_text(val_str(&certificate["refresh"])).clicked() {
                    actions.push(command("refresh_certificate"));
                }
            });
            if !certificate["admin"].as_bool().unwrap_or(false) {
                web_muted(ui, "Restart as Administrator to install or remove the certificate.");
            }
            ui.label(format!("{} {}", val_str(&certificate["proxyLabel"]), val_str(&certificate["proxy"])));
        });
        if backups["confirming"].as_bool().unwrap_or(false) {
            web_modal(ui.ctx(), "system_backups", 440.0, |ui| {
                web_heading(ui, backups["label"].as_str().unwrap_or_default());
                ui.add_space(6.0);
                ui.label(backups["note"].as_str().unwrap_or_default());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(backups["cancel"].as_str().unwrap_or_default()).clicked() {
                        actions.push(backup("cancel"));
                    }
                    let danger = ui.visuals().error_fg_color.gamma_multiply(0.6);
                    if ui.add(egui::Button::new(backups["confirm"].as_str().unwrap_or_default()).fill(danger)).clicked() {
                        actions.push(backup("confirm"));
                    }
                });
            });
        }
    }

    fn egui_discord_settings(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let flag = |key: &str| data[key].as_bool().unwrap_or(false);
        let set = |key: &str, enabled: bool| json!({ "type": "discord_option", "key": key, "enabled": enabled });
        let game = flag("game");
        web_card(ui, |ui| {
            if let Some(value) = web_toggle(ui, &label("enabled"), flag("enabled"), true) {
                actions.push(set("enabled", value));
            }
            if let Some(value) = web_toggle(ui, &label("only"), flag("only"), true) {
                actions.push(set("only", value));
            }
            ui.add_space(6.0);
            ui.label(label("message"));
            if let Some(value) = web_toggle(ui, &label("game"), game, true) {
                actions.push(set("game", value));
            }
            let count = ["score", "map", "mode"].iter().filter(|key| flag(key)).count();
            ui.indent("discord_game_options", |ui| {
                for key in ["score", "map", "mode"] {
                    if let Some(value) = web_toggle(ui, &label(key), flag(key), game && !(flag(key) && count <= 1)) {
                        actions.push(set(key, value));
                    }
                }
            });
            ui.horizontal(|ui| {
                if let Some(value) = web_toggle(ui, &label("custom"), !game, true) {
                    actions.push(set("game", !value));
                }
                let id = egui::Id::new("discord_message");
                let mut text: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| val_str(&data["message"]));
                let response = ui.add_enabled(!game, egui::TextEdit::singleline(&mut text).hint_text(label("placeholder")));
                if response.lost_focus() {
                    actions.push(json!({ "type": "discord_message", "message": text }));
                }
                ui.ctx().data_mut(|d| d.insert_temp(id, text));
            });
            if game {
                web_muted(ui, &label("disabled"));
            }
        });
        ui.add_space(12.0);
        let link = &data["link"];
        let link_labels = &link["labels"];
        let link_label = |key: &str| link_labels[key].as_str().unwrap_or_default().to_owned();
        let busy = link["busy"].as_bool().unwrap_or(false);
        web_card(ui, |ui| {
            web_heading(ui, &link_label("heading"));
            web_muted(ui, &link_label("instruction"));
            let id = egui::Id::new("discord_link_code");
            let mut code: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| val_str(&link["code"]));
            let mut submit = false;
            ui.horizontal_wrapped(|ui| {
                ui.label(link_label("code"));
                let response = ui.add_enabled(!busy, egui::TextEdit::singleline(&mut code).char_limit(64).desired_width(180.0));
                submit = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                submit |= web_primary(ui, &link_label("button"), !busy);
            });
            if submit && !busy {
                actions.push(json!({ "type": "discord_link", "code": code }));
            }
            ui.ctx().data_mut(|d| d.insert_temp(id, code));
            web_muted(ui, link["status"].as_str().unwrap_or_default());
            let accounts = link["accounts"].as_array().cloned().unwrap_or_default();
            if !accounts.is_empty() {
                web_heading(ui, &link_label("accounts"));
                for name in &accounts {
                    ui.label(format!("• {}", val_str(name)));
                }
            }
        });
    }
}

/// Image from a web data URI (or bytes the page already holds), keyed by a short hash.
fn web_image(ui: &mut egui::Ui, uri: &str, size: egui::Vec2) {
    let Some((_, encoded)) = uri.split_once(',') else {
        ui.allocate_space(size);
        return;
    };
    let id = egui::Id::new(("web_image", uri.len(), &uri[uri.len().saturating_sub(48)..]));
    let bytes: Option<std::sync::Arc<[u8]>> = ui.ctx().data(|d| d.get_temp(id)).or_else(|| {
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()?;
        let shared: std::sync::Arc<[u8]> = decoded.into();
        ui.ctx().data_mut(|d| d.insert_temp(id, shared.clone()));
        Some(shared)
    });
    match bytes {
        Some(bytes) => {
            let source = egui::ImageSource::Bytes { uri: format!("bytes://web/{:x}", id.value()).into(), bytes: egui::load::Bytes::Shared(bytes) };
            ui.add(egui::Image::new(source).fit_to_exact_size(size));
        }
        None => {
            ui.allocate_space(size);
        }
    }
}

fn web_rgb_of(value: &Value) -> [u8; 3] {
    let channel = |index: usize| value[index].as_u64().unwrap_or(0).min(255) as u8;
    [channel(0), channel(1), channel(2)]
}

/// Colour swatch; returns the new rgb when the user picked one.
fn web_colour(ui: &mut egui::Ui, rgb: [u8; 3], enabled: bool) -> Option<[u8; 3]> {
    let mut next = rgb;
    let changed = ui
        .scope(|ui| {
            // the colour button sizes itself from interact_size, which themes make tall
            ui.spacing_mut().interact_size = egui::vec2(56.0, 20.0);
            ui.add_enabled(enabled, |ui: &mut egui::Ui| ui.color_edit_button_srgb(&mut next))
        })
        .inner
        .changed();
    changed.then_some(next)
}

fn web_status(ui: &mut egui::Ui, data: &Value) {
    web_card(ui, |ui| {
        web_heading(ui, "Status");
        let status = data["status"].as_str().unwrap_or_default();
        let text = if status.is_empty() && data["busy"].as_bool().unwrap_or(false) { "Working…" } else { status };
        if data["error"].as_bool().unwrap_or(false) || data["error"].as_str().is_some_and(|text| !text.is_empty()) {
            ui.colored_label(ui.visuals().warn_fg_color, text);
        } else {
            ui.label(egui::RichText::new(text).small());
        }
    });
}

impl HebnixApp {
    fn egui_ball_appearance(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let update = |key: &str, value: Value| json!({ "type": "ball_appearance", "key": key, "value": value });
        let busy = data["busy"].as_bool().unwrap_or(false);
        let models: Vec<(String, String)> = data["balls"].as_array().into_iter().flatten().enumerate().map(|(index, name)| (index.to_string(), val_str(name))).collect();
        web_card(ui, |ui| {
            ui.label(label("description"));
            web_muted(ui, &label("hint"));
            ui.horizontal_wrapped(|ui| {
                ui.label(label("source"));
                if let Some(value) = web_select(ui, "ball_source", &val_str(&data["source"]), &models, !busy).and_then(|value| value.parse::<u64>().ok()) {
                    actions.push(update("source", json!(value)));
                }
                ui.label(label("donor"));
                if let Some(value) = web_select(ui, "ball_donor", &val_str(&data["donor"]), &models, !busy).and_then(|value| value.parse::<u64>().ok()) {
                    actions.push(update("donor", json!(value)));
                }
            });
            if let Some(value) = web_toggle(ui, &label("colour"), data["colours"].as_bool().unwrap_or(false), !busy && data["canColour"].as_bool().unwrap_or(false)) {
                actions.push(update("colours", json!(value)));
            }
            web_muted(ui, &label("colourHint"));
            if let Some(applied) = data["applied"].as_str().filter(|text| !text.is_empty()) {
                ui.label(applied);
                ui.label(egui::RichText::new(label("restoreFirst")).small());
            }
            if let Some(error) = data["ballError"].as_str().filter(|text| !text.is_empty()) {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            ui.horizontal(|ui| {
                if web_primary(ui, &label("apply"), data["canApply"].as_bool().unwrap_or(false) && !busy) {
                    actions.push(update("apply", Value::Null));
                }
                if ui.add_enabled(data["canRestore"].as_bool().unwrap_or(false) && !busy, egui::Button::new(label("restore"))).clicked() {
                    actions.push(update("restore", Value::Null));
                }
            });
        });
        ui.add_space(12.0);
        web_status(ui, data);
    }

    fn egui_goal_explosion(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        for (index, model) in data["panels"].as_array().into_iter().flatten().enumerate() {
            let busy = model["busy"].as_bool().unwrap_or(false);
            let (min, max) = (model["min"].as_f64().unwrap_or(0.0), model["max"].as_f64().unwrap_or(1.0));
            let id = egui::Id::new(("goal_volume", index));
            let mut value: f64 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| model["value"].as_f64().unwrap_or(min));
            web_card(ui, |ui| {
                web_heading(ui, model["title"].as_str().unwrap_or_default());
                web_muted(ui, model["description"].as_str().unwrap_or_default());
                ui.horizontal(|ui| {
                    let mut slider = egui::Slider::new(&mut value, min..=max);
                    if model["log"].as_bool().unwrap_or(false) {
                        slider = slider.logarithmic(true);
                    }
                    ui.add_enabled(!busy, slider);
                    ui.label(model["suffix"].as_str().unwrap_or_default());
                    if model["mute"].as_str().is_some_and(|text| !text.is_empty()) && value <= min {
                        web_muted(ui, model["mute"].as_str().unwrap_or_default());
                    }
                });
                ui.horizontal(|ui| {
                    if web_primary(ui, model["apply"].as_str().unwrap_or_default(), !busy) {
                        actions.push(json!({ "type": "goal_explosion", "volume": true, "value": value, "restore": false }));
                    }
                    if ui.add_enabled(!busy && model["active"].as_bool().unwrap_or(false), egui::Button::new(model["restore"].as_str().unwrap_or_default())).clicked() {
                        actions.push(json!({ "type": "goal_explosion", "volume": true, "value": value, "restore": true }));
                    }
                });
                let status = model["status"].as_str().unwrap_or_default();
                if busy || !status.is_empty() {
                    ui.label(egui::RichText::new(if status.is_empty() { "Working…" } else { status }).small());
                }
                web_progress(ui, busy, &model["progress"]);
            });
            ui.ctx().data_mut(|d| d.insert_temp(id, value));
            ui.add_space(8.0);
        }
    }

    fn egui_speed_patch(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let busy = data["busy"].as_bool().unwrap_or(false);
        let has_package = data["package"].as_str().is_some_and(|text| !text.is_empty());
        let id = egui::Id::new("speed_patch_value");
        let mut speed: f64 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| data["speed"].as_f64().unwrap_or(1.0));
        web_card(ui, |ui| {
            web_muted(ui, &label("description"));
            if let Some(value) = web_toggle(ui, &label("items"), data["items"].as_bool().unwrap_or(false), true) {
                actions.push(json!({ "type": "speed_items", "enabled": value }));
            }
            web_muted(ui, &label("itemsHint"));
            ui.separator();
            web_heading(ui, &label("oneoff"));
            web_muted(ui, &label("oneoffHint"));
            ui.horizontal(|ui| {
                if ui.button(label("choose")).clicked() {
                    actions.push(json!({ "type": "speed_choose" }));
                }
                ui.label(if has_package { val_str(&data["package"]) } else { label("noPackage") });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(label("multiplier"));
                ui.add(egui::Slider::new(&mut speed, data["min"].as_f64().unwrap_or(0.25)..=data["max"].as_f64().unwrap_or(10.0)).step_by(0.25));
                if ui.button("1x").clicked() {
                    speed = 1.0;
                }
                if ui.button("5x").clicked() {
                    speed = 5.0;
                }
            });
            ui.horizontal(|ui| {
                if web_primary(ui, &label("patch"), has_package && !busy) {
                    actions.push(json!({ "type": "speed_run", "speed": speed, "restore": false }));
                }
                if ui.add_enabled(has_package && !busy, egui::Button::new(label("restore"))).clicked() {
                    actions.push(json!({ "type": "speed_run", "speed": speed, "restore": true }));
                }
            });
            let status = data["status"].as_str().unwrap_or_default();
            if busy || !status.is_empty() {
                let text = if status.is_empty() { "Working…" } else { status };
                if data["error"].as_bool().unwrap_or(false) {
                    ui.colored_label(ui.visuals().warn_fg_color, text);
                } else {
                    web_muted(ui, text);
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, speed));
    }

    fn egui_wheel_alignment(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let busy = data["busy"].as_bool().unwrap_or(false);
        let models: Vec<String> = data["models"].as_array().into_iter().flatten().map(val_str).collect();
        let selected = data["selected"].as_u64().unwrap_or(0) as usize;
        let id = egui::Id::new("wheel_alignment_values");
        let mut values: Value = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| data["settings"].clone());
        let keyed = (selected, data["settings"].to_string());
        let key_id = id.with("key");
        if ui.ctx().data(|d| d.get_temp::<(usize, String)>(key_id)).as_ref() != Some(&keyed) {
            values = data["settings"].clone();
            ui.ctx().data_mut(|d| d.insert_temp(key_id, keyed));
        }
        web_card(ui, |ui| {
            let search_id = id.with("search");
            let mut term: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
            let disabled = busy || models.is_empty();
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled(!disabled, egui::TextEdit::singleline(&mut term).hint_text(label("filter")).desired_width(160.0));
                let lower = term.trim().to_lowercase();
                let options: Vec<(String, String)> = models.iter().enumerate().filter(|(_, name)| name.to_lowercase().contains(&lower)).map(|(index, name)| (index.to_string(), name.clone())).collect();
                if let Some(value) = web_select(ui, "wheel_car", &selected.to_string(), &options, !disabled).and_then(|value| value.parse::<u64>().ok()) {
                    actions.push(json!({ "type": "wheel_select", "index": value }));
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(search_id, term));
            if data["catalogLoading"].as_bool().unwrap_or(false) || data["catalogError"].as_str().is_some_and(|text| !text.is_empty()) || models.is_empty() {
                let text = data["catalogError"].as_str().filter(|text| !text.is_empty()).map(str::to_owned)
                    .unwrap_or_else(|| if data["catalogLoading"].as_bool().unwrap_or(false) { "Loading…".to_owned() } else { label("noCatalog") });
                web_muted(ui, &text);
            }
            if !models.is_empty() {
                for (key, max) in [("camber", 30.0), ("vertical", 15.0), ("lateral", 15.0)] {
                    let mut value = values[key].as_f64().unwrap_or(0.0);
                    web_row(ui, &label(key), |ui| {
                        ui.add_enabled(!busy, egui::Slider::new(&mut value, -max..=max).step_by(0.5));
                    });
                    values[key] = json!(value);
                    web_muted(ui, &label(&format!("{key}Hint")));
                }
                let running = data["running"].as_bool().unwrap_or(false);
                let response = ui.add_enabled(!busy && !running, egui::Button::new(label("apply")).fill(ui.visuals().selection.stroke.color.gamma_multiply(0.6)));
                if running {
                    response.clone().on_hover_text(label("close"));
                }
                if response.clicked() {
                    actions.push(json!({ "type": "wheel_apply", "settings": values }));
                }
            }
        });
        ui.add_space(12.0);
        web_card(ui, |ui| {
            web_heading(ui, &label("revert"));
            if ui.add_enabled(!busy, egui::Button::new(label("reset"))).clicked() {
                for key in ["camber", "vertical", "lateral"] {
                    values[key] = json!(0);
                }
            }
            web_muted(ui, &label("editedLabel"));
            let edited = data["edited"].as_array().cloned().unwrap_or_default();
            for row in &edited {
                ui.horizontal(|ui| {
                    let index = row["index"].as_u64();
                    let accent = ui.visuals().selection.stroke.color;
                    let on = index == Some(selected as u64);
                    if ui.add_enabled(!busy && index.is_some(), egui::Button::new(val_str(&row["label"])).fill(if on { accent.gamma_multiply(0.6) } else { egui::Color32::TRANSPARENT })).clicked() {
                        actions.push(json!({ "type": "wheel_select", "index": index }));
                    }
                    if ui.add_enabled(!busy && !data["running"].as_bool().unwrap_or(false), egui::Button::new("×")).clicked() {
                        actions.push(json!({ "type": "wheel_apply", "remove": row["file"] }));
                    }
                });
            }
            if edited.is_empty() {
                web_muted(ui, &label("empty"));
            }
        });
        ui.add_space(12.0);
        web_status(ui, &json!({ "status": data["status"], "busy": busy }));
        ui.ctx().data_mut(|d| d.insert_temp(id, values));
    }

    fn egui_game_colours(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let busy = data["busy"].as_bool().unwrap_or(false);
        let live_managed = data["liveManaged"].as_bool().unwrap_or(false);
        let id = egui::Id::new("game_colours_settings");
        let key_id = id.with("key");
        let origin = data["settings"].to_string();
        let mut settings: Value = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| data["settings"].clone());
        if ui.ctx().data(|d| d.get_temp::<String>(key_id)).as_deref() != Some(origin.as_str()) {
            settings = data["settings"].clone();
            ui.ctx().data_mut(|d| d.insert_temp(key_id, origin));
        }
        web_muted(ui, &label("description"));
        let team_group = |ui: &mut egui::Ui, settings: &mut Value, title: String, toggle_label: String, hint: String, key: &str, team: &str| {
            web_card(ui, |ui| {
                web_heading(ui, &title);
                let mut on = settings[key].as_bool().unwrap_or(false);
                if ui.checkbox(&mut on, toggle_label).changed() {
                    settings[key] = json!(on);
                }
                web_muted(ui, &hint);
                if on {
                    let (blue, orange) = (format!("{team}_blue"), format!("{team}_orange"));
                    for (field, name) in [(&blue, label("blue")), (&orange, label("orange"))] {
                        ui.horizontal(|ui| {
                            ui.label(name);
                            if let Some(rgb) = web_colour(ui, web_rgb_of(&settings[field.as_str()]), true) {
                                settings[field.as_str()] = json!(rgb);
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if team == "hud" && ui.button(label("match")).clicked() {
                            settings["hud_blue"] = settings["stadium_blue"].clone();
                            settings["hud_orange"] = settings["stadium_orange"].clone();
                        }
                        if ui.button(label("defaults")).clicked() {
                            settings[blue.as_str()] = data["defaults"][&blue].clone();
                            settings[orange.as_str()] = data["defaults"][&orange].clone();
                        }
                        if ui.button(label("swap")).clicked() {
                            let first = settings[blue.as_str()].clone();
                            settings[blue.as_str()] = settings[orange.as_str()].clone();
                            settings[orange.as_str()] = first;
                        }
                    });
                }
            });
            ui.add_space(8.0);
        };
        if !live_managed {
            team_group(ui, &mut settings, label("stadium"), label("stadiumOn"), label("stadiumHint"), "stadium_colours", "stadium");
            team_group(ui, &mut settings, label("hud"), label("hudOn"), label("hudHint"), "hud_colours", "hud");
        }
        web_card(ui, |ui| {
            web_heading(ui, &label("car"));
            let mut on = settings["car_colours"].as_bool().unwrap_or(false);
            if ui.checkbox(&mut on, label("carOn")).changed() {
                settings["car_colours"] = json!(on);
            }
            web_muted(ui, &label("carHint"));
            if on {
                let mut remove = None;
                let customs = settings["custom_colours"].as_array().cloned().unwrap_or_default();
                for (index, rgb) in customs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(label("custom"));
                        if let Some(next) = web_colour(ui, web_rgb_of(rgb), true) {
                            settings["custom_colours"][index] = json!(next);
                        }
                        if ui.button("×").clicked() {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    if let Some(list) = settings["custom_colours"].as_array_mut() {
                        list.remove(index);
                    }
                }
                if ui.button("+").clicked() {
                    if let Some(list) = settings["custom_colours"].as_array_mut() {
                        list.push(json!([255, 0, 0]));
                    }
                }
            }
            ui.separator();
            let mut palette = settings["extended_palette"].as_bool().unwrap_or(false);
            if ui.checkbox(&mut palette, label("paletteOn")).changed() {
                settings["extended_palette"] = json!(palette);
            }
            web_muted(ui, &label("paletteHint"));
            web_muted(ui, "Rocket League must be closed to apply palette colours.");
            if web_primary(ui, "Apply palette colours", !busy) {
                actions.push(json!({ "type": "game_colours", "settings": settings, "restore": false }));
            }
        });
        ui.add_space(8.0);
        if !live_managed {
            team_group_heatseeker(ui, &mut settings, &label("heatseeker"), &label("heatseekerOn"), &label("heatseekerHint"), &label("maxSpeed"));
        }
        ui.add_space(8.0);
        web_card(ui, |ui| {
            web_heading(ui, "Status");
            ui.horizontal(|ui| {
                if web_primary(ui, &label("apply"), !busy) {
                    actions.push(json!({ "type": "game_colours", "settings": settings, "restore": false }));
                }
                if ui.add_enabled(!busy && data["canRestore"].as_bool().unwrap_or(false), egui::Button::new(label("restore"))).clicked() {
                    actions.push(json!({ "type": "game_colours", "settings": settings, "restore": true }));
                }
            });
            let status = data["status"].as_str().unwrap_or_default();
            let text = if status.is_empty() && busy { "Working…" } else { status };
            if data["error"].as_bool().unwrap_or(false) {
                ui.colored_label(ui.visuals().warn_fg_color, text);
            } else {
                ui.label(egui::RichText::new(text).small());
            }
            web_muted(ui, data["backup"].as_str().unwrap_or_default());
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, settings));
    }

    fn egui_live_editor(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let page = data["page"].as_str().unwrap_or_default();
        let id = egui::Id::new(("live_cosmetics", page));
        let key_id = id.with("key");
        let origin = data["live"]["settings"].to_string();
        let mut settings: Value = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| data["live"]["settings"].clone());
        if ui.ctx().data(|d| d.get_temp::<String>(key_id)).as_deref() != Some(origin.as_str()) {
            settings = data["live"]["settings"].clone();
            ui.ctx().data_mut(|d| d.insert_temp(key_id, origin));
        }
        let mut changed = false;
        match page {
            "live-goal-explosion" => web_card(ui, |ui| {
                web_heading(ui, "Goal explosion scale");
                web_muted(ui, "Changes apply to the next goal explosion. Default preserves the game's original size.");
                let scale = settings["goal_explosion_scale"].as_f64().unwrap_or(0.0);
                let mut custom = scale != 0.0;
                if ui.checkbox(&mut custom, "Custom scale").changed() {
                    settings["goal_explosion_scale"] = json!(if custom { 1.0 } else { 0.0 });
                    changed = true;
                }
                let mut value = if scale == 0.0 { 1.0 } else { scale };
                ui.horizontal(|ui| {
                    if ui.add_enabled(custom, egui::Slider::new(&mut value, 0.1..=10.0).step_by(0.1).suffix("×")).changed() {
                        settings["goal_explosion_scale"] = json!(value);
                        changed = true;
                    }
                    if !custom {
                        ui.label(egui::RichText::new("Default").small());
                    }
                });
            }),
            "live-wheels" => web_card(ui, |ui| {
                web_heading(ui, "Live Wheel Alignment · All local cars");
                web_muted(ui, "Changes are saved automatically and update in Rocket League. Values remain available while Hebnix is closed.");
                let status = data["live"]["status"].as_str().filter(|text| !text.is_empty()).unwrap_or("Starting…");
                web_muted(ui, &format!("Live cosmetics: {status}"));
                for (key, name, max, unit) in [("camber", "Camber", 30.0, "°"), ("vertical", "Up / Down", 15.0, ""), ("lateral", "Track width", 15.0, "")] {
                    let mut value = settings["alignment"][key].as_f64().unwrap_or(0.0);
                    web_row(ui, name, |ui| {
                        if ui.add(egui::Slider::new(&mut value, -max..=max).step_by(0.5).suffix(unit)).changed() {
                            settings["alignment"][key] = json!(value);
                            changed = true;
                        }
                    });
                }
                if ui.button("Reset alignment").clicked() {
                    settings["alignment"] = json!({ "camber": 0, "vertical": 0, "lateral": 0 });
                    changed = true;
                }
            }),
            _ => {
                Self::egui_live_colours(ui, &mut settings, &mut changed);
                // team colours live in the game colours editor too
                let team_card = |ui: &mut egui::Ui, settings: &mut Value, changed: &mut bool, key: &str, title: &str| {
                    web_card(ui, |ui| {
                        web_heading(ui, title);
                        let mut on = settings[key]["enabled"].as_bool().unwrap_or(false);
                        if ui.checkbox(&mut on, "Enabled").changed() {
                            settings[key]["enabled"] = json!(on);
                            *changed = true;
                        }
                        if on {
                            for side in ["blue", "orange"] {
                                ui.horizontal(|ui| {
                                    ui.label(if side == "blue" { "Blue" } else { "Orange" });
                                    if let Some(rgb) = web_colour(ui, web_rgb_of(&settings[key][side]), true) {
                                        settings[key][side] = json!(rgb);
                                        *changed = true;
                                    }
                                });
                            }
                        }
                    });
                    ui.add_space(8.0);
                };
                ui.add_space(8.0);
                if settings["stadium"].is_object() {
                    team_card(ui, &mut settings, &mut changed, "stadium", "Stadium colours");
                }
                if settings["hud"].is_object() {
                    team_card(ui, &mut settings, &mut changed, "hud", "HUD colours");
                }
            }
        }
        if changed {
            actions.push(json!({ "type": "live_cosmetics", "settings": settings }));
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, settings));
    }

    fn egui_live_colours(ui: &mut egui::Ui, settings: &mut Value, changed: &mut bool) {
        web_card(ui, |ui| {
            web_heading(ui, "Colours");
            let sections: [(&str, &[(&str, &str)]); 4] = [
                ("Car Colours", &[("primary", "Primary paint"), ("secondary", "Secondary paint"), ("trim", "Body / trim"), ("decal", "Decal"), ("wheels", "Wheels"), ("topper", "Topper")]),
                ("Effects", &[("boost", "Boost"), ("trail", "Trail"), ("goal_explosion", "Goal explosion")]),
                ("Player Colours", &[("banner", "Player banner"), ("avatar_border", "Avatar border")]),
                ("Heatseeker Ball", &[("heatseeker", "Heatseeker ball")]),
            ];
            for (title, entries) in sections {
                ui.add_space(8.0);
                web_heading(ui, title);
                for (key, name) in entries {
                    if !settings[*key].is_object() {
                        continue;
                    }
                    let rainbow = *key != "goal_explosion" && settings[*key]["rainbow"].as_bool().unwrap_or(false);
                    ui.horizontal(|ui| {
                        let mut on = settings[*key]["enabled"].as_bool().unwrap_or(false);
                        if ui.checkbox(&mut on, *name).changed() {
                            settings[*key]["enabled"] = json!(on);
                            *changed = true;
                        }
                        if let Some(rgb) = web_colour(ui, web_rgb_of(&settings[*key]["rgb"]), !rainbow) {
                            settings[*key]["rgb"] = json!(rgb);
                            *changed = true;
                        }
                        if *key != "goal_explosion" {
                            let mut flag = rainbow;
                            if ui.checkbox(&mut flag, "Rainbow").changed() {
                                settings[*key]["rainbow"] = json!(flag);
                                *changed = true;
                            }
                        }
                    });
                    if *key == "goal_explosion" {
                        web_muted(ui, "Only in game");
                    }
                }
            }
        });
    }

    fn egui_studio_active(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let rows = data["rows"].as_array().cloned().unwrap_or_default();
        web_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(data["count"].as_str().unwrap_or_default());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!rows.is_empty(), egui::Button::new(data["all"].as_str().unwrap_or_default())).clicked() {
                        actions.push(json!({ "type": "studio_restore", "kind": Value::Null, "target": Value::Null }));
                    }
                });
            });
        });
        ui.add_space(8.0);
        if rows.is_empty() {
            web_muted(ui, data["empty"].as_str().unwrap_or_default());
            return;
        }
        let width = 220.0;
        let per_row = ((ui.available_width() / (width + 12.0)).floor() as usize).max(1);
        for chunk in rows.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for patch in chunk {
                    web_card(ui, |ui| {
                        ui.set_width(width - 34.0);
                        ui.vertical_centered(|ui| {
                            let images = patch["images"].as_array().cloned().unwrap_or_default();
                            if images.is_empty() {
                                web_muted(ui, data["missing"].as_str().unwrap_or_default());
                            }
                            for source in &images {
                                web_image(ui, source.as_str().unwrap_or_default(), egui::vec2(width - 34.0, 110.0));
                            }
                            ui.label(egui::RichText::new(patch["name"].as_str().unwrap_or_default()).strong());
                            web_muted(ui, patch["description"].as_str().unwrap_or_default());
                            if ui.button(data["restore"].as_str().unwrap_or_default()).clicked() {
                                actions.push(json!({ "type": "studio_restore", "kind": patch["kind"], "target": patch["target"] }));
                            }
                        });
                    });
                }
            });
            ui.add_space(8.0);
        }
    }

    fn egui_maintenance(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let cmd = |key: &str, enabled: Value| json!({ "type": "maintenance", "key": key, "enabled": enabled });
        web_card(ui, |ui| {
            if ui.button(format!("‹ {}", label("back"))).clicked() {
                actions.push(cmd("back", Value::Null));
            }
            web_heading(ui, &label("remove"));
            for entry in data["options"].as_array().into_iter().flatten() {
                if let Some(value) = web_toggle(ui, &val_str(&entry[1]), entry[2].as_bool().unwrap_or(false), true) {
                    actions.push(cmd(&val_str(&entry[0]), json!(value)));
                }
            }
        });
        ui.add_space(8.0);
        web_card(ui, |ui| {
            ui.label(label("note"));
            ui.label(egui::RichText::new(data["base"].as_str().unwrap_or_default()).monospace());
            egui::CollapsingHeader::new(label("files")).show(ui, |ui| {
                for path in data["paths"].as_array().into_iter().flatten() {
                    ui.label(egui::RichText::new(val_str(path)).monospace().small());
                }
            });
        });
        ui.add_space(8.0);
        web_card(ui, |ui| {
            if let Some(error) = data["error"].as_str().filter(|text| !text.is_empty()) {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
            if ui.add_enabled(data["selected"].as_bool().unwrap_or(false), egui::Button::new(label("remove")).fill(danger)).clicked() {
                actions.push(cmd("confirm", Value::Null));
            }
        });
        if data["confirm"].as_bool().unwrap_or(false) {
            web_modal(ui.ctx(), "maintenance_confirm", 440.0, |ui| {
                web_heading(ui, &label("remove"));
                ui.add_space(6.0);
                ui.label(label("confirm"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(label("cancel")).clicked() {
                        actions.push(cmd("cancel", Value::Null));
                    }
                    let danger = ui.visuals().error_fg_color.gamma_multiply(0.6);
                    if ui.add(egui::Button::new(label("confirmButton")).fill(danger)).clicked() {
                        actions.push(cmd("remove", Value::Null));
                    }
                });
            });
        }
    }
}

fn team_group_heatseeker(ui: &mut egui::Ui, settings: &mut Value, title: &str, toggle_label: &str, hint: &str, max_speed: &str) {
    web_card(ui, |ui| {
        web_heading(ui, title);
        let mut on = settings["heatseeker_glow"].as_bool().unwrap_or(false);
        if ui.checkbox(&mut on, toggle_label).changed() {
            settings["heatseeker_glow"] = json!(on);
        }
        web_muted(ui, hint);
        if on {
            ui.horizontal(|ui| {
                ui.label(max_speed);
                if let Some(rgb) = web_colour(ui, web_rgb_of(&settings["heatseeker_max_speed"]), true) {
                    settings["heatseeker_max_speed"] = json!(rgb);
                }
            });
        }
    });
}

/// Card-like image button used by the texture pickers.
fn web_tile(ui: &mut egui::Ui, name: &str, image: Option<&str>, detail: &str, selected: bool, enabled: bool) -> bool {
    let accent = ui.visuals().selection.stroke.color;
    let fill = if selected { accent.gamma_multiply(0.45) } else { egui::Color32::TRANSPARENT };
    let size = egui::vec2(110.0, 112.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let line = if selected { accent } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
    let fill = if response.hovered() && enabled { ui.visuals().widgets.hovered.weak_bg_fill } else { fill };
    ui.painter().rect(rect, 6.0, fill, egui::Stroke::new(1.0, line), egui::StrokeKind::Inside);
    if let Some(image) = image.filter(|text| !text.is_empty()) {
        let art = egui::Rect::from_min_size(rect.min + egui::vec2(20.0, 4.0), egui::vec2(70.0, 64.0));
        ui.put(art, |ui: &mut egui::Ui| {
            web_image(ui, image, art.size());
            ui.label("")
        });
    }
    let color = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
    let text = ui.painter().layout(name.to_owned(), egui::FontId::proportional(12.0), color, size.x - 8.0);
    let at = egui::pos2(rect.center().x - text.size().x / 2.0, rect.bottom() - 40.0);
    ui.painter().with_clip_rect(rect).galley(at, text, color);
    if !detail.is_empty() {
        let small = ui.painter().layout_no_wrap(detail.to_owned(), egui::FontId::proportional(10.0), ui.visuals().weak_text_color());
        ui.painter().with_clip_rect(rect).galley(egui::pos2(rect.center().x - small.size().x / 2.0, rect.bottom() - 16.0), small, ui.visuals().weak_text_color());
    }
    response.on_hover_text(name).clicked() && enabled
}

impl HebnixApp {
    fn egui_menu_textures(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let action = |key: &str, value: Value| json!({ "type": "menu_textures", "key": key, "value": value });
        let busy = data["busy"].as_bool().unwrap_or(false);
        let closed = data["closed"].as_bool().unwrap_or(false);
        let has_file = data["file"].as_str().is_some_and(|text| !text.is_empty());
        let active = data["active"].as_u64().unwrap_or(0);
        web_card(ui, |ui| {
            web_muted(ui, "Close Rocket League to replace textures in its menu packages.");
            ui.horizontal_wrapped(|ui| {
                let mut packages = vec![(String::new(), "Choose a menu package".to_owned())];
                packages.extend(data["packages"].as_array().into_iter().flatten().map(|file| (val_str(file), val_str(file))));
                if let Some(file) = web_select(ui, "menu_texture_package", &val_str(&data["file"]), &packages, !busy).filter(|file| !file.is_empty()) {
                    actions.push(action("package", json!(file)));
                }
                if ui.add_enabled(has_file && !busy, egui::Button::new("Reload")).clicked() {
                    actions.push(action("reload", Value::Null));
                }
                let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                if ui.add_enabled(active > 0 && !busy && closed, egui::Button::new(format!("Restore All ({active})")).fill(danger)).clicked() {
                    actions.push(action("restore-all", Value::Null));
                }
            });
            let message = data["error"].as_str().filter(|text| !text.is_empty()).or_else(|| data["message"].as_str().filter(|text| !text.is_empty()));
            if let Some(message) = message {
                ui.label(egui::RichText::new(message).small());
            }
        });
        ui.add_space(8.0);
        web_card(ui, |ui| {
            web_heading(ui, "Replacement");
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Image file…")).clicked() {
                    actions.push(action("image", Value::Null));
                }
                if ui.add_enabled(!busy, egui::Button::new("Minecraft resource pack…")).clicked() {
                    actions.push(action("pack", Value::Null));
                }
                if let Some(value) = web_toggle(ui, "Tile (4×)", data["tile"].as_bool().unwrap_or(false), !busy) {
                    actions.push(action("tile", json!(value)));
                }
                if let Some(value) = web_toggle(ui, "Keep original alpha", data["keepAlpha"].as_bool().unwrap_or(false), !busy) {
                    actions.push(action("alpha", json!(value)));
                }
            });
            if data["replacement"].is_object() {
                web_tile(ui, &val_str(&data["replacement"]["name"]), data["replacement"]["image"].as_str(), "", false, false);
            }
            let pack_count = data["packCount"].as_u64().unwrap_or(0);
            if pack_count > 0 {
                let id = egui::Id::new("menu_texture_pack_search");
                let mut term: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| val_str(&data["packSearch"]));
                if ui.add(egui::TextEdit::singleline(&mut term).hint_text(format!("Filter {pack_count} resource pack textures")).desired_width(f32::INFINITY)).changed() {
                    actions.push(action("pack-search", json!(term)));
                }
                ui.ctx().data_mut(|d| d.insert_temp(id, term));
                ui.horizontal_wrapped(|ui| {
                    for item in data["pack"].as_array().into_iter().flatten() {
                        if web_tile(ui, &val_str(&item["name"]), item["image"].as_str(), "", false, !busy) {
                            actions.push(action("pack-select", item["name"].clone()));
                        }
                    }
                });
            }
        });
        if !has_file {
            return;
        }
        ui.add_space(8.0);
        let rows = data["rows"].as_array().cloned().unwrap_or_default();
        let selected = rows.iter().filter(|row| row["selected"].as_bool().unwrap_or(false)).count();
        let restore_count = data["restoreCount"].as_u64().unwrap_or(0);
        web_card(ui, |ui| {
            let title = if busy { "Patching…" } else if data["loading"].as_bool().unwrap_or(false) { "Loading textures…" } else { "Package textures" };
            web_heading(ui, title);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(format!("{} textures, {selected} selected", rows.len())).small());
                if ui.add_enabled(!busy && !rows.is_empty(), egui::Button::new("Select all")).clicked() {
                    actions.push(action("select-all", Value::Null));
                }
                if ui.add_enabled(!busy && selected > 0, egui::Button::new("Clear")).clicked() {
                    actions.push(action("clear", Value::Null));
                }
                if web_primary(ui, "Apply to selected", !busy && closed && selected > 0 && data["replacement"].is_object()) {
                    actions.push(action("apply", Value::Null));
                }
                if ui.add_enabled(!busy && closed && restore_count > 0, egui::Button::new(format!("Restore selected ({restore_count})"))).clicked() {
                    actions.push(action("restore", Value::Null));
                }
            });
            ui.horizontal_wrapped(|ui| {
                for item in &rows {
                    let error = item["error"].as_str().is_some_and(|text| !text.is_empty()) || item["error"].as_bool().unwrap_or(false);
                    let detail = if error {
                        "Unreadable".to_owned()
                    } else {
                        let size: Vec<String> = item["size"].as_array().into_iter().flatten().map(val_str).collect();
                        format!("{}{}", size.join(" × "), if item["patched"].as_bool().unwrap_or(false) { " · Patched" } else { "" })
                    };
                    if web_tile(ui, &val_str(&item["name"]), item["image"].as_str(), &detail, item["selected"].as_bool().unwrap_or(false), !busy && !error) {
                        actions.push(action("select", item["id"].clone()));
                    }
                }
            });
        });
    }

    fn egui_upk_history(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let cmd = |key: &str, index: Value, value: Value| json!({ "type": "upk_history", "key": key, "index": index, "value": value });
        let busy = data["busy"].as_bool().unwrap_or(false);
        let running = data["running"].as_bool().unwrap_or(false);
        let search_id = egui::Id::new("upk_history_search");
        let mut term: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
        web_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut term).hint_text(label("search")).desired_width(ui.available_width() - 100.0));
                if ui.add_enabled(!busy, egui::Button::new(label("refresh"))).clicked() {
                    actions.push(cmd("refresh", Value::Null, Value::Null));
                }
            });
            let status = data["status"].as_str().unwrap_or_default();
            if running || busy || !status.is_empty() {
                ui.label(egui::RichText::new(if running { label("close") } else if busy { "Working…".to_owned() } else { status.to_owned() }).small());
            }
        });
        let lower = term.trim().to_lowercase();
        ui.ctx().data_mut(|d| d.insert_temp(search_id, term.clone()));
        ui.add_space(8.0);
        let groups = data["groups"].as_array().cloned().unwrap_or_default();
        let mut shown = 0;
        for group in &groups {
            let name = val_str(&group["name"]);
            let rows = group["rows"].as_array().cloned().unwrap_or_default();
            if !name.to_lowercase().contains(&lower) && !rows.iter().any(|row| format!("{} {}", val_str(&row["file"]), val_str(&row["label"])).to_lowercase().contains(&lower)) {
                continue;
            }
            shown += 1;
            web_card(ui, |ui| {
                web_heading(ui, &name);
                if let Some(first) = rows.first() {
                    Self::egui_history_point(ui, first, data, busy || running, actions);
                }
                if rows.len() > 1 {
                    egui::CollapsingHeader::new(label("checkpoints")).id_salt(("history_group", val_str(&rows[0]["index"]))).show(ui, |ui| {
                        for row in &rows[1..] {
                            Self::egui_history_point(ui, row, data, busy || running, actions);
                        }
                    });
                }
            });
            ui.add_space(8.0);
        }
        if shown == 0 {
            web_muted(ui, &if lower.is_empty() { label("empty") } else { "No matching checkpoints.".to_owned() });
        }
    }

    fn egui_history_point(ui: &mut egui::Ui, row: &Value, data: &Value, locked: bool, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let cmd = |key: &str, index: Value, value: Value| json!({ "type": "upk_history", "key": key, "index": index, "value": value });
        ui.separator();
        let meta: Vec<String> = [val_str(&row["file"]), val_str(&row["date"])].into_iter().filter(|text| !text.is_empty()).collect();
        web_muted(ui, &meta.join(" · "));
        ui.label(val_str(&row["label"]));
        let extra: Vec<String> = [val_str(&row["colour"]), val_str(&row["alignment"])].into_iter().filter(|text| !text.is_empty() && text != "null").collect();
        if !extra.is_empty() {
            web_muted(ui, &extra.join(" · "));
        }
        if row["linked"].as_bool().unwrap_or(false) {
            return;
        }
        let disabled = locked || row["disabled"].as_bool().unwrap_or(false);
        let index = row["index"].clone();
        ui.horizontal(|ui| {
            if ui.add_enabled(!disabled, egui::Button::new(val_str(&row["restore"]))).clicked() {
                actions.push(cmd(row["command"].as_str().unwrap_or("restore"), index.clone(), Value::Null));
            }
            if row["after"].as_bool().unwrap_or(false) && ui.add_enabled(!disabled, egui::Button::new(label("after"))).clicked() {
                actions.push(cmd("after", index.clone(), Value::Null));
            }
        });
        if !row["edit"].is_object() {
            return;
        }
        let edit = &row["edit"];
        let title = if edit["alignment"].is_object() { label("edit") } else { label("editColour") };
        egui::CollapsingHeader::new(title).id_salt(("history_edit", val_str(&index))).show(ui, |ui| {
            let id = egui::Id::new(("history_edit_state", val_str(&index)));
            let initial_paint = if edit["paint"]["Custom"].is_array() {
                json!({ "Custom": edit["paint"]["Custom"] })
            } else {
                edit["paint"].clone()
            };
            let mut state: (Value, Value) = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| (initial_paint, edit["alignment"].clone()));
            let paints: Vec<(String, String)> = data["paints"].as_array().into_iter().flatten()
                .map(|entry| (entry[1].to_string(), val_str(&entry[0]))).collect();
            let is_custom = state.0["Custom"].is_array();
            let current = if is_custom { json!({ "Custom": [255, 255, 255] }).to_string() } else { state.0.to_string() };
            ui.horizontal(|ui| {
                if let Some(value) = web_select(ui, ("history_paint", val_str(&index)), &current, &paints, true) {
                    state.0 = serde_json::from_str(&value).unwrap_or(Value::Null);
                }
                if state.0["Custom"].is_array() {
                    if let Some(rgb) = web_colour(ui, web_rgb_of(&state.0["Custom"]), true) {
                        state.0 = json!({ "Custom": rgb });
                    }
                }
            });
            if state.1.is_object() {
                for (key, name, max) in [("camber", "Camber", 30.0), ("vertical", "Up / down", 15.0), ("lateral", "Track width", 15.0)] {
                    let mut value = state.1[key].as_f64().unwrap_or(0.0);
                    web_row(ui, name, |ui| {
                        ui.add(egui::Slider::new(&mut value, -max..=max).step_by(0.25));
                    });
                    state.1[key] = json!(value);
                }
                if ui.add_enabled(!disabled, egui::Button::new(label("reset"))).clicked() {
                    state.1 = json!({ "camber": 0, "vertical": 0, "lateral": 0 });
                }
            }
            if web_primary(ui, &label("apply"), !disabled) {
                actions.push(cmd("edit", index.clone(), json!({ "paint": state.0, "alignment": if state.1.is_object() { state.1.clone() } else { Value::Null } })));
            }
            ui.ctx().data_mut(|d| d.insert_temp(id, state));
        });
    }
}

#[derive(Clone, Default)]
struct AssetView {
    key: String,
    search: String,
    page: usize,
    requested: String,
    applied: bool,
    downloaded: bool,
}

impl HebnixApp {
    fn egui_studio_asset(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let decal = data["kind"] == "decal";
        let custom = data["custom"].as_bool().unwrap_or(false);
        let busy = data["busy"].as_bool().unwrap_or(false);
        let has_error = data["error"].as_str().is_some_and(|text| !text.is_empty());
        let action = |command: &str, id: Value, value: Value| {
            if decal {
                json!({ "type": "studio_decal", "key": command, "id": id, "value": value })
            } else {
                json!({ "type": "studio_asset", "key": command, "id": id })
            }
        };
        let view_id = egui::Id::new("studio_asset_view");
        let mut view: AssetView = ui.ctx().data(|d| d.get_temp(view_id)).unwrap_or_default();
        let key = format!("{}:{custom}", val_str(&data["kind"]));
        if view.key != key {
            view = AssetView { key, ..Default::default() };
        }
        web_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let accent = ui.visuals().selection.stroke.color.gamma_multiply(0.6);
                if ui.add(egui::Button::new("Catalog").fill(if custom { egui::Color32::TRANSPARENT } else { accent })).clicked() {
                    actions.push(action("source", json!("catalog"), Value::Null));
                }
                if ui.add(egui::Button::new("Custom").fill(if custom { accent } else { egui::Color32::TRANSPARENT })).clicked() {
                    actions.push(action("source", json!("custom"), Value::Null));
                }
                if ui.button(label("refresh")).clicked() {
                    actions.push(action("refresh", Value::Null, Value::Null));
                }
                if ui.add_enabled(!busy, egui::Button::new(label("import"))).clicked() {
                    actions.push(action("import", Value::Null, Value::Null));
                }
                if ui.add_enabled(data["active"].as_bool().unwrap_or(false) && !busy && !(decal && has_error), egui::Button::new(label("restore"))).clicked() {
                    actions.push(action("restore", Value::Null, Value::Null));
                }
                if data["kind"] == "boost" && ui.add_enabled(!busy, egui::Button::new(label("ring"))).on_hover_text(label("ringHint")).clicked() {
                    actions.push(action("ring", Value::Null, Value::Null));
                }
            });
            if custom {
                ui.checkbox(&mut view.applied, label("applied"));
            } else {
                ui.checkbox(&mut view.downloaded, label("downloaded"));
            }
            ui.add(egui::TextEdit::singleline(&mut view.search).hint_text(label("search")).desired_width(f32::INFINITY));
        });
        ui.add_space(8.0);
        let term = view.search.trim().to_lowercase();
        let source = if custom { data["rows"].clone() } else { data["catalog"]["rows"].clone() };
        let items: Vec<Value> = source.as_array().cloned().unwrap_or_default().into_iter().filter(|item| {
            let text = format!("{} {}", val_str(&item["name"]), val_str(&item["author"])).to_lowercase();
            text.contains(&term) && if custom { !view.applied || item["active"].as_bool().unwrap_or(false) } else { !view.downloaded || item["downloaded"].as_bool().unwrap_or(false) }
        }).collect();
        let card_w = 175.0;
        let width = if decal { ui.available_width() * 0.62 } else { ui.available_width() };
        let per_row = ((width / (card_w + 16.0)).floor() as usize).max(1);
        let page_size = (per_row * 3).min(36);
        let pages = items.len().div_ceil(page_size).max(1);
        view.page = view.page.min(pages - 1);
        let visible: Vec<&Value> = items.iter().skip(view.page * page_size).take(page_size).collect();
        let ids: Vec<Value> = visible.iter().map(|item| item["id"].clone()).collect();
        let signature = json!(ids).to_string();
        if view.requested != signature {
            view.requested = signature;
            actions.push(json!({ "type": "studio_asset_previews", "ids": ids }));
        }
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                for chunk in visible.chunks(per_row) {
                    ui.horizontal_top(|ui| {
                        for item in chunk {
                            Self::egui_asset_card(ui, item, data, decal, custom, busy, card_w, actions, &action);
                        }
                    });
                    ui.add_space(8.0);
                }
                if items.is_empty() {
                    web_muted(ui, if data["catalog"]["loading"].as_bool().unwrap_or(false) { "Loading…" } else { "No matching items." });
                }
                if pages > 1 {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(view.page > 0, egui::Button::new("Previous")).clicked() {
                            view.page -= 1;
                        }
                        ui.label(format!("{} / {pages}", view.page + 1));
                        if ui.add_enabled(view.page + 1 < pages, egui::Button::new("Next")).clicked() {
                            view.page += 1;
                        }
                    });
                }
            });
            if decal {
                web_card(ui, |ui| {
                    let active = data["activeDecals"].as_array().cloned().unwrap_or_default();
                    web_heading(ui, &format!("{} · {}", label("active"), active.len()));
                    if active.is_empty() {
                        web_muted(ui, &label("empty"));
                    }
                    for row in &active {
                        ui.separator();
                        ui.horizontal(|ui| {
                            if let Some(image) = row["image"].as_str().filter(|text| !text.is_empty()) {
                                web_image(ui, image, egui::vec2(42.0, 42.0));
                            }
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(val_str(&row["name"])).strong().small());
                                web_muted(ui, &val_str(&row["target"]));
                            });
                            if ui.add_enabled(!busy, egui::Button::new("Restore")).clicked() {
                                actions.push(action("restore", row["id"].clone(), Value::Null));
                            }
                        });
                    }
                });
            }
        });
        if decal && custom && has_error {
            if let Some(error) = data["catalogError"].as_str().filter(|text| !text.is_empty()) {
                web_card(ui, |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.button(label("retry")).clicked() {
                        actions.push(action("reload", Value::Null, Value::Null));
                    }
                });
            }
        }
        if decal && busy {
            let status = data["status"].as_str().filter(|text| !text.is_empty()).unwrap_or("Working…");
            web_muted(ui, status);
            web_progress(ui, true, &data["progress"]);
        }
        if let Some(status) = data["catalog"]["status"].as_str().filter(|text| !text.is_empty()) {
            web_muted(ui, status);
        }
        if let Some(error) = data["error"].as_str().filter(|text| !text.is_empty()) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if data["confirmRestore"].as_bool().unwrap_or(false) {
            web_modal(ui.ctx(), "studio_restore_all", 420.0, |ui| {
                web_heading(ui, &label("restoreTitle"));
                ui.label(format!("Restore all {} patched decal targets?", data["activeDecals"].as_array().map_or(0, Vec::len)));
                ui.horizontal(|ui| {
                    if ui.button(label("no")).clicked() {
                        actions.push(action("cancel", Value::Null, Value::Null));
                    }
                    if ui.add_enabled(!busy, egui::Button::new(label("yes")).fill(ui.visuals().error_fg_color.gamma_multiply(0.6))).clicked() {
                        actions.push(action("restore-all", Value::Null, Value::Null));
                    }
                });
            });
        }
        if let Some(name) = data["confirm"].as_str().filter(|text| !text.is_empty()) {
            web_modal(ui.ctx(), "studio_delete", 420.0, |ui| {
                web_heading(ui, &label("confirm"));
                ui.label(format!("Delete {name}?"));
                ui.horizontal(|ui| {
                    if ui.button(label("no")).clicked() {
                        actions.push(action("cancel", Value::Null, Value::Null));
                    }
                    if ui.add_enabled(!busy, egui::Button::new(label("yes")).fill(ui.visuals().error_fg_color.gamma_multiply(0.6))).clicked() {
                        actions.push(action("confirm", Value::Null, Value::Null));
                    }
                });
            });
        }
        ui.ctx().data_mut(|d| d.insert_temp(view_id, view));
    }

    #[allow(clippy::too_many_arguments)]
    fn egui_asset_card(ui: &mut egui::Ui, item: &Value, data: &Value, decal: bool, custom: bool, busy: bool, width: f32, actions: &mut Vec<Value>, action: &dyn Fn(&str, Value, Value) -> Value) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let id = item["id"].clone();
        web_card(ui, |ui| {
            ui.set_width(width - 34.0);
            ui.vertical_centered(|ui| {
                let images: Vec<String> = item["images"].as_array().into_iter().flatten().filter_map(|value| value.as_str().map(str::to_owned)).collect();
                let step_id = egui::Id::new(("asset_preview", val_str(&id)));
                let mut current: usize = ui.ctx().data(|d| d.get_temp(step_id)).unwrap_or(0);
                let shown: Vec<&String> = if custom { images.iter().collect() } else { images.get(current % images.len().max(1)).into_iter().collect() };
                if shown.is_empty() {
                    web_muted(ui, "No preview");
                }
                for source in shown {
                    web_image(ui, source, egui::vec2(width - 34.0, 96.0));
                }
                if !custom && images.len() > 1 {
                    ui.horizontal(|ui| {
                        if ui.small_button("‹").clicked() {
                            current = (current + images.len() - 1) % images.len();
                        }
                        if ui.small_button("›").clicked() {
                            current = (current + 1) % images.len();
                        }
                    });
                    ui.ctx().data_mut(|d| d.insert_temp(step_id, current));
                }
                if item["pack"].as_bool().unwrap_or(false) {
                    ui.colored_label(ui.visuals().error_fg_color, "PACK");
                }
                ui.label(egui::RichText::new(val_str(&item["name"])).strong());
                if !custom {
                    if let Some(author) = item["author"].as_str().filter(|text| !text.is_empty()) {
                        ui.label(egui::RichText::new(author).small());
                    }
                    web_muted(ui, &format!("{} downloads", val_str(&item["downloads"])));
                    let item_busy = item["busy"].as_bool().unwrap_or(false);
                    let text = if item_busy { "Downloading…" } else if item["downloaded"].as_bool().unwrap_or(false) { "Import" } else { "Download" };
                    if web_primary(ui, text, !item_busy && !busy) {
                        actions.push(action("download", id.clone(), Value::Null));
                    }
                    return;
                }
                if let Some(subtitle) = item["subtitle"].as_str().filter(|text| !text.is_empty()) {
                    web_muted(ui, subtitle);
                }
                if decal {
                    if item["chassis"].as_bool().unwrap_or(false) {
                        web_muted(ui, "Chassis textures skipped");
                    }
                    let has_body = item["body"].as_bool().unwrap_or(false) || item["body"].as_str().is_some_and(|text| !text.is_empty());
                    let target = val_str(&item["target"]);
                    let targets: Vec<Value> = item["targets"].as_array().cloned().unwrap_or_default();
                    let target_open = targets.iter().any(|entry| val_str(&entry[0]) == target && !entry[2].as_bool().unwrap_or(false));
                    if !has_body {
                        let mut bodies = vec![(String::new(), "Select body…".to_owned())];
                        bodies.extend(web_pairs(&data["bodies"]));
                        if let Some(value) = web_select(ui, ("decal_body", val_str(&id)), "", &bodies, true).filter(|value| !value.is_empty()) {
                            actions.push(action("body", id.clone(), json!(value)));
                        }
                    } else {
                        let filter_id = egui::Id::new(("decal_filter", val_str(&id)));
                        let mut filter: String = ui.ctx().data(|d| d.get_temp(filter_id)).unwrap_or_default();
                        ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter decals").desired_width(f32::INFINITY));
                        let lower = filter.trim().to_lowercase();
                        let options: Vec<(String, String)> = targets.iter()
                            .filter(|entry| val_str(&entry[0]) == target || format!("{} {}", val_str(&entry[1]), val_str(&entry[0])).to_lowercase().contains(&lower))
                            .map(|entry| (val_str(&entry[0]), format!("{}{}", val_str(&entry[1]), if entry[2].as_bool().unwrap_or(false) { " (patched)" } else { "" })))
                            .collect();
                        if let Some(value) = web_select(ui, ("decal_target", val_str(&id)), &target, &options, !busy) {
                            actions.push(action("target", id.clone(), json!(value)));
                        }
                        ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter));
                    }
                    if data["speed"].as_bool().unwrap_or(false) {
                        let mut speed = item["speed"].as_f64().unwrap_or(1.0);
                        let response = ui.add_enabled(!busy, egui::Slider::new(&mut speed, 0.1..=20.0).step_by(0.05).suffix("×").text("Speed"));
                        if response.drag_stopped() || (response.changed() && !response.dragged()) {
                            actions.push(action("speed", id.clone(), json!(speed)));
                        }
                    }
                    ui.horizontal(|ui| {
                        if web_primary(ui, &label("apply"), !busy && !has_error_str(data) && target_open) {
                            actions.push(action("apply", id.clone(), Value::Null));
                        }
                        if ui.add_enabled(!busy && item["delete"].as_bool().unwrap_or(false), egui::Button::new(label("delete"))).clicked() {
                            actions.push(action("delete", id.clone(), Value::Null));
                        }
                    });
                    return;
                }
                let restoring = item["restore"].as_bool().or_else(|| item["active"].as_bool()).unwrap_or(false);
                let item_error = item["error"].as_str().filter(|text| !text.is_empty());
                if let Some(error) = item_error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.horizontal(|ui| {
                    let text = if restoring { label("restore") } else { label("apply") };
                    if ui.add_enabled(!busy && (restoring || item_error.is_none()), egui::Button::new(text)).clicked() {
                        actions.push(action(if restoring { "restore" } else { "apply" }, id.clone(), Value::Null));
                    }
                    if ui.add_enabled(!busy && item["delete"].as_bool().unwrap_or(false), egui::Button::new(label("delete"))).clicked() {
                        actions.push(action("delete", id.clone(), Value::Null));
                    }
                });
            });
        });
    }
}

fn has_error_str(data: &Value) -> bool {
    data["error"].as_str().is_some_and(|text| !text.is_empty())
}

#[derive(Clone, Default)]
struct ArtView {
    key: String,
    query: String,
    page: usize,
    names: bool,
    patched: bool,
    requested: String,
    draft: (String, String, String),
}

fn web_pager(ui: &mut egui::Ui, page: &mut usize, pages: usize) {
    if pages < 2 {
        return;
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(*page > 0, egui::Button::new("Previous")).clicked() {
            *page -= 1;
        }
        ui.label(format!("{} / {pages}", *page + 1));
        if ui.add_enabled(*page + 1 < pages, egui::Button::new("Next")).clicked() {
            *page += 1;
        }
    });
}

fn web_preview_box(ui: &mut egui::Ui, image: Option<&str>, size: egui::Vec2, missing: &str) {
    match image.filter(|text| !text.is_empty()) {
        Some(image) => web_image(ui, image, size),
        None => {
            ui.allocate_ui(size, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new(missing).small().color(ui.visuals().weak_text_color()));
                });
            });
        }
    }
}

impl HebnixApp {
    fn egui_item_artwork(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let cmd = |key: &str, value: Value| json!({ "type": "item_artwork", "key": key, "value": value });
        let names = data["kind"] == "names";
        let busy = data["busy"].as_bool().unwrap_or(false);
        let closed = data["closed"].as_bool().unwrap_or(false);
        let unavailable = busy || !closed;
        let view_id = egui::Id::new("item_artwork_view");
        let mut view: ArtView = ui.ctx().data(|d| d.get_temp(view_id)).unwrap_or_default();
        let key = ["kind", "category", "language", "owned", "patched"].iter().map(|name| val_str(&data[*name])).collect::<Vec<_>>().join(":");
        if view.key != key {
            view = ArtView { key, ..Default::default() };
        }
        let global_owned = data["globalOwnedOnly"].as_bool().unwrap_or(false);
        if !names && data["owned"].as_bool().unwrap_or(false) != global_owned {
            actions.push(cmd("owned", json!(global_owned)));
        }
        web_card(ui, |ui| {
            web_muted(ui, &label("description"));
            ui.horizontal_wrapped(|ui| {
                if names {
                    let languages: Vec<(String, String)> = data["languages"].as_array().into_iter().flatten()
                        .map(|file| (val_str(file), val_str(file).trim_start_matches("Coalesced_").trim_end_matches(".bin").to_owned())).collect();
                    if let Some(value) = web_select(ui, "artwork_language", &val_str(&data["language"]), &languages, true) {
                        actions.push(cmd("language", json!(value)));
                    }
                }
                if let Some(value) = web_select(ui, "artwork_category", &val_str(&data["category"]), &web_pairs(&data["categories"]), true) {
                    actions.push(cmd("category", json!(value)));
                }
                if ui.add_enabled(!busy, egui::Button::new("Refresh")).clicked() {
                    actions.push(cmd("refresh", Value::Null));
                }
                let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                if ui.add_enabled(!unavailable && data["active"].as_bool().unwrap_or(false), egui::Button::new(label("restoreAll")).fill(danger)).clicked() {
                    actions.push(cmd("restore-all", Value::Null));
                }
            });
            let error = data["error"].as_str().filter(|text| !text.is_empty());
            let status = data["status"].as_str().filter(|text| !text.is_empty());
            if error.is_some() || status.is_some() || busy {
                match error {
                    Some(error) => ui.colored_label(ui.visuals().error_fg_color, error),
                    None => ui.label(egui::RichText::new(if busy { "Working…" } else { status.unwrap_or_default() }).small()),
                };
            }
            ui.horizontal_wrapped(|ui| {
                let patched = if names { view.patched } else { data["patched"].as_bool().unwrap_or(false) };
                if let Some(value) = web_toggle(ui, &label("patched"), patched, !busy) {
                    if names {
                        view.patched = value;
                        view.page = 0;
                        actions.push(cmd("cancel", Value::Null));
                    } else {
                        actions.push(cmd("patched", json!(value)));
                    }
                }
                if names {
                    ui.checkbox(&mut view.names, label("showNames"));
                }
            });
        });
        ui.add_space(8.0);
        let selected = &data["selection"];
        if selected.is_object() {
            web_card(ui, |ui| {
                web_heading(ui, selected["original"].as_str().filter(|text| !text.is_empty()).unwrap_or_else(|| selected["name"].as_str().unwrap_or_default()));
                if names {
                    let id = selected.to_string();
                    if view.draft.0 != id {
                        view.draft = (id, val_str(&selected["name"]), val_str(&selected["artist"]));
                    }
                    for field in ["name", "artist"] {
                        if selected[field].is_null() {
                            continue;
                        }
                        ui.label(label(field));
                        let target = if field == "name" { &mut view.draft.1 } else { &mut view.draft.2 };
                        ui.add_enabled(!busy, egui::TextEdit::singleline(target).char_limit(128).desired_width(f32::INFINITY));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if web_primary(ui, &label("apply"), !unavailable && !view.draft.1.trim().is_empty()) {
                            actions.push(cmd("save", json!({ "name": view.draft.1, "artist": view.draft.2 })));
                        }
                        let fields: Vec<String> = selected["fields"].as_array().into_iter().flatten().map(val_str).collect();
                        if !fields.is_empty() && ui.add_enabled(!unavailable, egui::Button::new(label("restore"))).clicked() {
                            actions.push(cmd("restore", Value::Null));
                        }
                        if !selected["artist"].is_null() {
                            for (field, name) in [("Label", label("restoreSong")), ("Artist", label("restoreArtist"))] {
                                if fields.iter().any(|entry| entry == field) && ui.add_enabled(!unavailable, egui::Button::new(name)).clicked() {
                                    actions.push(cmd("restore-field", json!(field)));
                                }
                            }
                        }
                        if ui.add_enabled(!busy, egui::Button::new(label("cancel"))).clicked() {
                            actions.push(cmd("cancel", Value::Null));
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        for (name, image) in [(label("current"), selected["image"].as_str()), (label("replacement"), selected["replacement"].as_str())] {
                            if name == label("replacement") && image.is_none_or(str::is_empty) {
                                continue;
                            }
                            ui.vertical(|ui| {
                                web_preview_box(ui, image, egui::vec2(144.0, 100.0), "No preview");
                                web_muted(ui, &name);
                            });
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new(label("choose"))).clicked() {
                            actions.push(cmd("choose", Value::Null));
                        }
                        let ready = selected["image"].as_str().is_some_and(|text| !text.is_empty()) && selected["replacement"].as_str().is_some_and(|text| !text.is_empty());
                        if web_primary(ui, &label("apply"), !unavailable && ready) {
                            actions.push(cmd("save", Value::Null));
                        }
                        if !selected["export"].is_null() {
                            let id = val_str(&selected["id"]);
                            let file = id.rsplit_once('|').map_or(id.as_str(), |(file, _)| file).to_owned();
                            if ui.add_enabled(!unavailable, egui::Button::new(label("restore"))).clicked() {
                                actions.push(cmd("restore", json!(format!("{file}|{}", val_str(&selected["export"])))));
                            }
                        }
                        if ui.add_enabled(!busy, egui::Button::new(label("cancel"))).clicked() {
                            actions.push(cmd("cancel", Value::Null));
                        }
                    });
                }
            });
            ui.add_space(8.0);
        } else {
            view.draft = Default::default();
        }
        ui.add(egui::TextEdit::singleline(&mut view.query).hint_text(label("search")).desired_width(f32::INFINITY));
        ui.add_space(6.0);
        let term = view.query.trim().to_lowercase();
        let active_thumbs = !names && data["patched"].as_bool().unwrap_or(false);
        let items: Vec<&Value> = data["rows"].as_array().into_iter().flatten().filter(|item| {
            let owned_ok = !names || ((!global_owned || item["owned"].as_bool().unwrap_or(false)) && (!view.patched || item["patched"].as_bool().unwrap_or(false)));
            let text = ["name", "current", "artist", "currentArtist", "file", "id"].iter().map(|k| val_str(&item[*k])).collect::<Vec<_>>().join(" ").to_lowercase();
            owned_ok && text.contains(&term)
        }).collect();
        let card_w = 175.0;
        let per_row = if active_thumbs { 1 } else { ((ui.available_width() / (card_w + 16.0)).floor() as usize).max(1) };
        let size = if active_thumbs { 36 } else { (per_row * if names { 3 } else { 2 }).min(36) };
        let pages = items.len().div_ceil(size).max(1);
        view.page = view.page.min(pages - 1);
        let visible: Vec<&Value> = items.iter().skip(view.page * size).take(size).copied().collect();
        let ids: Vec<Value> = visible.iter().map(|item| item["id"].clone()).collect();
        let signature = json!(ids).to_string();
        if view.requested != signature {
            view.requested = signature;
            actions.push(json!({ "type": "studio_asset_previews", "ids": ids }));
        }
        for chunk in visible.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for item in chunk {
                    web_card(ui, |ui| {
                        if active_thumbs {
                            ui.horizontal(|ui| {
                                web_preview_box(ui, item["image"].as_str(), egui::vec2(60.0, 50.0), "No preview");
                                ui.label(egui::RichText::new(val_str(&item["name"])).strong());
                                if ui.add_enabled(!unavailable, egui::Button::new(label("restore"))).clicked() {
                                    actions.push(cmd("restore", item["id"].clone()));
                                }
                            });
                            return;
                        }
                        ui.set_width(card_w - 34.0);
                        ui.vertical_centered(|ui| {
                            let missing = item["error"].as_str().filter(|text| !text.is_empty()).unwrap_or("No preview");
                            web_preview_box(ui, item["image"].as_str(), egui::vec2(card_w - 34.0, 90.0), missing);
                            let title = if names && view.names { val_str(&item["current"]) } else { val_str(&item["name"]) };
                            ui.label(egui::RichText::new(title).strong());
                            let artist = if view.names { val_str(&item["currentArtist"]) } else { val_str(&item["artist"]) };
                            if !artist.is_empty() {
                                web_muted(ui, &artist);
                            }
                            if names && item["patched"].as_bool().unwrap_or(false) {
                                web_muted(ui, "Patched");
                            }
                            let image_missing = item["image"].as_str().is_none_or(str::is_empty);
                            if ui.add_enabled(!busy && (names || !image_missing), egui::Button::new(label("edit"))).clicked() {
                                actions.push(cmd("select", item["id"].clone()));
                            }
                        });
                    });
                }
            });
            ui.add_space(8.0);
        }
        if visible.is_empty() {
            web_muted(ui, &label("empty"));
        }
        web_pager(ui, &mut view.page, pages);
        ui.ctx().data_mut(|d| d.insert_temp(view_id, view));
    }

    fn egui_anthems(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let cmd = |key: &str, id: Value, value: Value| json!({ "type": "anthem", "key": key, "id": id, "value": value });
        let busy = data["busy"].as_bool().unwrap_or(false);
        let closed = data["closed"].as_bool().unwrap_or(false);
        let error = data["error"].as_str().filter(|text| !text.is_empty());
        if let Some(text) = error.or_else(|| data["status"].as_str().filter(|text| !text.is_empty())) {
            if error.is_some() {
                ui.colored_label(ui.visuals().error_fg_color, text);
            } else {
                web_muted(ui, text);
            }
        }
        let editor = &data["editor"];
        if editor.is_object() {
            web_card(ui, |ui| {
                if ui.add_enabled(!busy, egui::Button::new("‹ All anthems")).clicked() {
                    actions.push(cmd("back", Value::Null, Value::Null));
                }
                ui.horizontal(|ui| {
                    if let Some(image) = editor["image"].as_str().filter(|text| !text.is_empty()) {
                        web_image(ui, image, egui::vec2(85.0, 75.0));
                    }
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(val_str(&editor["name"])).size(18.0).strong());
                        web_muted(ui, &val_str(&editor["file"]));
                    });
                });
                if ui.add_enabled(!busy, egui::Button::new("Import Audio")).clicked() {
                    actions.push(cmd("import", Value::Null, Value::Null));
                }
                web_muted(ui, editor["source"].as_str().filter(|text| !text.is_empty()).unwrap_or("Import audio to replace the anthem."));
                if let Some(region) = editor["region"].as_str().filter(|text| !text.is_empty()) {
                    ui.label(egui::RichText::new(format!("All {} regions · longest: {region} ({:.3} s)", val_str(&editor["regions"]), editor["length"].as_f64().unwrap_or(0.0))).small());
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!busy && editor["original"].as_bool().unwrap_or(false), egui::Button::new(format!("Preview original {region}"))).clicked() {
                            actions.push(cmd("original", Value::Null, Value::Null));
                        }
                        if editor["previewType"] == "original" {
                            let playing = editor["playing"].as_bool().unwrap_or(false);
                            if ui.add_enabled(!busy, egui::Button::new(if playing { "Pause original" } else { "Resume original" })).clicked() {
                                actions.push(cmd("original-toggle", Value::Null, Value::Null));
                            }
                            if ui.add_enabled(!busy, egui::Button::new("Stop original")).clicked() {
                                actions.push(cmd("stop", Value::Null, Value::Null));
                            }
                        }
                    });
                } else {
                    ui.colored_label(ui.visuals().error_fg_color, editor["error"].as_str().filter(|text| !text.is_empty()).unwrap_or("Anthem clip timings could not be read."));
                }
                if editor["duration"].as_f64().is_some_and(|value| value > 0.0) && editor["maxStart"].is_null() {
                    ui.colored_label(ui.visuals().warn_fg_color, "The imported audio must cover the longest region.");
                }
                if let Some(max_start) = editor["maxStart"].as_f64() {
                    let duration = editor["duration"].as_f64().unwrap_or(1.0).max(0.001);
                    let length = editor["length"].as_f64().unwrap_or(0.0);
                    let start = editor["start"].as_f64().unwrap_or(0.0);
                    let drag_id = egui::Id::new("anthem_drag");
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 120.0), egui::Sense::click_and_drag());
                    let to_time = |x: f32| ((x - rect.left()) / rect.width()) as f64 * duration;
                    let mut shown_start = start;
                    let mut drag: Option<(f64, f64)> = ui.ctx().data(|d| d.get_temp(drag_id));
                    if response.drag_started() && !busy {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let time = to_time(pos.x);
                            let threshold = 8.0 / rect.width() as f64 * duration;
                            if time >= start - threshold && time <= start + length + threshold {
                                drag = Some((time, time - start));
                            }
                        }
                    }
                    if let (Some((_, offset)), Some(pos)) = (drag, response.interact_pointer_pos()) {
                        shown_start = (to_time(pos.x) - offset).clamp(0.0, max_start);
                    }
                    if response.drag_stopped() {
                        if let Some((origin, _)) = drag {
                            if (shown_start - start).abs() > 0.002 || (to_time(response.interact_pointer_pos().map_or(rect.left(), |p| p.x)) - origin).abs() > 0.002 {
                                actions.push(cmd("start", Value::Null, json!(shown_start)));
                            }
                        }
                        drag = None;
                    } else if response.clicked() && drag.is_none() && !busy {
                        if let Some(pos) = response.interact_pointer_pos() {
                            actions.push(cmd("seek", Value::Null, json!(to_time(pos.x))));
                        }
                    }
                    ui.ctx().data_mut(|d| match drag {
                        Some(value) => { d.insert_temp(drag_id, value); }
                        None => d.remove::<(f64, f64)>(drag_id),
                    });
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                    let x_of = |time: f64| rect.left() + (time / duration) as f32 * rect.width();
                    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x_of(shown_start), rect.top()), egui::pos2(x_of(shown_start + length), rect.bottom())), 0.0, egui::Color32::from_rgb(23, 55, 84));
                    let peaks = editor["peaks"].as_array().cloned().unwrap_or_default();
                    for (index, peak) in peaks.iter().enumerate() {
                        let x = rect.left() + index as f32 / peaks.len().max(1) as f32 * rect.width();
                        let half = peak.as_f64().unwrap_or(0.0) as f32 * rect.height() / 2.0;
                        painter.line_segment([egui::pos2(x, rect.center().y - half), egui::pos2(x, rect.center().y + half)], egui::Stroke::new(1.0, egui::Color32::from_rgb(131, 187, 223)));
                    }
                    let cursor = if drag.is_some() { shown_start } else { editor["position"].as_f64().unwrap_or(start) };
                    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(x_of(cursor), rect.top()), egui::vec2(2.0, rect.height())), 0.0, egui::Color32::from_rgb(214, 239, 255));
                    ui.horizontal_wrapped(|ui| {
                        let playing = editor["playing"].as_bool().unwrap_or(false) && editor["previewType"] == "imported";
                        if ui.add_enabled(!busy, egui::Button::new(if playing { "Pause" } else { "Play" })).clicked() {
                            actions.push(cmd("play", Value::Null, Value::Null));
                        }
                        if ui.add_enabled(!busy, egui::Button::new("Stop")).clicked() {
                            actions.push(cmd("stop", Value::Null, Value::Null));
                        }
                        ui.label("Start (s)");
                        if let Some(value) = web_number(ui, start, Some((0.0, max_start)), 0.001, !busy) {
                            actions.push(cmd("start", Value::Null, json!(value)));
                        }
                        web_muted(ui, &format!("Stop {:.3} s", start + length));
                    });
                    let mut volume = editor["volume"].as_f64().unwrap_or(1.0) * 100.0;
                    let response = ui.add_enabled(!busy, egui::Slider::new(&mut volume, 0.0..=100.0).text("Song volume"));
                    if response.drag_stopped() || (response.changed() && !response.dragged()) {
                        actions.push(cmd("volume", Value::Null, json!(volume / 100.0)));
                    }
                    if web_primary(ui, "Save and patch anthem", !busy && closed) {
                        actions.push(cmd("save", Value::Null, Value::Null));
                    }
                }
            });
            return;
        }
        let view_id = egui::Id::new("anthem_view");
        let mut view: ArtView = ui.ctx().data(|d| d.get_temp(view_id)).unwrap_or_default();
        let owned_only = data["globalOwnedOnly"].as_bool().unwrap_or(false);
        web_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Refresh")).clicked() {
                    actions.push(cmd("refresh", Value::Null, Value::Null));
                }
                let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                if ui.add_enabled(!busy && closed && data["active"].as_bool().unwrap_or(false), egui::Button::new("Restore all anthems").fill(danger)).clicked() {
                    actions.push(cmd("restore-all", Value::Null, Value::Null));
                }
            });
            if ui.checkbox(&mut view.patched, "Show patched anthems").changed() {
                view.page = 0;
            }
            ui.add(egui::TextEdit::singleline(&mut view.query).hint_text("Search anthems").desired_width(f32::INFINITY));
        });
        ui.add_space(8.0);
        let term = view.query.trim().to_lowercase();
        let items: Vec<&Value> = data["rows"].as_array().into_iter().flatten().filter(|item| {
            (!owned_only || item["owned"].as_bool().unwrap_or(false))
                && (!view.patched || item["active"].as_bool().unwrap_or(false))
                && format!("{} {}", val_str(&item["name"]), val_str(&item["file"])).to_lowercase().contains(&term)
        }).collect();
        let card_w = 175.0;
        let per_row = ((ui.available_width() / (card_w + 16.0)).floor() as usize).max(1);
        let size = (per_row * 3).min(36);
        let pages = items.len().div_ceil(size).max(1);
        view.page = view.page.min(pages - 1);
        let visible: Vec<&Value> = items.iter().skip(view.page * size).take(size).copied().collect();
        let ids: Vec<Value> = visible.iter().map(|item| item["id"].clone()).collect();
        let signature = json!(ids).to_string();
        if view.requested != signature {
            view.requested = signature;
            actions.push(json!({ "type": "studio_asset_previews", "ids": ids }));
        }
        for chunk in visible.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for item in chunk {
                    web_card(ui, |ui| {
                        ui.set_width(card_w - 34.0);
                        ui.vertical_centered(|ui| {
                            web_preview_box(ui, item["image"].as_str(), egui::vec2(card_w - 34.0, 90.0), "No preview");
                            ui.label(egui::RichText::new(val_str(&item["name"])).strong()).on_hover_text(val_str(&item["file"]));
                            if ui.add_enabled(!busy, egui::Button::new("Select")).clicked() {
                                actions.push(cmd("select", item["id"].clone(), Value::Null));
                            }
                            if item["patched"].as_bool().unwrap_or(false) {
                                if let Some(value) = web_toggle(ui, "Patch", item["active"].as_bool().unwrap_or(false), !busy && closed) {
                                    actions.push(cmd("patch", item["id"].clone(), json!(value)));
                                }
                            }
                        });
                    });
                }
            });
            ui.add_space(8.0);
        }
        if items.is_empty() {
            web_muted(ui, if busy { "Loading…" } else { "No installed anthems match this filter." });
        }
        web_pager(ui, &mut view.page, pages);
        ui.ctx().data_mut(|d| d.insert_temp(view_id, view));
    }
}

impl HebnixApp {
    fn egui_profile_pictures(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let set = |key: &str, value: Value| json!({ "type": "profile_picture", "key": key, "value": value });
        let busy = data["busy"].as_bool().unwrap_or(false);
        let owner = data["owner"].as_str().is_some_and(|text| !text.is_empty());
        ui.label("Choose a picture to share with other Hebnix players.");
        web_muted(ui, "Click a picture under Recent pictures to upload it as your avatar.");
        ui.add_space(6.0);
        let gap = 16.0;
        let column = ((ui.available_width() - gap) / 2.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(column - 34.0);
                    if let Some(value) = web_toggle(ui, "Show player profile pictures", data["enabled"].as_bool().unwrap_or(false), data["canToggle"].as_bool().unwrap_or(false)) {
                        actions.push(set("enabled", json!(value)));
                    }
                    // the account is picked once, in the header's quick settings
                    if !owner {
                        web_muted(ui, "Open Rocket League to upload");
                    }
                    if data["update"].as_bool().unwrap_or(false) && ui.add_enabled(data["canToggle"].as_bool().unwrap_or(false), egui::Button::new("Update avatar support")).clicked() {
                        actions.push(set("update", json!(true)));
                    }
                    if data["retry"].as_bool().unwrap_or(false) && ui.button("Retry connection").clicked() {
                        actions.push(set("retry", Value::Null));
                    }
                    if let Some(current) = data["current"].as_str().filter(|text| !text.is_empty()) {
                        web_heading(ui, "Current picture");
                        web_image(ui, current, egui::vec2(128.0, 128.0));
                    }
                });
            });
            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(column - 34.0);
                    web_heading(ui, "Recent pictures");
                    if ui.add_enabled(!busy, egui::Button::new("Import picture…")).clicked() {
                        actions.push(set("import", Value::Null));
                    }
                    ui.add_space(6.0);
                    let per_row = ((ui.available_width() / 84.0).floor() as usize).max(1);
                    let rows = data["rows"].as_array().cloned().unwrap_or_default();
                    for chunk in rows.chunks(per_row) {
                        ui.horizontal_top(|ui| {
                            for entry in chunk {
                                let active = entry["id"] == data["active"];
                                let accent = ui.visuals().selection.stroke.color;
                                let size = egui::vec2(76.0, 76.0);
                                let (rect, response) = ui.allocate_exact_size(size, if busy || !owner { egui::Sense::hover() } else { egui::Sense::click() });
                                ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(if active { 2.0 } else { 1.0 }, if active { accent } else { ui.visuals().widgets.noninteractive.bg_stroke.color }), egui::StrokeKind::Inside);
                                if let Some(image) = entry["image"].as_str() {
                                    ui.put(rect.shrink(4.0), |ui: &mut egui::Ui| {
                                        web_image(ui, image, rect.shrink(4.0).size());
                                        ui.label("")
                                    });
                                }
                                // the remove button sits on the thumbnail's corner, like the web tile
                                let corner = egui::Rect::from_center_size(rect.right_top() + egui::vec2(-4.0, 4.0), egui::vec2(18.0, 18.0));
                                let remove = ui.interact(corner, response.id.with("remove"), if busy { egui::Sense::hover() } else { egui::Sense::click() });
                                let fill = if remove.hovered() && !busy { ui.visuals().error_fg_color } else { ui.visuals().widgets.inactive.bg_fill };
                                ui.painter().circle(corner.center(), 9.0, fill, ui.visuals().widgets.noninteractive.bg_stroke);
                                ui.painter().text(corner.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), ui.visuals().strong_text_color());
                                if remove.on_hover_text("Remove from recent pictures").clicked() {
                                    actions.push(set("remove", entry["id"].clone()));
                                } else if response.on_hover_text(if active { "Upload current picture again" } else { "Upload picture" }).clicked() {
                                    actions.push(set("upload", entry["id"].clone()));
                                }
                            }
                        });
                    }
                });
            });
        });
        let status = data["status"].as_str().unwrap_or_default();
        if !status.is_empty() || busy {
            let text = if status.is_empty() { "Working…" } else { status };
            if data["error"].as_bool().unwrap_or(false) || data["error"].as_str().is_some_and(|text| !text.is_empty()) {
                ui.colored_label(ui.visuals().warn_fg_color, text);
            } else {
                web_muted(ui, text);
            }
        }
    }
}

#[derive(Clone, Default)]
struct LoadoutView {
    car: Option<Value>,
    page: usize,
    search: String,
    previews: String,
    import_open: bool,
    import_draft: Option<String>,
    modal_key: String,
    name: String,
    open_slot: Option<Value>,
    item_search: String,
    palette: Option<String>,
    custom_rgb: [u8; 3],
}

fn web_loadout_preview(ui: &mut egui::Ui, image: &Value, product: &Value, size: egui::Vec2) {
    if product.as_i64() == Some(0) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(20.0), ui.visuals().weak_text_color());
    } else {
        web_preview_box(ui, image.as_str(), size, "");
    }
}

impl HebnixApp {
    fn egui_presets(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let send = |kind: &str, extra: Value| web_act("preset", kind, extra);
        let id = egui::Id::new("presets_form");
        let mut form: (String, bool, String, String, String) = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_else(|| (
            val_str(&data["name"]), data["patches"].as_bool().unwrap_or(false), val_str(&data["key"]), String::new(), String::new(),
        ));
        let editing = &data["editing"];
        let edit_id = val_str(&editing["name"]) + &editing["include_patches"].to_string();
        if editing.is_object() && form.3 != edit_id {
            form.3 = edit_id;
            form.4 = val_str(&editing["name"]);
        }
        let current = &data["current"];
        web_card(ui, |ui| {
            web_muted(ui, &label("intro"));
            ui.horizontal_wrapped(|ui| {
                ui.label(label("name"));
                ui.add(egui::TextEdit::singleline(&mut form.0).desired_width(180.0));
                ui.checkbox(&mut form.1, label("patches"));
                if web_primary(ui, &label("capture"), true) {
                    actions.push(send("capture", json!({ "name": form.0, "patches": form.1 })));
                }
            });
            ui.horizontal(|ui| {
                if ui.button(label("refresh")).clicked() {
                    actions.push(send("refresh", Value::Null));
                }
                if ui.button(label("import")).clicked() {
                    actions.push(send("import_file", Value::Null));
                }
            });
            if current.is_null() || current == false {
                web_muted(ui, &label("empty"));
            } else {
                let list: Vec<(String, String)> = data["presets"].as_array().into_iter().flatten().enumerate().map(|(index, name)| (index.to_string(), val_str(name))).collect();
                ui.horizontal_wrapped(|ui| {
                    if let Some(value) = web_select(ui, "preset_pick", &val_str(&data["selected"]), &list, true).and_then(|value| value.parse::<u64>().ok()) {
                        actions.push(send("select", json!({ "index": value })));
                    }
                    if web_primary(ui, "Apply Preset", true) {
                        actions.push(send("apply", Value::Null));
                    }
                    if ui.button("Edit Preset").clicked() {
                        actions.push(send("edit", Value::Null));
                    }
                    let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                    if ui.add(egui::Button::new(label("delete")).fill(danger)).clicked() {
                        actions.push(send("delete", Value::Null));
                    }
                });
            }
        });
        let draft = editing.is_object();
        let preset = if draft { editing } else { current };
        if preset.is_object() {
            ui.add_space(8.0);
            web_card(ui, |ui| {
                if draft {
                    web_heading(ui, "Edit preset");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut form.4).desired_width(180.0));
                        ui.checkbox(&mut form.1, label("patches"));
                    });
                } else {
                    web_heading(ui, &format!("{}{}", val_str(&preset["name"]), if preset["include_patches"].as_bool().unwrap_or(false) { " (with patches)" } else { " (swaps only)" }));
                }
                let save_draft = |actions: &mut Vec<Value>, form: &(String, bool, String, String, String)| {
                    actions.push(send("draft", json!({ "name": form.4, "patches": form.1 })));
                };
                for (index, swap) in preset["swaps"].as_array().into_iter().flatten().enumerate() {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(format!("{} → {}", swap["source_name"].as_str().unwrap_or("?"), swap["target_name"].as_str().unwrap_or("?")));
                        if draft {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Remove").clicked() {
                                    save_draft(actions, &form);
                                    actions.push(send("remove_swap", json!({ "index": index })));
                                }
                            });
                        }
                    });
                }
                if let Some(patches) = preset["patches"].as_object() {
                    for (kind, value) in patches.iter().filter(|(_, value)| !value.is_null()) {
                        let entries: Vec<(String, Option<String>, String)> = match value.as_object() {
                            Some(object) if draft => object.iter().map(|(key, item)| (format!("{kind} / {key}: {item}"), Some(key.clone()), kind.clone())).collect(),
                            _ => vec![(format!("{kind}: {value}"), None, kind.clone())],
                        };
                        for (text, key, kind) in entries {
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.label(&text);
                                if draft {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("Remove").clicked() {
                                            save_draft(actions, &form);
                                            actions.push(send("remove_patch", match &key {
                                                Some(key) => json!({ "kind": kind, "key": key }),
                                                None => json!({ "kind": kind }),
                                            }));
                                        }
                                    });
                                }
                            });
                        }
                    }
                }
                if draft {
                    ui.horizontal(|ui| {
                        if web_primary(ui, "Save Changes", true) {
                            save_draft(actions, &form);
                            actions.push(send("save", Value::Null));
                        }
                        if ui.button("Cancel").clicked() {
                            actions.push(send("cancel", Value::Null));
                        }
                    });
                }
            });
        }
        let status = data["status"].as_str().filter(|text| !text.is_empty());
        let missing = data["missing"].as_bool().unwrap_or(false);
        if status.is_some() || missing {
            ui.add_space(8.0);
            web_card(ui, |ui| {
                if let Some(status) = status {
                    ui.label(status);
                }
                if missing {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Remove Missing Items from Preset").clicked() {
                            actions.push(send("remove_missing", Value::Null));
                        }
                        if ui.button("Ignore Missing Items").clicked() {
                            actions.push(send("ignore_missing", Value::Null));
                        }
                    });
                }
            });
        }
        ui.add_space(8.0);
        web_card(ui, |ui| {
            web_heading(ui, "Preset Key");
            web_muted(ui, "Share or paste a base64 preset key. Imported items remain listed even when their local files are missing.");
            ui.add(egui::TextEdit::multiline(&mut form.2).desired_rows(3).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.add_enabled(!current.is_null() && current != false, egui::Button::new("Show Preset Key")).clicked() {
                    actions.push(send("show_key", Value::Null));
                }
                if ui.button("Import Preset").clicked() {
                    actions.push(send("import_key", json!({ "key": form.2 })));
                }
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, form));
    }

    fn egui_loadouts(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let send = |kind: &str, extra: Value| web_act("loadouts", kind, extra);
        let closed = data["closed"].as_bool().unwrap_or(false);
        let view_id = egui::Id::new("loadouts_view");
        let mut view: LoadoutView = ui.ctx().data(|d| d.get_temp(view_id)).unwrap_or_default();
        if data["catalogLoading"].as_bool().unwrap_or(false) {
            web_muted(ui, "Loading item catalogs…");
        }
        if let Some(error) = data["catalogError"].as_str().filter(|text| !text.is_empty()) {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, error);
                if ui.button("Reload catalogs").clicked() {
                    actions.push(json!({ "type": "loadouts_reload_catalogs" }));
                }
            });
        }
        let accounts_empty = data["accounts"].as_array().is_none_or(Vec::is_empty);
        let message = data["message"].as_str().filter(|text| !text.is_empty());
        if accounts_empty || (!view.import_open && message.is_some()) {
            web_card(ui, |ui| {
                if accounts_empty {
                    web_muted(ui, &label("noSave"));
                }
                if let (false, Some(message)) = (view.import_open, message) {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(message).small());
                        if ui.small_button("×").on_hover_text("Dismiss warning").clicked() {
                            actions.push(send("dismiss_message", Value::Null));
                        }
                    });
                }
            });
            ui.add_space(8.0);
        }
        let rows = data["rows"].as_array().cloned().unwrap_or_default();
        let gap = 16.0;
        let left = 220.0f32.min(ui.available_width() * 0.28).floor();
        let right = (ui.available_width() - left - gap).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(egui::vec2(left, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(left - 34.0);
                    web_heading(ui, &label("cars"));
                    let mut counts: Vec<(Value, String, usize)> = Vec::new();
                    for row in &rows {
                        match counts.iter_mut().find(|(body, _, _)| *body == row["body"]) {
                            Some(entry) => entry.2 += 1,
                            None => counts.push((row["body"].clone(), val_str(&row["car"]), 1)),
                        }
                    }
                    counts.sort_by(|a, b| a.1.cmp(&b.1));
                    let accent = ui.visuals().selection.stroke.color.gamma_multiply(0.6);
                    let list_height = (ui.ctx().content_rect().height() * 0.6).clamp(320.0, 640.0);
                    egui::ScrollArea::vertical().id_salt("loadout_cars").max_height(list_height).min_scrolled_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
                        let width = ui.available_width();
                        let pick = |ui: &mut egui::Ui, body: Option<Value>, name: String, view: &mut LoadoutView| {
                            let on = view.car == body;
                            if ui.add(egui::Button::new(name).fill(if on { accent } else { egui::Color32::TRANSPARENT }).min_size(egui::vec2(width, 26.0))).clicked() {
                                view.car = body;
                                view.page = 0;
                            }
                        };
                        pick(ui, None, format!("All ({})", rows.len()), &mut view);
                        for (body, car, count) in counts {
                            pick(ui, Some(body), format!("{car} ({count})"), &mut view);
                        }
                    });
                });
            });
            ui.allocate_ui_with_layout(egui::vec2(right, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(right - 34.0);
                    ui.horizontal(|ui| {
                        web_heading(ui, &label("list"));
                        ui.add(egui::TextEdit::singleline(&mut view.search).hint_text(label("search")).desired_width(180.0));
                        if ui.button("Import").clicked() {
                            view.import_open = true;
                        }
                    });
                    let term = view.search.trim().to_lowercase();
                    let matching: Vec<&Value> = rows.iter().filter(|row| (view.car.is_none() || view.car.as_ref() == Some(&row["body"])) && val_str(&row["name"]).to_lowercase().contains(&term)).collect();
                    let tile_w = 220.0;
                    let tile_gap = 10.0;
                    let columns = (((right - 34.0 + tile_gap) / (tile_w + tile_gap)).floor() as usize).max(1);
                    let page_size = columns * 3;
                    let pages = matching.len().div_ceil(page_size).max(1);
                    view.page = view.page.min(pages - 1);
                    let visible: Vec<&Value> = matching.iter().skip(view.page * page_size).take(page_size).copied().collect();
                    let positions: Vec<Value> = visible.iter().map(|row| row["position"].clone()).collect();
                    let key = json!(positions).to_string();
                    let known: Vec<Value> = data["preview_positions"].as_array().cloned().unwrap_or_default();
                    if view.previews != key || positions.iter().any(|position| !known.contains(position)) {
                        view.previews = key;
                        actions.push(send("previews", json!({ "positions": positions })));
                    }
                    for chunk in visible.chunks(columns) {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = tile_gap;
                            for row in chunk {
                                // web_card fills the width it is given, so each tile gets its own slot
                                ui.allocate_ui_with_layout(egui::vec2(tile_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| web_card(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    ui.horizontal(|ui| {
                                        web_loadout_preview(ui, &row["image"], &row["body"], egui::vec2(56.0, 56.0));
                                        ui.vertical(|ui| {
                                            ui.label(egui::RichText::new(val_str(&row["name"])).strong());
                                            ui.label(egui::RichText::new(val_str(&row["car"])).small().color(ui.visuals().weak_text_color()));
                                        });
                                    });
                                    ui.horizontal(|ui| {
                                        ui.menu_button("⧉", |ui| {
                                            if ui.button(label("lds")).clicked() {
                                                actions.push(send("copy", json!({ "position": row["position"], "bakkes": false })));
                                                ui.close();
                                            }
                                            if ui.button(label("bakkes")).clicked() {
                                                actions.push(send("copy", json!({ "position": row["position"], "bakkes": true })));
                                                ui.close();
                                            }
                                        }).response.on_hover_text(label("copy"));
                                        for (kind, symbol) in [("edit", "✎"), ("move", "↕")] {
                                            if ui.add_enabled(closed, egui::Button::new(symbol)).on_hover_text(label(kind)).clicked() {
                                                actions.push(send("open", json!({ "position": row["position"], "kind": kind })));
                                            }
                                        }
                                        let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                                        if ui.add_enabled(closed, egui::Button::new("×").fill(danger)).on_hover_text(label("delete")).clicked() {
                                            actions.push(send("open", json!({ "position": row["position"], "kind": "delete" })));
                                        }
                                    });
                                }));
                            }
                        });
                    }
                    web_pager(ui, &mut view.page, pages);
                });
            });
        });
        if view.import_open {
            Self::egui_loadout_import(ui.ctx(), data, &mut view, actions);
        } else if data["dialog"].is_object() {
            Self::egui_loadout_dialog(ui.ctx(), data, &mut view, actions);
        } else {
            view.modal_key.clear();
        }
        ui.ctx().data_mut(|d| d.insert_temp(view_id, view));
    }

    fn egui_loadout_import(ctx: &egui::Context, data: &Value, view: &mut LoadoutView, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let send = |kind: &str, extra: Value| web_act("loadouts", kind, extra);
        let closed = data["closed"].as_bool().unwrap_or(false);
        let plan = &data["import"];
        let mut close = false;
        web_modal(ctx, "loadout_import", 560.0, |ui| {
            ui.horizontal(|ui| {
                web_heading(ui, &label("import"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        close = true;
                    }
                });
            });
            if !closed {
                ui.colored_label(ui.visuals().warn_fg_color, label("closed"));
            }
            if data["account"].is_null() {
                ui.colored_label(ui.visuals().warn_fg_color, "Select a Rocket League save from the settings cog before importing.");
            }
            if !plan.is_object() {
                web_muted(ui, &label("hint"));
                let mut draft = view.import_draft.clone().unwrap_or_else(|| val_str(&data["input"]));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut draft).hint_text("Paste a code here").desired_width(ui.available_width() - 80.0));
                    if web_primary(ui, "Import", closed && !data["account"].is_null()) {
                        actions.push(send("import", json!({ "code": draft })));
                        view.import_draft = None;
                        return;
                    }
                });
                view.import_draft = Some(draft);
            } else {
                ui.label(val_str(&plan["summary"]));
                let rows = data["rows"].as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    web_muted(ui, &label("empty"));
                } else {
                    let options: Vec<(String, String)> = rows.iter().map(|row| (val_str(&row["position"]), val_str(&row["name"]))).collect();
                    let update = |change: Value| {
                        let mut value = json!({ "base": plan["base"], "target": plan["target"], "replace": plan["replace"], "name": plan["name"] });
                        if let (Some(target), Some(extra)) = (value.as_object_mut(), change.as_object()) {
                            for (key, item) in extra {
                                target.insert(key.clone(), item.clone());
                            }
                        }
                        send("plan", value)
                    };
                    ui.horizontal(|ui| {
                        ui.label(label("base"));
                        if let Some(value) = web_select(ui, "import_base", &val_str(&plan["base"]), &options, true).and_then(|value| value.parse::<u64>().ok()) {
                            actions.push(update(json!({ "base": value })));
                        }
                    });
                    web_muted(ui, &label("baseHint"));
                    let replace = plan["replace"].as_bool().unwrap_or(false);
                    if ui.add(egui::Button::new(label("replace")).fill(if replace { ui.visuals().selection.stroke.color.gamma_multiply(0.6) } else { egui::Color32::TRANSPARENT })).clicked() {
                        actions.push(update(json!({ "replace": true })));
                    }
                    ui.horizontal(|ui| {
                        if replace {
                            ui.label(label("target"));
                            if let Some(value) = web_select(ui, "import_target", &val_str(&plan["target"]), &options, true).and_then(|value| value.parse::<u64>().ok()) {
                                actions.push(update(json!({ "target": value })));
                            }
                        } else {
                            ui.label(label("name"));
                            let mut name = val_str(&plan["name"]);
                            if ui.text_edit_singleline(&mut name).lost_focus() {
                                actions.push(update(json!({ "name": name })));
                            }
                        }
                    });
                    if let Some(preview_rows) = plan["preview"]["rows"].as_array() {
                        ui.label(egui::RichText::new(val_str(&plan["preview"]["name"])).strong());
                        egui::Grid::new("import_plan").striped(true).show(ui, |ui| {
                            for heading in labels["plan"].as_array().into_iter().flatten() {
                                ui.label(egui::RichText::new(val_str(heading)).strong());
                            }
                            ui.end_row();
                            for cells in preview_rows {
                                for cell in cells.as_array().into_iter().flatten() {
                                    ui.label(val_str(cell));
                                }
                                ui.end_row();
                            }
                        });
                        web_muted(ui, &val_str(&plan["preview"]["note"]));
                    } else if let Some(error) = plan["preview"]["error"].as_str() {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                    }
                    let blocked = !closed || !plan["preview"]["rows"].is_array();
                    ui.horizontal(|ui| {
                        if web_primary(ui, &label("apply"), !blocked) {
                            actions.push(send("save_import", json!({ "apply": true })));
                        }
                        if ui.add_enabled(!blocked, egui::Button::new(label("importOnly"))).clicked() {
                            actions.push(send("save_import", json!({ "apply": false })));
                        }
                    });
                }
            }
            if let Some(message) = data["message"].as_str().filter(|text| !text.is_empty()) {
                ui.label(egui::RichText::new(message).small());
            }
        });
        if close {
            view.import_open = false;
            view.import_draft = None;
            if plan.is_object() {
                actions.push(send("cancel_import", Value::Null));
            }
        }
    }

    fn egui_loadout_dialog(ctx: &egui::Context, data: &Value, view: &mut LoadoutView, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let send = |kind: &str, extra: Value| web_act("loadouts", kind, extra);
        let model = &data["dialog"];
        let closed = data["closed"].as_bool().unwrap_or(false);
        let kind = model["kind"].as_str().unwrap_or_default().to_owned();
        let key = format!("{kind}:{}", val_str(&model["position"]));
        if kind == "edit" && view.modal_key != key {
            view.name = val_str(&model["name"]);
            view.open_slot = None;
            view.item_search.clear();
        }
        let move_id = egui::Id::new("loadout_move_target");
        let mut destination: String = ctx.data(|d| d.get_temp(move_id)).unwrap_or_else(|| val_str(&model["target"]));
        let mut commit: Option<Value> = None;
        let mut cancel = false;
        web_modal(ctx, "loadout_dialog", 560.0, |ui| {
            web_heading(ui, &label(&kind));
            match kind.as_str() {
                "delete" => {
                    ui.label(val_str(&model["message"]));
                }
                "move" => {
                    ui.label(val_str(&model["name"]));
                    web_muted(ui, &label("position"));
                    let options: Vec<(String, String)> = data["rows"].as_array().into_iter().flatten().map(|row| (val_str(&row["position"]), format!("{} · {}", row["position"].as_u64().unwrap_or(0) + 1, val_str(&row["name"])))).collect();
                    if let Some(value) = web_select(ui, "loadout_move", &destination, &options, true) {
                        destination = value;
                    }
                }
                _ => {
                    ui.add(egui::TextEdit::singleline(&mut view.name).desired_width(f32::INFINITY));
                    let teams = vec![("0".to_owned(), label("blue")), ("1".to_owned(), label("orange"))];
                    if let Some(value) = web_select(ui, "loadout_team", &val_str(&model["team"]), &teams, true).and_then(|value| value.parse::<u64>().ok()) {
                        actions.push(send("team", json!({ "team": value })));
                    }
                    let palette = &model["palette"];
                    if palette["loading"].as_bool().unwrap_or(false) {
                        ui.label(egui::RichText::new("Loading colours…").small());
                    }
                    if let Some(error) = palette["error"].as_str().filter(|text| !text.is_empty()) {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                    }
                    ui.horizontal(|ui| {
                        for colour_key in ["primary", "secondary"] {
                            if palette[colour_key].is_null() || palette[colour_key] == false {
                                continue;
                            }
                            let index = model[colour_key].as_u64().unwrap_or(0) as usize;
                            let rgb = web_rgb_of(&palette[colour_key][index]);
                            let swatch = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                            ui.label(egui::RichText::new("■").color(swatch));
                            if ui.button(label(colour_key)).clicked() {
                                view.palette = Some(colour_key.to_owned());
                                view.custom_rgb = rgb;
                            }
                        }
                    });
                    web_muted(ui, &label("owned"));
                    egui::ScrollArea::vertical().id_salt("loadout_slots").max_height(320.0).show(ui, |ui| {
                        for slot in model["slots"].as_array().into_iter().flatten() {
                            let open = view.open_slot.as_ref() == Some(&slot["slot"]);
                            let header = format!("{} · {}", val_str(&slot["name"]), val_str(&slot["current"]));
                            let response = egui::CollapsingHeader::new(header).id_salt(("loadout_slot", val_str(&slot["slot"]))).open(Some(open)).show(ui, |ui| {
                                ui.add(egui::TextEdit::singleline(&mut view.item_search).hint_text(label("itemSearch")).desired_width(f32::INFINITY));
                                let term = view.item_search.trim().to_lowercase();
                                let mut owned: Vec<(String, bool, Value)> = slot["choices"].as_array().into_iter().flatten()
                                    .map(|choice| (val_str(&choice[0]), choice[2].as_bool().unwrap_or(false), send("item", json!({ "team": model["team"], "slot": slot["slot"], "option": choice[1] })))).collect();
                                for engine in slot["engines"].as_array().into_iter().flatten() {
                                    let name = val_str(&engine["label"]);
                                    if !owned.iter().any(|(label, _, _)| *label == name) {
                                        owned.push((name, slot["product"] == engine["product"], send("stock_engine", json!({ "team": model["team"], "product": engine["product"] }))));
                                    }
                                }
                                owned.sort_by(|a, b| a.0.cmp(&b.0));
                                let accent = ui.visuals().selection.stroke.color.gamma_multiply(0.6);
                                for (name, selected, action) in owned.into_iter().filter(|(name, _, _)| name.to_lowercase().contains(&term)) {
                                    if ui.add(egui::Button::new(&name).fill(if selected { accent } else { egui::Color32::TRANSPARENT }).min_size(egui::vec2(ui.available_width(), 24.0))).clicked() {
                                        actions.push(action);
                                    }
                                }
                            });
                            if response.header_response.clicked() {
                                if open {
                                    view.open_slot = None;
                                } else {
                                    view.open_slot = Some(slot["slot"].clone());
                                    view.item_search.clear();
                                }
                            }
                        }
                    });
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(label("cancel")).clicked() {
                    cancel = true;
                }
                let danger = kind == "delete";
                let fill = if danger { ui.visuals().error_fg_color.gamma_multiply(0.6) } else { ui.visuals().selection.stroke.color.gamma_multiply(0.6) };
                let text = if danger { label("delete") } else { label("confirm") };
                let ready = closed && (kind != "edit" || !view.name.trim().is_empty());
                if ui.add_enabled(ready, egui::Button::new(text).fill(fill)).clicked() {
                    commit = Some(match kind.as_str() {
                        "move" => json!({ "destination": destination.parse::<u64>().unwrap_or(0) }),
                        "edit" => json!({ "name": view.name }),
                        _ => json!({}),
                    });
                }
            });
        });
        if kind == "edit" && view.palette.is_some() {
            Self::egui_loadout_palette(ctx, data, view, actions);
        }
        view.modal_key = key;
        ctx.data_mut(|d| d.insert_temp(move_id, destination));
        if cancel {
            actions.push(send("close", Value::Null));
        }
        if let Some(extra) = commit {
            actions.push(send("confirm", extra));
        }
    }

    fn egui_loadout_palette(ctx: &egui::Context, data: &Value, view: &mut LoadoutView, actions: &mut Vec<Value>) {
        let labels = &data["labels"];
        let label = |key: &str| labels[key].as_str().unwrap_or_default().to_owned();
        let send = |kind: &str, extra: Value| web_act("loadouts", kind, extra);
        let model = &data["dialog"];
        let Some(key) = view.palette.clone() else { return };
        let colours = model["palette"][&key].as_array().cloned().unwrap_or_default();
        if colours.is_empty() {
            view.palette = None;
            return;
        }
        let columns = if key == "secondary" { 15 } else { 10 };
        let mut close = false;
        web_modal(ctx, "loadout_palette", 460.0, |ui| {
            ui.horizontal(|ui| {
                web_heading(ui, &label(&key));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        close = true;
                    }
                });
            });
            egui::Grid::new("loadout_palette_grid").spacing([4.0, 4.0]).show(ui, |ui| {
                for (index, entry) in colours.iter().enumerate() {
                    let rgb = web_rgb_of(entry);
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                    if model[&key].as_u64() == Some(index as u64) {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
                    }
                    if response.clicked() {
                        actions.push(send("colour", json!({ "team": model["team"], "primary": key == "primary", "index": index })));
                        close = true;
                    }
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Any colour");
                ui.color_edit_button_srgb(&mut view.custom_rgb);
                if web_primary(ui, "Use colour", data["closed"].as_bool().unwrap_or(false)) {
                    actions.push(send("custom_colour", json!({ "team": model["team"], "primary": key == "primary", "rgb": view.custom_rgb })));
                    close = true;
                }
            });
            web_muted(ui, "A custom colour is added to your garage palette, so Rocket League must be closed. Restart it to see the colour.");
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        if close {
            view.palette = None;
        }
    }
}

#[derive(Clone, Default)]
struct CreatorView {
    step: usize,
    name: String,
    search: String,
    copy: bool,
    copy_pick: Option<String>,
    pending: Option<Value>,
    replacement: String,
    requested: String,
    item_page: usize,
    initialized: bool,
    palette: Option<(u64, String)>,
    creating: bool,
    attempt: Value,
}

const CREATOR_STEPS: [(&str, Option<u64>); 12] = [
    ("Name", None), ("Body", Some(0)), ("Decal", Some(1)), ("Colours", None), ("Wheels", Some(2)), ("Boost", Some(3)),
    ("Antenna", Some(4)), ("Topper", Some(5)), ("Trail", Some(14)), ("Goal Explosion", Some(15)), ("Engine Audio", Some(13)), ("Review", None),
];

fn creator_act(kind: &str, extra: Value) -> Value {
    json!({ "type": "loadouts", "action": { "type": "creator", "action": web_act("", kind, extra)["action"] } })
}

impl HebnixApp {
    fn egui_preset_creator(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let view_id = egui::Id::new("creator_view");
        let mut view: CreatorView = ui.ctx().data(|d| d.get_temp(view_id)).unwrap_or_default();
        let closed = data["closed"].as_bool().unwrap_or(false);
        let draft = &data["creator"];
        let rows = data["rows"].as_array().cloned().unwrap_or_default();
        if view.creating && view.attempt != data["creatorAttempt"] {
            view.creating = false;
        }
        if !draft.is_object() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(520.0);
                web_card(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        let message = data["message"].as_str().filter(|text| !text.is_empty());
                        match message {
                            Some("Loadout Created") => { ui.colored_label(ui.visuals().hyperlink_color, "Loadout Created"); }
                            Some(text) => { ui.colored_label(ui.visuals().warn_fg_color, text); }
                            None => {}
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(((ui.available_width() - 330.0) / 2.0).max(0.0));
                            if web_primary(ui, "Create New Preset", !rows.is_empty() && closed) {
                                view = CreatorView { copy: false, ..Default::default() };
                                actions.push(creator_act("begin", json!({ "copy": Value::Null })));
                            }
                            if ui.add_enabled(!rows.is_empty() && closed, egui::Button::new("Copy Existing Loadout")).clicked() {
                                view.copy = true;
                            }
                        });
                        if !closed {
                            ui.colored_label(ui.visuals().warn_fg_color, "Rocket League must be closed.");
                        }
                        if data["loading"].as_bool().unwrap_or(false) {
                            web_muted(ui, "Loading loadouts…");
                        } else if rows.is_empty() {
                            web_muted(ui, "Select an account with a Rocket League loadout from quick settings first.");
                        }
                        if view.copy && !rows.is_empty() {
                            let options: Vec<(String, String)> = rows.iter().map(|row| (val_str(&row["position"]), val_str(&row["name"]))).collect();
                            let current = view.copy_pick.clone().unwrap_or_else(|| options[0].0.clone());
                            ui.horizontal(|ui| {
                                if let Some(value) = web_select(ui, "creator_copy", &current, &options, true) {
                                    view.copy_pick = Some(value);
                                }
                                if web_primary(ui, "Continue", closed) {
                                    let position = current.parse::<u64>().unwrap_or(0);
                                    view = CreatorView { copy: false, ..Default::default() };
                                    actions.push(creator_act("begin", json!({ "copy": position })));
                                }
                                if ui.button("Cancel").clicked() {
                                    view.copy = false;
                                }
                            });
                        }
                    });
                });
            });
            ui.ctx().data_mut(|d| d.insert_temp(view_id, view));
            return;
        }
        if !view.initialized {
            view.name = val_str(&draft["name"]);
            view.initialized = true;
        }
        view.step = view.step.min(CREATOR_STEPS.len() - 1);
        let (step_name, step_slot) = CREATOR_STEPS[view.step];
        let mut goto: Option<usize> = None;
        let gap = 16.0;
        let rail = 180.0f32.min(ui.available_width() * 0.25);
        let body = ui.available_width() - rail - gap;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(egui::vec2(rail, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(rail - 34.0);
                    let accent = ui.visuals().selection.stroke.color.gamma_multiply(0.6);
                    for (index, (name, _)) in CREATOR_STEPS.iter().enumerate() {
                        let fill = if index == view.step { accent } else { egui::Color32::TRANSPARENT };
                        if ui.add(egui::Button::new(format!("{}. {name}", index + 1)).fill(fill).min_size(egui::vec2(ui.available_width(), 26.0))).clicked() {
                            goto = Some(index);
                        }
                    }
                });
            });
            ui.allocate_ui_with_layout(egui::vec2(body, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                web_card(ui, |ui| {
                    ui.set_width(body - 34.0);
                    web_heading(ui, step_name);
                    if let Some(message) = data["message"].as_str().filter(|text| !text.is_empty() && *text != "Loadout Created") {
                        ui.colored_label(ui.visuals().warn_fg_color, message);
                    }
                    let slots = draft["slots"].as_array().cloned().unwrap_or_default();
                    if view.step == 0 {
                        web_muted(ui, "Give this preset a name. Your original loadout stays unchanged.");
                        ui.add(egui::TextEdit::singleline(&mut view.name).hint_text("Name your preset").char_limit(64).desired_width(f32::INFINITY));
                    } else if step_name == "Colours" {
                        if draft["palette"]["loading"].as_bool().unwrap_or(false) {
                            ui.label("Loading colours…");
                        }
                        if let Some(error) = draft["palette"]["error"].as_str().filter(|text| !text.is_empty()) {
                            ui.colored_label(ui.visuals().warn_fg_color, error);
                        }
                        ui.horizontal_top(|ui| {
                            for team in draft["colours"].as_array().into_iter().flatten() {
                                ui.vertical(|ui| {
                                    web_heading(ui, if team["team"].as_u64() == Some(0) { "Blue team" } else { "Orange team" });
                                    for key in ["primary", "secondary"] {
                                        if team["palette"][key].is_null() {
                                            continue;
                                        }
                                        let index = team[key].as_u64().unwrap_or(0) as usize;
                                        let rgb = web_rgb_of(&team["palette"][key][index]);
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("■").color(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])));
                                            if ui.button(if key == "primary" { "Primary Colour" } else { "Secondary Colour" }).clicked() {
                                                view.palette = Some((team["team"].as_u64().unwrap_or(0), key.to_owned()));
                                            }
                                        });
                                    }
                                });
                            }
                        });
                    } else if let Some(slot_id) = step_slot {
                        let selected = slots.iter().find(|slot| slot["slot"].as_u64() == Some(slot_id));
                        if slot_id == 1 {
                            if let Some(body_slot) = slots.iter().find(|slot| slot["slot"].as_u64() == Some(0)).filter(|slot| slot["product"].as_i64().unwrap_or(0) != 0) {
                                web_muted(ui, &format!("Choose a decal for {}.", val_str(&body_slot["label"])));
                            }
                        }
                        web_muted(ui, &format!("Selected: {}", selected.and_then(|slot| slot["label"].as_str()).filter(|text| !text.is_empty()).unwrap_or("Default")));
                        if ui.add(egui::TextEdit::singleline(&mut view.search).hint_text("Search all items").desired_width(f32::INFINITY)).changed() {
                            view.item_page = 0;
                        }
                        if draft["slot"].as_u64() != Some(slot_id) {
                            ui.label("Loading items…");
                        } else {
                            let lower = view.search.trim().to_lowercase();
                            let items: Vec<Value> = draft["catalog"]["items"].as_array().cloned().unwrap_or_default().into_iter().filter(|item| val_str(&item["label"]).to_lowercase().contains(&lower)).collect();
                            let optional = ![0, 2, 3, 14, 15].contains(&slot_id);
                            let tile_w = 150.0;
                            let columns = (((body - 34.0) / (tile_w + 10.0)).floor() as usize).max(1);
                            let page_size = (columns * 3).saturating_sub(optional as usize).max(1);
                            let pages = items.len().div_ceil(page_size).max(1);
                            view.item_page = view.item_page.min(pages - 1);
                            let visible: Vec<&Value> = items.iter().skip(view.item_page * page_size).take(page_size).collect();
                            let ids: Vec<Value> = visible.iter().map(|item| item["id"].clone()).collect();
                            let signature = json!([slot_id, ids]).to_string();
                            let known: Vec<Value> = draft["preview_ids"].as_array().cloned().unwrap_or_default();
                            if view.requested != signature || ids.iter().any(|id| !known.contains(id)) {
                                view.requested = signature;
                                actions.push(creator_act("previews", json!({ "ids": ids })));
                            }
                            let mut tiles: Vec<Option<&Value>> = Vec::new();
                            if optional {
                                tiles.push(None);
                            }
                            tiles.extend(visible.iter().map(|item| Some(*item)));
                            for chunk in tiles.chunks(columns) {
                                ui.horizontal_top(|ui| {
                                    for item in chunk {
                                        let (title, detail, image, on) = match item {
                                            None => (if slot_id == 13 { "Default" } else { "None" }.to_owned(), String::new(), None, selected.is_none_or(|slot| slot["product"].as_i64().unwrap_or(0) == 0)),
                                            Some(item) => {
                                                let preview = &draft["catalog"]["previews"][val_str(&item["id"])];
                                                (val_str(&item["label"]), if item["owned"].as_bool().unwrap_or(false) { "Owned".to_owned() } else { "Swap required".to_owned() },
                                                    preview["image"].as_str().map(str::to_owned), selected.is_some_and(|slot| slot["product"] == item["product"]))
                                            }
                                        };
                                        if web_tile(ui, &title, image.as_deref(), &detail, on, true) {
                                            match item {
                                                None => actions.push(creator_act("pick", json!({ "product": 0, "replacement": Value::Null }))),
                                                Some(item) if item["owned"].as_bool().unwrap_or(false) => actions.push(creator_act("pick", json!({ "product": item["product"], "replacement": Value::Null }))),
                                                Some(item) => {
                                                    view.pending = Some((*item).clone());
                                                    view.replacement.clear();
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                            web_pager(ui, &mut view.item_page, pages);
                        }
                    } else {
                        ui.label(egui::RichText::new(&view.name).size(20.0).strong());
                        web_muted(ui, "Review each team's selections before creating your preset. Owned replacement items will also change wherever they are used.");
                        for slot in &slots {
                            ui.label(format!("{}: {}", val_str(&slot["name"]), val_str(&slot["label"])));
                        }
                        web_heading(ui, "Affected loadouts");
                        let affected = draft["affected"].as_array().cloned().unwrap_or_default();
                        if affected.is_empty() {
                            web_muted(ui, "No existing loadouts use these replacement items.");
                        }
                        for loadout in &affected {
                            web_card(ui, |ui| {
                                ui.label(egui::RichText::new(val_str(&loadout["name"])).strong());
                                for reason in loadout["changes"].as_array().into_iter().flatten() {
                                    ui.label(egui::RichText::new(val_str(reason)).small());
                                }
                            });
                        }
                    }
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Cancel").clicked() {
                            view.copy = false;
                            actions.push(creator_act("cancel", Value::Null));
                        }
                        if ui.add_enabled(view.step > 0, egui::Button::new("← Back")).clicked() {
                            goto = Some(view.step - 1);
                        }
                        if view.step + 1 < CREATOR_STEPS.len() {
                            if web_primary(ui, "Next →", true) {
                                goto = Some(view.step + 1);
                            }
                        } else {
                            let missing: Vec<String> = draft["missing_required"].as_array().into_iter().flatten().map(val_str).collect();
                            if web_primary(ui, "Create Preset", closed && missing.is_empty() && !view.creating) {
                                view.creating = true;
                                view.attempt = data["creatorAttempt"].clone();
                                actions.push(creator_act("finish", json!({ "name": view.name })));
                            }
                            if !missing.is_empty() {
                                ui.colored_label(ui.visuals().warn_fg_color, format!("Required: {}", missing.join(", ")));
                            }
                        }
                        if !closed {
                            ui.colored_label(ui.visuals().warn_fg_color, "Close Rocket League before creating the preset.");
                        }
                    });
                });
            });
        });
        if let Some(index) = goto {
            if view.name.trim().is_empty() && index > 0 {
                // the web page refocuses the name field instead of leaving it
            } else {
                view.step = index;
                view.search.clear();
                view.item_page = 0;
                view.pending = None;
                view.requested.clear();
                if CREATOR_STEPS[index].0 == "Review" {
                    actions.push(creator_act("review", Value::Null));
                }
                if let Some(slot) = CREATOR_STEPS[index].1 {
                    actions.push(creator_act("slot", json!({ "slot": slot })));
                }
            }
        }
        if let Some(item) = view.pending.clone() {
            let mut close = false;
            web_modal(ui.ctx(), "creator_replacement", 480.0, |ui| {
                web_heading(ui, "Choose an owned item to replace");
                let body_slot = draft["slots"].as_array().into_iter().flatten().find(|slot| slot["slot"].as_u64() == Some(0));
                let hint = if draft["slot"].as_u64() == Some(1) && body_slot.is_some_and(|slot| slot["equipped"].as_bool().unwrap_or(false)) {
                    format!("Choose an owned decal compatible with {}.", body_slot.and_then(|slot| slot["equipped_label"].as_str()).unwrap_or("the body you'll equip in Rocket League"))
                } else {
                    "Choose the owned item you'll equip in Rocket League.".to_owned()
                };
                ui.label(format!("{hint} It will appear as {}.", val_str(&item["label"])));
                ui.colored_label(ui.visuals().warn_fg_color, "This also changes other loadouts that use the same owned item.");
                let mut options = vec![(String::new(), "Choose an owned item".to_owned())];
                options.extend(draft["replacements"].as_array().into_iter().flatten().map(|entry| (val_str(&entry["index"]), val_str(&entry["label"]))));
                if let Some(value) = web_select(ui, "creator_replacement_pick", &view.replacement, &options, true) {
                    view.replacement = value;
                }
                if draft["replacements"].as_array().is_none_or(Vec::is_empty) {
                    web_muted(ui, "No compatible owned replacements are available.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if web_primary(ui, "Use Replacement", !view.replacement.is_empty()) {
                        actions.push(creator_act("pick", json!({ "product": item["product"], "replacement": view.replacement.parse::<u64>().unwrap_or(0) })));
                        close = true;
                    }
                });
            });
            if close {
                view.pending = None;
            }
        }
        if let Some((team_id, key)) = view.palette.clone() {
            let team = draft["colours"].as_array().into_iter().flatten().find(|team| team["team"].as_u64() == Some(team_id)).cloned();
            match team {
                Some(team) => {
                    let colours = team["palette"][&key].as_array().cloned().unwrap_or_default();
                    let columns = if key == "secondary" { 15 } else { 10 };
                    let mut close = false;
                    web_modal(ui.ctx(), "creator_palette", 460.0, |ui| {
                        web_heading(ui, if key == "primary" { "Primary Colour" } else { "Secondary Colour" });
                        egui::Grid::new("creator_palette_grid").spacing([4.0, 4.0]).show(ui, |ui| {
                            for (index, entry) in colours.iter().enumerate() {
                                let rgb = web_rgb_of(entry);
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                                if team[&key].as_u64() == Some(index as u64) {
                                    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
                                }
                                if response.clicked() {
                                    actions.push(creator_act("colour", json!({ "team": team_id, "primary": key == "primary", "index": index })));
                                    close = true;
                                }
                                if (index + 1) % columns == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                    if close {
                        view.palette = None;
                    }
                }
                None => view.palette = None,
            }
        }
        if view.creating {
            web_modal(ui.ctx(), "creator_busy", 300.0, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new("Creating loadout…").strong());
                    web_muted(ui, "Applying item swaps and saving your loadout.");
                });
            });
        }
        ui.ctx().data_mut(|d| d.insert_temp(view_id, view));
    }
}

impl HebnixApp {
    fn egui_replay_library(ui: &mut egui::Ui, data: &Value, actions: &mut Vec<Value>) {
        let call = |kind: &str, extra: Value| web_act("replay_library", kind, extra);
        let ready = data["ready"].as_bool().unwrap_or(false);
        let playing = data["playing"].as_bool().unwrap_or(false);
        let warn = ui.visuals().warn_fg_color;
        web_card(ui, |ui| {
            if let Some(error) = data["error"].as_str().filter(|text| !text.is_empty()) {
                ui.colored_label(warn, error);
            }
            if let Some(scan) = data["scan"].as_f64() {
                ui.label(egui::RichText::new("Scanning replay events · 16×").small());
                ui.add(egui::ProgressBar::new(scan.clamp(0.0, 1.0) as f32));
                if ui.button("Cancel scan").clicked() {
                    actions.push(call("cancel_scan", Value::Null));
                }
            }
            if data["scanFailed"].as_bool().unwrap_or(false) && ui.add_enabled(ready, egui::Button::new("Retry event scan")).clicked() {
                actions.push(call("retry_scan", Value::Null));
            }
            ui.horizontal_wrapped(|ui| {
                let connected = data["connected"].as_bool().unwrap_or(false);
                let colour = if connected { egui::Color32::from_rgb(80, 200, 120) } else { ui.visuals().weak_text_color() };
                ui.colored_label(colour, if connected { "Stats API connected" } else { "Stats API disconnected" });
                if ui.button("Detect current replay").clicked() {
                    actions.push(call("detect", Value::Null));
                }
                let capturing = data["capturing"].as_bool().unwrap_or(false);
                let keybind = data["keybind"].as_str().filter(|text| !text.is_empty());
                ui.label("Play/pause key");
                ui.label(egui::RichText::new(if capturing { "Press a key…" } else { keybind.unwrap_or("Not set") }).strong());
                if ui.add_enabled(!capturing, egui::Button::new("Set")).clicked() {
                    actions.push(call("capture", Value::Null));
                }
                if ui.add_enabled(keybind.is_some() || capturing, egui::Button::new("Clear")).clicked() {
                    actions.push(call("clear", Value::Null));
                }
            });
            ui.label(egui::RichText::new(val_str(&data["status"])).small());
        });
        ui.add_space(8.0);
        let model = &data["timeline"];
        let full = data["full"].as_bool().unwrap_or(false);
        web_card(ui, |ui| {
            web_heading(ui, "Playback & Keyframes");
            if !full || !model.is_object() {
                ui.label(egui::RichText::new("Waiting for a replay").strong());
                web_muted(ui, "Open a replay in Rocket League, then return here. Timeline controls become available while you are watching it.");
                if full {
                    web_muted(ui, "Could not match this replay to a local file. Check your configured replay folder, then detect again.");
                }
                return;
            }
            let frame = model["frame"].as_i64().unwrap_or(1);
            ui.label(egui::RichText::new(format!("{} · {} frames · {} fps", val_str(&model["title"]), model["max"], model["fps"])).strong());
            ui.horizontal_wrapped(|ui| {
                if web_primary(ui, if playing { "Pause" } else { "Play timeline" }, ready) {
                    actions.push(call("toggle", Value::Null));
                }
                for (text, target) in [("← Frame", frame - 1), ("Frame →", frame + 1), ("Frame 1", 1)] {
                    if ui.add_enabled(ready, egui::Button::new(text)).clicked() {
                        actions.push(call("scrub", json!({ "frame": target })));
                    }
                }
                if ui.add_enabled(ready && !playing, egui::Button::new("+ Keyframe")).clicked() {
                    actions.push(call("add_key", Value::Null));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Keyframes");
                let accent = ui.visuals().selection.stroke.color.gamma_multiply(0.6);
                for key in model["keys"].as_array().into_iter().flatten() {
                    let on = model["selected"]["frame"] == key["frame"];
                    if ui.add(egui::Button::new(format!("Frame {}", key["frame"])).fill(if on { accent } else { egui::Color32::TRANSPARENT })).clicked() {
                        actions.push(call("pick_key", json!({ "frame": key["frame"] })));
                    }
                }
            });
            if model["selected"].is_object() {
                let edit = ready && !playing;
                let selected = &model["selected"];
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(format!("Keyframe · Frame {}", selected["frame"])).strong());
                    let choices: Vec<(String, String)> = std::iter::once((String::new(), "+ Add command".to_owned()))
                        .chain(model["commands"].as_array().into_iter().flatten().enumerate().map(|(index, command)| (index.to_string(), val_str(command)))).collect();
                    if let Some(value) = web_select(ui, "replay_add_command", "", &choices, edit).and_then(|value| value.parse::<u64>().ok()) {
                        actions.push(call("add_command", json!({ "index": value })));
                    }
                    let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                    if ui.add_enabled(edit, egui::Button::new("Delete keyframe").fill(danger)).clicked() {
                        actions.push(call("delete_key", Value::Null));
                    }
                });
                for (index, entry) in selected["commands"].as_array().into_iter().flatten().enumerate() {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(val_str(&entry["label"]));
                        let Some((kind, value)) = entry["command"].as_object().and_then(|map| map.iter().next()) else { return };
                        let change = |value: Value| call("command_value", json!({ "index": index, "value": value }));
                        match kind.as_str() {
                            "Pov" => {
                                let focus = val_str(&value["focus"]);
                                let mut players: Vec<(String, String)> = model["players"].as_array().into_iter().flatten()
                                    .map(|player| (val_str(&player[1]), format!("{} ({})", val_str(&player[0]), val_str(&player[1])))).collect();
                                if !players.iter().any(|(key, _)| *key == focus) {
                                    players.insert(0, (focus.clone(), focus.clone()));
                                }
                                if let Some(choice) = web_select(ui, &format!("replay_pov_{index}"), &focus, &players, edit) {
                                    actions.push(change(json!({ "focus": choice })));
                                }
                                web_muted(ui, "Player View");
                            }
                            "Hud" => {
                                if let Some(on) = web_toggle(ui, "Visible", value.as_bool().unwrap_or(false), edit) {
                                    actions.push(change(json!(on)));
                                }
                            }
                            _ => {
                                let (low, high, speed) = match kind.as_str() {
                                    "Seek" => (1.0, model["max"].as_f64().unwrap_or(1.0), 1.0),
                                    "Pause" => (0.1, 3600.0, 0.1),
                                    _ => (0.0, 16.0, 0.05),
                                };
                                let mut number = value.as_f64().unwrap_or(0.0);
                                let response = ui.add_enabled(edit, egui::DragValue::new(&mut number).range(low..=high).speed(speed));
                                if response.changed() && !response.dragged() || response.drag_stopped() {
                                    actions.push(change(json!(number)));
                                }
                                if kind == "Speed" && number == 0.0 {
                                    web_muted(ui, "0 freezes replay progression");
                                }
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let danger = ui.visuals().error_fg_color.gamma_multiply(0.5);
                            if ui.add_enabled(edit, egui::Button::new("Remove").fill(danger)).clicked() {
                                actions.push(call("remove_command", json!({ "index": index })));
                            }
                        });
                    });
                }
            } else {
                web_muted(ui, "Select a frame and add a keyframe, or select a diamond to edit its commands.");
            }
        });
        if model.is_object() {
            ui.add_space(8.0);
            Self::egui_replay_track(ui, data, model, ready, actions);
        }
    }

    fn egui_replay_track(ui: &mut egui::Ui, data: &Value, model: &Value, ready: bool, actions: &mut Vec<Value>) {
        let call = |kind: &str, extra: Value| web_act("replay_library", kind, extra);
        let total = model["total"].as_f64().unwrap_or(1.0).max(0.001);
        let zoom = model["zoom"].as_f64().unwrap_or(1.0).max(1.0);
        let start = model["start"].as_f64().unwrap_or(0.0);
        let fps = model["fps"].as_f64().unwrap_or(30.0).max(1.0);
        let max_frame = model["max"].as_i64().unwrap_or(1);
        let span = total / zoom;
        let zoom_limit = (max_frame as f64 / 4.0).max(1.0);
        let playhead = model["playhead"].as_f64().unwrap_or(0.0);
        let view = |zoom: f64, start: f64| call("view", json!({ "zoom": zoom, "start": start }));
        web_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(format!("Frame {} / {}", model["frame"], max_frame)).strong());
                ui.label(val_str(&model["clock"]));
                if ui.button("−").on_hover_text("Zoom out").clicked() {
                    actions.push(view((zoom / 1.6).max(1.0), start));
                }
                if ui.button("+").on_hover_text("Zoom in").clicked() {
                    actions.push(view((zoom * 1.6).min(zoom_limit), start));
                }
                if ui.button("Fit").clicked() {
                    actions.push(view(1.0, 0.0));
                }
                if ui.button("Follow playhead").clicked() {
                    actions.push(view(zoom, playhead - span / 2.0));
                }
                web_muted(ui, &format!("{zoom:.1}× zoom · G goal · S save · blue/orange team · gold pause"));
            });
            if zoom > 1.0 {
                let mut pan = start;
                let response = ui.add(egui::Slider::new(&mut pan, 0.0..=(total - span).max(0.001)).show_value(false));
                if response.drag_stopped() || (response.changed() && !response.dragged()) {
                    actions.push(view(zoom, pan));
                }
            }
            let height = 74.0;
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            let weak = ui.visuals().weak_text_color();
            painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            let x_of = |at: f64| rect.left() + (((at - start) / span).clamp(0.0, 1.0) as f32) * rect.width();
            let inside = |at: f64| ((at - start) / span) >= 0.0 && ((at - start) / span) <= 1.0;
            let wanted = (span * fps / 9.0).max(1.0);
            let base = 10f64.powf(wanted.log10().floor());
            let step = [1.0, 2.0, 5.0, 10.0].iter().map(|value| value * base).find(|value| *value >= wanted).unwrap_or(base * 10.0);
            let mut index = ((start * fps / step).ceil() * step).max(0.0);
            while index <= (start + span) * fps {
                let seconds = index / fps;
                let x = x_of(seconds);
                painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + 8.0)], egui::Stroke::new(1.0, weak));
                painter.text(egui::pos2(x + 2.0, rect.top() + 9.0), egui::Align2::LEFT_TOP, (index as i64 + 1).to_string(), egui::FontId::proportional(10.0), weak);
                painter.text(egui::pos2(x + 2.0, rect.top() + 21.0), egui::Align2::LEFT_TOP, format!("{}:{:02}", (seconds / 60.0).floor() as i64, (seconds % 60.0).floor() as i64), egui::FontId::proportional(10.0), weak);
                index += step;
            }
            let gold = egui::Color32::from_rgb(230, 180, 60);
            let mut hits: Vec<(egui::Rect, Value)> = Vec::new();
            for key in model["keys"].as_array().into_iter().flatten() {
                let at = key["at"].as_f64().unwrap_or(0.0);
                let hold = key["hold"].as_f64().unwrap_or(0.0);
                if hold > 0.0 && at + hold > start && at < start + span {
                    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x_of(at), rect.bottom() - 18.0), egui::pos2(x_of(at + hold), rect.bottom() - 10.0)), 2.0, gold.gamma_multiply(0.5));
                }
                if inside(at) {
                    let centre = egui::pos2(x_of(at), rect.bottom() - 28.0);
                    painter.text(centre, egui::Align2::CENTER_CENTER, "◆", egui::FontId::proportional(14.0), gold);
                    hits.push((egui::Rect::from_center_size(centre, egui::vec2(16.0, 16.0)), call("pick_key", json!({ "frame": key["frame"] }))));
                }
            }
            for marker in model["markers"].as_array().into_iter().flatten() {
                let at = marker["at"].as_f64().unwrap_or(0.0);
                if !inside(at) {
                    continue;
                }
                let event = val_str(&marker["event"]);
                let text = if event == "Goal" { "G" } else if event.to_lowercase().contains("save") { "S" } else { "•" };
                let colour = match marker["team"].as_i64() {
                    Some(0) => egui::Color32::from_rgb(80, 140, 255),
                    Some(1) => egui::Color32::from_rgb(255, 150, 60),
                    _ => weak,
                };
                let centre = egui::pos2(x_of(at), rect.bottom() - 10.0);
                painter.text(centre, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), colour);
                if ready {
                    hits.push((egui::Rect::from_center_size(centre, egui::vec2(14.0, 14.0)), call("scrub", json!({ "frame": marker["frame"] }))));
                }
            }
            if inside(playhead) {
                let x = x_of(playhead);
                painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(2.0, egui::Color32::from_rgb(235, 90, 90)));
            }
            if let Some(pointer) = response.interact_pointer_pos().filter(|_| response.clicked() || response.dragged()) {
                if let Some((_, action)) = hits.iter().find(|(area, _)| response.clicked() && area.contains(pointer)) {
                    actions.push(action.clone());
                } else if ready {
                    let at = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
                    let frame = (1 + ((start + at * span) * fps).round() as i64).clamp(1, max_frame);
                    let id = egui::Id::new("replay_scrub_last");
                    let last: Option<i64> = ui.ctx().data(|d| d.get_temp(id));
                    if last != Some(frame) {
                        ui.ctx().data_mut(|d| d.insert_temp(id, frame));
                        actions.push(call("scrub", json!({ "frame": frame })));
                    }
                }
            }
            if response.hovered() {
                let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let at = ui.input(|input| input.pointer.hover_pos()).map_or(0.5, |pos| ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64);
                    let next = (zoom * (scroll as f64 * 0.005).exp()).clamp(1.0, zoom_limit);
                    actions.push(view(next, start + at * (span - total / next)));
                }
            }
            if ready && ui.memory(|memory| memory.focused().is_none()) {
                let offset = ui.input(|input| (input.key_pressed(egui::Key::ArrowRight) as i64) - (input.key_pressed(egui::Key::ArrowLeft) as i64));
                if offset != 0 {
                    actions.push(call("scrub", json!({ "frame": model["frame"].as_i64().unwrap_or(1) + offset })));
                }
            }
        });
        let _ = data;
    }
}

impl HebnixApp {
    /// Arena previews for the Menu Map tiles come from the same service the arena pages use.
    fn menu_map_previews(&mut self, ctx: &egui::Context) {
        use crate::ui::workshop::background_changer::ArenaAction;
        let key = egui::Id::new("menu_map_images");
        let mut images: std::collections::HashMap<String, Option<std::sync::Arc<[u8]>>> = ctx.data(|d| d.get_temp(key)).unwrap_or_default();
        let wanted: Vec<String> = ctx.data(|d| d.get_temp(egui::Id::new("menu_map_visible"))).unwrap_or_default();
        if wanted.iter().all(|id| images.contains_key(id)) {
            return;
        }
        let rl = self.config.settings.rl_path.clone();
        let background = &mut self.workshop.background_changer;
        if let Ok(action) = serde_json::from_value::<ArenaAction>(json!({ "type": "preview", "ids": wanted })) {
            background.web_action(action, &rl, true, &self.tx, ctx);
        }
        if let Some(previews) = background.web_snapshot(&rl, true, ctx)["previews"].as_object() {
            for (id, value) in previews {
                let bytes = value.as_str().and_then(|uri| uri.split_once(',')).and_then(|(_, encoded)| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok());
                images.insert(id.clone(), bytes.map(Into::into));
            }
        }
        ctx.data_mut(|d| d.insert_temp(key, images));
    }
}
