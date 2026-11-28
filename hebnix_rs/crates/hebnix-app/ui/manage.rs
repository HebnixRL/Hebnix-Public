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

    pub(super) fn render_modern_manager(&mut self, ui: &egui::Ui) -> bool {
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
