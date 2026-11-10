use crate::{
    i18n::{format, layout::bordered_panel, t, t_args},
    profile_pictures::{accounts, download_profile_avatar},
    ui::browser,
};
use crossbeam_channel::{Receiver, TryRecvError, bounded};
use eframe::egui;
use hebnix_sdk::tracker::{PlayerStats, PlaylistRank, TrackerClient};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use time::OffsetDateTime;

const CHECK_INTERVAL: Duration = Duration::from_secs(3 * 60 * 60);
const AVATAR_RETRY_INTERVAL: Duration = Duration::from_secs(60);
const IDENTITY_INTERVAL: Duration = Duration::from_secs(15);
const HISTORY_DAYS: usize = 10;
const PROFILE_RETENTION_DAYS: i64 = 15;
const CASUAL_PLAYLIST_ID: i64 = 0;

const TIERS: [egui::ImageSource<'static>; 23] = [
    egui::include_image!("../../assets/ingame-rank/Tiers/0.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/1.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/2.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/3.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/4.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/5.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/6.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/7.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/8.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/9.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/10.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/11.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/12.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/13.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/14.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/15.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/16.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/17.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/18.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/19.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/20.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/21.png"),
    egui::include_image!("../../assets/ingame-rank/Tiers/22.png"),
];
const DIVISIONS: [egui::ImageSource<'static>; 8] = [
    egui::include_image!("../../assets/ingame-rank/Divisions/0.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/1.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/2.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/3.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/4.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/5.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/6.png"),
    egui::include_image!("../../assets/ingame-rank/Divisions/7.png"),
];

#[derive(Deserialize, Serialize)]
struct Change {
    at: i64,
    day: i32,
    mmr: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_mmr: Option<i64>,
}

#[derive(Default, Deserialize, Serialize)]
struct Profile {
    name: String,
    checked: i64,
    playlists: BTreeMap<i64, Playlist>,
    #[serde(skip)]
    avatar: Option<egui::TextureHandle>,
}

#[derive(Deserialize, Serialize)]
struct Playlist {
    rank: PlaylistRank,
    seen: i64,
    changes: Vec<Change>,
}

impl Playlist {
    fn trim_history(&mut self) -> bool {
        let excess = self.changes.len().saturating_sub(HISTORY_DAYS);
        self.changes.drain(..excess);
        excess > 0
    }

    fn mmr_change(&self) -> Option<i64> {
        let first = self.changes.first()?;
        let baseline = first.previous_mmr.or_else(|| {
            (self.changes.len() > 1).then_some(first.mmr)
        })?;
        self.rank.mmr.map(|mmr| mmr.saturating_sub(baseline))
    }
}

impl Profile {
    fn update(&mut self, stats: PlayerStats, now: OffsetDateTime) {
        self.name = stats.display_name;
        self.checked = now.unix_timestamp();
        for (id, rank) in stats.ranks {
            let playlist = self.playlists.entry(id).or_insert_with(|| Playlist {
                rank: PlaylistRank::default(),
                seen: self.checked,
                changes: Vec::new(),
            });
            if let (Some(previous), Some(mmr)) = (playlist.rank.mmr, rank.mmr)
                && previous != mmr
            {
                let day = now.date().to_julian_day();
                let previous_mmr = playlist
                    .changes
                    .iter()
                    .find(|point| point.day == day)
                    .and_then(|point| point.previous_mmr)
                    .unwrap_or(previous);
                playlist.changes.retain(|point| point.day != day);
                playlist.changes.push(Change {
                    at: self.checked,
                    day,
                    mmr,
                    previous_mmr: Some(previous_mmr),
                });
                playlist.changes.sort_by_key(|point| point.day);
                playlist.trim_history();
            }
            playlist.seen = self.checked;
            playlist.rank = rank;
        }
    }
}

type Profiles = BTreeMap<String, Profile>;

