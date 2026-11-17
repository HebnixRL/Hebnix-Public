use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

const LOGIN: &str = "https://api.hebnix.com/network/v1";
const REQUEST: &str = "https://req.hebnix.com/network/v1";
const HEARTBEAT: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    device: String,
    token: String,
    #[serde(default)]
    player_id: Option<String>,
}

#[cfg(windows)]
fn credential(save: Option<&Session>) -> Result<Option<Session>, String> {
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::Security::Credentials::*,
    };
    let mut target: Vec<u16> = "Hebnix Network".encode_utf16().chain([0]).collect();
    unsafe {
        if let Some(session) = save {
            let mut bytes = serde_json::to_vec(session).map_err(|e| e.to_string())?;
            let entry = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_mut_ptr()),
                CredentialBlobSize: bytes.len() as u32,
                CredentialBlob: bytes.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            CredWriteW(&entry, 0).map_err(|e| e.to_string())?;
            return Ok(None);
        }
        let mut entry = std::ptr::null_mut();
        if let Err(error) = CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut entry)
        {
            if error.code().0 as u32 == 0x80070490 {
                return Ok(None);
            }
            return Err(error.to_string());
        }
        let result = if (*entry).CredentialBlobSize == 0 {
            Ok(None)
        } else {
            serde_json::from_slice(std::slice::from_raw_parts(
                (*entry).CredentialBlob,
                (*entry).CredentialBlobSize as usize,
            ))
            .map(Some)
            .map_err(|e| e.to_string())
        };
        CredFree(entry.cast());
        result
    }
}

#[cfg(not(windows))]
fn credential(_: Option<&Session>) -> Result<Option<Session>, String> {
    Ok(None)
}

fn secret() -> String {
    use rand::RngCore;
    let mut bytes = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Player {
    player_id: String,
    username: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    direction: String,
    presence: Option<Presence>,
}

#[derive(Clone, Deserialize)]
struct Presence {
    state: String,
}

impl Player {
    fn presence_label(&self) -> &'static str {
        match self
            .presence
            .as_ref()
            .map(|presence| presence.state.as_str())
        {
            Some("main_menu") => "Main Menu",
            Some("in_game") => "In Game",
            Some("in_training") => "In Training",
            Some("in_lan_match") => "In LAN Match",
            _ => "Offline",
        }
    }

    fn online(&self) -> bool {
        self.presence_label() != "Offline"
    }
}

struct Snapshot {
    unavailable_updates: Vec<String>,
    friends: Vec<Player>,
    players: Vec<Player>,
}

enum Update {
    GuestPresence,
    Linked(Value),
    SignedIn(Session),
    Snapshot(Snapshot),
    Modified,
    Directory(Vec<Player>),
}

struct Failure {
    message: String,
    status: u16,
    retry_after: Duration,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            status: 0,
            retry_after: Duration::ZERO,
        }
    }
}

fn response(response: reqwest::blocking::Response) -> Result<Value, Failure> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.parse().ok())
        .map_or(HEARTBEAT, Duration::from_secs);
    if !status.is_success() {
        let body = response.json::<Value>().unwrap_or_default();
        hebnix_sdk::req_auth::observe_response(status.as_u16(), &body);
        return Err(Failure {
            message: body["error"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Hebnix Network returned HTTP {status}")),
            status: status.as_u16(),
            retry_after,
        });
    }
    response
        .json()
        .map_err(|_| "Invalid Hebnix Network response".to_owned().into())
}

