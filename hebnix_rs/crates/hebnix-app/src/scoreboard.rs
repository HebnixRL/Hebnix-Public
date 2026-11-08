//! The game reads short per-match rating files. Network access stays on Hebnix
//! workers, and only the UI thread publishes results for the active match.
use crate::{i18n::t, patcher::profile_pictures::scoreboard as patch};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use hebnix_sdk::{stats::UpdateStateData, tracker::TrackerClient};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Reply {
    guid: String,
    id: String,
    ratings: Result<(String, Vec<(i64, i64)>), String>,
}

pub(crate) struct State {
    game: PathBuf,
    enabled: bool,
    show_mmr: bool,
    unanonymize: bool,
    operation: Option<Receiver<Result<patch::Options, String>>>,
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
impl State {
    pub(crate) fn end_match(&mut self) {
        self.guid.clear();
        self.requested.clear();
        for path in self.files.drain(..) {
            let _ = fs::remove_file(path);
        }
    }
    pub(crate) fn update(&mut self, guid: Option<&str>, state: &UpdateStateData) {
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
        for player in &state.players {
            let id = &player.primary_id;
            if !valid_id(id)
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
                    Ok((
                        profile.display_name,
                        profile
                            .ranks
                            .into_values()
                            .filter_map(|r| Some((r.playlist_id, r.mmr?)))
                            .collect(),
                    ))
                });
                let _ = tx.send(Reply { guid, id, ratings });
            });
        }
    }
    fn publish(&mut self, reply: Reply) -> Result<(), String> {
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
        if self.unanonymize && !name.trim().is_empty() && !name.chars().any(char::is_control) {
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
        for (playlist, mmr) in ratings {
            let file = dir.join(format!(
                "m_{}_{}_{}.mmr",
                reply.guid,
                reply.id.replace('|', "_"),
                playlist
            ));
            let tmp = file.with_extension("tmp");
            fs::write(&tmp, format!("[{mmr}] "))
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
                Some(options) => patch::set_options(&game, options).map(|()| options),
                None => patch::installed(&game),
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub(crate) fn tick(&mut self, ctx: &egui::Context, game: &Path) {
        if self.game != game && self.operation.is_none() {
            self.end_match();
            self.game = game.to_owned();
            self.enabled = false;
            self.show_mmr = false;
            self.unanonymize = false;
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
                Ok(options) => {
                    self.show_mmr = options.show_mmr;
                    self.unanonymize = options.unanonymize;
                    self.enabled = self.show_mmr || self.unanonymize;
                    self.status.clear();
                }
                Err(e) => self.status = e,
            }
            ctx.request_repaint();
        }
        while let Ok(reply) = self.rx.try_recv() {
            if let Err(e) = self.publish(reply) {
                self.status = e;
            }
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading(t("nav-scoreboard"));
        let mut on = self.show_mmr;
        if ui
            .add_enabled(
                self.operation.is_none() && !self.game.as_os_str().is_empty(),
                egui::Checkbox::new(&mut on, t("scoreboard-show-mmr")),
            )
            .changed()
        {
            self.end_match();
            self.run(
                Some(patch::Options {
                    show_mmr: on,
                    unanonymize: self.unanonymize,
                }),
                ui.ctx(),
            );
        }
        let mut names = self.unanonymize;
        if ui
            .add_enabled(
                self.operation.is_none() && !self.game.as_os_str().is_empty(),
                egui::Checkbox::new(&mut names, "Unanonymize Players"),
            )
            .changed()
        {
            self.end_match();
            self.run(
                Some(patch::Options {
                    show_mmr: self.show_mmr,
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
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.end_match();
    }
}