fn prune(profiles: &mut Profiles, now: OffsetDateTime) -> bool {
    let cutoff = (now - time::Duration::days(PROFILE_RETENTION_DAYS)).unix_timestamp();
    let before = profiles.len();
    let mut changed = false;
    profiles.retain(|_, profile| {
        let count = profile.playlists.len();
        profile
            .playlists
            .retain(|_, playlist| playlist.seen > cutoff);
        changed |= profile.playlists.len() != count;
        for playlist in profile.playlists.values_mut() {
            changed |= playlist.trim_history();
        }
        profile.checked > cutoff
    });
    changed || before != profiles.len()
}

type FetchedProfile = (PlayerStats, Result<Option<egui::ColorImage>, String>);
type PendingFetch = (String, CheckKind, Receiver<Result<FetchedProfile, String>>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckKind {
    Ratings,
    Avatar,
}

struct CheckSchedule {
    ratings_at: Instant,
    avatar_at: Option<Instant>,
}

impl CheckSchedule {
    fn next(&self) -> Instant {
        self.avatar_at.unwrap_or(self.ratings_at).min(self.ratings_at)
    }

    fn take_due(
        checks: &mut BTreeMap<String, Self>,
        key: &str,
        now: Instant,
    ) -> Option<CheckKind> {
        match checks.get_mut(key) {
            Some(check) if check.avatar_at.is_some_and(|due| due <= now) => {
                check.avatar_at = None;
                Some(CheckKind::Avatar)
            }
            Some(_) => None,
            None => {
                checks.insert(
                    key.to_owned(),
                    Self {
                        ratings_at: now + CHECK_INTERVAL,
                        avatar_at: None,
                    },
                );
                Some(CheckKind::Ratings)
            }
        }
    }

    fn record_avatar_result(&mut self, kind: CheckKind, available: bool, now: Instant) {
        if available {
            self.avatar_at = None;
        } else if kind == CheckKind::Ratings {
            self.avatar_at = Some(now + AVATAR_RETRY_INTERVAL);
        }
    }
}

pub(super) struct Tracker {
    path: PathBuf,
    profiles: Profiles,
    dirty: bool,
    next_cleanup: Instant,
    next_save: Instant,
    running: Option<bool>,
    account: Option<accounts::Account>,
    identity: Option<Receiver<(bool, Option<accounts::Account>)>>,
    next_identity: Instant,
    fetch: Option<PendingFetch>,
    requested: BTreeMap<String, CheckSchedule>,
    error: bool,
    selected: Option<i64>,
}

impl Tracker {
    pub(super) fn new(base: &Path) -> Self {
        let path = base.join("tracker-history.json");
        let profiles = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            profiles,
            dirty: false,
            next_cleanup: Instant::now(),
            next_save: Instant::now(),
            running: None,
            account: None,
            identity: None,
            next_identity: Instant::now(),
            fetch: None,
            requested: BTreeMap::new(),
            error: false,
            selected: None,
        }
    }

    pub(super) fn set_running(&mut self, running: bool) {
        if self.running != Some(running) {
            self.running = Some(running);
            self.next_identity = Instant::now();
        }
    }

    pub(super) fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if now >= self.next_cleanup {
            self.next_cleanup = now + Duration::from_secs(60);
            self.dirty |= prune(&mut self.profiles, local_now());
        }
        let Some(running) = self.running else {
            return;
        };

        if let Some(rx) = &self.identity {
            match rx.try_recv() {
                Ok((was_running, account)) => {
                    self.identity = None;
                    if was_running == running {
                        if self.account.as_ref().map(|a| &a.key) != account.as_ref().map(|a| &a.key)
                        {
                            self.selected = None;
                            self.error = false;
                        }
                        self.account = account;
                    } else {
                        self.next_identity = now;
                    }
                }
                Err(TryRecvError::Disconnected) => self.identity = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.identity.is_none() && now >= self.next_identity {
            self.next_identity = now
                + if running || self.account.is_none() {
                    IDENTITY_INTERVAL
                } else {
                    CHECK_INTERVAL
                };
            let (tx, rx) = bounded(1);
            self.identity = Some(rx);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let account = if running {
                    accounts::current()
                } else {
                    accounts::latest_saved()
                };
                let _ = tx.send((running, account));
                ctx.request_repaint();
            });
        }

        if let Some((key, kind, rx)) = &self.fetch {
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err("Tracker worker stopped".into())),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let key = key.clone();
                let kind = *kind;
                self.fetch = None;
                if kind == CheckKind::Ratings
                    && self
                    .account
                    .as_ref()
                    .is_some_and(|account| account.key == key)
                {
                    self.error = result.is_err();
                }
                match result {
                    Ok((stats, avatar)) => {
                        if let Some(check) = self.requested.get_mut(&key) {
                            check.record_avatar_result(
                                kind,
                                avatar.as_ref().is_ok_and(Option::is_some),
                                now,
                            );
                        }
                        if kind == CheckKind::Ratings {
                            self.profiles
                                .entry(key.clone())
                                .or_default()
                                .update(stats, local_now());
                            self.dirty = true;
                        }
                        match (self.profiles.get_mut(&key), avatar) {
                            (Some(profile), Ok(avatar)) => {
                                profile.avatar = avatar.map(|image| {
                                    ctx.load_texture(
                                        "tracker-avatar",
                                        image,
                                        egui::TextureOptions::LINEAR,
                                    )
                                });
                            }
                            (_, Err(error)) => tracing::warn!(%error, "Home tracker avatar download failed"),
                            (None, Ok(_)) => {}
                        }
                    }
                    Err(error) if kind == CheckKind::Ratings => {
                        tracing::warn!(%error, "Home tracker refresh failed");
                    }
                    Err(error) => tracing::warn!(%error, "Home tracker avatar lookup failed"),
                }
            }
        }

        self.requested.retain(|_, check| check.ratings_at > now);
        if self.fetch.is_none()
            && self.identity.is_none()
            && let Some(account) = &self.account
            && let Some(kind) = CheckSchedule::take_due(&mut self.requested, &account.key, now)
        {
            if kind == CheckKind::Ratings {
                self.error = false;
            }
            let (tx, rx) = bounded(1);
            self.fetch = Some((account.key.clone(), kind, rx));
            let account = account.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = TrackerClient::default()
                    .fetch_profile(&account.platform, &account.id)
                    .and_then(|mut profile| match profile.error.take() {
                        Some(error) => Err(error),
                        None => {
                            let avatar = profile_avatar(profile.avatar_url.as_deref());
                            Ok((profile, avatar))
                        }
                    });
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }

        if self.dirty && now >= self.next_save {
            let result = (|| {
                let bytes = serde_json::to_vec(&self.profiles)?;
                let tmp = self.path.with_extension("json.tmp");
                fs::write(&tmp, bytes)?;
                fs::rename(tmp, &self.path)
            })();
            self.dirty = result.is_err();
            if let Err(error) = result {
                self.next_save = now + Duration::from_secs(60);
                tracing::warn!(%error, "Could not save Home tracker history");
            }
        }
        let due = self
            .account
            .as_ref()
            .and_then(|account| self.requested.get(&account.key))
            .map(CheckSchedule::next);
        let next = due
            .unwrap_or(self.next_identity)
            .min(self.next_identity)
            .min(self.next_cleanup);
        if next > now {
            ctx.request_repaint_after(next - now);
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        let account = self.account.as_ref();
        let profile = account.and_then(|account| self.profiles.get(&account.key));
        let checking = self.fetch.as_ref().and_then(|(key, kind, _)| {
            account.filter(|account| account.key == *key).map(|_| *kind)
        });
        let playlists: Vec<_> = profile
            .into_iter()
            .flat_map(|profile| {
                profile.playlists.iter().filter(|(_, playlist)| playlist.seen == profile.checked)
            })
            .collect();
        let selected = playlists
            .iter()
            .find(|(id, _)| self.selected == Some(**id))
            .or_else(|| playlists.first());
        self.selected = selected.map(|(id, _)| **id);

        egui::ScrollArea::vertical()
            .id_salt("home_tracker")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                bordered_panel(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading(t("home-tracker"));
                        if let (Some(account), Some(profile)) = (account, profile)
                            && profile.avatar.is_none()
                            && checking.is_none()
                            && ui.small_button(t("home-tracker-avatar-retry")).clicked()
                        {
                            if let Some(check) = self.requested.get_mut(&account.key) {
                                check.avatar_at = Some(Instant::now());
                            }
                            ui.ctx().request_repaint();
                        }
                    });
                    ui.weak(t("home-tracker-history-window"));
                    if let Some(account) = account {
                        ui.add_space(4.0);
                        summary(ui, account, profile, selected.map(|(_, playlist)| *playlist));
                    } else {
                        ui.label(t("home-tracker-waiting"));
                    }
                    if self.error {
                        ui.label(t("home-tracker-error"));
                    }
                    if let Some(kind) = checking {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(t(match kind {
                                CheckKind::Ratings => "home-tracker-loading",
                                CheckKind::Avatar => "home-tracker-avatar-loading",
                            }));
                        });
                    }
                });

                let Some((_, playlist)) = selected else {
                    if profile.is_some() {
                        ui.label(t("home-tracker-empty"));
                    }
                    return;
                };
                let mut playlist = *playlist;
                ui.add_space(8.0);
                bordered_panel(ui, |ui| {
                    let columns = browser::columns(ui, 148.0);
                    for row in playlists.chunks(columns) {
                        ui.columns(columns, |cells| {
                            for (&(id, choice), cell) in row.iter().zip(cells) {
                                let rank = &choice.rank;
                                let badge = rank_badge(rank, 22.0);
                                let text = format!("{} · {}", rank.playlist_name, rating_value(rank));
                                let button = egui::Button::opt_image_and_text(badge, Some((&text).into()))
                                    .selected(self.selected == Some(*id))
                                    .truncate();
                                if cell
                                    .add_sized([cell.available_width(), 34.0], button)
                                    .on_hover_text(text)
                                    .clicked()
                                {
                                    self.selected = Some(*id);
                                    playlist = choice;
                                    cell.ctx().request_repaint();
                                }
                            }
                        });
                    }
                });
                ui.add_space(8.0);
                bordered_panel(ui, |ui| {
                    let rank = &playlist.rank;
                    ui.horizontal_wrapped(|ui| {
                        if let Some(badge) = rank_badge(rank, 48.0) {
                            ui.add(badge);
                        }
                        ui.vertical(|ui| {
                            ui.heading(&rank.playlist_name);
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(rating_text(rank));
                                if rank.playlist_id != CASUAL_PLAYLIST_ID {
                                    ui.label(rank_name(rank));
                                    if let Some(label) = division_icon(ui, rank) {
                                        ui.label(label);
                                    }
                                }
                            });
                        });
                    });
                    ui.separator();
                    ui.heading(t("home-tracker-history"));
                    if playlist.changes.is_empty() {
                        ui.label(t("home-tracker-no-history"));
                    } else {
                        chart(ui, &playlist.changes);
                    }
                });
            });
    }
}

