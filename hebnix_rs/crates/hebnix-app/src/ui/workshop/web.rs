use super::{
    AppMsg, CardAction, ImageState, WorkshopState, id_of, is_local_entry, str_of, t, t_args,
};
use base64::Engine;
use crossbeam_channel::Sender;
use eframe::egui;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};
use super::ingame_menu::WebMenuAction;

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum BrowseMapsAction {
    Filter { search: String, downloaded: bool },
    Page { index: usize },
    Columns { count: usize },
    Load { id: String },
    Unload { id: String },
    Host { id: String },
    Delete { id: String },
    EnableMenu,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AddMapsAction {
    Local { key: String, fields: Option<[String; 3]> },
    Steam { key: String, api: Option<String>, input: Option<String> },
    Archive { key: String, search: Option<String>, id: Option<usize> },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum MultiplayerAction {
    Connect, Disconnect, Sync, CloseGame, Help,
    Preference { key: String, enabled: bool },
    Download { peer: String, map: String },
    Block { peer: String },
    Unblock { peer: String },
    Confirm { accept: bool },
}

fn peer_token(ip: std::net::IpAddr) -> String {
    use std::hash::{BuildHasher, RandomState};
    static KEY: std::sync::OnceLock<RandomState> = std::sync::OnceLock::new();
    format!("{:016x}", KEY.get_or_init(RandomState::new).hash_one(ip))
}

impl WorkshopState {
    pub(crate) fn menu_map_web_snapshot(&mut self, rl_path: &str, ctx: &egui::Context) -> Value {
        let maps = self.cached_maps();
        self.menu_map.web_snapshot(rl_path, &maps, ctx)
    }

    pub(crate) fn menu_map_web_action(&mut self, action: super::WebMapAction, rl_path: &str, tx: &Sender<AppMsg>, ctx: &egui::Context) {
        let maps = self.cached_maps();
        self.menu_map.web_action(action, rl_path, &maps, tx, ctx);
    }
    pub(crate) fn multiplayer_web_snapshot(&mut self) -> Value {
        use std::sync::atomic::Ordering;
        let received: Vec<_> = self.multiplayer.received_maps.lock().map(|mut maps| maps.drain(..).collect()).unwrap_or_default();
        for map in received { self.add_local_map(map); }
        let state = &self.multiplayer;
        let offers = state.relay.as_ref().map(super::HostSession::peer_offers).unwrap_or_default();
        let mut peers: Vec<_> = state.players.iter().map(|player| {
            let offer = player.alias.and_then(|ip| offers.iter().find(|offer| offer.ip == ip));
            let name = offer.map_or_else(|| player.name.clone(), super::PeerOffer::label);
            let latency = player.latency_ms.filter(|_| player.path != "unreachable");
            (name, latency, offer)
        }).collect();
        peers.extend(offers.iter().filter(|offer| !state.players.iter().any(|player| player.alias == Some(offer.ip)))
            .map(|offer| (offer.label(), None, Some(offer))));
        peers.sort_by_cached_key(|(name, _, _)| name.to_lowercase());
        let rows: Vec<_> = peers.iter().map(|(name, latency, offer)| {
            let hosting = offer.and_then(|offer| offer.maps.iter().find(|map| map.slot == super::HOSTING_SLOT)).map(|map| {
                json!({"id": map.id, "label": t_args("player-maps-hosting", &[("name", map.name.as_str().into())]),
                    "have": super::peer_workshop_file(&self.workshop_maps, map).is_some()})
            });
            json!({"name": name, "latency": latency, "peer": offer.map(|offer| peer_token(offer.ip)), "map": hosting})
        }).collect();
        let own: Vec<_> = self.manager.active_maps().iter().map(|(slot, data)| {
            let id = id_of(data);
            let problems: Vec<_> = offers.iter().filter_map(|offer| super::slot_problem(offer, slot, &id)).collect();
            let status = if offers.is_empty() { String::new() } else if problems.is_empty() { t("my-slot-everyone-has-it") }
                else { t_args("my-slot-problems-of-offers-players-differ", &[("problems", problems.len().to_string().into()), ("offers", offers.len().to_string().into())]) };
            json!({ "label": format!("{slot}: {}", str_of(data, "name", slot)), "status": status, "issues": problems })
        }).collect();
        let blocked: Vec<_> = state.relay.as_ref().into_iter().flat_map(|session| session.blocked())
            .map(|(ip, name)| (peer_token(ip), name)).collect();
        let tunnel = state.tunnel_ready;
        let waiting = self.waiting_for_multihome_check();
        let setup = t(if !tunnel { "multiplayer-connecting-to-the-private-workshop-netwo" }
            else if !state.rl_open { "multiplayer-start-rocket-league-normally" }
            else if !state.launch_ready && waiting { "multiplayer-step-1-waiting-for-rocket-league" }
            else if !state.launch_ready { "multiplayer-rocket-league-still-uses-multihome" }
            else if state.relay.is_none() { "multiplayer-starting-the-workshop-relay" }
            else { "multiplayer-ready-host-or-join-from-rocket" });
        let total = state.map_transfer.total.load(Ordering::Relaxed);
        let done = state.map_transfer.done.load(Ordering::Relaxed);
        let busy = state.map_install_busy.load(Ordering::Relaxed);
        let progress = if busy && total > 0 { Some(json!({ "done": done, "total": total })) } else { None };
        let transfer = if busy { t("maps-in-use-installing-the-map") } else { state.map_install_result.lock().map(|status| status.clone()).unwrap_or_default() };
        let warning = state.pending_peer_install.as_ref().map(|(map, _, _)| {
            t_args("peer-install-warning-name-will-be-downloaded-straight-from", &[("name", map.name.as_str().into())])
        });
        let labels = json!({
            "intro": t("multiplayer-connect-intro"), "relay": t("multiplayer-hide-my-ip-from-other-players"),
            "relayHint": t("multiplayer-relay-only-hover"), "connect": t("multiplayer-connect"),
            "help": t("multiplayer-help-button-using-a-vpn-proxy-or-having"), "setup": t("multiplayer-setup"),
            "stop": t("multiplayer-disconnect"), "closeGame": t("action-close-rocket-league"),
            "toasts": t("multiplayer-show-player-online-toasts"), "password": t("multiplayer-set-a-match-password"),
            "checking": t("multiplayer-checking-launch-command"), "empty": t("multiplayer-no-other-players-online"),
            "search": t("spoofer-search"), "noMatch": t("maps-in-use-no-players-match-that-search"),
            "have": t("player-maps-you-have-it"), "download": t("player-maps-download-to-workshop"),
            "block": t("maps-in-use-block-player"), "unblock": t("multiplayer-unblock"),
            "blockHint": t("multiplayer-blocks-last-until-you-disconnect"),
            "warning": t("peer-install-warning-hebnix-only-checks-that-the-file"),
            "confirm": t("hebnix-install-install"), "cancel": t("spawner-enable-prompt-cancel"),
        });
        let connection = json!({
            "started": state.wizard_started, "stopping": state.tailnet_requested || state.setup_progress.is_some(),
            "status": state.status, "progress": state.setup_progress, "ready": tunnel, "setup": setup,
            "close": tunnel && state.rl_open && !state.launch_ready && !waiting,
            "checking": tunnel && state.rl_open && !state.launch_ready && waiting,
            "host": tunnel && state.rl_open && state.launch_ready && state.relay.is_some(),
            "detected": state.detected_map.as_ref().map(|name| t_args("multiplayer-detected-lan-match-on-map-name", &[("name", name.as_str().into())])),
        });
        let options = json!({ "relay": state.relay_only, "close": state.disconnect_on_close, "toasts": state.player_online_toasts });
        json!({
            "page": "multiplayer", "connection": connection, "options": options, "labels": labels, "rows": rows,
            "search": peers.len() > super::PLAYER_SEARCH_THRESHOLD,
            "heading": super::t_n("multiplayer-players-online", peers.len() as i64),
            "own": own, "busy": busy, "transfer": transfer, "progress": progress, "warning": warning, "blocked": blocked,
        })
    }

    pub(crate) fn multiplayer_web_action(&mut self, action: MultiplayerAction, rl_path: &str, tx: &Sender<AppMsg>, ctx: &egui::Context) {
        match action {
            MultiplayerAction::Connect if !self.multiplayer.wizard_started && !self.multiplayer.tailnet_requested && self.multiplayer.setup_progress.is_none() => self.connect_multiplayer(tx, ctx),
            MultiplayerAction::Disconnect if self.multiplayer.wizard_started => self.disconnect_multiplayer(tx, true),
            MultiplayerAction::Sync if self.multiplayer.tunnel_ready => self.sync_multiplayer_slots(rl_path, ctx),
            MultiplayerAction::CloseGame if self.multiplayer.tunnel_ready && self.multiplayer.rl_open && !self.multiplayer.launch_ready && !self.waiting_for_multihome_check() => {
                self.multiplayer.status = t("multiplayer-closing-rocket-league-start-it-again");
                std::thread::spawn(|| { let _ = crate::winutil::kill_rocket_league(); });
            }
            MultiplayerAction::Help => { let _ = open::that(super::MULTIPLAYER_HELP_URL); }
            MultiplayerAction::Preference { key, enabled } => {
                match key.as_str() {
                    "relay" if !self.multiplayer.wizard_started => self.multiplayer.relay_only = enabled,
                    "close" => self.multiplayer.disconnect_on_close = enabled,
                    "toasts" => self.multiplayer.player_online_toasts = enabled,
                    _ => return,
                }
                self.save_multiplayer_settings();
            }
            MultiplayerAction::Download { peer, map } => {
                let Some(session) = &self.multiplayer.relay else { return };
                let Some((ip, item)) = session.peer_offers().into_iter().find(|offer| peer_token(offer.ip) == peer)
                    .and_then(|offer| offer.maps.into_iter().find(|item| item.slot == super::HOSTING_SLOT && item.id == map)
                        .map(|item| (offer.ip, item))) else { return; };
                if super::peer_workshop_file(&self.workshop_maps, &item).is_some() { return; }
                let target = super::SlotMap { slot: super::WORKSHOP_TARGET.to_owned(), ..item };
                if self.multiplayer.peer_warning_accepted { self.install_peer_map(target, ip, true, rl_path, ctx); }
                else { self.multiplayer.pending_peer_install = Some((target, ip, true)); }
            }
            MultiplayerAction::Block { peer } => {
                if let Some(session) = &self.multiplayer.relay && let Some(offer) = session.peer_offers().into_iter().find(|offer| peer_token(offer.ip) == peer) {
                    session.block(offer.ip, offer.label());
                }
            }
            MultiplayerAction::Unblock { peer } => {
                if let Some(session) = &self.multiplayer.relay && let Some((ip, _)) = session.blocked().into_iter().find(|(ip, _)| peer_token(*ip) == peer) {
                    session.unblock(ip);
                }
            }
            MultiplayerAction::Confirm { accept } => {
                if let Some((map, ip, from_peer)) = self.multiplayer.pending_peer_install.take() && accept {
                    self.multiplayer.peer_warning_accepted = true;
                    self.install_peer_map(map, ip, from_peer, rl_path, ctx);
                }
            }
            _ => {}
        }
    }
    pub(crate) fn add_maps_web_snapshot(&mut self) -> Value {
        let local = self.import_wizard.web_snapshot();
        let (steam, downloaded) = self.steam_downloader.web_snapshot(&self.manager.runtime_dir);
        let (archive, archived) = self.archive_browser.web_snapshot();
        for map in [downloaded, archived].into_iter().flatten() { self.add_local_map(map); }
        json!({ "page": "add-maps", "local": local, "steam": steam, "archive": archive })
    }

    pub(crate) fn add_maps_web_action(&mut self, action: AddMapsAction, ctx: &egui::Context) {
        let cache = &self.manager.cache_dir;
        let runtime = &self.manager.runtime_dir;
        match action {
            AddMapsAction::Local { key, fields } => {
                if let Some(map) = self.import_wizard.web_action(&key, fields, cache, runtime) {
                    self.add_local_map(map);
                }
            }
            AddMapsAction::Steam { key, api, input } => self.steam_downloader.web_action(&key, api, input, cache, runtime, ctx),
            AddMapsAction::Archive { key, search, id } => self.archive_browser.web_action(&key, search, id, cache, runtime, ctx),
        }
    }
    pub(crate) fn menu_web_snapshot(&mut self, cooked: &Path, ctx: &egui::Context) -> Value {
        self.ingame_menu.web_snapshot(cooked, self.panel_mode, ctx)
    }

    pub(crate) fn menu_web_action(&mut self, action: WebMenuAction, cooked: &Path, tx: &Sender<AppMsg>, ctx: &egui::Context) {
        self.ingame_menu.web_action(action, cooked, self.panel_mode, tx, ctx);
    }
    pub(crate) fn browse_web_snapshot(
        &mut self,
        rl_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &egui::Context,
    ) -> Value {
        self.prepare_view(rl_path, ctx);
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        self.ingame_menu.tick(&cooked, ctx);
        self.page_size = self.page_size.clamp(2, 24) / 2 * 2;
        self.page = self.page.min(self.total_pages() - 1);
        let indices: Vec<_> = self
            .valid
            .iter()
            .skip(self.page * self.page_size)
            .take(self.page_size)
            .copied()
            .filter(|index| *index < self.catalog.len())
            .collect();
        let banners: HashSet<_> = indices
            .iter()
            .map(|&index| str_of(&self.catalog[index], "banner_path", "").to_owned())
            .collect();
        self.web_banners.retain(|key, _| banners.contains(key));
        for banner in &banners {
            self.ensure_image(banner, tx, ctx);
            if let Some(ImageState::Ready(bytes)) = self.images.get(banner) {
                self.web_banners
                    .entry(banner.clone())
                    .or_insert_with(|| preview(bytes));
            }
        }
        let maps: Vec<_> = indices.into_iter().map(|index| {
            let map = &self.catalog[index];
            let id = id_of(map);
            let banner = str_of(map, "banner_path", "");
            let active = self.active_targets(&id);
            let loaded = active.contains(&self.target);
            let cached = self.is_downloaded(&id);
            let busy = self.busy.contains(&id);
            let name = str_of(map, "name", "Unknown");
            json!({
                "id": id, "name": name,
                "description": str_of(map, "short_description", ""),
                "author": t_args("hebnix-install-by-author", &[("author", str_of(map, "author", "Unknown").into())]),
                "banner": self.web_banners.get(banner).and_then(|entry| entry.as_deref()),
                "banner_status": t(if banner.is_empty() {
                    "card-no-image-available"
                } else if matches!(self.images.get(banner), Some(ImageState::Failed | ImageState::Ready(_))) {
                    "card-failed-to-load"
                } else {
                    "card-loading-image"
                }),
                "busy": busy, "loaded": loaded, "cached": cached,
                "can_delete": cached && active.is_empty() && !busy,
                "status": if !active.is_empty() {
                    format!("Active on: {}", active.join(", "))
                } else if is_local_entry(map) { "Imported".into() }
                else if cached { "Cached".into() } else { "Cloud".into() },
                "primary": if busy { t("tab-working") }
                else if loaded { format!("Unload {}", self.target) }
                else if cached { format!("Load to {}", self.target) }
                else { format!("Download for {}", self.target) },
                "delete_prompt": t_args(if is_local_entry(map) {
                    "render-remove-the-imported-map-name-its"
                } else { "render-are-you-sure-you-want-to-2" }, &[("name", name.into())]),
            })
        }).collect();
        json!({
            "search": self.search, "search_label": t("ball-name-or-author"),
            "downloaded": self.view_downloaded, "downloaded_label": t("render-view-downloaded"),
            "page": self.page, "pages": self.total_pages(), "count": self.valid.len(),
            "maps": maps,
            "empty": if self.catalog.is_empty() { self.catalog_status.clone() } else { t("handle-messages-no-maps-found") },
            "help_title": t("render-can-t-find-a-map"),
            "help": [t("render-can-t-find-the-map-you"), t("render-no-hubcap-api-key-the-import")],
            "menu_disabled": self.ingame_menu.is_enabled() == Some(false),
            "menu_busy": self.ingame_menu.busy(),
        })
    }

    pub(crate) fn browse_web_action(
        &mut self,
        action: BrowseMapsAction,
        rl_path: &str,
        tx: &Sender<AppMsg>,
        ctx: &egui::Context,
    ) {
        self.prepare_view(rl_path, ctx);
        let (id, action) = match action {
            BrowseMapsAction::Filter { search, downloaded } => {
                self.search = search.chars().take(512).collect();
                self.view_downloaded = downloaded;
                self.execute_search(true);
                return;
            }
            BrowseMapsAction::Page { index } => {
                self.page = index.min(self.total_pages() - 1);
                return;
            }
            BrowseMapsAction::Columns { count } => {
                let previous_first = self.page * self.page_size;
                self.page_size = count.clamp(1, 12) * 2;
                self.page = (previous_first / self.page_size).min(self.total_pages() - 1);
                return;
            }
            BrowseMapsAction::EnableMenu => {
                if !self.ingame_menu.busy() && self.ingame_menu.is_enabled() == Some(false) {
                    let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
                    self.ingame_menu.start(ctx, &cooked, true, tx);
                }
                return;
            }
            BrowseMapsAction::Load { id } => (id, CardAction::InstallOrDownload),
            BrowseMapsAction::Unload { id } => (id, CardAction::Unload),
            BrowseMapsAction::Host { id } => (id, CardAction::HostLan),
            BrowseMapsAction::Delete { id } => (id, CardAction::DeleteCache),
        };
        if id.is_empty() || self.busy.contains(&id) {
            return;
        }
        let Some(index) = self.catalog.iter().position(|map| id_of(map) == id) else {
            return;
        };
        let active = self.active_targets(&id);
        match action {
            CardAction::InstallOrDownload if active.contains(&self.target) => {}
            CardAction::Unload if !active.contains(&self.target) => {}
            CardAction::DeleteCache => {
                if active.is_empty() && self.is_downloaded(&id) {
                    let map = self.catalog[index].clone();
                    self.delete_browse_map(&map, tx);
                }
            }
            action => self.handle_action(index, action, rl_path, tx, ctx),
        }
    }
}

fn preview(bytes: &[u8]) -> Option<String> {
    let image = image::load_from_memory(bytes).ok()?.thumbnail(480, 270);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 78)
        .encode_image(&image.to_rgb8())
        .ok()?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}
