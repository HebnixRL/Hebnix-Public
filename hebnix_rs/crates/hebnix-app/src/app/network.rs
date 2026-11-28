use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
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
        Win32::Security::Credentials::*,
        core::{PCWSTR, PWSTR},
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
    #[serde(default)]
    links: Vec<FriendLink>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FriendLink {
    platform: String,
    platform_id: String,
    #[serde(default)]
    display_name: String,
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountSettings {
    presence_visibility: String,
    friend_code: String,
}

enum Update {
    GuestPresence,
    AccountSettings(AccountSettings),
    FriendRequestSent,
    FriendRanks(String, hebnix_sdk::tracker::PlayerStats),
    Linked(Value),
    SignedIn(Session),
    Snapshot(Snapshot),
    Modified,
    FriendRemoved(String),
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
    Ok(Update::Snapshot(Snapshot {
        friends,
        unavailable_updates,
    }))
}

#[derive(Default, PartialEq)]
enum View {
    #[default]
    Friends,
    Pending,
    AddFriend,
    Settings,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct PresenceNotificationSettings {
    hide_match_users: bool,
    hide: bool,
    #[serde(skip)]
    loaded: bool,
}

impl PresenceNotificationSettings {
    fn path() -> std::io::Result<std::path::PathBuf> {
        dirs::config_dir()
            .map(|dir| dir.join("Hebnix").join("network-notifications.json"))
            .ok_or_else(|| std::io::Error::other("Hebnix configuration directory unavailable"))
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        if let Ok(path) = Self::path() {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(settings) = serde_json::from_slice(&bytes) {
                    *self = settings;
                }
            }
        }
        self.loaded = true;
    }

    fn save(&self) -> std::io::Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }
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
    linked: Option<Value>,
    next_link_check: Option<Instant>,
    match_presence: Option<Value>,
    presence_account: Option<Value>,
    next_presence_account_check: Option<Instant>,
    match_state: Option<&'static str>,
    open: bool,
    session: Option<Session>,
    error: String,
    job: Option<mpsc::Receiver<Result<Update, Failure>>>,
    next_poll: Option<Instant>,
    rate_limited_until: Option<Instant>,
    next_heartbeat: Option<Instant>,
    rocket_league_running: bool,
    snapshot: Option<Snapshot>,
    last_online: HashMap<String, Instant>,
    first_directory: Option<Instant>,
    friend_ranks: HashMap<String, hebnix_sdk::tracker::PlayerStats>,
    match_players: Vec<(String, String, String)>,
    match_notified: std::collections::HashSet<(String, String)>,
    match_lookup: Option<mpsc::Receiver<Vec<(String, String, String)>>>,
    next_match_lookup: Option<Instant>,
    notification_settings: PresenceNotificationSettings,
    account_settings: Option<AccountSettings>,
    account_settings_session: Option<String>,
    account_settings_requested: bool,
    friend_code_input: String,
    view: View,
    confirm_block: Option<String>,
    confirm_remove: Option<String>,
}

impl Network {
    #[cfg(windows)]
    pub(super) fn web_snapshot(&self) -> Value {
        let session = self.session.as_ref().filter(|session| !session.token.is_empty());
        let signed_in = session.is_some();
        let settings = self.account_settings.as_ref().filter(|_| {
            session.is_some_and(|session| self.account_settings_session.as_ref() == Some(&session.token))
        });
        let players: Vec<_> = self.snapshot.iter().flat_map(|snapshot| &snapshot.friends)
            .map(|player| {
                let links: Vec<_> = player.links.iter().enumerate().map(|(index, link)| {
                    let stats = self.friend_ranks.get(&format!("{}:{}", link.platform, link.platform_id));
                    let mut ranks: Vec<_> = stats.into_iter().flat_map(|stats| stats.ranks.values()).collect();
                    ranks.sort_by_key(|rank| rank.playlist_id);
                    json!({
                        "index": index, "platform": link.platform, "name": link.display_name,
                        "loaded": stats.is_some(), "error": stats.and_then(|stats| stats.error.as_deref()),
                        "ranks": ranks.into_iter().map(|rank| json!({
                            "mode": rank.playlist_name, "rank": format!("{} {}", rank.tier_name, rank.division_name),
                        })).collect::<Vec<_>>(),
                    })
                }).collect();
                json!({
                    "id": player.player_id, "name": player.username, "status": player.status,
                    "direction": player.direction, "presence": player.presence_label(), "links": links,
                })
            }).collect();
        json!({
            "signed_in": signed_in, "enabled": self.actions_enabled(), "busy": self.job.is_some(),
            "error": self.error, "players": players,
            "warnings": self.snapshot.as_ref().filter(|_| self.rocket_league_running).map(|snapshot| &snapshot.unavailable_updates),
            "friend_code": settings.map(|settings| &settings.friend_code),
            "presence": settings.map(|settings| &settings.presence_visibility),
            "hide_match_users": self.notification_settings.hide_match_users,
            "hide_presence": self.notification_settings.hide,
        })
    }