fn summary(
    ui: &mut egui::Ui,
    account: &accounts::Account,
    profile: Option<&Profile>,
    playlist: Option<&Playlist>,
) {
    let Some(playlist) = playlist else {
        player_card(ui, account, profile);
        return;
    };
    let current = |ui: &mut egui::Ui| {
        let label = t_args(
            "home-tracker-current",
            &[("mode", playlist.rank.playlist_name.as_str().into())],
        );
        summary_metric(ui, rating_text(&playlist.rank), label, Some(&playlist.rank));
    };
    let change = |ui: &mut egui::Ui| {
        let value = playlist.mmr_change().map_or_else(
            || "—".into(),
            |change| {
                let sign = if change > 0 { "+" } else { "" };
                format!("{sign}{}", format::int(change))
            },
        );
        summary_metric(ui, value, t("home-tracker-change"), None);
    };
    if ui.available_width() >= 540.0 {
        ui.columns(3, |cells| {
            player_card(&mut cells[0], account, profile);
            current(&mut cells[1]);
            change(&mut cells[2]);
        });
    } else {
        player_card(ui, account, profile);
        ui.columns(2, |cells| {
            current(&mut cells[0]);
            change(&mut cells[1]);
        });
    }
}

fn player_card(ui: &mut egui::Ui, account: &accounts::Account, profile: Option<&Profile>) {
    bordered_panel(ui, |ui| {
        ui.set_min_height(42.0);
        ui.horizontal(|ui| {
            let avatar = profile.and_then(|profile| profile.avatar.as_ref());
            let source = avatar.map_or_else(
                || egui::include_image!("../../assets/hebnix.png"),
                |texture| egui::ImageSource::Texture(texture.into()),
            );
            let picture = ui.add(
                egui::Image::new(source)
                    .fit_to_exact_size(egui::Vec2::splat(40.0))
                    .corner_radius(4.0),
            );
            if avatar.is_none() {
                picture.on_hover_text(t("home-tracker-avatar-unavailable"));
            }
            ui.vertical(|ui| {
                let name = profile
                    .map(|profile| profile.name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&account.name);
                ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate())
                    .on_hover_text(name);
                if let Some(profile) = profile
                    && let Ok(checked) = OffsetDateTime::from_unix_timestamp(profile.checked)
                {
                    let offset = time::UtcOffset::local_offset_at(checked)
                        .unwrap_or(time::UtcOffset::UTC);
                    let local = checked.to_offset(offset);
                    let text = format!(
                        "{} · {} {:02}:{:02}",
                        t("home-tracker-last-checked"),
                        local.date(),
                        local.hour(),
                        local.minute()
                    );
                    ui.add(egui::Label::new(egui::RichText::new(&text).weak()).truncate())
                        .on_hover_text(text);
                }
            });
        });
    });
}

