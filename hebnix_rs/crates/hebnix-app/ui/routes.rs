use super::{
    HebnixApp, HebnixSettingsTab, ItemsSubTab, PatcherSubTab, PluginsSubTab, ReplaysSubTab,
    SpooferSubTab, StudioSection, Tab, home::HomeTab, t,
};
use crate::modern_ui::{
    ConsoleAction, ContentPage, HeaderAction, HeaderPanel, ReplayUploadAction, SwapperAction,
    TrackerAction,
};
use crate::swapper::SwapCategory;
use crate::ui::workshop::WorkshopView;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Subtab {
    Home(HomeTab),
    Workshop(WorkshopView),
    Items(ItemsSubTab),
    Studio(StudioSection),
    Player(SpooferSubTab),
    Replays(ReplaysSubTab),
    Plugins(PluginsSubTab),
    Settings(HebnixSettingsTab),
}

pub(super) fn subtabs(tab: Tab) -> &'static [(Subtab, &'static str)] {
    use Subtab::*;
    match tab {
        Tab::Home => &[
            (Home(HomeTab::LiveMatch), "home-live-match"),
            (Home(HomeTab::Tracker), "home-tracker"),
        ],
        Tab::Workshop => &[
            (Workshop(WorkshopView::Browse), "render-browse-maps"),
            (Workshop(WorkshopView::Import), "nav-add-maps"),
            (
                Workshop(WorkshopView::ArenaBackgrounds),
                "nav-arena-backgrounds",
            ),
            (Workshop(WorkshopView::MapSwapper), "nav-map-swapper"),
            (Workshop(WorkshopView::InGameMenu), "studio-ingame-menu"),
            (Workshop(WorkshopView::MenuMap), "menu-map-title"),
            (Workshop(WorkshopView::Multiplayer), "render-multiplayer"),
        ],
        Tab::Items => &[
            (Items(ItemsSubTab::Swapper), "nav-swapper"),
            (Items(ItemsSubTab::Colours), "nav-colours"),
            (Items(ItemsSubTab::Active), "nav-active"),
            (Items(ItemsSubTab::Presets), "presets-presets"),
            (Items(ItemsSubTab::Loadouts), "loadouts-title"),
        ],
        Tab::Patcher => &[
            (Studio(StudioSection::CustomAssets), "studio-custom-assets"),
            (Studio(StudioSection::Appearance), "studio-appearance"),
            (Studio(StudioSection::Active), "nav-active"),
            (Studio(StudioSection::History), "upk-history"),
        ],
        Tab::Spoofer => &[
            (Player(SpooferSubTab::Username), "nav-player-name"),
            (Player(SpooferSubTab::Titles), "nav-titles"),
            (
                Player(SpooferSubTab::ProfilePictures),
                "nav-profile-pictures",
            ),
            (Player(SpooferSubTab::Scoreboard), "nav-scoreboard"),
            (Player(SpooferSubTab::TitleRank), "spoofer-rank"),
            (Player(SpooferSubTab::Friends), "spoofer-friends"),
        ],
        Tab::Replays => &[
            (Replays(ReplaysSubTab::AutoUploadReplays), "nav-auto-upload"),
            (Replays(ReplaysSubTab::ReplayTimeline), "nav-replay-library"),
        ],
        Tab::Plugins => &[
            (Plugins(PluginsSubTab::Installed), "nav-installed"),
            (Plugins(PluginsSubTab::Browse), "nav-browse"),
            (Plugins(PluginsSubTab::Settings), "settings-subtab-plugin"),
        ],
        Tab::Settings => &[
            (
                Settings(HebnixSettingsTab::Interface),
                "nav-settings-appearance",
            ),
            (
                Settings(HebnixSettingsTab::Directories),
                "nav-files-folders",
            ),
            (Settings(HebnixSettingsTab::System), "settings-nav-system"),
            (Settings(HebnixSettingsTab::Discord), "settings-nav-discord"),
            (
                Settings(HebnixSettingsTab::ActionButton),
                "settings-nav-action-button",
            ),
        ],
        Tab::Freeplay | Tab::GameMods | Tab::Console | Tab::About => &[],
    }
}

