//! The game reads short per-match rating files. Network access stays on Hebnix
//! workers, and only the UI thread publishes results for the active match.
use crate::{
    config::{ScoreboardCfg, ScoreboardMmrSource},
    i18n::{layout::bordered_panel, t},
    patcher::profile_pictures::scoreboard as patch,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use hebnix_sdk::{
    log::LogGameInfo,
    stats::UpdateStateData,
    tracker::{PlaylistRank, TrackerClient},
};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Reply {
    guid: String,
    id: String,
    ratings: Result<(String, Vec<PlaylistRank>), String>,
}

pub(crate) struct State {
    game: PathBuf,
    enabled: bool,
    show_mmr: bool,
    unanonymize: bool,
    operation: Option<Receiver<Result<(patch::Options, bool), String>>>,
    legacy: bool,
    upgrade_pending: bool,
    local_id: Option<String>,
    match_info: LogGameInfo,
    context_retry: Option<Instant>,
    status: String,
    guid: String,
    requested: HashMap<String, Option<Instant>>,
    files: Vec<PathBuf>,
    tracker: Arc<TrackerClient>,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            game: PathBuf::new(),
            enabled: false,
            show_mmr: false,
            unanonymize: false,
            operation: None,
            legacy: false,
            upgrade_pending: false,
            local_id: None,
            match_info: LogGameInfo::default(),
            context_retry: None,
            status: String::new(),
            guid: String::new(),
            requested: HashMap::new(),
            files: Vec::new(),
            tracker: Arc::new(TrackerClient::default()),
            tx,
            rx,
        }
    }
}
fn valid_guid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_{}".contains(&b))
}
fn valid_id(id: &str) -> bool {
    let mut parts = id.split('|');
    matches!(
        parts.next().map(str::to_ascii_lowercase).as_deref(),
        Some(
            "steam"
                | "epic"
                | "epicgames"
                | "ps4"
                | "ps5"
                | "psn"
                | "playstation"
                | "switch"
                | "xbox"
                | "xbl"
                | "xboxone"
                | "nintendo"
        )
    ) && parts.next().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.~".contains(&b))
    }) && parts.next().is_some_and(|s| s.parse::<u8>().is_ok())
        && parts.next().is_none()
}
const DOUBLES: i64 = 11;
const TOURNAMENT: i64 = 34;
const CASUAL: i64 = 0;

// The selector is available before the first tracker response. Requests
// supply each player's rating for the selected playlist ID.
const MANUAL_PLAYLISTS: &[(i64, &str)] = &[
    (CASUAL, "Casual"),
    (10, "Ranked 1v1"),
    (DOUBLES, "Ranked 2v2"),
    (13, "Ranked 3v3"),
    (61, "Ranked Quads"),
    (27, "Ranked Hoops"),
    (28, "Ranked Rumble"),
    (29, "Ranked Dropshot"),
    (30, "Ranked Snow Day"),
    (63, "Ranked Heatseeker"),
    (TOURNAMENT, "Tournaments"),
];

fn tournament(game: &LogGameInfo) -> bool {
    matches!(game.playlist_id, Some(22 | TOURNAMENT))
        || game
            .game_tags
            .as_deref()
            .is_some_and(|tags| tags.to_ascii_lowercase().contains("tournament"))
}