fn summary_metric(ui: &mut egui::Ui, value: String, label: String, rank: Option<&PlaylistRank>) {
    bordered_panel(ui, |ui| {
        ui.set_min_height(42.0);
        ui.horizontal(|ui| {
            if let Some(rank) = rank
                && let Some(badge) = rank_badge(rank, 30.0)
            {
                ui.allocate_ui_with_layout(
                    egui::vec2(42.0, 42.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add(badge).on_hover_text(rank_name(rank));
                        division_icon(ui, rank);
                    },
                );
            }
            ui.vertical(|ui| {
                ui.heading(value);
                ui.add(egui::Label::new(egui::RichText::new(&label).weak()).truncate())
                    .on_hover_text(label);
            });
        });
    });
}

fn local_now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

fn profile_avatar(url: Option<&str>) -> Result<Option<egui::ColorImage>, String> {
    let Some(url) = url.map(str::trim).filter(|url| {
        !url.is_empty() && !url.eq_ignore_ascii_case("null")
    }) else {
        return Ok(None);
    };
    let jpeg = download_profile_avatar(url)?;
    let rgba = image::load_from_memory(&jpeg)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    Ok(Some(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )))
}

fn rank_badge(rank: &PlaylistRank, size: f32) -> Option<egui::Image<'static>> {
    if rank.playlist_id == CASUAL_PLAYLIST_ID {
        return None;
    }
    let image = egui::Image::new(
        TIERS
            .get(rank.tier_id as usize)
            .unwrap_or(&TIERS[0])
            .clone(),
    );
    Some(image.fit_to_exact_size(egui::Vec2::splat(size)))
}