    #[cfg(windows)]
    pub(super) fn web_action(&mut self, ctx: &egui::Context, action: WebAction) {
        use WebAction::*;
        match action {
            Notifications { hide_match_users, hide_presence } => {
                self.notification_settings.hide_match_users = hide_match_users;
                self.notification_settings.hide = hide_presence;
                if let Err(error) = self.notification_settings.save() {
                    self.error = error.to_string();
                }
                return;
            }
            _ if !self.actions_enabled() => return,
            _ => {}
        }
        let session = self.session.clone().filter(|session| !session.token.is_empty());
        if matches!(&action, SignIn) && session.is_none() {
            let session = self.session.clone().unwrap_or_else(|| Session {
                device: secret(), token: String::new(), player_id: None,
            });
            self.start(ctx, move |client| {
                credential(Some(&session))?;
                sign_in(client, session)
            });
            return;
        }
        let Some(session) = session else { return };
        if self.account_settings_session.as_ref() != Some(&session.token) {
            self.account_settings_session = Some(session.token.clone());
            self.account_settings = None;
            self.account_settings_requested = false;
        }
        match action {
            LoadSettings if self.account_settings.is_none() => {
                self.account_settings_requested = true;
                self.account_settings_request(ctx, session, None);
            }
            Presence { value } if matches!(value.as_str(), "friends" | "offline") => {
                self.account_settings_request(ctx, session, Some(value));
            }
            Add { code } if code.len() == 8 && code.bytes().all(|byte| byte.is_ascii_alphanumeric()) => {
                self.start(ctx, move |client| {
                    request(&client, &session, reqwest::Method::POST, "friends", None,
                        Some(json!({"operation": "send", "friendCode": code.to_ascii_uppercase()})))?;
                    Ok(Update::FriendRequestSent)
                });
            }
            Friend { id, operation } => {
                let Some(player) = self.snapshot.as_ref().and_then(|snapshot| snapshot.friends.iter().find(|player| player.player_id == id)) else { return };
                let valid = match operation.as_str() {
                    "remove" => player.status == "accepted",
                    "block" => matches!(player.status.as_str(), "accepted" | "pending"),
                    "accept" | "deny" => player.status == "pending" && player.direction == "incoming",
                    "unblock" => player.status == "blocked",
                    _ => false,
                };
                if valid {
                    self.start(ctx, move |client| {
                        request(&client, &session, reqwest::Method::POST, "friends", None,
                            Some(json!({"peer": id, "operation": operation})))?;
                        if operation == "remove" { Ok(Update::FriendRemoved(id)) } else { Ok(Update::Modified) }
                    });
                }
            }
            Ranks { id, index } => {
                let Some(link) = self.snapshot.as_ref().and_then(|snapshot| snapshot.friends.iter()
                    .find(|player| player.player_id == id && player.status == "accepted"))
                    .and_then(|player| player.links.get(index)).cloned() else { return };
                let key = format!("{}:{}", link.platform, link.platform_id);
                if !self.friend_ranks.contains_key(&key) {
                    self.start(ctx, move |_| {
                        let stats = hebnix_sdk::tracker::TrackerClient::default()
                            .fetch_profile(&link.platform, &link.platform_id)
                            .unwrap_or_else(|error| hebnix_sdk::tracker::PlayerStats { error: Some(error), ..Default::default() });
                        Ok(Update::FriendRanks(key, stats))
                    });
                }
            }
            _ => {}
        }
    }

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
        let mut presence = if !rocket_league_running {
            self.clear_match_presence();
            json!({"state": "offline"})
        } else {
            self.match_presence
                .clone()
                .unwrap_or_else(|| json!({"state": "main_menu"}))
        };
        if rocket_league_running {
            if self
                .next_presence_account_check
                .is_none_or(|next| next <= now)
            {
                self.presence_account = local_player_link();
                self.next_presence_account_check = Some(now + Duration::from_secs(5));
            }
            if let Some(account) = &self.presence_account {
                presence["account"] = account.clone();
            }
        } else {
            self.presence_account = None;
            self.next_presence_account_check = None;
        }
        if let Ok(mut current) = self.live_presence.lock() {
            *current = presence.clone();
        }
        self.notification_settings.load();
        self.poll_match_users(ctx, toasts, now);
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
                Ok(Update::GuestPresence) => {}
                Ok(Update::AccountSettings(settings)) => self.account_settings = Some(settings),
                Ok(Update::FriendRanks(key, ranks)) => {
                    self.friend_ranks.insert(key, ranks);
                }
                Ok(Update::FriendRequestSent) => {
                    self.friend_code_input.clear();
                    self.error.clear();
                    self.next_poll = None;
                    toasts.push(
                        "hebnix-network",
                        "Network",
                        "Friend request sent",
                        Default::default(),
                    );
                }
                Ok(Update::Modified) => self.next_poll = None,
                Ok(Update::FriendRemoved(peer)) => {
                    if let Some(snapshot) = &mut self.snapshot {
                        snapshot.friends.retain(|friend| friend.player_id != peer);
                    }
                    self.next_poll = None;
                }
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
                    if error.status == 401 {
                        if let Some(session) = &mut self.session {
                            session.token.clear();
                            if let Err(error) = credential(Some(session)) {
                                self.error = error;
                            }
                        }
                        self.snapshot = None;
                        self.last_online.clear();
                        self.first_directory = None;
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
        self.notify_online(&snapshot.friends, toasts, now);
        self.snapshot = Some(snapshot);
    }