fn sign_in(client: reqwest::blocking::Client, mut session: Session) -> Result<Update, Failure> {
    let claim = secret();
    let challenge = |value: &str| format!("{:x}", Sha256::digest(value.as_bytes()));
    let start = response(client.post(format!("{LOGIN}/login/start"))
        .json(&json!({"deviceChallenge": challenge(&session.device), "claimChallenge": challenge(&claim)}))
        .send().map_err(|_| "Could not start Discord sign-in".to_owned())?)?;
    let url = start["authorizeUrl"]
        .as_str()
        .and_then(|url| reqwest::Url::parse(url).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && matches!(
                    url.host_str(),
                    Some("discord.com" | "discordapp.com" | "api.hebnix.com")
                )
        })
        .ok_or_else(|| "Invalid Discord authorization URL".to_owned())?;
    let id = start["loginId"]
        .as_str()
        .ok_or_else(|| "Missing login ID".to_owned())?;
    let expires = start["expiresIn"]
        .as_u64()
        .ok_or_else(|| "Missing login expiry".to_owned())?;
    open::that(url.as_str()).map_err(|_| "Could not open the Discord sign-in page".to_owned())?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(expires) {
        std::thread::sleep(Duration::from_secs(5));
        if started.elapsed() >= Duration::from_secs(expires) {
            break;
        }
        let redeem = client
            .post(format!("{LOGIN}/login/redeem"))
            .json(&json!({"loginId": id, "claimSecret": claim, "deviceSecret": session.device}))
            .send()
            .map_err(|_| "Could not check Discord sign-in".to_owned())?;
        if redeem.status().as_u16() == 202 {
            continue;
        }
        let redeemed = response(redeem)?;
        session.player_id = redeemed["playerId"].as_str().map(str::to_owned);
        session.token = redeemed["token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| "Missing Network session token".to_owned())?
            .to_owned();
        credential(Some(&session))?;
        return Ok(Update::SignedIn(session));
    }
    Err("Discord sign-in expired".to_owned().into())
}