fn rank_name(rank: &PlaylistRank) -> &'static str {
    hebnix_sdk::utils::ranks::get_tier_name(rank.tier_id as usize)
}

fn rating_text(rank: &PlaylistRank) -> String {
    format!("{} {}", rating_value(rank), t("home-tracker-mmr"))
}

fn rating_value(rank: &PlaylistRank) -> String {
    rank.mmr.map_or_else(|| "—".into(), format::int)
}

fn division_icon(ui: &mut egui::Ui, rank: &PlaylistRank) -> Option<String> {
    if !(1..22).contains(&rank.tier_id) || !(0..=3).contains(&rank.division_id) {
        return None;
    }
    let label = format!(
        "{} {}",
        t("home-tracker-division"),
        ["I", "II", "III", "IV"][rank.division_id as usize]
    );
    let color = hebnix_sdk::utils::ranks::get_div_color_id(rank.tier_id) as usize;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for pip in 0..4 {
            let index = if pip <= rank.division_id { color } else { 0 };
            ui.add(
                egui::Image::new(DIVISIONS[index].clone()).fit_to_exact_size(egui::vec2(9.0, 9.0)),
            );
        }
    })
    .response
    .on_hover_text(&label);
    Some(label)
}

fn chart(ui: &mut egui::Ui, history: &[Change]) {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), 220.0),
        egui::Sense::hover(),
    );
    let bounds = response.rect;
    let plot = egui::Rect::from_min_max(
        bounds.left_top() + egui::vec2(46.0, 12.0),
        bounds.right_bottom() - egui::vec2(23.0, 25.0),
    );
    if plot.width() <= 0.0 {
        return;
    }
    let min = history.iter().map(|point| point.mmr).min().unwrap_or(0) as f64;
    let max = history.iter().map(|point| point.mmr).max().unwrap_or(0) as f64;
    let padding = ((max - min) * 0.12).ceil().max(3.0);
    let low = min - padding;
    let high = max + padding;
    let first = history[0].day;
    let span = history[history.len() - 1].day - first;
    let text = ui.visuals().text_color();
    let accent = ui.visuals().selection.stroke.color;
    let font = egui::TextStyle::Small.resolve(ui.style());
    for fraction in [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
        let y = egui::lerp(plot.y_range(), fraction);
        let mmr = high - (high - low) * f64::from(fraction);
        painter.hline(
            plot.x_range(),
            y,
            ui.visuals().widgets.noninteractive.bg_stroke,
        );
        painter.text(
            egui::pos2(plot.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            format::number(mmr, 0),
            font.clone(),
            text,
        );
    }
    let mut points = Vec::with_capacity(history.len());
    let mut label_x = f32::NEG_INFINITY;
    for (index, change) in history.iter().enumerate() {
        let x_fraction = if span == 0 {
            0.5
        } else {
            (change.day - first) as f32 / span as f32
        };
        let position = egui::pos2(
            egui::lerp(plot.x_range(), x_fraction),
            egui::lerp(
                plot.y_range(),
                ((high - change.mmr as f64) / (high - low)) as f32,
            ),
        );
        points.push(position);
        let last = index + 1 == history.len();
        if let Ok(day) = time::Date::from_julian_day(change.day)
            && (last || (position.x - label_x >= 46.0 && plot.right() - position.x >= 46.0))
        {
            painter.text(
                egui::pos2(position.x, plot.bottom() + 6.0),
                egui::Align2::CENTER_TOP,
                format!("{:02}/{:02}", day.day(), u8::from(day.month())),
                font.clone(),
                text,
            );
            label_x = position.x;
        }
        ui.interact(
            egui::Rect::from_center_size(position, egui::vec2(14.0, 14.0)),
            response.id.with(index),
            egui::Sense::hover(),
        )
        .on_hover_text(format!(
            "{} · {} {}",
            time::Date::from_julian_day(change.day)
                .map_or_else(|_| String::new(), |day| day.to_string()),
            format::int(change.mmr),
            t("home-tracker-mmr")
        ));
    }
    if points.len() > 1 {
        let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
        let mut area = egui::Mesh::default();
        for &point in &points {
            area.colored_vertex(point, fill);
            area.colored_vertex(
                egui::pos2(point.x, plot.bottom()),
                egui::Color32::TRANSPARENT,
            );
        }
        for i in 0..points.len() as u32 - 1 {
            let top = i * 2;
            area.add_triangle(top, top + 1, top + 2);
            area.add_triangle(top + 1, top + 3, top + 2);
        }
        painter.add(egui::Shape::mesh(area));
        painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(2.0, accent)));
    }
    if let Some(&last) = points.last() {
        painter.circle_stroke(last, 6.0, egui::Stroke::new(1.0, accent));
    }
    for point in points {
        painter.circle_filled(point, 3.5, accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_the_last_change_per_day_and_skips_flat_checks() {
        let first = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let stats = |mmr| PlayerStats {
            ranks: [(
                11,
                PlaylistRank {
                    playlist_id: 11,
                    mmr: Some(mmr),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let mut profiles = Profiles::default();
        let key = "steam:fixture".to_string();
        let profile = profiles.entry(key.clone()).or_default();
        profile.update(stats(800), first);
        assert!(profile.playlists[&11].changes.is_empty());
        profile.update(stats(805), first + time::Duration::minutes(5));
        profile.update(stats(810), first + time::Duration::minutes(15));
        let second = first + time::Duration::days(1);
        profile.update(stats(810), second);
        assert_eq!(profile.playlists[&11].changes.len(), 1);
        assert_eq!(profile.playlists[&11].changes[0].mmr, 810);
        assert_eq!(profile.playlists[&11].mmr_change(), Some(10));

        profile.update(stats(820), second);
        assert_eq!(profile.playlists[&11].changes.len(), 2);
        let later = second + time::Duration::days(PROFILE_RETENTION_DAYS);
        profile.update(stats(820), later - time::Duration::hours(3));
        assert!(!prune(&mut profiles, later));
        let playlist = &profiles[&key].playlists[&11];
        assert_eq!(playlist.changes.len(), 2);
        assert_eq!(playlist.mmr_change(), Some(20));
        assert!(prune(
            &mut profiles,
            later + time::Duration::days(PROFILE_RETENTION_DAYS)
        ));
        assert!(profiles.is_empty());
    }

    #[test]
    fn history_keeps_the_latest_ten_dates_with_changes_even_with_gaps() {
        let first = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let mut profile = Profile::default();
        for step in 0..=12 {
            let rank = PlaylistRank {
                playlist_id: 11,
                mmr: Some(800 + step * 10),
                ..Default::default()
            };
            profile.update(
                PlayerStats {
                    ranks: [(11, rank)].into(),
                    ..Default::default()
                },
                first + time::Duration::days(step * 2),
            );
        }
        let playlist = &profile.playlists[&11];
        assert_eq!(playlist.changes.len(), HISTORY_DAYS);
        assert_eq!(playlist.changes[0].mmr, 830);
        assert_eq!(playlist.changes[9].mmr, 920);
        assert_eq!(playlist.mmr_change(), Some(100));
    }

    #[test]
    fn casual_omits_the_badge_but_unranked_competitive_keeps_it() {
        let mut rank = PlaylistRank {
            playlist_id: CASUAL_PLAYLIST_ID,
            ..Default::default()
        };
        assert!(rank_badge(&rank, 30.0).is_none());
        rank.playlist_id = 11;
        assert!(rank_badge(&rank, 30.0).is_some());
    }

    #[test]
    fn missing_req_avatar_is_distinct_from_a_download_error() {
        for url in [None, Some(""), Some(" null ")] {
            assert!(profile_avatar(url).unwrap().is_none());
        }
        assert!(profile_avatar(Some("http://example.com/avatar.png")).is_err());
    }

    #[test]
    fn missing_avatar_retries_once_before_the_next_ratings_check() {
        let now = Instant::now();
        let key = "steam:fixture";
        let mut checks = BTreeMap::new();
        assert_eq!(
            CheckSchedule::take_due(&mut checks, key, now),
            Some(CheckKind::Ratings)
        );
        checks.get_mut(key).unwrap().record_avatar_result(CheckKind::Ratings, false, now);
        let retry_at = now + AVATAR_RETRY_INTERVAL;
        assert_eq!(checks[key].next(), retry_at);
        assert_eq!(CheckSchedule::take_due(&mut checks, key, now), None);
        assert_eq!(
            CheckSchedule::take_due(&mut checks, key, retry_at),
            Some(CheckKind::Avatar)
        );
        checks.get_mut(key).unwrap().record_avatar_result(CheckKind::Avatar, false, retry_at);
        assert_eq!(checks[key].next(), now + CHECK_INTERVAL);
        assert_eq!(CheckSchedule::take_due(&mut checks, key, retry_at + AVATAR_RETRY_INTERVAL), None);
    }

    #[test]
    fn avatar_retry_updates_the_picture_without_updating_the_ratings_snapshot() {
        let now = Instant::now();
        let account = accounts::Account::from_key("steam:76561198000000000").unwrap();
        let key = account.key.clone();
        let (tx, rx) = bounded(1);
        let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
        tx.send(Ok((PlayerStats::default(), Ok(Some(image))))).unwrap();
        let mut tracker = Tracker {
            profiles: [(key.clone(), Profile { checked: 123, ..Default::default() })].into(),
            account: Some(account),
            running: Some(false),
            fetch: Some((key.clone(), CheckKind::Avatar, rx)),
            requested: [(key.clone(), CheckSchedule {
                ratings_at: now + CHECK_INTERVAL,
                avatar_at: None,
            })].into(),
            next_identity: now + CHECK_INTERVAL,
            next_cleanup: now + CHECK_INTERVAL,
            ..Tracker::new(Path::new(""))
        };
        tracker.tick(&egui::Context::default());
        assert!(tracker.profiles[&key].avatar.is_some());
        assert_eq!(tracker.profiles[&key].checked, 123);
        assert!(!tracker.dirty);
    }

    #[test]
    fn selecting_a_mode_updates_the_summary_at_narrow_and_wide_widths() {
        fn find_text(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find_text(shape, label)),
                _ => None,
            }
        }

        for width in [430.0, 1_200.0] {
            let ctx = egui::Context::default();
            let account = accounts::Account::from_key("steam:76561198000000000").unwrap();
            let mut profile = Profile::default();
            let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
            for (days, casual, doubles) in [(0, 1580, 800), (1, 1590, 823), (2, 1596, 834)] {
                let ranks = [(0, "Casual", casual), (11, "Doubles", doubles)]
                    .into_iter()
                    .map(|(id, name, mmr)| {
                        (
                            id,
                            PlaylistRank {
                                playlist_id: id,
                                playlist_name: name.into(),
                                mmr: Some(mmr),
                                tier_id: if id == CASUAL_PLAYLIST_ID { 0 } else { 13 },
                                division_id: 2,
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                profile.update(
                    PlayerStats {
                        display_name: "Test Player".into(),
                        ranks,
                        ..Default::default()
                    },
                    now + time::Duration::days(days),
                );
            }
            let mut tracker = Tracker {
                profiles: [(account.key.clone(), profile)].into(),
                account: Some(account),
                ..Tracker::new(Path::new(""))
            };
            let render = |tracker: &mut Tracker, events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| tracker.show(ui));
                    },
                )
            };
            let position = |frame: &egui::FullOutput, label: &str| {
                frame.shapes.iter().find_map(|shape| find_text(&shape.shape, label))
            };
            render(&mut tracker, vec![]);
            let initial = render(&mut tracker, vec![]);
            assert!(position(&initial, "Test Player").is_some());
            assert!(position(&initial, &t("home-tracker-avatar-retry")).is_some());
            assert!(position(&initial, "+16").is_some());
            assert!(position(&initial, "Unranked").is_none());
            let pos = position(&initial, "Doubles · 834").unwrap();
            for pressed in [true, false] {
                render(
                    &mut tracker,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    }],
                );
            }
            let updated = render(&mut tracker, vec![]);
            assert_eq!(tracker.selected, Some(11));
            assert!(position(&updated, "+34").is_some());
            assert!(position(&updated, &format!("834 {}", t("home-tracker-mmr"))).is_some());
            assert!(position(&updated, "Diamond I").is_some());
        }
    }
}