    fn notify_online(
        &mut self,
        friends: &[Player],
        toasts: &mut crate::toast::ToastCenter,
        now: Instant,
    ) {
        let first = *self.first_directory.get_or_insert(now);
        for player in friends
            .iter()
            .filter(|p| p.status == "accepted" && p.online())
        {
            let last = self
                .last_online
                .insert(player.player_id.clone(), now)
                .unwrap_or(first);
            if now.duration_since(last) > Duration::from_secs(600)
                && !self.notification_settings.hide
            {
                toasts.push(
                    "hebnix-network",
                    "Network",
                    &format!("{} is Online", player.username),
                    Default::default(),
                );
            }
        }
        self.last_online
            .retain(|_, last| now.duration_since(*last) < Duration::from_secs(86400));
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
        let child_popup_open = egui::Popup::is_any_open(ui.ctx());
        let popup = egui::Popup::from_response(&button)
            .open_bool(&mut open)
            .align(egui::RectAlign::BOTTOM_END)
            .align_alternatives(&[])
            .gap(4.0)
            .width(width)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
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
                let size = egui::vec2(width, crate::ui::toasts::toolbar_popup_body_height(ui));
                ui.allocate_ui_with_layout(size, *ui.layout(), |ui| {
                    ui.set_height(size.y);
                    egui::ScrollArea::vertical()
                        .id_salt("network_popup_body")
                        .max_height(size.y)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.render(ui));
                });
            });
        if self.open
            && !button.clicked()
            && !child_popup_open
            && !egui::Popup::is_any_open(ui.ctx())
            && popup.is_some_and(|popup| popup.response.clicked_elsewhere())
        {
            open = false;
        }
        self.open = open;
    }

    fn account_settings_request(
        &mut self,
        ctx: &egui::Context,
        session: Session,
        visibility: Option<String>,
    ) {
        self.start(ctx, move |client| {
            let method = if visibility.is_some() {
                reqwest::Method::PUT
            } else {
                reqwest::Method::GET
            };
            let body = visibility.map(|value| json!({"presenceVisibility": value}));
            let settings = request(&client, &session, method, "settings", None, body)?;
            serde_json::from_value(settings)
                .map(Update::AccountSettings)
                .map_err(|_| "Invalid Network settings".to_owned().into())
        });
    }

    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        let signed_in = self
            .session
            .clone()
            .filter(|session| !session.token.is_empty());
        let account_session = signed_in.as_ref().map(|session| &session.token);
        if self.account_settings_session.as_ref() != account_session {
            self.account_settings_session = account_session.cloned();
            self.account_settings = None;
            self.account_settings_requested = false;
            self.friend_code_input.clear();
        }
        if signed_in.is_none() && self.view == View::AddFriend {
            self.view = View::Friends;
        }

        if !self.error.is_empty() {
            ui.label(&self.error);
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view, View::Friends, "Friends");
            ui.selectable_value(&mut self.view, View::Pending, "Pending");
            if signed_in.is_some() {
                ui.selectable_value(&mut self.view, View::AddFriend, "Add Friend");
            }
            ui.selectable_value(&mut self.view, View::Settings, "Settings");
        });
        ui.separator();
        if self.view == View::AddFriend || self.view == View::Settings {
            if let Some(session) = signed_in.as_ref() {
                if self.account_settings.is_none()
                    && self.actions_enabled()
                    && (!self.account_settings_requested
                        || ui.button("Retry loading Network settings").clicked())
                {
                    self.account_settings_requested = true;
                    self.account_settings_request(ui.ctx(), session.clone(), None);
                }
            }
        }
        if self.view == View::AddFriend {
            let Some(session) = signed_in else {
                return;
            };
            ui.label("Enter Friend Code:");
            let edited =
                ui.add(egui::TextEdit::singleline(&mut self.friend_code_input).char_limit(8));
            if edited.changed() {
                self.friend_code_input
                    .retain(|ch| ch.is_ascii_alphanumeric());
                self.friend_code_input.make_ascii_uppercase();
            }
            let enabled = self.friend_code_input.len() == 8 && self.actions_enabled();
            if ui
                .add_enabled(enabled, egui::Button::new("Send friend request"))
                .clicked()
            {
                let code = self.friend_code_input.clone();
                self.start(ui.ctx(), move |client| {
                    request(
                        &client,
                        &session,
                        reqwest::Method::POST,
                        "friends",
                        None,
                        Some(json!({"operation": "send", "friendCode": code})),
                    )?;
                    Ok(Update::FriendRequestSent)
                });
            }
            ui.label("Your Friend Code");
            if let Some(settings) = &self.account_settings {
                ui.add(egui::TextEdit::singleline(
                    &mut settings.friend_code.as_str(),
                ));
            }
            return;
        }
        if self.view == View::Settings {
            if let (Some(session), Some(settings)) =
                (signed_in.as_ref(), self.account_settings.as_ref())
            {
                let mut selected = if settings.presence_visibility == "offline" {
                    "offline"
                } else {
                    "friends"
                };
                let label = match selected {
                    "offline" => "Offline",
                    _ => "Online for Friends",
                };
                ui.horizontal(|ui| {
                    ui.label("Presence");
                    ui.add_enabled_ui(self.actions_enabled(), |ui| {
                        egui::ComboBox::from_id_salt("network_presence_visibility")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for (value, label) in
                                    [("friends", "Online for Friends"), ("offline", "Offline")]
                                {
                                    ui.selectable_value(&mut selected, value, label);
                                }
                            });
                    });
                });
                if selected != settings.presence_visibility
                    && settings.presence_visibility != "online"
                {
                    let value = selected.to_owned();
                    self.account_settings_request(ui.ctx(), session.clone(), Some(value));
                }
            }
            let changed = ui
                .checkbox(
                    &mut self.notification_settings.hide_match_users,
                    "Don’t Notify About Hebnix Users In Game",
                )
                .changed();
            let hidden = ui
                .checkbox(
                    &mut self.notification_settings.hide,
                    "Hide All Presence Notifications",
                )
                .changed();
            if changed || hidden {
                if let Err(error) = self.notification_settings.save() {
                    self.error = error.to_string();
                }
            }
            return;
        }
        let Some(session) = self
            .session
            .clone()
            .filter(|session| !session.token.is_empty())
        else {
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
        let mut rank_lookup = None;
        let enabled = self.actions_enabled();
        ui.scope(|ui| {
            ui.add_enabled_ui(enabled, |ui| match self.view {
                View::Friends => {
                    for player in snapshot.friends.iter().filter(|p| p.status == "accepted") {
                        let color = if player.online() {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::LIGHT_GRAY
                        };
                        let title = format!("{} - {}", player.username, player.presence_label());
                        egui::CollapsingHeader::new(egui::RichText::new(title).color(color))
                            .id_salt(&player.player_id)
                            .show(ui, |ui| {
                                for link in &player.links {
                                    let key = format!("{}:{}", link.platform, link.platform_id);
                                    if link.display_name.is_empty() {
                                        ui.label(&link.platform);
                                    } else {
                                        ui.label(format!(
                                            "{} ({})",
                                            link.display_name, link.platform
                                        ));
                                    }
                                    if let Some(stats) = self.friend_ranks.get(&key) {
                                        if let Some(error) = &stats.error {
                                            ui.label(error);
                                        } else if stats.ranks.is_empty() {
                                            ui.label("No ranked playlists");
                                        } else {
                                            let mut entries: Vec<_> =
                                                stats.ranks.values().collect();
                                            entries.sort_by_key(|rank| rank.playlist_id);
                                            let selector = ui.id().with((&key, "rank_playlist"));
                                            let mut selected = ui
                                                .data_mut(|data| data.get_temp::<i64>(selector))
                                                .unwrap_or(entries[0].playlist_id);
                                            selected = entries
                                                .iter()
                                                .find(|rank| rank.playlist_id == selected)
                                                .unwrap_or(&entries[0])
                                                .playlist_id;
                                            egui::ComboBox::from_id_salt(&key)
                                                .selected_text(
                                                    &stats.ranks[&selected].playlist_name,
                                                )
                                                .show_ui(ui, |ui| {
                                                    for rank in entries {
                                                        ui.selectable_value(
                                                            &mut selected,
                                                            rank.playlist_id,
                                                            &rank.playlist_name,
                                                        );
                                                    }
                                                });
                                            ui.data_mut(|data| {
                                                data.insert_temp(selector, selected)
                                            });
                                            let rank = &stats.ranks[&selected];
                                            ui.label(format!(
                                                "{} {}",
                                                rank.tier_name, rank.division_name
                                            ));
                                        }
                                    } else {
                                        ui.label("Loading ranks...");
                                        if enabled && rank_lookup.is_none() {
                                            rank_lookup = Some((key, link.clone()));
                                        }
                                    }
                                }
                                if self.confirm_remove.as_deref() == Some(player.player_id.as_str())
                                {
                                    ui.horizontal(|ui| {
                                        ui.label("Remove this friend?");
                                        if ui.button("Remove").clicked() {
                                            operation = Some((player.player_id.clone(), "remove"));
                                            self.confirm_remove = None;
                                        }
                                        if ui.button("Cancel").clicked() {
                                            self.confirm_remove = None;
                                        }
                                    });
                                } else if ui.button("Remove Friend").clicked() {
                                    self.confirm_remove = Some(player.player_id.clone());
                                    self.confirm_block = None;
                                }
                                if self.confirm_block.as_deref() == Some(player.player_id.as_str())
                                {
                                    ui.horizontal(|ui| {
                                        ui.label("Are you sure?");
                                        if ui.button("Yes").clicked() {
                                            operation = Some((player.player_id.clone(), "block"));
                                            self.confirm_block = None;
                                        }
                                        if ui.button("No").clicked() {
                                            self.confirm_block = None;
                                        }
                                    });
                                } else if ui.button("Block").clicked() {
                                    self.confirm_block = Some(player.player_id.clone());
                                    self.confirm_remove = None;
                                }
                            });
                    }
                }
                View::Pending => {
                    egui::CollapsingHeader::new("Blocked players").show(ui, |ui| {
                        for player in snapshot.friends.iter().filter(|p| p.status == "blocked") {
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
                View::Settings | View::AddFriend => {}
            });
        });
        if let Some((key, link)) = rank_lookup {
            self.start(ui.ctx(), move |_| {
                let stats = hebnix_sdk::tracker::TrackerClient::default()
                    .fetch_profile(&link.platform, &link.platform_id)
                    .unwrap_or_else(|error| hebnix_sdk::tracker::PlayerStats {
                        error: Some(error),
                        ..Default::default()
                    });
                Ok(Update::FriendRanks(key, stats))
            });
        } else if let Some((peer, operation)) = operation {
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
                if operation == "remove" {
                    Ok(Update::FriendRemoved(peer))
                } else {
                    Ok(Update::Modified)
                }
            });
        }
    }
}

#[cfg(windows)]
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WebAction {
    SignIn,
    LoadSettings,
    Presence { value: String },
    Notifications { hide_match_users: bool, hide_presence: bool },
    Add { code: String },
    Friend { id: String, operation: String },
    Ranks { id: String, index: usize },
}
#[derive(Deserialize)]
struct LiveFrame {
    #[serde(rename = "type")]
    kind: String,
    friends: Option<Vec<Player>>,
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
    use tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};
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
                    if signed_in && replace && frame.friends.is_none() {
                        continue;
                    }
                    self.live_healthy = true;
                    self.live_failures = 0;
                    if !signed_in {
                        continue;
                    }
                    let friends = frame
                        .friends
                        .or_else(|| self.snapshot.take().map(|snapshot| snapshot.friends))
                        .unwrap_or_default();
                    self.apply_snapshot(
                        Snapshot {
                            friends,
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
                                self.last_online.clear();
                                self.first_directory = None;
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

    pub(super) fn update_match_players(
        &mut self,
        players: &[hebnix_sdk::stats::models::PlayerState],
    ) {
        self.match_players = players
            .iter()
            .filter_map(|player| {
                let (platform, id, _) = hebnix_sdk::utils::parse_primary_id(&player.primary_id)?;
                let platform = match platform.as_str() {
                    "steam" => "Steam",
                    "epic" | "epicgames" => "Epic",
                    "psn" | "ps4" | "ps5" => "PS4",
                    "xbl" | "xbox" | "xboxone" => "XboxOne",
                    "switch" => "Switch",
                    _ => return None,
                };
                (!player.name.trim().is_empty() && !id.is_empty() && id != "0")
                    .then(|| (platform.to_owned(), id, player.name.clone()))
            })
            .take(20)
            .collect();
    }

    fn poll_match_users(
        &mut self,
        ctx: &egui::Context,
        toasts: &mut crate::toast::ToastCenter,
        now: Instant,
    ) {
        if self.notification_settings.hide_match_users || self.match_players.is_empty() {
            self.match_lookup = None;
            return;
        }
        if let Some(receiver) = &self.match_lookup {
            match receiver.try_recv() {
                Ok(players) => {
                    self.match_lookup = None;
                    for (platform, id, name) in players {
                        if self
                            .match_players
                            .iter()
                            .any(|(p, i, _)| p == &platform && i == &id)
                            && self.match_notified.insert((platform, id))
                        {
                            toasts.push(
                                "hebnix-network",
                                "Network",
                                &format!("{name} is using Hebnix"),
                                Default::default(),
                            );
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => self.match_lookup = None,
            }
        }
        if self.next_match_lookup.is_some_and(|next| next > now) {
            return;
        }
        let local = self
            .presence_account
            .as_ref()
            .and_then(|account| account["platformId"].as_str());
        let players: Vec<_> = self
            .match_players
            .iter()
            .filter(|(platform, id, _)| {
                local.is_some_and(|local| local != id)
                    && !self
                        .match_notified
                        .contains(&(platform.clone(), id.clone()))
            })
            .cloned()
            .collect();
        if players.is_empty() {
            return;
        }
        self.next_match_lookup = Some(now + Duration::from_secs(6));
        let session = self
            .session
            .clone()
            .filter(|session| !session.token.is_empty());
        let (tx, rx) = mpsc::channel();
        self.match_lookup = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let payload: Vec<_> = players
                .iter()
                .map(|(platform, id, _)| json!({"platform": platform, "platformId": id}))
                .collect();
            let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
            else {
                return;
            };
            let mut request = client
                .post(format!("{REQUEST}/match-players"))
                .json(&json!({"players": payload}));
            if let Some(session) = session {
                request = request
                    .bearer_auth(session.token)
                    .header("X-Hebnix-Device", session.device);
            }
            let matched = request
                .send()
                .ok()
                .and_then(|resp| resp.error_for_status().ok())
                .and_then(|resp| resp.json::<Value>().ok());
            if let Some(rows) = matched.as_ref().and_then(|data| data["players"].as_array()) {
                let _ = tx.send(
                    players
                        .into_iter()
                        .filter(|(platform, id, _)| {
                            rows.iter().any(|row| {
                                row["platform"].as_str() == Some(platform)
                                    && row["platformId"].as_str() == Some(id)
                            })
                        })
                        .collect(),
                );
            }
            ctx.request_repaint();
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
        self.match_players.clear();
        self.match_notified.clear();
        self.match_lookup = None;
        self.next_match_lookup = None;
    }
}