fn request(
    client: &reqwest::blocking::Client,
    session: &Session,
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    body: Option<Value>,
) -> Result<Value, Failure> {
    let mut request = client
        .request(method.clone(), format!("{REQUEST}/{path}"))
        .bearer_auth(&session.token)
        .header("X-Hebnix-Device", &session.device)
        .header("Content-Type", "application/json");
    if path != "presence" {
        request = request.header("X-App-Token", hebnix_sdk::req_auth::app_token()?);
    }
    if let Some(cursor) = query {
        request = request.query(&[("after", cursor)]);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    response(
        request
            .send()
            .map_err(|_| "Could not reach Hebnix Network".to_owned())?,
    )
    .map_err(|mut failure| {
        failure.message = format!("{method} /network/v1/{path} · {}", failure.message);
        failure
    })
}

fn local_player_link() -> Option<Value> {
    let (platform, id, _) =
        hebnix_sdk::utils::parse_primary_id(&hebnix_sdk::log::local_player_id()?)?;
    let platform = match platform.as_str() {
        "steam" => "steam",
        "epic" | "epicgames" => "epic",
        "psn" | "ps4" | "ps5" | "playstation" => "psn",
        "xbl" | "xbox" | "xboxone" => "xbox",
        "switch" | "nintendo" => "switch",
        _ => return None,
    };
    let id = id.trim();
    if id.is_empty() || id == "0" {
        return None;
    }
    Some(json!({"platform": platform, "platformId": id}))
}

fn refresh(
    client: reqwest::blocking::Client,
    session: Session,
    presence: Option<Value>,
) -> Result<Update, Failure> {
    let mut unavailable_updates = Vec::new();
    if let Some(presence) = presence {
        if let Err(failure) = request(
            &client,
            &session,
            reqwest::Method::PUT,
            "presence",
            None,
            Some(presence),
        ) {
            if failure.status != 404 {
                return Err(failure);
            }
            unavailable_updates.push(failure.message);
        }
    }
    let me = request(&client, &session, reqwest::Method::GET, "me", None, None)?;
    let friends = request(
        &client,
        &session,
        reqwest::Method::GET,
        "friends",
        None,
        None,
    )?;
    let friends = serde_json::from_value(friends["friends"].clone())
        .map_err(|_| "Invalid friends list".to_owned())?;
    let players = directory(&client, Some(&session), me["playerId"].as_str())?;
    Ok(Update::Snapshot(Snapshot {
        friends,
        players,
        unavailable_updates,
    }))
}

fn directory(
    client: &reqwest::blocking::Client,
    session: Option<&Session>,
    own_id: Option<&str>,
) -> Result<Vec<Player>, Failure> {
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::HashSet::new();
    let mut players = Vec::new();
    loop {
        let page = if let Some(session) = session {
            request(
                client,
                session,
                reqwest::Method::GET,
                "members",
                cursor.as_deref(),
                None,
            )?
        } else {
            let mut request = client.get(format!("{REQUEST}/members"));
            if let Some(cursor) = &cursor {
                request = request.query(&[("after", cursor)]);
            }
            response(
                request
                    .send()
                    .map_err(|_| "Could not reach Hebnix Network".to_owned())?,
            )?
        };
        let rows: Vec<Player> = serde_json::from_value(page["members"].clone())
            .map_err(|_| "Invalid player directory".to_owned())?;
        players.extend(
            rows.into_iter()
                .filter(|player| Some(player.player_id.as_str()) != own_id),
        );
        let Some(next) = page["nextCursor"].as_str() else {
            break;
        };
        if !seen.insert(next.to_owned()) {
            return Err("Invalid player directory cursor".to_owned().into());
        }
        cursor = Some(next.to_owned());
    }
    Ok(players)
}

#[derive(Default, PartialEq)]
enum View {
    #[default]
    Friends,
    Pending,
    AllPlayers,
}

#[derive(Default)]
pub(super) struct Network {
    loaded: bool,
    live: Option<mpsc::Receiver<LiveEvent>>,
    live_presence: Arc<Mutex<Value>>,
    live_token: Option<String>,
    live_cancel: Option<mpsc::Sender<()>>,
    live_healthy: bool,
    live_retry: Option<Instant>,
    live_failures: u32,
    live_me: Option<String>,
    linked: Option<Value>,
    next_link_check: Option<Instant>,
    match_presence: Option<Value>,
    match_state: Option<&'static str>,
    open: bool,
    public_players: Option<Vec<Player>>,
    public_retry: Option<Instant>,
    session: Option<Session>,
    error: String,
    job: Option<mpsc::Receiver<Result<Update, Failure>>>,
    next_poll: Option<Instant>,
    rate_limited_until: Option<Instant>,
    next_heartbeat: Option<Instant>,
    rocket_league_running: bool,
    snapshot: Option<Snapshot>,
    offline: HashMap<String, Instant>,
    view: View,
    confirm_block: Option<String>,
}

impl Network {
    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(reqwest::blocking::Client) -> Result<Update, Failure> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.error.clear();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not initialize Hebnix Network".to_owned().into())
                .and_then(work);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub(super) fn tick(
        &mut self,
        ctx: &egui::Context,
        toasts: &mut crate::toast::ToastCenter,
        rocket_league_running: bool,
    ) {
        if self.rocket_league_running != rocket_league_running {
            self.rocket_league_running = rocket_league_running;
            self.next_heartbeat = None;
            if rocket_league_running {
                self.next_poll = self.rate_limited_until;
            }
        }
        if !self.loaded {
            self.loaded = true;
            match credential(None) {
                Ok(Some(session)) => self.session = Some(session),
                Ok(None) => {
                    let session = Session {
                        device: secret(),
                        token: String::new(),
                        player_id: None,
                    };
                    if let Err(error) = credential(Some(&session)) {
                        self.error = error;
                    }
                    self.session = Some(session);
                }
                Err(error) => self.error = error,
            }
        }
        let now = Instant::now();
        let presence = if !rocket_league_running {
            self.clear_match_presence();
            json!({"state": "offline"})
        } else {
            self.match_presence
                .clone()
                .unwrap_or_else(|| json!({"state": "main_menu"}))
        };
        if let Ok(mut current) = self.live_presence.lock() {
            *current = presence.clone();
        }
        self.poll_live(ctx, toasts, now);
        if let Some(job) = &self.job {
            let result = match job.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err("Network request stopped".to_owned().into())
                }
            };
            self.job = None;
            match result {
                Ok(Update::SignedIn(session)) => {
                    self.session = Some(session);
                    self.next_poll = None;
                    self.next_heartbeat = None;
                    self.linked = None;
                    self.next_link_check = None;
                }
                Ok(Update::Directory(players)) if !self.live_healthy => {
                    self.public_players = Some(players)
                }
                Ok(Update::Directory(_)) => {}
                Ok(Update::GuestPresence) => {}
                Ok(Update::Modified) => self.next_poll = None,
                Ok(Update::Linked(link)) => self.linked = Some(link),
                Ok(Update::Snapshot(snapshot)) if !self.live_healthy => {
                    self.apply_snapshot(snapshot, toasts, now)
                }
                Ok(Update::Snapshot(_)) => {}
                Err(error) => {
                    self.error = error.message;
                    if error.status == 429 {
                        self.rate_limited_until = Some(now + error.retry_after);
                    }
                    self.next_poll = Some(
                        now + if error.status == 429 {
                            error.retry_after
                        } else {
                            HEARTBEAT
                        },
                    );
                    self.public_retry = self.next_poll;
                    if error.status == 401 {
                        if let Some(session) = &mut self.session {
                            session.token.clear();
                            if let Err(error) = credential(Some(session)) {
                                self.error = error;
                            }
                        }
                        self.snapshot = None;
                        self.offline.clear();
                    }
                }
            }
        }
        if let Some(until) = self.rate_limited_until.filter(|until| *until > now) {
            ctx.request_repaint_after(until - now);
        }
        if rocket_league_running
            && self.actions_enabled()
            && self.next_link_check.is_none_or(|next| next <= now)
        {
            if let Some(session) = self
                .session
                .clone()
                .filter(|session| !session.token.is_empty())
            {
                self.next_link_check = Some(now + Duration::from_secs(5));
                let last_link = self.linked.clone();
                self.start(ctx, move |client| {
                    let Some(link) = local_player_link() else {
                        return Ok(Update::GuestPresence);
                    };
                    if last_link.as_ref() == Some(&link) {
                        return Ok(Update::GuestPresence);
                    }
                    request(
                        &client,
                        &session,
                        reqwest::Method::PUT,
                        "links",
                        None,
                        Some(link.clone()),
                    )?;
                    Ok(Update::Linked(link))
                });
                return;
            }
        }
        if self.live_healthy {
            return;
        }
        let session = self
            .session
            .clone()
            .filter(|session| !session.token.is_empty());
        let Some(session) = session else {
            if !rocket_league_running {
                return;
            }
            let deadline = self
                .next_heartbeat
                .into_iter()
                .chain(self.rate_limited_until)
                .max();
            if let Some(next) = deadline.filter(|next| *next > now) {
                ctx.request_repaint_after(next - now);
                return;
            }
            self.next_heartbeat = Some(now + HEARTBEAT);
            let Some(device) = self.session.as_ref().map(|session| session.device.clone()) else {
                return;
            };
            self.start(ctx, move |client| {
                let reply = client
                    .put(format!("{REQUEST}/presence"))
                    .header("X-Hebnix-Device", device)
                    .json(&presence)
                    .send()
                    .map_err(|_| "Could not reach Hebnix Network".to_owned())?;
                response(reply).map_err(|mut error| {
                    error.message = format!("PUT /network/v1/presence · {}", error.message);
                    error
                })?;
                Ok(Update::GuestPresence)
            });
            return;
        };
        if let Some(next) = self.next_poll.filter(|next| *next > now) {
            ctx.request_repaint_after(next - now);
            return;
        }
        let heartbeat = rocket_league_running && self.next_heartbeat.is_none_or(|next| next <= now);
        if heartbeat {
            self.next_heartbeat = Some(now + HEARTBEAT);
        }
        self.next_poll = self.next_heartbeat.or(Some(now + HEARTBEAT));
        self.start(ctx, move |client| {
            refresh(client, session, heartbeat.then_some(presence))
        });
    }

    fn apply_snapshot(
        &mut self,
        mut snapshot: Snapshot,
        toasts: &mut crate::toast::ToastCenter,
        now: Instant,
    ) {
        if !self.rocket_league_running {
            snapshot.unavailable_updates.clear();
        }
        self.offline.retain(|id, _| {
            snapshot
                .friends
                .iter()
                .any(|p| p.status == "accepted" && p.player_id == *id)
        });
        for friend in snapshot.friends.iter().filter(|p| p.status == "accepted") {
            if !friend.online() {
                self.offline.entry(friend.player_id.clone()).or_insert(now);
            } else if self
                .offline
                .remove(&friend.player_id)
                .is_some_and(|offline| now.duration_since(offline) > Duration::from_secs(600))
            {
                toasts.push(
                    "hebnix-network",
                    "Network",
                    &format!("{} came online", friend.username),
                    Default::default(),
                );
            }
        }
        self.snapshot = Some(snapshot);
    }

    fn actions_enabled(&self) -> bool {
        self.job.is_none()
            && self
                .rate_limited_until
                .is_none_or(|until| until <= Instant::now())
    }

    pub(super) fn render_button(&mut self, ui: &mut egui::Ui) {
        let button = ui
            .add(
                egui::Button::image(
                    egui::Image::new(egui::include_image!("../../assets/network-globe.svg"))
                        .fit_to_exact_size(egui::vec2(16.0, 16.0))
                        .tint(ui.visuals().text_color()),
                )
                .selected(self.open)
                .min_size(egui::vec2(28.0, 24.0)),
            )
            .on_hover_text("Network");
        let mut open = self.open ^ button.clicked();
        let width = (ui.ctx().content_rect().width() - 32.0).clamp(180.0, 400.0);
        egui::Popup::from_response(&button)
            .open_bool(&mut open)
            .align(egui::RectAlign::BOTTOM_END)
            .align_alternatives(&[])
            .gap(4.0)
            .width(width)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.heading("Network");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            ui.close();
                        }
                    });
                });
                ui.separator();
                self.render(ui);
            });
        self.open = open;
    }

    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        if !self.error.is_empty() {
            ui.label(&self.error);
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view, View::Friends, "Friends");
            ui.selectable_value(&mut self.view, View::Pending, "Pending");
            ui.selectable_value(&mut self.view, View::AllPlayers, "All Players");
        });
        ui.separator();
        let Some(session) = self
            .session
            .clone()
            .filter(|session| !session.token.is_empty())
        else {
            if self.view == View::AllPlayers {
                if !self.live_healthy
                    && self.public_retry.is_none_or(|at| at <= Instant::now())
                    && self.actions_enabled()
                {
                    self.public_retry = Some(Instant::now() + HEARTBEAT);
                    self.start(ui.ctx(), |client| {
                        directory(&client, None, None).map(Update::Directory)
                    });
                }
                if let Some(at) = self
                    .public_retry
                    .filter(|at| !self.live_healthy && *at > Instant::now())
                {
                    ui.ctx()
                        .request_repaint_after(at.saturating_duration_since(Instant::now()));
                }
                if let Some(players) = &self.public_players {
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            for player in players {
                                ui.horizontal(|ui| {
                                    ui.label(&player.username);
                                    if player.presence.is_some() {
                                        ui.label(if player.online() {
                                            "Online"
                                        } else {
                                            "Offline"
                                        });
                                    }
                                });
                            }
                        });
                } else if self.job.is_some() {
                    ui.spinner();
                }
                return;
            }
            ui.add_enabled_ui(self.actions_enabled(), |ui| {
                if ui.button("Sign in with Discord").clicked() {
                    let session = self.session.clone().unwrap_or_else(|| Session {
                        device: secret(),
                        token: String::new(),
                        player_id: None,
                    });
                    self.start(ui.ctx(), move |client| {
                        credential(Some(&session))?;
                        sign_in(client, session)
                    });
                }
            });
            if self.job.is_some() {
                ui.label("Waiting for Discord sign-in…");
            }
            return;
        };
        let Some(snapshot) = self.snapshot.as_ref() else {
            if self.job.is_some() {
                ui.spinner();
            }
            return;
        };
        if self.rocket_league_running {
            for warning in &snapshot.unavailable_updates {
                ui.label(warning);
            }
        }
        let mut operation = None;
        let enabled = self.actions_enabled();
        egui::ScrollArea::vertical()
            .id_salt("network_players")
            .max_height(420.0)
            .show(ui, |ui| {
                ui.add_enabled_ui(enabled, |ui| match self.view {
                    View::Friends => {
                        for player in snapshot.friends.iter().filter(|p| p.status == "accepted") {
                            let color = if player.online() {
                                egui::Color32::WHITE
                            } else {
                                egui::Color32::LIGHT_GRAY
                            };
                            let title =
                                format!("{} - {}", player.username, player.presence_label());
                            egui::CollapsingHeader::new(egui::RichText::new(title).color(color))
                                .id_salt(&player.player_id)
                                .show(ui, |ui| {
                                    if self.confirm_block.as_deref()
                                        == Some(player.player_id.as_str())
                                    {
                                        ui.horizontal(|ui| {
                                            ui.label("Are you sure?");
                                            if ui.button("Yes").clicked() {
                                                operation =
                                                    Some((player.player_id.clone(), "block"));
                                                self.confirm_block = None;
                                            }
                                            if ui.button("No").clicked() {
                                                self.confirm_block = None;
                                            }
                                        });
                                    } else if ui.button("Block").clicked() {
                                        self.confirm_block = Some(player.player_id.clone());
                                    }
                                });
                        }
                    }
                    View::Pending => {
                        egui::CollapsingHeader::new("Blocked players").show(ui, |ui| {
                            for player in snapshot.friends.iter().filter(|p| p.status == "blocked")
                            {
                                ui.horizontal(|ui| {
                                    ui.label(&player.username);
                                    if ui.button("Unblock").clicked() {
                                        operation = Some((player.player_id.clone(), "unblock"));
                                    }
                                });
                            }
                        });
                        for player in snapshot.friends.iter().filter(|p| p.status == "pending") {
                            ui.horizontal(|ui| {
                                ui.label(&player.username);
                                if player.direction == "incoming" {
                                    if ui.button("Accept").clicked() {
                                        operation = Some((player.player_id.clone(), "accept"));
                                    }
                                    if ui.button("Deny").clicked() {
                                        operation = Some((player.player_id.clone(), "deny"));
                                    }
                                } else {
                                    ui.label("Request sent");
                                }
                                if ui.button("Block").clicked() {
                                    operation = Some((player.player_id.clone(), "block"));
                                }
                            });
                        }
                    }
                    View::AllPlayers => {
                        for player in &snapshot.players {
                            let relationship = snapshot
                                .friends
                                .iter()
                                .find(|p| p.player_id == player.player_id);
                            ui.horizontal(|ui| {
                                ui.label(&player.username);
                                if let Some(status) = relationship
                                    .filter(|p| p.status == "accepted")
                                    .or_else(|| player.presence.as_ref().map(|_| player))
                                {
                                    ui.label(if status.online() { "Online" } else { "Offline" });
                                }
                                if relationship.is_none_or(|p| p.status == "denied")
                                    && ui.button("Add friend").clicked()
                                {
                                    operation = Some((player.player_id.clone(), "send"));
                                }
                                if ui.button("Block").clicked() {
                                    operation = Some((player.player_id.clone(), "block"));
                                }
                            });
                        }
                    }
                });
            });
        if let Some((peer, operation)) = operation {
            self.start(ui.ctx(), move |client| {
                request(
                    &client,
                    &session,
                    reqwest::Method::POST,
                    "friends",
                    None,
                    Some(json!({
                        "peer": peer, "operation": operation,
                    })),
                )?;
                Ok(Update::Modified)
            });
        }
    }
}
#[derive(Deserialize)]
struct LiveFrame {
    #[serde(rename = "type")]
    kind: String,
    members: Option<Vec<Player>>,
    friends: Option<Vec<Player>>,
    me: Option<LiveIdentity>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveIdentity {
    player_id: String,
}

enum LiveEvent {
    Frame(LiveFrame),
    End {
        planned: bool,
        session_rejected: bool,
    },
}

fn live_connection(
    session: Session,
    presence: Arc<Mutex<Value>>,
    tx: &mpsc::Sender<LiveEvent>,
    cancel: mpsc::Receiver<()>,
    ctx: &egui::Context,
) -> Result<bool, tungstenite::Error> {
    use std::net::{TcpStream, ToSocketAddrs};
    use tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};
    let mut request = "wss://req.hebnix.com/network/v1/live".into_client_request()?;
    let headers = request.headers_mut();
    let header = |value: &str| {
        HeaderValue::from_str(value).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid Network credential",
            )
        })
    };
    headers.insert("X-Hebnix-Device", header(&session.device)?);
    if !session.token.is_empty() {
        headers.insert(
            "Authorization",
            header(&format!("Bearer {}", session.token))?,
        );
        let token = hebnix_sdk::req_auth::app_token().map_err(std::io::Error::other)?;
        headers.insert("X-App-Token", header(&token)?);
    }
    let address = ("req.hebnix.com", 443)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other("Network host unavailable"))?;
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let (mut socket, _) =
        tungstenite::client_tls(request, stream).map_err(|error| match error {
            tungstenite::HandshakeError::Failure(error) => error,
            tungstenite::HandshakeError::Interrupted(_) => {
                std::io::Error::other("Network handshake interrupted").into()
            }
        })?;
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::NativeTls(stream) => stream.get_ref(),
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream,
        _ => return Ok(false),
    }
    .set_read_timeout(Some(Duration::from_millis(250)))?;
    let started = Instant::now();
    let mut sent = Value::Null;
    let mut last_ping = started;
    let mut snapshot = false;
    while started.elapsed() < HEARTBEAT
        && matches!(cancel.try_recv(), Err(mpsc::TryRecvError::Empty))
    {
        let current = presence
            .lock()
            .map_err(|_| std::io::Error::other("Network presence unavailable"))?
            .clone();
        if current != sent {
            socket.send(Message::Text(
                json!({"type": "presence", "presence": current}).to_string(),
            ))?;
            sent = current;
        }
        if last_ping.elapsed() >= Duration::from_secs(20) {
            socket.send(Message::Ping(Vec::new()))?;
            last_ping = Instant::now();
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Ok(frame) = serde_json::from_str::<LiveFrame>(&text) {
                    if frame.kind == "snapshot" || (snapshot && frame.kind == "update") {
                        snapshot = true;
                        if tx.send(LiveEvent::Frame(frame)).is_err() {
                            return Ok(false);
                        }
                        ctx.request_repaint();
                    }
                }
            }
            Ok(Message::Ping(_)) => socket.flush()?,
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return Ok(snapshot && started.elapsed() >= Duration::from_secs(58));
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if !snapshot && started.elapsed() >= Duration::from_secs(5) {
                    return Ok(false);
                }
            }
            Err(error) => return Err(error),
            _ => {}
        }
    }
    let _ = socket.close(None);
    Ok(snapshot)
}