impl HebnixApp {
    pub(super) fn render_wry_page(&mut self, ui: &egui::Ui) {
        match self.modern_content_page() {
            Some(ContentPage::About) => { self.render_modern_about(ui); }
            Some(ContentPage::Tracker | ContentPage::LiveMatch) => { self.render_modern_home(ui); }
            Some(ContentPage::BrowseMaps) => { self.render_modern_browse_maps(ui); }
            Some(ContentPage::Arena) => { self.render_modern_arena(ui, self.arena_page().unwrap_or(false)); }
            Some(ContentPage::Manage) => { self.render_modern_manager(ui); }
            Some(ContentPage::Swapper) => { self.render_modern_swapper(ui); }
            Some(ContentPage::Console) => { self.render_modern_console(ui); }
            Some(ContentPage::ReplayUpload) => { self.render_modern_replay_upload(ui); }
            None => {
                tracing::error!(tab = %self.tab.label(), "No Wry route for selected page");
            }
        }
    }

    fn modern_page_title(&self) -> String {
        match self.modern_content_page() {
            Some(ContentPage::About) => return t("about-hebnix"),
            Some(ContentPage::Swapper) if self.items_subtab == ItemsSubTab::Swapper => {
                return t("nav-swapper");
            }
            Some(ContentPage::ReplayUpload) => return "Auto Save + Upload".into(),
            _ => {}
        }
        if self.tab == Tab::About && self.maintenance.tab {
            return t("maintenance-tab");
        }
        let (_, child) = self.modern_tab_selection();
        child
            .and_then(|index| subtabs(self.tab).get(index))
            .map_or_else(|| self.tab.label(), |(_, key)| t(key))
    }

    pub(super) fn on_tracker_page(&self) -> bool {
        self.tab == Tab::Home && self.home.tab == HomeTab::Tracker
    }

    pub(super) fn on_browse_maps_page(&self) -> bool {
        self.tab == Tab::Workshop && self.workshop.view == WorkshopView::Browse
    }

    pub(super) fn arena_page(&self) -> Option<bool> {
        if self.tab != Tab::Workshop {
            return None;
        }
        match self.workshop.view {
            WorkshopView::ArenaBackgrounds => Some(false),
            WorkshopView::MapSwapper => Some(true),
            _ => None,
        }
    }