fn competitive_playlist(game: &LogGameInfo) -> Option<i64> {
    let playlist = game.playlist_id;
    let tournament = tournament(game);
    if !tournament {
        match playlist {
            Some(1..=4) => return playlist.map(|id| [10, 11, 13, 61][id as usize - 1]),
            Some(10 | 11 | 13 | 27..=30 | 61 | 63) => return playlist,
            Some(15) => return Some(30),
            Some(17) => return Some(27),
            Some(18 | 25) => return Some(28),
            Some(23) => return Some(29),
            Some(38 | 43) => return Some(63),
            _ => {}
        }
    }
    let mode = format!(
        "{} {} {}",
        game.game_class.as_deref().unwrap_or_default(),
        game.game_tags.as_deref().unwrap_or_default(),
        game.playlist_name.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    for (names, id) in [
        (&["heatseeker", "godball"][..], 63),
        (&["basketball", "hoops"], 27),
        (&["breakout", "dropshot"], 29),
        (&["hockey", "snowday", "snow day"], 30),
        (&["rumble", "items"], 28),
    ] {
        if names.iter().any(|name| mode.contains(name)) {
            return Some(id);
        }
    }
    if mode.contains("soccar") || (playlist == Some(CASUAL) && mode.trim().is_empty()) {
        return match game.player_count {
            Some(2) => Some(10),
            Some(4) => Some(11),
            Some(6) => Some(13),
            Some(8) => Some(61),
            _ => None,
        };
    }
    None
}

fn should_publish_name(preferences: &ScoreboardCfg, local_id: Option<&str>, id: &str) -> bool {
    // Account IDs match across split-screen suffixes and platform casing.
    let account = |id: &str| {
        let (platform, tail) = id.split_once('|')?;
        let (account, _) = tail.rsplit_once('|')?;
        let platform = platform.to_ascii_lowercase();
        let platform = if platform == "epicgames" {
            "epic"
        } else {
            &platform
        };
        Some(format!("{platform}|{}", account.to_ascii_lowercase()))
    };
    !preferences.ignore_self || local_id.is_some_and(|local| account(local) != account(id))
}

fn rating_files(
    preferences: &ScoreboardCfg,
    game: &LogGameInfo,
    ratings: &BTreeMap<i64, i64>,
    legacy: bool,
) -> BTreeMap<String, String> {
    let rating = |id| ratings.get(&id).copied().map(|mmr| (id, mmr));
    let chosen = match preferences.mmr_source {
        ScoreboardMmrSource::Playlist => None,
        ScoreboardMmrSource::Gamemode => {
            competitive_playlist(game).and_then(rating).or_else(|| {
                rating(if tournament(game) {
                    TOURNAMENT
                } else {
                    DOUBLES
                })
            })
        }
        ScoreboardMmrSource::Manual => preferences.fixed_playlist.and_then(rating),
    };
    let format = |(_, mmr)| format!("[{mmr}] ");
    let mut files = BTreeMap::new();
    let is_tournament = tournament(game);
    if preferences.mmr_source == ScoreboardMmrSource::Playlist {
        if is_tournament {
            for id in [22, TOURNAMENT] {
                if let Some(exact) = rating(id).or_else(|| rating(TOURNAMENT)) {
                    files.insert(id.to_string(), format(exact));
                }
            }
        } else if game.offline || game.playlist_id == Some(6) {
            if let Some(exact) = rating(6) {
                files.insert("6".into(), format(exact));
            }
        } else {
            for (&id, &mmr) in ratings {
                // The old hook used 11 as a fallback for every mode. Only
                // publish it when it is known to be the actual playlist.
                if !legacy || id != DOUBLES || game.playlist_id == Some(DOUBLES) {
                    files.insert(id.to_string(), format((id, mmr)));
                }
            }
            if let Some(mmr) = ratings.get(&CASUAL) {
                for id in 1..=4 {
                    files
                        .entry(id.to_string())
                        .or_insert_with(|| format((CASUAL, *mmr)));
                }
            }
        }
    }
    let slot = if legacy { "11" } else { "selected" };
    // Some tournament sessions have no game playlist object, but the match
    // log still identifies the tournament bucket.
    if preferences.mmr_source == ScoreboardMmrSource::Playlist && is_tournament {
        if let Some(exact) = rating(TOURNAMENT) {
            files.entry(slot.into()).or_insert_with(|| format(exact));
        }
    }
    if let Some((_, mmr)) = chosen {
        files
            .entry(slot.into())
            .or_insert_with(|| format!("[{mmr}] "));
    }
    files
}

impl State {
    pub(crate) fn busy(&self) -> bool {
        self.operation.is_some()
    }

    pub(crate) fn end_match(&mut self) {
        self.guid.clear();
        self.local_id = None;
        self.match_info = LogGameInfo::default();
        self.context_retry = None;
        self.clear_player_files();
    }

    fn clear_player_files(&mut self) {
        self.requested.clear();
        for path in self.files.drain(..) {
            let _ = fs::remove_file(path);
        }
    }
    pub(crate) fn update(
        &mut self,
        guid: Option<&str>,
        state: &UpdateStateData,
        preferences: &ScoreboardCfg,
    ) {
        let Some(guid) = guid.filter(|s| valid_guid(s)) else {
            return;
        };
        if !self.enabled {
            return;
        }
        if self.guid != guid {
            self.end_match();
            self.guid = guid.to_owned();
        }
        let needs_context = self.context_retry.is_none()
            || ((self.unanonymize && preferences.ignore_self && self.local_id.is_none())
                || (self.show_mmr && self.match_info.game_class.is_none()))
                && self
                    .context_retry
                    .is_some_and(|retry| Instant::now() >= retry);
        if needs_context {
            self.context_retry = Some(Instant::now() + Duration::from_secs(2));
            if let Some((id, mut info)) = hebnix_sdk::log::local_match_context() {
                let new_identity = id.is_some() && id != self.local_id;
                if id.is_some() {
                    self.local_id = id;
                }
                if info.player_count.is_none() {
                    let team_size = (0..2)
                        .map(|team| state.players.iter().filter(|p| p.team_num == team).count())
                        .max();
                    info.player_count = team_size
                        .filter(|size| *size > 0)
                        .map(|size| size as i64 * 2);
                }
                if new_identity
                    || info.playlist_id != self.match_info.playlist_id
                    || info.offline != self.match_info.offline
                    || tournament(&info) != tournament(&self.match_info)
                    || competitive_playlist(&info) != competitive_playlist(&self.match_info)
                {
                    self.clear_player_files();
                }
                self.match_info = info;
            }
        }
        for player in &state.players {
            let id = &player.primary_id;
            if !valid_id(id)
                || (!self.show_mmr
                    && !should_publish_name(preferences, self.local_id.as_deref(), id))
                || self
                    .requested
                    .get(id)
                    .is_some_and(|next| next.is_none_or(|at| Instant::now() < at))
            {
                continue;
            }
            self.requested.insert(id.clone(), None);
            let tracker = self.tracker.clone();
            let tx = self.tx.clone();
            let guid = guid.to_owned();
            let id = id.clone();

            thread::spawn(move || {
                let ratings = tracker.fetch(&id, "").and_then(|profile| {
                    if profile.not_found {
                        return Ok((String::new(), Vec::new()));
                    }
                    if let Some(error) = profile.error {
                        return Err(error);
                    }
                    Ok((profile.display_name, profile.ranks.into_values().collect()))
                });
                let _ = tx.send(Reply { guid, id, ratings });
            });
        }
    }
    fn publish(&mut self, reply: Reply, preferences: &ScoreboardCfg) -> Result<(), String> {
        if reply.guid != self.guid || !self.enabled {
            return Ok(());
        }
        let (name, ratings) = match reply.ratings {
            Ok(v) => v,
            Err(_) => {
                self.requested
                    .insert(reply.id, Some(Instant::now() + Duration::from_secs(60)));
                return Ok(());
            }
        };
        let dir = self.game.join("TAGame/HebnixScoreboard");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if self.unanonymize
            && should_publish_name(preferences, self.local_id.as_deref(), &reply.id)
            && !name.trim().is_empty()
            && !name.chars().any(char::is_control)
        {
            let file = dir.join(format!(
                "m_{}_{}_name.mmr",
                reply.guid,
                reply.id.replace('|', "_")
            ));
            let tmp = file.with_extension("tmp");
            // UE reads these files as ANSI or BOM-marked UTF-16.
            let mut content = vec![0xff, 0xfe];
            for unit in name.encode_utf16() {
                content.extend_from_slice(&unit.to_le_bytes());
            }
            fs::write(&tmp, content)
                .and_then(|_| fs::rename(&tmp, &file))
                .map_err(|e| e.to_string())?;
            self.files.push(file);
        }
        if !self.show_mmr {
            return Ok(());
        }
        let ratings = ratings
            .into_iter()
            .filter_map(|rank| Some((rank.playlist_id, rank.mmr?)))
            .collect();
        for (playlist, value) in rating_files(preferences, &self.match_info, &ratings, self.legacy)
        {
            let file = dir.join(format!(
                "m_{}_{}_{}.mmr",
                reply.guid,
                reply.id.replace('|', "_"),
                playlist
            ));
            let tmp = file.with_extension("tmp");
            fs::write(&tmp, value)
                .and_then(|_| fs::rename(&tmp, &file))
                .map_err(|e| e.to_string())?;
            self.files.push(file);
        }
        Ok(())
    }
    fn run(&mut self, target: Option<patch::Options>, ctx: &egui::Context) {
        let (tx, rx) = unbounded();
        self.operation = Some(rx);
        let game = self.game.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = match target {
                Some(options) => patch::set_options(&game, options).map(|()| (options, false)),
                None => patch::installed_with_version(&game),
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub(crate) fn tick(
        &mut self,
        ctx: &egui::Context,
        game: &Path,
        preferences: &ScoreboardCfg,
        game_running: bool,
    ) {
        if self.game != game && self.operation.is_none() {
            self.end_match();
            self.game = game.to_owned();
            self.enabled = false;
            self.show_mmr = false;
            self.unanonymize = false;
            self.legacy = false;
            self.upgrade_pending = false;
            self.status.clear();
            if !game.as_os_str().is_empty() {
                self.run(None, ctx);
            }
        }
        if let Some(result) = self.operation.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Some(Err("Scoreboard patch operation stopped.".into()))
            }
            Err(crossbeam_channel::TryRecvError::Empty) => None,
        }) {
            self.operation = None;
            match result {
                Ok((options, legacy)) => {
                    self.legacy = legacy;
                    self.upgrade_pending = legacy;
                    self.show_mmr = options.show_mmr;
                    self.unanonymize = options.unanonymize;
                    self.enabled = self.show_mmr || self.unanonymize;
                    self.status.clear();
                }
                Err(e) => self.status = e,
            }
            ctx.request_repaint();
        }
        if self.upgrade_pending
            && self.operation.is_none()
            && !game_running
            && !hebnix_sdk::process::is_rocket_league_running()
        {
            self.upgrade_pending = false;
            self.run(
                Some(patch::Options {
                    show_mmr: self.show_mmr,
                    unanonymize: self.unanonymize,
                }),
                ctx,
            );
        }
        while let Ok(reply) = self.rx.try_recv() {
            if let Err(e) = self.publish(reply, preferences) {
                self.status = e;
            }
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, preferences: &mut ScoreboardCfg) -> bool {
        let before = preferences.clone();
        let ready = self.operation.is_none() && !self.game.as_os_str().is_empty();
        let mut names = self.unanonymize;
        let mut mmr = self.show_mmr;
        bordered_panel(ui, |ui| {
            ui.heading(t("scoreboard-unanonymize"));
            ui.add_enabled(
                ready,
                egui::Checkbox::new(&mut names, t("scoreboard-enable-unanonymize")),
            );
            ui.checkbox(&mut preferences.ignore_self, t("scoreboard-ignore-self"));
        });
        ui.add_space(8.0);
        bordered_panel(ui, |ui| {
            ui.heading(t("scoreboard-show-mmr"));
            ui.add_enabled(
                ready,
                egui::Checkbox::new(&mut mmr, t("scoreboard-enable-mmr")),
            );
            ui.radio_value(
                &mut preferences.mmr_source,
                ScoreboardMmrSource::Playlist,
                t("scoreboard-current-playlist"),
            );
            ui.radio_value(
                &mut preferences.mmr_source,
                ScoreboardMmrSource::Gamemode,
                t("scoreboard-current-gamemode"),
            );
            ui.radio_value(
                &mut preferences.mmr_source,
                ScoreboardMmrSource::Manual,
                t("scoreboard-manual-selection"),
            );
            if preferences.mmr_source == ScoreboardMmrSource::Manual {
                let selected = MANUAL_PLAYLISTS
                    .iter()
                    .find(|(id, _)| Some(*id) == preferences.fixed_playlist)
                    .map_or_else(|| t("scoreboard-choose-mode"), |(_, label)| (*label).into());
                egui::ComboBox::from_id_salt("scoreboard_fixed_playlist")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for &(id, label) in MANUAL_PLAYLISTS {
                            ui.selectable_value(&mut preferences.fixed_playlist, Some(id), label);
                        }
                    });
            }
        });
        let preferences_changed = *preferences != before;
        let patch_changed = mmr != self.show_mmr || names != self.unanonymize;
        if preferences_changed || patch_changed {
            self.end_match();
        }
        if patch_changed {
            self.run(
                Some(patch::Options {
                    show_mmr: mmr,
                    unanonymize: names,
                }),
                ui.ctx(),
            );
        }
        if self.operation.is_some() {
            ui.spinner();
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        preferences_changed
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.end_match();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: i64, class: &str, players: i64) -> LogGameInfo {
        LogGameInfo {
            playlist_id: Some(id),
            game_class: Some(class.into()),
            player_count: Some(players),
            ..Default::default()
        }
    }

    #[test]
    fn current_playlist_uses_casual_and_ranked_ratings_without_doubles_fallback() {
        let preferences = ScoreboardCfg::default();
        let ratings = BTreeMap::from([(0, 800), (10, 1000), (11, 1200), (27, 1400)]);
        for legacy in [false, true] {
            let files = rating_files(&preferences, &game(2, "Soccar", 4), &ratings, legacy);
            assert_eq!(files["2"], "[800] ");
            assert_eq!(files["10"], "[1000] ");
            assert_eq!(files["27"], "[1400] ");
            assert!(!files.contains_key("selected"));
            assert_eq!(files.contains_key("11"), !legacy);
        }
        let files = rating_files(&preferences, &game(11, "Soccar", 4), &ratings, true);
        assert_eq!(files["11"], "[1200] ");
        let files = rating_files(&preferences, &game(6, "Soccar", 4), &ratings, false);
        assert!(files.is_empty());
    }

    #[test]
    fn gamemode_resolves_casual_private_competitive_and_tournament_matches() {
        let preferences = ScoreboardCfg {
            mmr_source: ScoreboardMmrSource::Gamemode,
            ..Default::default()
        };
        let ratings = BTreeMap::from([
            (10, 1000),
            (11, 1200),
            (13, 1300),
            (27, 1400),
            (34, 1500),
            (63, 1600),
        ]);
        for (game, expected) in [
            (game(1, "Soccar", 2), "[1000] "),
            (game(6, "Soccar", 6), "[1300] "),
            (game(27, "Basketball", 4), "[1400] "),
            (game(34, "GodBall", 4), "[1600] "),
            (game(34, "Soccar", 4), "[1200] "),
            (game(34, "UnknownPentathlonMode", 4), "[1500] "),
            (game(34, "Breakout", 6), "[1500] "),
            (game(6, "Unknown", 4), "[1200] "),
        ] {
            let files = rating_files(&preferences, &game, &ratings, false);
            assert_eq!(
                files,
                BTreeMap::from([("selected".into(), expected.into())])
            );
        }
        let mut pentathlon = game(34, "Pentathlon", 4);
        pentathlon.game_tags = Some("Heatseeker".into());
        assert_eq!(
            rating_files(&preferences, &pentathlon, &ratings, false)["selected"],
            "[1600] "
        );
    }

    #[test]
    fn tournament_playlist_gamemode_and_manual_select_one_rating_each() {
        let mut preferences = ScoreboardCfg::default();
        let ratings = BTreeMap::from([(0, 800), (34, 1500), (63, 1600)]);
        let game = game(34, "GodBall", 4);
        let files = rating_files(&preferences, &game, &ratings, false);
        assert_eq!(files["34"], "[1500] ");
        assert_eq!(files["22"], "[1500] ");
        assert!(!files.contains_key("0"));

        preferences.mmr_source = ScoreboardMmrSource::Gamemode;
        assert_eq!(
            rating_files(&preferences, &game, &ratings, false),
            BTreeMap::from([("selected".into(), "[1600] ".into())])
        );

        preferences.mmr_source = ScoreboardMmrSource::Manual;
        preferences.fixed_playlist = Some(0);
        assert_eq!(
            rating_files(&preferences, &game, &ratings, false),
            BTreeMap::from([("selected".into(), "[800] ".into())])
        );
    }

    #[test]
    fn ignore_self_applies_to_names_only_and_is_on_by_default() {
        let mut preferences = ScoreboardCfg::default();
        let local = Some("Epic|local-id|0");
        assert!(!should_publish_name(
            &preferences,
            local,
            "EpicGames|LOCAL-id|1"
        ));
        assert!(should_publish_name(
            &preferences,
            local,
            "Epic|another-id|0"
        ));
        assert!(!should_publish_name(&preferences, None, "Epic|local-id|0"));
        assert_eq!(
            rating_files(
                &preferences,
                &game(11, "Soccar", 4),
                &BTreeMap::from([(11, 1200)]),
                false
            )["11"],
            "[1200] "
        );
        preferences.ignore_self = false;
        assert!(should_publish_name(&preferences, local, "Epic|local-id|0"));
        assert!(should_publish_name(&preferences, None, "Epic|local-id|0"));
    }

    #[test]
    fn publishes_mmr_for_self_while_leaving_the_local_name_anonymous() {
        let game =
            std::env::temp_dir().join(format!("hebnix-scoreboard-{:016x}", rand::random::<u64>()));
        let mut state = State::default();
        state.game = game.clone();
        state.enabled = true;
        state.show_mmr = true;
        state.unanonymize = true;
        state.guid = "match".into();
        state.local_id = Some("Epic|local|0".into());
        state.match_info = self::game(2, "Soccar", 4);
        for id in ["local", "other"] {
            state
                .publish(
                    Reply {
                        guid: "match".into(),
                        id: format!("Epic|{id}|0"),
                        ratings: Ok((
                            id.into(),
                            vec![PlaylistRank {
                                playlist_id: 0,
                                playlist_name: "Casual".into(),
                                mmr: Some(800),
                                ..Default::default()
                            }],
                        )),
                    },
                    &ScoreboardCfg::default(),
                )
                .unwrap();
        }
        let dir = game.join("TAGame/HebnixScoreboard");
        assert!(!dir.join("m_match_Epic_local_0_name.mmr").exists());
        assert!(dir.join("m_match_Epic_other_0_name.mmr").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("m_match_Epic_local_0_2.mmr")).unwrap(),
            "[800] "
        );
        state.end_match();
        assert_eq!(fs::read_dir(dir).unwrap().count(), 0);
        fs::remove_dir_all(game).unwrap();
    }
}