impl Network {
    fn poll_live(
        &mut self,
        ctx: &egui::Context,
        toasts: &mut crate::toast::ToastCenter,
        now: Instant,
    ) {
        if self.live.is_some()
            && self.live_token.as_ref() != self.session.as_ref().map(|session| &session.token)
        {
            self.live_healthy = false;
            self.live_cancel.take();
        }
        while let Some(event) = self.live.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Disconnected) => Some(LiveEvent::End {
                planned: false,
                session_rejected: false,
            }),
            Err(mpsc::TryRecvError::Empty) => None,
        }) {
            match event {
                LiveEvent::Frame(frame) => {
                    let active_token = self.session.as_ref().map(|session| &session.token);
                    if self.live_token.as_ref() != active_token {
                        continue;
                    }
                    let signed_in = active_token.is_some_and(|token| !token.is_empty());
                    let replace = frame.kind == "snapshot";
                    if signed_in && replace && (frame.me.is_none() || frame.friends.is_none()) {
                        continue;
                    }
                    self.live_healthy = true;
                    self.live_failures = 0;
                    if !signed_in {
                        if replace || frame.members.is_some() {
                            self.public_players = frame.members;
                        }
                        continue;
                    }
                    if let Some(me) = frame.me {
                        self.live_me = Some(me.player_id);
                    }
                    let previous = if replace { None } else { self.snapshot.take() };
                    let (old_friends, old_players) = previous
                        .map(|snapshot| (snapshot.friends, snapshot.players))
                        .unwrap_or_default();
                    let friends = frame.friends.unwrap_or(old_friends);
                    let mut players = frame.members.unwrap_or(old_players);
                    players.retain(|player| Some(&player.player_id) != self.live_me.as_ref());
                    self.apply_snapshot(
                        Snapshot {
                            friends,
                            players,
                            unavailable_updates: Vec::new(),
                        },
                        toasts,
                        now,
                    );
                }
                LiveEvent::End {
                    planned,
                    session_rejected,
                } => {
                    self.live = None;
                    self.live_cancel.take();
                    self.live_healthy = false;
                    if session_rejected {
                        if let Some(session) = &mut self.session {
                            if self.live_token.as_ref() == Some(&session.token)
                                && !session.token.is_empty()
                            {
                                session.token.clear();
                                session.player_id = None;
                                let _ = credential(Some(session));
                                self.snapshot = None;
                                self.offline.clear();
                                self.linked = None;
                                self.error = "Invalid or expired network session".into();
                            }
                        }
                    }
                    self.live_retry = if planned {
                        None
                    } else {
                        self.live_failures = self.live_failures.saturating_add(1);
                        let millis = 1000u64 << self.live_failures.min(5);
                        Some(
                            now + Duration::from_millis(
                                millis / 2 + rand::random::<u64>() % (millis / 2),
                            ),
                        )
                    };
                }
            }
        }
        if let Some(next) = self.live_retry.filter(|next| *next > now) {
            ctx.request_repaint_after(next - now);
            return;
        }
        if self.live.is_some() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.live_token = Some(session.token.clone());
        self.live_me = session.player_id.clone();
        let (tx, rx) = mpsc::channel();
        self.live = Some(rx);
        let (cancel_tx, cancel_rx) = mpsc::channel();
        self.live_cancel = Some(cancel_tx);
        let ctx = ctx.clone();
        let presence = Arc::clone(&self.live_presence);
        std::thread::spawn(move || {
            let result = live_connection(session, presence, &tx, cancel_rx, &ctx);
            let session_rejected = matches!(&result, Err(tungstenite::Error::Http(response)) if response.status().as_u16() == 401);
            let _ = tx.send(LiveEvent::End {
                planned: matches!(result, Ok(true)),
                session_rejected,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn set_match_kind(&mut self, game: Option<&hebnix_sdk::log::LogGameInfo>) {
        self.match_state = game.and_then(|game| {
            let tags = game
                .game_tags
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let class = game
                .game_class
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if class.contains("training") || tags.contains("training") || tags.contains("freeplay")
            {
                Some("in_training")
            } else if tags
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tag| tag == "lan")
                || game
                    .server_ip
                    .as_deref()
                    .is_some_and(|ip| ip.starts_with("127.10."))
            {
                Some("in_lan_match")
            } else {
                None
            }
        });
    }

    pub(super) fn update_match_presence(
        &mut self,
        game: &hebnix_sdk::stats::models::GameState,
        mode: Option<&str>,
    ) {
        if let Some(state) = self.match_state {
            self.match_presence = Some(json!({"state": state}));
            return;
        }
        let team = |index| game.teams.iter().find(|team| team.team_num == index);
        let (Some(blue), Some(orange)) = (team(0), team(1)) else {
            return;
        };
        let mut presence = json!({
            "state": "in_game",
            "map": crate::discord_presence::display_map_name(&game.arena),
            "timeSeconds": game.time_seconds.max(0),
            "score": {"blue": blue.score.clamp(0, 999), "orange": orange.score.clamp(0, 999)},
            "teamNames": {"blue": blue.name, "orange": orange.name}
        });
        if let Some(mode) = mode.filter(|mode| !mode.is_empty()) {
            presence["gameMode"] = json!(mode);
        }
        self.match_presence = Some(presence);
    }

    pub(super) fn clear_match_presence(&mut self) {
        self.match_presence = None;
        self.match_state = None;
    }
}
