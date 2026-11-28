use super::{
    FreeplayPage, LiveEditorPage, MatchToolsPage, HebnixApp, HebnixSettingsTab, ItemsSubTab, PatcherSubTab, PluginsSubTab, ReplaysSubTab,
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
    MatchTools(MatchToolsPage),
    Freeplay(FreeplayPage),
    LiveEditor(LiveEditorPage),
}

pub(super) fn subtabs(tab: Tab) -> &'static [(Subtab, &'static str)] {
    use Subtab::*;
    match tab {
        Tab::Home => &[
            (Home(HomeTab::Dashboard), "home-dashboard"),
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
            (Items(ItemsSubTab::Loadouts), "loadouts-title"),
            (Items(ItemsSubTab::PresetCreator), "nav-preset-creator"),
            (Items(ItemsSubTab::Presets), "nav-swapper-presets"),
        ],
        Tab::Patcher => &[
            (Studio(StudioSection::CustomAssets), "studio-custom-assets"),
            (Studio(StudioSection::Appearance), "studio-appearance"),
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
            (Settings(HebnixSettingsTab::Active), "nav-active"),
        ],
        Tab::GameMods => &[
            (MatchTools(MatchToolsPage::PAGES[0].0), MatchToolsPage::PAGES[0].1),
            (MatchTools(MatchToolsPage::PAGES[1].0), MatchToolsPage::PAGES[1].1),
            (MatchTools(MatchToolsPage::PAGES[2].0), MatchToolsPage::PAGES[2].1),
        ],
        Tab::Freeplay => &[
            (Freeplay(FreeplayPage::PAGES[0].0), FreeplayPage::PAGES[0].1),
            (Freeplay(FreeplayPage::PAGES[1].0), FreeplayPage::PAGES[1].1),
        ],
        Tab::LiveEditor => &[
            (LiveEditor(LiveEditorPage::PAGES[0].0), LiveEditorPage::PAGES[0].1),
            (LiveEditor(LiveEditorPage::PAGES[1].0), LiveEditorPage::PAGES[1].1),
        ],
        Tab::Console | Tab::About => &[],
    }
}

impl HebnixApp {
    pub(super) fn on_active_garage_page(&self) -> bool {
        self.tab == Tab::Settings && self.hebnix_settings_tab == HebnixSettingsTab::Active
            && self.active_settings_page == super::ActiveSettingsPage::Garage
    }
    pub(super) fn on_active_cosmetics_page(&self) -> bool {
        self.tab == Tab::Settings && self.hebnix_settings_tab == HebnixSettingsTab::Active
            && self.active_settings_page == super::ActiveSettingsPage::Cosmetics
    }
    pub(super) fn on_active_history_page(&self) -> bool {
        self.tab == Tab::Settings && self.hebnix_settings_tab == HebnixSettingsTab::Active
            && self.active_settings_page == super::ActiveSettingsPage::History
    }

    /// True when the sidebar, header and pages come from egui rather than web views.
    pub(super) fn egui_shell(&self) -> bool {
        self.modern_navigation.as_ref().is_some_and(|nav| nav.egui_shell)
    }

    /// Pages drawn by egui instead of a web view.
    pub(super) fn egui_ported_page(&self) -> bool {
        self.egui_shell()
    }

    fn egui_light_page(&self) -> bool {
        self.egui_ported_page()
    }