    pub(super) fn render_modern_arena(&mut self, ui: &egui::Ui, full: bool) -> bool {
        let data = self.workshop.background_changer.web_snapshot(
            &self.config.settings.rl_path,
            full,
            ui.ctx(),
        );
        let Some(navigation) = self.modern_navigation.as_mut() else {
            return false;
        };
        match navigation.show_content(ui, ContentPage::Arena, &data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not show the arena editor");
                navigation.keep_content(None);
                false
            }
        }
    }

    pub(super) fn render_modern_browse_maps(&mut self, ui: &egui::Ui) -> bool {
        let data =
            self.workshop
                .browse_web_snapshot(&self.config.settings.rl_path, &self.tx, ui.ctx());
        let Some(navigation) = &mut self.modern_navigation else {
            return false;
        };
        match navigation.show_content(ui, ContentPage::BrowseMaps, &data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not show Browse Maps");
                navigation.keep_content(None);
                false
            }
        }
    }

    pub(super) fn render_modern_home(&mut self, ui: &egui::Ui) -> bool {
        let (page, data) = if self.on_tracker_page() {
            (ContentPage::Tracker, self.home_tracker.web_snapshot())
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
            (ContentPage::LiveMatch, self.live_match_web_data())
        };
        let Some(navigation) = &mut self.modern_navigation else {
            return false;
        };
        match navigation.show_content(ui, page, &data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not show the Home web view");
                navigation.keep_content(None);
                false
            }
        }
    }

    pub(super) fn handle_modern_tracker(&mut self, action: TrackerAction, ctx: &egui::Context) {
        match action {
            TrackerAction::Select { id } => self.home_tracker.web_select(id),
            TrackerAction::RetryAvatar => self.home_tracker.web_retry_avatar(ctx),
        }
    }

    pub(super) fn render_modern_header(&mut self, ui: &mut egui::Ui) -> bool {
        use serde_json::json;
        if ui.input(|input| input.pointer.any_pressed())
            && let Some(navigation) = &mut self.modern_navigation
        {
            navigation.header_panel = None;
        }
        let panel = self
            .modern_navigation
            .as_ref()
            .and_then(|nav| nav.header_panel);
        let notifications = (panel == Some(HeaderPanel::Notifications)).then(|| {
            self.toasts.mark_read();
            let today = crate::toast::Stamp::now();
            let mut plugins: Vec<_> = self
                .toasts
                .plugins()
                .iter()
                .map(|plugin| {
                    let count = self
                        .toasts
                        .history()
                        .filter(|toast| toast.plugin.slug == plugin.slug)
                        .count();
                    (&plugin.slug, &plugin.name, count)
                })
                .collect();
            plugins.sort_by_cached_key(|(_, name, _)| name.to_lowercase());
            json!({
                "all": t("toast-filter-all"), "clear": t("spoofer-clear"), "empty": t("toast-empty"),
                "plugins": plugins.into_iter().map(|(slug, name, count)| json!({
                    "slug": slug, "name": name, "count": count,
                })).collect::<Vec<_>>(),
                "entries": self.toasts.history().rev().map(|toast| {
                    let (preview, cut) = crate::toast::preview(&toast.text);
                    json!({
                        "id": toast.id, "plugin": toast.plugin.slug, "name": toast.plugin.name,
                        "preview": if cut { format!("{preview}…") } else { preview.to_owned() },
                        "time": toast.at.label(today), "text": toast.text,
                    })
                }).collect::<Vec<_>>(),
            })
        });
        let page_title = self.modern_page_title();
        let actions = self.enabled_action_button_actions();
        let action_count = actions.len().saturating_sub(1);
        let action_inset = self
            .modern_navigation
            .as_ref()
            .map_or(0, |nav| nav.action_inset);
        let action_width = self
            .modern_navigation
            .as_ref()
            .map_or(180, |nav| nav.action_width);
        let data = json!({
            "title": page_title,
            "controller": self.panel.active,
            "page_warning": (self.tab == Tab::Items
                && matches!(self.items_subtab, ItemsSubTab::Swapper | ItemsSubTab::Active)
                && self.last_rl_open).then_some("Close Rocket League before changing item swaps."),
            "panel": panel,
            "action_inset": action_inset,
            "action_width": action_width,
            "status": self.status_text,
            "connected": self.currently_connected,
            "unread": self.toasts.unread(),
            "notifications_label": t("tab-notifications"),
            "notifications": notifications,
            "network": (panel == Some(HeaderPanel::Network)).then(|| self.network.web_snapshot()),
            "actions": actions.into_iter().map(|action| json!({
                "action": action, "label": action.label(),
            })).collect::<Vec<_>>(),
        });
        let Some(navigation) = &mut self.modern_navigation else {
            return false;
        };
        match navigation.show_header(ui, &data, action_count) {
            Ok(Some(height)) => {
                crate::theme::set_workspace_title(ui.ctx(), Some(page_title));
                egui::Panel::top("modern_header")
                    .exact_size(height)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| ui.set_min_height(height));
                true
            }
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(%error, "could not show the Modern UI header");
                navigation.hide_header();
                false
            }
        }
    }

    pub(super) fn handle_modern_header(&mut self, action: HeaderAction, ctx: &egui::Context) {
        let Some(navigation) = &mut self.modern_navigation else {
            return;
        };
        match action {
            HeaderAction::Panel {
                panel,
                action_inset,
                action_width,
            } => {
                navigation.header_panel = panel;
                if panel == Some(HeaderPanel::Actions) {
                    navigation.action_inset = action_inset;
                    navigation.action_width = action_width.clamp(100, 720);
                }
            }
            HeaderAction::ActionAnchor { inset, width }
                if navigation.header_panel == Some(HeaderPanel::Actions) =>
            {
                navigation.action_inset = inset;
                navigation.action_width = width.clamp(100, 720);
            }
            HeaderAction::Run { action } => {
                if navigation.header_panel == Some(HeaderPanel::Actions) {
                    navigation.header_panel = None;
                }
                if self.enabled_action_button_actions().contains(&action) {
                    self.execute_action_button_action(action, ctx);
                }
            }
            HeaderAction::ClearNotifications
                if navigation.header_panel == Some(HeaderPanel::Notifications) =>
            {
                self.toasts.clear();
            }
            HeaderAction::Network { action }
                if navigation.header_panel == Some(HeaderPanel::Network) =>
            {
                self.network.web_action(ctx, action);
            }
            _ => {}
        }
    }

    pub(super) fn modern_content_page(&self) -> Option<ContentPage> {
        match self.tab {
            Tab::Home if self.on_tracker_page() => Some(ContentPage::Tracker),
            Tab::Home => Some(ContentPage::LiveMatch),
            Tab::Workshop if self.on_browse_maps_page() => Some(ContentPage::BrowseMaps),
            Tab::Workshop if self.arena_page().is_some() => Some(ContentPage::Arena),
            Tab::About => Some(if self.maintenance.tab { ContentPage::Manage } else { ContentPage::About }),
            Tab::Console => Some(ContentPage::Console),
            Tab::Plugins | Tab::Spoofer | Tab::Settings | Tab::Freeplay | Tab::GameMods | Tab::Items | Tab::Workshop | Tab::Patcher | Tab::Replays if self.has_modern_manager() => Some(ContentPage::Manage),
            Tab::Items
                if matches!(
                    self.items_subtab,
                    ItemsSubTab::Swapper | ItemsSubTab::Active
                ) =>
            {
                Some(ContentPage::Swapper)
            }
            Tab::Replays if self.replays_subtab == ReplaysSubTab::AutoUploadReplays => {
                Some(ContentPage::ReplayUpload)
            }
            _ => None,
        }
    }

    pub(super) fn render_modern_swapper(&mut self, ui: &egui::Ui) -> bool {
        let cooked = self.swapper_cooked_path();
        if self.items_subtab == ItemsSubTab::Active {
            let data = serde_json::json!({
                "mode": "active",
                "catalog": self.swapper.web_active_catalog(ui.ctx(), &cooked, &self.swapper_preview_ids),
                "game_running": self.last_rl_open,
                "status": self.modern_swapper_status(),
            });
            return self.show_modern_swapper(ui, &data);
        }
        let PatcherSubTab::Swapper(category) = self.swapper_subtab else {
            return false;
        };
        self.owned_inventory.poll(ui.ctx());
        self.swapper.speed_enabled = self.config.speed_patch.items_page;
        let mut categories: Vec<_> = SwapCategory::ALL
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index, value.label(), value == category))
            .collect();
        categories.sort_by_key(|(_, label, _)| label.to_lowercase());
        let catalog = if self.catalogs_loaded && !self.catalogs_loading {
            self.swapper.web_catalog(
                ui.ctx(),
                category,
                &cooked,
                &self.owned_inventory.ids,
                &self.swapper_preview_ids,
            )
        } else {
            serde_json::Value::Null
        };
        let data = serde_json::json!({
            "categories": categories,
            "category_label": category.label(),
            "decals": category == SwapCategory::Skins,
            "goals": category == SwapCategory::Goals,
            "catalog": catalog,
            "loading": self.catalogs_loading,
            "catalog_error": self.catalogs_error,
            "inventory": self.owned_inventory.web_snapshot(),
            "paint": category.supports_recolour().then(|| crate::patcher::painted_swap::SwapPaint::ALL.map(|paint| paint.label())),
            "speed": self.swapper.speed_enabled,
            "game_running": self.last_rl_open,
            "status": self.modern_swapper_status(),
        });
        self.show_modern_swapper(ui, &data)
    }

    fn modern_swapper_status(&self) -> Option<(bool, &str)> {
        self.swapper_web_status.as_ref().map(|result| match result {
            Ok(message) => (false, message.as_str()),
            Err(message) => (true, message.as_str()),
        })
    }

    fn show_modern_swapper(&mut self, ui: &egui::Ui, data: &serde_json::Value) -> bool {
        let Some(navigation) = &mut self.modern_navigation else {
            return false;
        };
        match navigation.show_content(ui, ContentPage::Swapper, data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not show the Item Swapper web view");
                navigation.keep_content(None);
                false
            }
        }
    }

    pub(super) fn swapper_cooked_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.config.settings.rl_path)
            .join("TAGame")
            .join("CookedPCConsole")
    }

    pub(super) fn handle_modern_swapper(&mut self, action: SwapperAction, ctx: &egui::Context) {
        if self.items_subtab == ItemsSubTab::Active {
            match action {
                SwapperAction::Previews {
                    category: None,
                    ids,
                } => {
                    self.swapper_preview_ids = ids.into_iter().take(36).collect();
                }
                SwapperAction::Restore { key }
                    if !hebnix_sdk::process::is_rocket_league_running() =>
                {
                    let cooked = self.swapper_cooked_path();
                    let result = if let Some(key) = key {
                        self.swapper.restore_web_active(&cooked, &key)
                    } else {
                        self.swapper
                            .restore_all_active(&cooked, &cooked.join("Backups"), &self.tx)
                            .map(|count| format!("Restored {count} swaps"))
                    };
                    self.report_web_swap(result);
                }
                _ => {}
            }
            return;
        }
        let PatcherSubTab::Swapper(category) = self.swapper_subtab else {
            return;
        };
        match action {
            SwapperAction::Category { index } => {
                if let Some(category) = crate::swapper::web_category(index) {
                    self.swapper_subtab = PatcherSubTab::Swapper(category);
                    self.swapper_web_status = None;
                    self.swapper_preview_ids.clear();
                }
            }
            SwapperAction::Reload => self.reload_catalogs(ctx),
            SwapperAction::Inventory { account } => {
                self.owned_inventory.refresh_web(ctx, account.as_deref())
            }
            SwapperAction::Previews {
                category: Some(index),
                ids,
            } => {
                if crate::swapper::web_category(index) == Some(category) {
                    self.swapper_preview_ids = ids.into_iter().take(36).collect();
                }
            }
            SwapperAction::Apply { .. } if hebnix_sdk::process::is_rocket_league_running() => {}
            SwapperAction::Apply { swap } => {
                if !self.catalogs_loaded || self.catalogs_loading {
                    return;
                }
                let cooked = self.swapper_cooked_path();
                self.swapper.speed_enabled = self.config.speed_patch.items_page;
                let result = self.swapper.apply_web_swap(
                    ctx,
                    category,
                    &cooked,
                    &self.owned_inventory.ids,
                    swap,
                );
                self.report_web_swap(result);
            }
            SwapperAction::Restore { .. } | SwapperAction::Previews { category: None, .. } => {}
        }
    }

    fn report_web_swap(&mut self, result: Result<String, String>) {
        let text = match &result {
            Ok(message) => message.clone(),
            Err(error) => format!("Error: {error}"),
        };
        let _ = self
            .tx
            .send(crate::messages::AppMsg::Log(format!("[Swapper] {text}")));
        self.swapper_web_status = Some(result);
    }

    pub(super) fn render_modern_console(&mut self, ui: &egui::Ui) -> bool {
        #[derive(serde::Serialize)]
        struct ConsoleData<'a> {
            placeholder: String,
            lines: &'a [String],
            history: &'a [String],
            draft: &'a str,
            suggestions: &'a [String],
        }
        let data = ConsoleData {
            placeholder: t("render-enter-system-command"),
            lines: &self.console.lines,
            history: &self.console.history,
            draft: &self.console.input,
            suggestions: &self.console.suggestions,
        };
        let Some(navigation) = self.modern_navigation.as_mut() else {
            return false;
        };
        match navigation.show_content(ui, ContentPage::Console, &data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not show the Console web view");
                navigation.keep_content(None);
                false
            }
        }
    }

    pub(super) fn handle_modern_console(&mut self, action: ConsoleAction, ctx: &egui::Context) {
        match action {
            ConsoleAction::Draft { value } => {
                let plugin_names = self.console_plugin_names();
                self.console.edit_web_input(value, Some(&plugin_names));
            }
            ConsoleAction::HistoryDraft { value } => self.console.edit_web_input(value, None),
            ConsoleAction::Submit { value } => {
                self.console.edit_web_input(value, None);
                if let Some(command) = self.console.submit() {
                    self.execute_command(command, ctx);
                }
            }
        }
    }

    pub(super) fn render_modern_replay_upload(&mut self, ui: &egui::Ui) -> bool {
        let settings = &self.config.replay_upload;
        let (enabled, patch_status) = self.replay_history_patch.toggle_state();
        let (status, pending) = self.auto_upload_replays.web_status();
        let history: Vec<_> = self
            .auto_upload_replays
            .recent_history()
            .map(|entry| {
                serde_json::json!({
                    "guid": entry.match_guid,
                    "title": entry.title,
                    "warning": entry.warning,
                    "has_link": ballchasing_url(&entry.location).is_some(),
                    "outcome": crate::auto_upload_replays::replay_outcome(entry),
                })
            })
            .collect();
        let data = serde_json::json!({
            "enabled": enabled,
            "can_toggle": self.replay_history_patch.can_toggle(),
            "options_available": self.replay_history_patch.enabled(),
            "can_check": self.replay_options_enabled(),
            "patch_status": patch_status,
            "save_locally": settings.save_locally,
            "upload_enabled": settings.upload_enabled,
            "provider": "ballchasing.com",
            "has_key": !settings.api_key.trim().is_empty(),
            "naming_template": settings.naming_template,
            "visibility": settings.visibility,
            "group_id": settings.group_id,
            "debug_logging": settings.debug_logging,
            "status": status,
            "pending": pending,
            "history": history,
        });
        let Some(navigation) = self.modern_navigation.as_mut() else {
            return false;
        };
        match navigation.show_content(ui, ContentPage::ReplayUpload, &data) {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not show Replay Auto Save + Upload");
                navigation.keep_content(None);
                false
            }
        }
    }

    pub(super) fn handle_modern_replay_upload(
        &mut self,
        action: ReplayUploadAction,
        ctx: &egui::Context,
    ) {
        use ReplayUploadAction::*;
        let available = self.replay_history_patch.enabled();
        let settings = &mut self.config.replay_upload;
        match action {
            Enable { value } => {
                self.replay_history_patch.request_enabled(ctx, value);
                return;
            }
            Check => {
                self.check_replay_upload(ctx);
                return;
            }
            Open { guid } => {
                if let Some(url) = self
                    .auto_upload_replays
                    .recent_history()
                    .find(|entry| entry.match_guid == guid)
                    .and_then(|entry| ballchasing_url(&entry.location))
                {
                    if let Err(error) = open::that(url.as_str()) {
                        tracing::warn!(%error, "could not open replay link");
                    }
                }
                return;
            }
            SaveLocally { value } if available => settings.save_locally = value,
            UploadEnabled { value } if available => settings.upload_enabled = value,
            Provider { value } if settings.upload_enabled && value == "ballchasing.com" => {
                settings.provider = value
            }
            ApiKey { value } if settings.upload_enabled => settings.api_key = value,
            Visibility { value }
                if settings.upload_enabled
                    && matches!(value.as_str(), "private" | "unlisted" | "public") =>
            {
                settings.visibility = value
            }
            GroupId { value } if settings.upload_enabled => settings.group_id = value,
            NamingTemplate { value } => settings.naming_template = value,
            DebugLogging { value } => settings.debug_logging = value,
            _ => return,
        }
        self.replay_upload_settings_changed();
    }

    pub(super) fn modern_tab_labels() -> [crate::modern_ui::TabLabels; Tab::ALL.len()] {
        Tab::ALL.map(|tab| crate::modern_ui::TabLabels {
            label: tab.label(),
            children: subtabs(tab).iter().map(|(_, key)| t(key)).collect(),
            descendants: subtabs(tab).iter().map(|(page, _)| Self::modern_leaf_options(*page)).collect(),
        })
    }

    pub(super) fn select_modern_tab(&mut self, tab: usize, child: Option<usize>) {
        let Some(&tab) = Tab::ALL.get(tab) else {
            return;
        };
        let children = subtabs(tab);
        let child = child.or_else(|| {
            (!children.is_empty()).then(|| self.config.settings.navigation_memory.get(tab.config_key()).copied().unwrap_or(0).min(children.len() - 1))
        });
        let page = match child {
            Some(index) => match children.get(index) {
                Some(&(page, _)) => page,
                None => return,
            },
            None if children.is_empty() => {
                self.tab = tab;
                return;
            }
            None => return,
        };
        self.tab = tab;
        if let Some(index) = child {
            self.config.settings.navigation_memory.insert(tab.config_key().into(), index);
        }
        match page {
            Subtab::Home(value) => self.home.tab = value,
            Subtab::Workshop(value) => self.workshop.view = value,
            Subtab::Items(value) => self.items_subtab = value,
            Subtab::Studio(value) => self.select_studio_section(value),
            Subtab::Player(value) => self.spoofer_subtab = value,
            Subtab::Replays(value) => self.replays_subtab = value,
            Subtab::Plugins(value) => {
                let previous = self.plugins_subtab;
                self.plugins_subtab = value;
                self.maybe_fetch_plugin_catalog(previous);
            }
            Subtab::Settings(value) => self.hebnix_settings_tab = value,
        }
        let options = Self::modern_leaf_options(page);
        if let Some(&(fallback, _)) = options.first() {
            let key = format!("{}/{}", tab.config_key(), children[child.unwrap_or(0)].1);
            let leaf = self.config.settings.navigation_memory.get(&key).copied().filter(|index| options.iter().any(|(id, _)| id == index)).unwrap_or(fallback);
            self.select_modern_leaf(child.unwrap_or(0), leaf);
        }
        self.save_config();
    }

    fn modern_leaf_options(page: Subtab) -> Vec<(usize, String)> {
        use super::modern_manage::{STUDIO_APPEARANCE, STUDIO_CUSTOM};
        let mut rows: Vec<(usize, String)> = match page {
            Subtab::Studio(StudioSection::CustomAssets) => STUDIO_CUSTOM.iter().enumerate().map(|(i, (_, key))| (i, t(key))).collect(),
            Subtab::Studio(StudioSection::Appearance) => STUDIO_APPEARANCE.iter().enumerate().map(|(i, (_, key))| (i, t(key))).collect(),
            Subtab::Settings(HebnixSettingsTab::Interface) => return vec![(0, "General".into()), (1, "Theme".into())],
            _ => return Vec::new(),
        };
        rows.sort_by_cached_key(|(_, label): &(usize, String)| label.to_lowercase());
        rows
    }

    pub(super) fn select_modern_leaf(&mut self, child: usize, leaf: usize) {
        let Some(&(page, _)) = subtabs(self.tab).get(child) else { return };
        if !Self::modern_leaf_options(page).iter().any(|(index, _)| *index == leaf) { return; }
        let key = format!("{}/{}", self.tab.config_key(), subtabs(self.tab)[child].1);
        self.config.settings.navigation_memory.insert(key, leaf);

        match page {
            Subtab::Studio(StudioSection::CustomAssets) => {
                if let Some(&(tab, _)) = super::modern_manage::STUDIO_CUSTOM.get(leaf) {
                    if let Some(tab) = tab { self.items_mode = super::ItemsMode::Patching; self.patcher_subtab = tab; }
                    else { self.items_mode = super::ItemsMode::ItemNames; }
                }
            },
            Subtab::Studio(StudioSection::Appearance) => {
                if let Some(&(mode, _)) = super::modern_manage::STUDIO_APPEARANCE.get(leaf) { self.items_mode = mode; }
            },
            Subtab::Settings(HebnixSettingsTab::Interface) if leaf < 2 => {
                self.appearance_subtab = if leaf == 0 { super::AppearanceSubTab::General } else { super::AppearanceSubTab::Theme };
            }
            _ => {}
        }
        self.save_config();
    }

    pub(super) fn modern_tab_selection(&self) -> (usize, Option<usize>) {
        let tab = Tab::ALL
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or_default();
        let child = subtabs(self.tab).iter().position(|(page, _)| match page {
            Subtab::Home(value) => self.home.tab == *value,
            Subtab::Workshop(value) => self.workshop.view == *value,
            Subtab::Items(value) => self.items_subtab == *value,
            Subtab::Studio(value) => self.items_mode.section() == Some(*value),
            Subtab::Player(value) => self.spoofer_subtab == *value,
            Subtab::Replays(value) => self.replays_subtab == *value,
            Subtab::Plugins(value) => self.plugins_subtab == *value,
            Subtab::Settings(value) => self.hebnix_settings_tab == *value,
        });
        (tab, child)
    }
}

fn ballchasing_url(location: &str) -> Option<url::Url> {
    let url = url::Url::parse(location).ok()?;
    (url.scheme() == "https" && url.host_str() == Some("ballchasing.com")).then_some(url)
}
