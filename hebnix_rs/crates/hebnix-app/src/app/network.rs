use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

const LOGIN: &str = "https://api.hebnix.com/network/v1";
const REQUEST: &str = "https://req.hebnix.com/network/v1";
const HEARTBEAT: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    device: String,
    token: String,
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
    discord_id: String,
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
        if status.as_u16() == 498 {
            if let Ok(body) = response.json() {
                hebnix_sdk::req_auth::observe_response(498, &body);
            }
        }
        return Err(Failure {
            message: format!("Hebnix Network returned HTTP {status}"),
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
        session.token = response(redeem)?["token"]
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
        .header("X-App-Token", hebnix_sdk::req_auth::app_token()?)
        .header("Content-Type", "application/json");
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
    let log = hebnix_sdk::log::parse_launch_log(None, false, "INT");
    let (platform, id, _) =
        hebnix_sdk::utils::parse_primary_id(log.session.primary_id.as_deref()?)?;
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
    heartbeat: bool,
) -> Result<Update, Failure> {
    let mut unavailable_updates = Vec::new();
    if heartbeat {
        let mut updates = vec![("presence", json!({"state": "main_menu"}))];
        if let Some(link) = local_player_link() {
            updates.push(("links", link));
        }
        for (path, payload) in updates {
            if let Err(failure) = request(
                &client,
                &session,
                reqwest::Method::PUT,
                path,
                None,
                Some(payload),
            ) {
                if failure.status != 404 {
                    return Err(failure);
                }
                unavailable_updates.push(failure.message);
            }
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
    let players = directory(&client, Some(&session), me["discordId"].as_str())?;
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
            let mut request = client
                .get(format!("{REQUEST}/members"))
                .header("X-App-Token", hebnix_sdk::req_auth::app_token()?);
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
                .filter(|player| Some(player.discord_id.as_str()) != own_id),
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
                Ok(session) => self.session = session,
                Err(error) => self.error = error,
            }
        }
        let now = Instant::now();
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
                }
                Ok(Update::Directory(players)) => self.public_players = Some(players),
                Ok(Update::GuestPresence) => {}
                Ok(Update::Modified) => self.next_poll = None,
                Ok(Update::Snapshot(mut snapshot)) => {
                    if !self.rocket_league_running {
                        snapshot.unavailable_updates.clear();
                    }

                    self.offline.retain(|id, _| {
                        snapshot
                            .friends
                            .iter()
                            .any(|p| p.status == "accepted" && p.discord_id == *id)
                    });
                    for friend in snapshot.friends.iter().filter(|p| p.status == "accepted") {
                        if !friend.online() {
                            self.offline.entry(friend.discord_id.clone()).or_insert(now);
                        } else if self
                            .offline
                            .remove(&friend.discord_id)
                            .is_some_and(|offline| {
                                now.duration_since(offline) > Duration::from_secs(600)
                            })
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
            self.start(ctx, |client| {
                let reply = client
                    .put(format!("{REQUEST}/presence"))
                    .header("X-App-Token", hebnix_sdk::req_auth::app_token()?)
                    .json(&json!({"state": "main_menu"}))
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
        self.start(ctx, move |client| refresh(client, session, heartbeat));
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
                egui::Button::new(egui::RichText::new("🌐").size(20.0))
                    .selected(self.open)
                    .min_size(egui::vec2(30.0, 26.0)),
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
                if self.public_retry.is_none_or(|at| at <= Instant::now()) && self.actions_enabled()
                {
                    self.public_retry = Some(Instant::now() + HEARTBEAT);
                    self.start(ui.ctx(), |client| {
                        directory(&client, None, None).map(Update::Directory)
                    });
                }
                if let Some(at) = self.public_retry {
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
                                .id_salt(&player.discord_id)
                                .show(ui, |ui| {
                                    if self.confirm_block.as_deref()
                                        == Some(player.discord_id.as_str())
                                    {
                                        ui.horizontal(|ui| {
                                            ui.label("Are you sure?");
                                            if ui.button("Yes").clicked() {
                                                operation =
                                                    Some((player.discord_id.clone(), "block"));
                                                self.confirm_block = None;
                                            }
                                            if ui.button("No").clicked() {
                                                self.confirm_block = None;
                                            }
                                        });
                                    } else if ui.button("Block").clicked() {
                                        self.confirm_block = Some(player.discord_id.clone());
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
                                        operation = Some((player.discord_id.clone(), "unblock"));
                                    }
                                });
                            }
                        });
                        for player in snapshot.friends.iter().filter(|p| p.status == "pending") {
                            ui.horizontal(|ui| {
                                ui.label(&player.username);
                                if player.direction == "incoming" {
                                    if ui.button("Accept").clicked() {
                                        operation = Some((player.discord_id.clone(), "accept"));
                                    }
                                    if ui.button("Deny").clicked() {
                                        operation = Some((player.discord_id.clone(), "deny"));
                                    }
                                } else {
                                    ui.label("Request sent");
                                }
                                if ui.button("Block").clicked() {
                                    operation = Some((player.discord_id.clone(), "block"));
                                }
                            });
                        }
                    }
                    View::AllPlayers => {
                        for player in &snapshot.players {
                            let relationship = snapshot
                                .friends
                                .iter()
                                .find(|p| p.discord_id == player.discord_id);
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
                                    operation = Some((player.discord_id.clone(), "send"));
                                }
                                if ui.button("Block").clicked() {
                                    operation = Some((player.discord_id.clone(), "block"));
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