    fn render_egui_page(&mut self, ui: &mut egui::Ui) {
        if let Some(navigation) = self.modern_navigation.as_mut() {
            navigation.keep_content(None);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            bootstrap_scope(ui, |ui| match self.tab {
                Tab::Home => self.render_home(ui, true),
                Tab::Console => self.render_console_tab(ui, false),
                Tab::About => self.render_about_tab(ui, false),
                _ => self.render_current_page(ui, true),
            });
        });
    }

    pub(super) fn render_wry_page(&mut self, ui: &mut egui::Ui) {
        if self.egui_light_page() {
            self.render_egui_page(ui);
            return;
        }
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
            Some(ContentPage::Swapper) if self.tab == Tab::Items && self.items_subtab == ItemsSubTab::Swapper => {
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

    pub(super) fn render_modern_arena(&mut self, ui: &mut egui::Ui, full: bool) -> bool {
        if self.egui_shell() {
            self.show_arena_egui(ui, full);
            return true;
        }
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
        if self.egui_ported_page() {
            return false;
        }
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

    /// Sidebar for the egui shell, drawn to match the web sidebar: bordered rows, a
    /// chevron on tabs with sub-pages, and an animated sub-menu.
    pub(super) fn show_egui_sidebar(&mut self, ui: &mut egui::Ui) {
        let (tab_index, child) = self.modern_tab_selection();
        let paint = NavPaint::new(ui.ctx());
        let expanded_id = egui::Id::new("egui_nav_expanded");
        let mut expanded: Option<usize> = ui.ctx().data(|d| d.get_temp(expanded_id)).unwrap_or(Some(tab_index));
        let mut pick: Option<(usize, Option<usize>, Option<usize>)> = None;
        egui::Panel::left("modern_navigation")
            .default_size(crate::modern_ui::SIDEBAR_WIDTH)
            .size_range(crate::modern_ui::SIDEBAR_WIDTH..=420.0)
            .frame(egui::Frame::NONE.fill(paint.surface).inner_margin(egui::Margin::symmetric(8, 14)))
            .show(ui, |ui| {
                // still scrolls with the wheel and by dragging
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (index, tab) in Tab::ALL.iter().enumerate() {
                        let pages = subtabs(*tab);
                        let open = expanded == Some(index) && !pages.is_empty();
                        let openness = ui.ctx().animate_bool_with_time(egui::Id::new(("nav_open", index)), open, 0.15);
                        let chevron = (!pages.is_empty()).then_some(openness);
                        if paint.button(ui, &tab.label(), index == tab_index, chevron).clicked() {
                            if open {
                                expanded = None;
                            } else {
                                expanded = (!pages.is_empty()).then_some(index);
                                pick = Some((index, None, None));
                            }
                        }
                        Self::animated_group(ui, egui::Id::new(("nav_group", index)), openness, |ui| {
                            for (at, (page, key)) in pages.iter().enumerate() {
                                let leaves = Self::modern_leaf_options(*page);
                                let selected = index == tab_index && child == Some(at);
                                let leaf_open = selected && !leaves.is_empty();
                                let leaf_t = ui.ctx().animate_bool_with_time(egui::Id::new(("nav_leaf", index, at)), leaf_open, 0.15);
                                let chevron = (!leaves.is_empty()).then_some(leaf_t);
                                if paint.button(ui, &t(key), selected && leaves.is_empty(), chevron).clicked() {
                                    pick = Some((index, Some(at), None));
                                }
                                let memory = format!("{}/{}", tab.config_key(), key);
                                let current = self.config.settings.navigation_memory.get(&memory).copied();
                                Self::animated_group(ui, egui::Id::new(("nav_leaves", index, at)), leaf_t, |ui| {
                                    for (leaf, label) in &leaves {
                                        if paint.button(ui, label, current == Some(*leaf), None).clicked() {
                                            pick = Some((index, Some(at), Some(*leaf)));
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
            });
        ui.ctx().data_mut(|d| d.insert_temp(expanded_id, expanded));
        match pick {
            Some((tab, child, Some(leaf))) => {
                self.select_modern_tab(tab, child);
                self.select_modern_leaf(child.unwrap_or(0), leaf);
            }
            Some((tab, child, None)) => self.select_modern_tab(tab, child),
            None => {}
        }
    }

    /// Indented, left-ruled group that slides open and fades in like the web sub-menu.
    fn animated_group(ui: &mut egui::Ui, id: egui::Id, openness: f32, content: impl FnOnce(&mut egui::Ui)) {
        if openness <= 0.0 {
            return;
        }
        let natural: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(200.0);
        let width = ui.available_width() - 10.0;
        let corner = ui.cursor().min;
        let origin = corner + egui::vec2(10.0, 3.0);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(origin, egui::vec2(width, f32::INFINITY)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let visible = egui::Rect::from_min_size(corner, egui::vec2(width + 10.0, (natural + 6.0) * openness));
        child.set_clip_rect(visible.intersect(ui.clip_rect()));
        child.set_opacity(openness);
        child.spacing_mut().item_spacing.y = 3.0;
        content(&mut child);
        let full = child.min_rect().height();
        ui.ctx().data_mut(|d| d.insert_temp(id, full));
        let rule = egui::Rect::from_min_max(
            corner + egui::vec2(4.0, 3.0),
            egui::pos2(corner.x + 5.0, corner.y + 3.0 + full * openness),
        );
        let stroke = ui.visuals().widgets.noninteractive.bg_stroke.color;
        ui.painter().with_clip_rect(visible).rect_filled(rule, 0.0, stroke);
        ui.allocate_space(egui::vec2(ui.available_width(), (full + 6.0) * openness));
    }

    /// Header for the egui shell: page title plus the controls the legacy top bar has.
    fn render_egui_header(&mut self, ui: &mut egui::Ui) -> bool {
        let title = self.modern_page_title();
        crate::theme::set_workspace_title(ui.ctx(), Some(title.clone()));
        let warn = self.tab == Tab::Items
            && matches!(self.items_subtab, ItemsSubTab::Swapper | ItemsSubTab::Active)
            && self.last_rl_open;
        let rule = ui.visuals().widgets.noninteractive.bg_stroke;
        egui::Panel::top("modern_header")
            .exact_size(52.0)
            .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ui, |ui| {
            ui.painter().hline(ui.max_rect().x_range(), ui.max_rect().bottom() - 0.5, rule);
            bootstrap_scope(ui, |ui| ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(&title).size(20.0).strong());
                if warn {
                    ui.colored_label(ui.visuals().warn_fg_color, "Close Rocket League before changing item swaps.");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    crate::ui::toasts::render_bell(ui, &mut self.toast_view, &mut self.toasts);
                    self.render_network_button(ui);
                    ui.label(egui::RichText::new(&self.status_text).strong().size(12.0).color(self.status_color));
                    self.render_action_button(ui);
                });
            }));
        });
        true
    }

    pub(super) fn render_modern_header(&mut self, ui: &mut egui::Ui) -> bool {
        self.owned_inventory.poll(ui.ctx());
        if self.modern_navigation.as_ref().is_some_and(|nav| nav.egui_shell) {
            return self.render_egui_header(ui);
        }
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
            "show_action_button": self.config.action_button.show_in_header,
            "inventory": self.owned_inventory.web_snapshot(),
            "ownedOnly": self.config.settings.owned_items_only,
            "controller": self.panel.active,
            "page_warning": ((self.on_active_garage_page() || self.tab == Tab::Items
                && self.items_subtab == ItemsSubTab::Swapper)
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
                if panel == Some(HeaderPanel::Settings) { self.owned_inventory.refresh_web(ctx, None); }
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
                if self.enabled_action_button_actions().contains(&action) || (self.tab == Tab::Home && self.home.tab == HomeTab::Dashboard
                    && matches!(action, crate::config::ActionButtonAction::StartRocketLeague | crate::config::ActionButtonAction::CloseRocketLeague)) {
                    self.execute_action_button_action(action, ctx);
                }
            }
            HeaderAction::OwnedAccount { account } => {
                self.owned_inventory.refresh_web(ctx, Some(&account));
            }
            HeaderAction::RefreshSave => {
                self.owned_inventory.refresh_web(ctx, None);
                let path = self.owned_inventory.selected_save().map(std::path::Path::to_path_buf);
                self.loadouts.sync_global_save(path.as_deref(), true);
            }
            HeaderAction::OwnedOnly { enabled } => {
                self.config.settings.owned_items_only = enabled;
                self.config.patcher.decal_only_owned = enabled;
                self.save_config();
            }
            HeaderAction::MoreSettings => {
                navigation.header_panel = None;
                self.tab = Tab::Settings;
                self.hebnix_settings_tab = HebnixSettingsTab::Interface;
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
            Tab::Settings if self.on_active_garage_page() => Some(ContentPage::Swapper),
            Tab::Plugins | Tab::Spoofer | Tab::Settings | Tab::Freeplay | Tab::GameMods | Tab::LiveEditor | Tab::Items | Tab::Workshop | Tab::Patcher | Tab::Replays if self.has_modern_manager() => Some(ContentPage::Manage),
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
        if self.on_active_garage_page() {
            let data = serde_json::json!({
                "mode": "active",
                "catalog": self.swapper.web_active_catalog(ui.ctx(), &cooked, &self.swapper_preview_ids),
                "preview_ids": self.swapper_preview_ids,
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
            "ownedOnly": self.config.settings.owned_items_only,
            "categories": categories,
            "category_label": category.label(),
            "decals": category == SwapCategory::Skins,
            "goals": category == SwapCategory::Goals,
            "catalog": catalog,
            "preview_ids": self.swapper_preview_ids,
            "loading": self.catalogs_loading,
            "catalog_error": self.catalogs_error,
            "inventory": self.owned_inventory.web_snapshot(),
            "paint": category.supports_recolour().then(|| crate::patcher::painted_swap::SwapPaint::ALL.map(|paint| paint.label())),
            "speed": self.swapper.speed_enabled,
            "game_running": self.last_rl_open,
            "status": self.modern_swapper_status(),
            "infinium_choice": self.swapper_web_infinium,
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
        if self.on_active_garage_page() {
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
                    if self.swapper_subtab != PatcherSubTab::Swapper(category) { self.swapper_preview_ids.clear(); }
                    self.swapper_subtab = PatcherSubTab::Swapper(category);
                    self.swapper_web_status = None;
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
                // The egui prompt is hidden behind the web page, so ask for the choice there.
                self.swapper_web_infinium |= self.swapper.take_infinium_prompt();
                self.report_web_swap(result);
            }
            SwapperAction::Infinium { bakkes } => {
                use crate::config::BlackInfinium;
                let choice = if bakkes { BlackInfinium::Bakkes } else { BlackInfinium::Vanilla };
                self.config.settings.black_infinium = choice;
                self.swapper.set_black_infinium(choice);
                let _ = self.config.save(&self.base_dir);
                self.swapper_web_infinium = false;
                self.report_web_swap(Ok(t("infinium-prompt-saved")));
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
            lines: Vec<String>,
            history: &'a [String],
            draft: &'a str,
            suggestions: &'a [String],
        }
        let data = ConsoleData {
            placeholder: t("render-enter-system-command"),
            lines: self.console.lines.iter().map(|line| line.display(self.config.settings.console_timestamps)).collect(),
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
        if self.egui_ported_page() {
            return false;
        }
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
            Subtab::MatchTools(value) => self.match_tools_page = value,
            Subtab::Freeplay(value) => self.freeplay_page = value,
            Subtab::LiveEditor(value) => self.live_editor_page = value,
        }
        let options = Self::modern_leaf_options(page);
        if let Some(&(fallback, _)) = options.first() {
            let key = format!("{}/{}", tab.config_key(), children[child.unwrap_or(0)].1);
            let leaf = self.config.settings.navigation_memory.get(&key).copied().filter(|index| options.iter().any(|(id, _)| id == index)).unwrap_or(fallback);
            self.select_modern_leaf(child.unwrap_or(0), leaf);
        }
        self.config.save_navigation_memory(&self.base_dir);
    }

    fn modern_leaf_options(page: Subtab) -> Vec<(usize, String)> {
        use super::modern_manage::{STUDIO_APPEARANCE, STUDIO_CUSTOM};
        let mut rows: Vec<(usize, String)> = match page {
            Subtab::Studio(StudioSection::CustomAssets) => STUDIO_CUSTOM.iter().enumerate().map(|(i, (_, key))| (i, t(key))).collect(),
            Subtab::Studio(StudioSection::Appearance) => STUDIO_APPEARANCE.iter().enumerate().map(|(i, (_, key))| (i, t(key))).collect(),
            Subtab::Settings(HebnixSettingsTab::Interface) => return vec![(0, "General".into()), (1, "Theme".into())],
            Subtab::Settings(HebnixSettingsTab::Active) => return super::ActiveSettingsPage::PAGES.iter()
                .enumerate().map(|(index, (_, key))| (index, t(key))).collect(),
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
            Subtab::Settings(HebnixSettingsTab::Active) => {
                if let Some(&(page, _)) = super::ActiveSettingsPage::PAGES.get(leaf) {
                    self.active_settings_page = page;
                    if page == super::ActiveSettingsPage::History { self.upk_history.enter(); }
                }
            }
            _ => {}
        }
        self.config.save_navigation_memory(&self.base_dir);
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
            Subtab::MatchTools(value) => self.match_tools_page == *value,
            Subtab::Freeplay(value) => self.freeplay_page == *value,
            Subtab::LiveEditor(value) => self.live_editor_page == *value,
        });
        (tab, child)
    }
}

/// Colours and metrics for the sidebar rows, taken from the active theme.
struct NavPaint {
    surface: egui::Color32,
    selected: egui::Color32,
    text: egui::Color32,
    height: f32,
    radius: u8,
}

impl NavPaint {
    fn new(ctx: &egui::Context) -> Self {
        let visuals = ctx.global_style().visuals.clone();
        match crate::theme::presentation(ctx) {
            Some(p) => {
                let nav = &p.components.navigation;
                Self {
                    surface: crate::theme::color(&nav.surface),
                    selected: crate::theme::color(&nav.selected),
                    text: crate::theme::color(&nav.text),
                    height: nav.height.max(30.0),
                    radius: nav.corner_radius.max(5),
                }
            }
            None => Self {
                surface: visuals.panel_fill,
                selected: visuals.hyperlink_color,
                text: visuals.text_color(),
                height: 34.0,
                radius: 6,
            },
        }
    }

    /// One full-width row. `chevron` is Some(openness) for rows that open a sub-menu.
    fn button(&self, ui: &mut egui::Ui, label: &str, selected: bool, chevron: Option<f32>) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), self.height), egui::Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
        let pick = ui.ctx().animate_bool_with_time(response.id.with("pick"), selected, 0.15);
        let bright = (u32::from(self.selected.r()) * 3 + u32::from(self.selected.g()) * 6 + u32::from(self.selected.b())) / 10 > 150;
        let on_selected = if bright { egui::Color32::from_rgb(16, 18, 28) } else { egui::Color32::WHITE };
        let fill = egui::Color32::TRANSPARENT
            .lerp_to_gamma(self.text.gamma_multiply(0.09), hover)
            .lerp_to_gamma(self.selected, pick);
        let ink = self.text.lerp_to_gamma(on_selected, pick);
        // the outline only fades in on hover; a selected row is just its fill
        let border = egui::Color32::TRANSPARENT.lerp_to_gamma(self.text.gamma_multiply(0.4), hover * (1.0 - pick));
        let painter = ui.painter();
        painter.rect(rect, egui::CornerRadius::same(self.radius.max(9)), fill, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
        painter.text(
            egui::pos2(rect.left() + 11.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.0),
            ink,
        );
        if let Some(open) = chevron {
            // a "v" that turns into a "^" as the menu opens
            let centre = egui::pos2(rect.right() - 16.0, rect.center().y);
            let dy = 2.5 * (1.0 - 2.0 * open);
            let points = vec![
                egui::pos2(centre.x - 4.0, centre.y - dy),
                egui::pos2(centre.x, centre.y + dy),
                egui::pos2(centre.x + 4.0, centre.y - dy),
            ];
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.4, ink)));
        }
        response
    }
}

/// Roomier, bordered controls like the web pages use, in the active theme's colours.
fn bootstrap_scope<R>(ui: &mut egui::Ui, render: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    ui.scope(|ui| {
        let style = ui.style_mut();
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style.spacing.interact_size.y = 34.0;
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        let border = style.visuals.text_color().gamma_multiply(0.3);
        // the same accent the sidebar highlights with, so buttons match the equipped theme
        let accent = NavPaint::new(&ctx).selected;
        style.visuals.hyperlink_color = accent;
        style.visuals.selection.stroke.color = accent;
        let widgets = &mut style.visuals.widgets;
        // purple only while hovered or pressed; resting widgets keep the theme's normal colours
        for (widget, strength) in [(&mut widgets.hovered, 0.6), (&mut widgets.active, 1.0)] {
            widget.bg_fill = accent.gamma_multiply(strength);
            widget.weak_bg_fill = accent.gamma_multiply(strength);
        }
        for widget in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
            widget.corner_radius = egui::CornerRadius::same(6);
            widget.bg_stroke = egui::Stroke::new(1.0, border);
        }
        // group frames double as the web panels: soft border, 8px corners
        widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);
        widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border);
        widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent);
        widgets.active.bg_stroke = egui::Stroke::new(1.0, accent);
        render(ui)
    })
    .inner
}

fn ballchasing_url(location: &str) -> Option<url::Url> {
    let url = url::Url::parse(location).ok()?;
    (url.scheme() == "https" && url.host_str() == Some("ballchasing.com")).then_some(url)
}
