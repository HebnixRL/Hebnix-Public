use crate::replay_metadata::{self, ReplayMetadata};
use crate::replay_scan::{CacheKey, Marker, Scan};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use hebnix_sdk::stats::{StatsEvent, WsCommand, WsStatsClient};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashSet, VecDeque},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
enum Command {
    Pov { focus: String, perspective: String },
    Speed(f64),
    Hud(bool),
    Pause(f64),
    Seek(i64),
}
impl Command {
    fn label(&self) -> &'static str {
        match self {
            Self::Pov { .. } => "Change POV",
            Self::Speed(_) => "Set Game Speed",
            Self::Hud(_) => "Set HUD Visibility",
            Self::Pause(_) => "Pause",
            Self::Seek(_) => "Seek Replay",
        }
    }
    fn defaults() -> [Self; 5] {
        [
            Self::Pov {
                focus: "1".into(),
                perspective: "PlayerView".into(),
            },
            Self::Speed(1.0),
            Self::Hud(true),
            Self::Pause(2.0),
            Self::Seek(1),
        ]
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Keyframe {
    frame: i64,
    commands: Vec<Command>,
}
#[derive(Default, Serialize, Deserialize)]
struct Project {
    keys: Vec<Keyframe>,
}
enum Phase {
    Idle,
    Preparing { at: Instant, seek_sent: bool },
}

pub struct ReplayTimeline {
    folder: Option<PathBuf>,
    files: Vec<PathBuf>,
    chosen: Option<PathBuf>,
    metadata: Option<ReplayMetadata>,
    project_dir: PathBuf,
    keys: Vec<Keyframe>,
    markers: Vec<Marker>,
    players: Vec<(String, i64)>,
    frame: i64,
    selected: i64,
    selected_key: Option<i64>,
    full_replay: bool,
    live_match: bool,
    active: bool,
    phase: Phase,
    playing: bool,
    run_started: bool,
    resume_due: Option<Instant>,
    resume_check: Option<(Instant, i64, u8)>,
    speed: f64,
    reconnect_pending: bool,
    run_start: i64,
    fired: HashSet<i64>,
    pending: VecDeque<Command>,
    hold: Option<(Instant, f64)>,
    held_remaining: Option<f64>,
    zoom: f64,
    view_start: f64,
    status: String,
    keybind: String,
    hotkey: Option<crate::hotkey::ToggleHotkey>,
    hotkey_rx: crossbeam_channel::Receiver<()>,
    capture_rx: Option<crossbeam_channel::Receiver<Option<String>>>,
    capture_guard: Option<String>,
    seek_target: Option<(i64, Instant)>,
    scan: Option<Scan>,
    scan_failed: bool,
    last_world_check: Option<Instant>,
    pending_detach: Option<Instant>,
    needs_resume: bool,
    last_state_at: Option<Instant>,
    replay_guids: HashSet<String>,
    replay_identity: Option<(String, String)>,
    last_attach_check: Option<Instant>,
}
impl ReplayTimeline {
    pub fn new(base: &Path, ctx: &egui::Context) -> Self {
        let project_dir = base.join("replay-timelines");
        let mut keybind = std::fs::read_to_string(project_dir.join("play-pause-key.txt"))
            .unwrap_or_else(|_| "f8".into())
            .trim()
            .to_lowercase();
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut hotkey = crate::hotkey::ToggleHotkey::new();
        if let Some(h) = &mut hotkey {
            if !keybind.is_empty() && !h.rebind(&keybind) {
                keybind = "f8".into();
                h.rebind("f8");
            }
            let ctx = ctx.clone();
            h.listen(move || {
                let _ = tx.send(());
                ctx.request_repaint();
            });
        }
        Self {
            folder: None,
            files: vec![],
            chosen: None,
            metadata: None,
            project_dir,
            keys: vec![],
            markers: vec![],
            players: vec![],
            frame: 1,
            selected: 1,
            selected_key: None,
            full_replay: false,
            live_match: false,
            active: false,
            phase: Phase::Idle,
            playing: false,
            run_started: false,
            resume_due: None,
            resume_check: None,
            speed: 1.0,
            reconnect_pending: false,
            run_start: 1,
            fired: HashSet::new(),
            pending: VecDeque::new(),
            hold: None,
            held_remaining: None,
            zoom: 1.0,
            view_start: 0.0,
            status: "Open a replay in Rocket League to start.".into(),
            keybind,
            hotkey,
            hotkey_rx: rx,
            capture_rx: None,
            capture_guard: None,
            seek_target: None,
            scan: None,
            scan_failed: false,
            last_world_check: None,
            pending_detach: None,
            needs_resume: false,
            last_state_at: None,
            replay_guids: HashSet::new(),
            replay_identity: None,
            last_attach_check: None,
        }
    }
    fn send(&mut self, ws: &WsStatsClient, name: &str, data: serde_json::Value) -> bool {
        if !ws.connected() {
            self.status =
                "Stats API disconnected. Start Rocket League with Stats API enabled.".into();
            return false;
        }
        match WsCommand::with_data(name, data)
            .ok()
            .and_then(|c| ws.send_command(c).ok())
        {
            Some(_) => true,
            None => {
                self.status = format!("Could not send {name} to Rocket League.");
                false
            }
        }
    }
    fn max_frame(&self) -> i64 {
        self.metadata.as_ref().map_or(1, |m| m.frames)
    }
    fn fps(&self) -> f64 {
        self.metadata.as_ref().map_or(30.0, |m| m.fps)
    }
    fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Idle) || self.scan.is_some()
    }
    fn ready(&self, ws: &WsStatsClient) -> bool {
        self.full_replay && self.metadata.is_some() && ws.connected() && !self.busy()
    }
    fn project_path(&self) -> Option<PathBuf> {
        let path = self.chosen.as_ref()?.to_string_lossy();
        Some(
            self.project_dir
                .join(format!("{:08x}.json", crc32fast::hash(path.as_bytes()))),
        )
    }
    fn save(&mut self) {
        let Some(path) = self.project_path() else {
            return;
        };
        let result = (|| -> Result<(), String> {
            std::fs::create_dir_all(&self.project_dir).map_err(|e| e.to_string())?;
            let data = serde_json::to_vec_pretty(&Project {
                keys: self.keys.clone(),
            })
            .map_err(|e| e.to_string())?;
            let temp = path.with_extension("json.tmp");
            std::fs::write(&temp, data).map_err(|e| e.to_string())?;
            std::fs::rename(temp, path).map_err(|e| e.to_string())
        })();
        if let Err(e) = result {
            self.status = format!("Could not save timeline: {e}");
        }
    }
    fn select_file(&mut self, path: PathBuf) -> bool {
        match replay_metadata::read(&path) {
            Ok(meta) => {
                self.scan = None;
                self.scan_failed = false;
                self.chosen = Some(path);
                self.keys = self
                    .project_path()
                    .and_then(|p| std::fs::read(p).ok())
                    .and_then(|d| serde_json::from_slice::<Project>(&d).ok())
                    .map_or_else(Vec::new, |p| p.keys);
                self.keys.retain(|k| (1..=meta.frames).contains(&k.frame));
                self.keys.sort_by_key(|k| k.frame);
                self.keys.dedup_by_key(|k| k.frame);
                self.markers = meta
                    .goals
                    .iter()
                    .map(|(f, p, t)| Marker {
                        frame: *f,
                        event: "Goal".into(),
                        player: p.clone(),
                        team: *t,
                    })
                    .collect();
                self.metadata = Some(meta);
                self.selected_key = None;
                self.zoom = 1.0;
                self.view_start = 0.0;
                self.selected = 1;
                true
            }
            Err(e) => {
                self.status = format!("Cannot read replay: {e}");
                false
            }
        }
    }
    fn prepare(&mut self, ws: &WsStatsClient) {
        self.playing = false;
        self.run_started = false;
        self.resume_check = None;
        self.resume_due = None;
        self.speed = 1.0;
        self.pending.clear();
        self.hold = None;
        self.held_remaining = None;
        self.seek_target = None;
        self.send(ws, "SetMatchPaused", json!({"bPaused": true}));
        self.send(ws, "SetGameSpeed", json!({"Speed": 1.0}));
        self.phase = Phase::Preparing {
            at: Instant::now(),
            seek_sent: false,
        };
    }
    pub fn is_replay_event(&self, event: &StatsEvent) -> bool {
        self.full_replay
            || event.event_type == "ReplayCreated"
            || event
                .raw_data
                .get("MatchGuid")
                .and_then(|v| v.as_str())
                .is_some_and(|id| self.replay_guids.contains(id))
    }
    pub fn event(&mut self, event: &StatsEvent, _ws: &WsStatsClient) {
        if self.full_replay && event.event_type != "UpdateState" {
            if let Some(scan) = &mut self.scan {
                scan.observe(event);
            }
        }
        match event.event_type.as_str() {
            "ReplayCreated" => {
                let guid = event.raw_data["MatchGuid"].as_str().unwrap_or("");
                let file = event.raw_data["FileName"].as_str().unwrap_or("");
                let same = self.metadata.as_ref().is_some_and(|m| {
                    (!guid.is_empty() && m.ids.iter().any(|id| id.eq_ignore_ascii_case(guid)))
                        || self
                            .chosen
                            .as_ref()
                            .and_then(|p| p.file_stem())
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.eq_ignore_ascii_case(file))
                });
                if self.full_replay && same {
                    return;
                }
                if let Some(guid) = event.raw_data.get("MatchGuid").and_then(|v| v.as_str()) {
                    self.replay_guids.insert(guid.into());
                }
                self.full_replay = true;
                self.last_world_check = Some(Instant::now());
                self.live_match = false;
                if self.scan.take().is_some() {
                    self.send(_ws, "SetGameSpeed", json!({"Speed":1.0}));
                }
                self.metadata = None;
                self.chosen = None;
                self.playing = false;
                self.phase = Phase::Idle;
                self.hold = None;
                self.pending.clear();
                self.last_attach_check = None;
                self.replay_identity = Some((
                    event.raw_data["FileName"].as_str().unwrap_or("").into(),
                    event.raw_data["MatchGuid"].as_str().unwrap_or("").into(),
                ));
            }
            "MatchDestroyed" => {
                // Replay seeks may rebuild the replay world. Confirm the map changed before detaching.
                if self.full_replay {
                    self.pending_detach = Some(Instant::now());
                } else {
                    self.detach();
                }
            }
            "UpdateState" => {
                let Some(state) = event.update_state() else {
                    return;
                };
                if !state.game.replay && !self.full_replay {
                    self.live_match = true;
                    self.full_replay = false;
                    self.playing = false;
                    self.phase = Phase::Idle;
                    self.metadata = None;
                    self.replay_identity = None;
                    return;
                }
                // bReplay is transient, including goal replay transitions. A confirmed full-replay
                // attachment remains valid until its map changes; it must not reinitialise on each tick.
                if !self.full_replay {
                    return;
                }
                if self.last_state_at.is_none() && matches!(current_replay_map(), Some(None)) {
                    self.detach();
                    return;
                }
                if event
                    .raw_data
                    .get("Game")
                    .and_then(|g| g.get("Frame"))
                    .and_then(|f| f.as_i64())
                    .is_none()
                {
                    return;
                }
                if let Some(scan) = &mut self.scan {
                    scan.observe(event);
                }
                // The exporter reports the next frame cursor: SeekReplay(N) produces Frame=N+1.
                // Keep timeline coordinates aligned with the actual replay-file frame indices.
                let frame = state
                    .game
                    .frame
                    .saturating_sub(1)
                    .clamp(1, self.max_frame());
                self.players = state
                    .players
                    .iter()
                    .map(|p| (p.name.clone(), p.shortcut))
                    .collect();
                self.last_state_at = Some(Instant::now());
                if let Some((target, sent)) = self.seek_target {
                    let tolerance = if self.playing {
                        ((self.fps() * self.speed.max(1.0) * 0.15).ceil() as i64).max(2)
                    } else {
                        0
                    };
                    if (frame - target).abs() <= tolerance
                        && sent.elapsed() >= Duration::from_millis(60)
                    {
                        self.seek_target = None;
                    } else if sent.elapsed() < Duration::from_millis(750) {
                        // Ignore in-flight packets from the previous position while seeking.
                        return;
                    }
                }
                self.frame = frame;
                if self.seek_target.is_none() && matches!(self.phase, Phase::Idle) {
                    self.selected = frame;
                }
            }
            "MatchPaused"
                if self.full_replay
                    && self.playing
                    && self.hold.is_none()
                    && self.held_remaining.is_none() =>
            {
                // A late pause/seek/speed acknowledgement must not reverse the user's Play request.
                self.needs_resume = true;
                self.resume_due = Some(Instant::now() + Duration::from_millis(80));
            }
            _ => {}
        }
    }
    pub fn tick(&mut self, ctx: &egui::Context, ws: &WsStatsClient, active: bool) {
        self.active = active;
        if !ws.connected() {
            if self.scan.is_some() {
                self.finish_scan(
                    ws,
                    false,
                    "Event scan interrupted by disconnection; no cache saved.",
                );
            }
            if self.playing || self.busy() {
                self.status =
                    "Stats API disconnected; timeline paused. Waiting for connection.".into();
            }
            self.reconnect_pending |= self.full_replay;
            self.phase = Phase::Idle;
            // Keep the file attachment across replay-world/socket resets. Reconnecting must
            // not re-run initialisation and seek to frame 1 again.
            self.last_state_at = None;
            if let Some((at, seconds)) = self.hold.take() {
                self.held_remaining = Some((seconds - at.elapsed().as_secs_f64()).max(0.0));
            }
            self.resume_check = None;
        }
        let check_world = self.full_replay
            && ws.connected()
            && (self
                .pending_detach
                .is_some_and(|at| at.elapsed() > Duration::from_millis(250))
                || self
                    .last_world_check
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(2)));
        if check_world {
            self.last_world_check = Some(Instant::now());
            self.pending_detach = None;
            match current_replay_map() {
                Some(None) => {
                    if self.scan.take().is_some() {
                        self.send(ws, "SetGameSpeed", json!({"Speed":1.0}));
                    }
                    self.detach();
                }
                Some(Some(path))
                    if self.chosen.as_ref().is_some_and(|p| {
                        !p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .eq_ignore_ascii_case(
                                &path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                    }) =>
                {
                    if self.scan.take().is_some() {
                        self.send(ws, "SetGameSpeed", json!({"Speed":1.0}));
                    }
                    self.detach();
                    self.full_replay = true;
                    self.last_attach_check = None;
                }
                _ => {}
            }
        }
        if self.reconnect_pending && self.ready(ws) {
            self.reconnect_pending = false;
            if let Some((target, _)) = self.seek_target {
                self.send(ws, "SetMatchPaused", json!({"bPaused":true}));
                self.send(ws, "SeekReplay", json!({"Frame":target}));
                self.seek_target = Some((target, Instant::now()));
            }
            self.send(ws, "SetGameSpeed", json!({"Speed":self.speed}));
            if self.playing {
                if let Some(seconds) = self.held_remaining.take() {
                    self.hold = Some((Instant::now(), seconds));
                }
                self.needs_resume = true;
                self.resume_due = Some(Instant::now() + Duration::from_millis(120));
            } else {
                self.send(ws, "SetMatchPaused", json!({"bPaused":true}));
            }
        }
        let capturing = self.capture_rx.is_some() || self.capture_guard.is_some();
        let pressed = self.hotkey_rx.try_iter().count() > 0;
        if self
            .capture_guard
            .as_deref()
            .is_some_and(|key| !hebnix_sdk::input::is_key_pressed(key))
        {
            self.capture_guard = None;
        }
        if let Some(rx) = &self.capture_rx {
            if let Ok(key) = rx.try_recv() {
                self.capture_rx = None;
                match key {
                    Some(key) if key == "esc" || key == "escape" => {
                        self.status = "Hotkey capture cancelled.".into()
                    }
                    Some(key) => {
                        self.capture_guard = Some(key.clone());
                        if self.hotkey.as_mut().is_some_and(|h| h.rebind(&key)) {
                            self.keybind = key;
                            self.save_keybind();
                        } else {
                            self.status = "That key is not supported. Choose another key.".into();
                        }
                    }
                    None => self.status = "Hotkey capture timed out; previous binding kept.".into(),
                }
            }
        }
        if pressed
            && !capturing
            && !self.keybind.is_empty()
            && self.ready(ws)
            && (hebnix_sdk::process::is_rocket_league_focused()
                || (crate::winutil::foreground_window_is_ours()
                    && !ctx.egui_wants_keyboard_input()))
        {
            self.toggle(ws);
        }
        let now = Instant::now();
        match self.phase {
            Phase::Preparing {
                at,
                seek_sent: false,
            } if at.elapsed() > Duration::from_millis(150) => {
                if let Some(scan) = &mut self.scan {
                    scan.armed = true;
                }
                self.send(ws, "SeekReplay", json!({"Frame": 1}));
                self.selected = 1;
                self.frame = 1;
                self.phase = Phase::Preparing {
                    at: now,
                    seek_sent: true,
                };
            }
            Phase::Preparing {
                at,
                seek_sent: true,
            } if (at.elapsed() > Duration::from_millis(200)
                && self.frame <= 2
                && self.last_state_at.is_some_and(|seen| seen >= at))
                || at.elapsed() > Duration::from_millis(750) =>
            {
                self.phase = Phase::Idle;
                self.selected = 1;
            }
            _ => {}
        }
        self.tick_scan(ws);
        if let Some((_, sent)) = self.seek_target {
            if self.playing && ws.connected() && sent.elapsed() > Duration::from_millis(750) {
                // SeekReplay has no command acknowledgement, and paused state packets can
                // omit the exact target. Finish settling without cancelling the Play intent.
                // Paused frame stepping keeps its target pinned until the exact frame arrives.
                self.seek_target = None;
            }
        }
        if self.playing && self.ready(ws) && self.seek_target.is_none() {
            if let Some((start, seconds)) = self.hold {
                if start.elapsed().as_secs_f64() >= seconds {
                    self.hold = None;
                }
            }
            if self.hold.is_none() {
                if self.pending.is_empty() {
                    let due = self
                        .keys
                        .iter()
                        .find(|k| {
                            k.frame >= self.run_start
                                && k.frame <= self.frame
                                && !self.fired.contains(&k.frame)
                        })
                        .cloned();
                    if let Some(key) = due {
                        self.fired.insert(key.frame);
                        if !key.commands.is_empty() {
                            self.pending.extend(key.commands);
                            // Execute once when the reported frame crosses the keyframe.
                            // Rewinding on every crossing interrupts speed changes and playback.
                        }
                    }
                }
                while let Some(command) = self.pending.pop_front() {
                    match command {
                        Command::Pause(seconds) => {
                            self.send(ws, "SetMatchPaused", json!({"bPaused": true}));
                            self.hold = Some((now, seconds.clamp(0.1, 3600.0)));
                            self.needs_resume = true;
                            break;
                        }
                        Command::Seek(target) => {
                            self.selected = target.clamp(1, self.max_frame());
                            self.frame = self.selected;
                            self.send(ws, "SeekReplay", json!({"Frame": self.selected}));
                            self.seek_target = Some((self.selected, now));
                            self.needs_resume = true;
                            self.resume_due = Some(now + Duration::from_millis(120));
                            break;
                        }
                        other => self.apply(&other, ws),
                    }
                }
                if self.playing && self.hold.is_none() && self.seek_target.is_none() {
                    if self.frame >= self.max_frame() - 1 {
                        self.stop(ws);
                        self.status = "Timeline finished.".into();
                    }
                }
            }
        }
        // Transport intent is independent of seek confirmation. In particular Play after
        // scrubbing must unpause once, without submitting another seek or waiting forever.
        if self.playing && self.ready(ws) && self.hold.is_none() && self.held_remaining.is_none() {
            if let Some((sent, frame, attempt)) = self.resume_check {
                if self.frame != frame {
                    self.resume_check = None;
                } else if sent.elapsed() > Duration::from_millis(350)
                    && self.seek_target.is_none()
                    && self.speed > 0.0
                    && attempt < 3
                {
                    self.needs_resume = true;
                    self.resume_due = None;
                }
            }
            if self.needs_resume && self.resume_due.is_none_or(|at| now >= at) {
                let attempt = self.resume_check.map_or(1, |(_, _, n)| n + 1);
                if self.send(ws, "SetMatchPaused", json!({"bPaused":false})) {
                    self.needs_resume = false;
                    self.resume_due = None;
                    self.resume_check = Some((now, self.frame, attempt));
                }
            }
        }
        if self.playing
            || self.busy()
            || self.seek_target.is_some()
            || self.scan.is_some()
            || self.pending_detach.is_some()
            || self.capture_rx.is_some()
            || self.capture_guard.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    fn save_keybind(&mut self) {
        let result = std::fs::create_dir_all(&self.project_dir).and_then(|_| {
            std::fs::write(self.project_dir.join("play-pause-key.txt"), &self.keybind)
        });
        self.status = result.map_or_else(
            |e| format!("Could not save hotkey: {e}"),
            |_| {
                if self.keybind.is_empty() {
                    "Play/pause hotkey cleared.".into()
                } else {
                    "Play/pause hotkey saved.".into()
                }
            },
        );
    }
    fn capture_keybind(&mut self, ctx: &egui::Context) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("timeline-hotkey-capture".into())
            .spawn(move || {
                let key = hebnix_sdk::input::detect_hotkey(Some(Duration::from_secs(10)));
                let _ = tx.send(key);
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.capture_rx = Some(rx);
                self.status = "Press a play/pause key (Escape cancels).".into();
            }
            Err(e) => self.status = format!("Could not capture hotkey: {e}"),
        }
    }
    fn detach(&mut self) {
        self.full_replay = false;
        self.live_match = false;
        self.playing = false;
        self.run_started = false;
        self.resume_check = None;
        self.resume_due = None;
        self.reconnect_pending = false;
        self.phase = Phase::Idle;
        self.metadata = None;
        self.chosen = None;
        self.replay_identity = None;
        self.hold = None;
        self.held_remaining = None;
        self.pending.clear();
        self.seek_target = None;
        self.scan = None;
        self.scan_failed = false;
        self.pending_detach = None;
        self.status = "Open a replay in Rocket League to start.".into();
    }
    fn start_analysis(&mut self, ws: &WsStatsClient, _ctx: &egui::Context) {
        self.prepare(ws);
        self.scan_failed = false;
        self.last_world_check = Some(Instant::now());
        let Some(path) = &self.chosen else {
            return;
        };
        let key = match CacheKey::new(
            path,
            &self.project_dir.join("events"),
            self.max_frame(),
            self.fps(),
        ) {
            Ok(key) => key,
            Err(error) => {
                self.status = format!("Cannot identify replay for event cache: {error}");
                return;
            }
        };
        if let Some(cache) = key.load() {
            self.markers = cache.markers;
            self.status = "Replay events loaded from cache.".into();
            return;
        }
        self.scan = Some(Scan::new(key, self.markers.clone()));
        self.status = "Preparing first-time event scan…".into();
    }
    fn tick_scan(&mut self, ws: &WsStatsClient) {
        let Some(scan) = &self.scan else {
            return;
        };
        if scan.interrupted {
            self.finish_scan(
                ws,
                false,
                "Replay position changed during the scan; no cache saved.",
            );
            return;
        }
        if scan.finished() {
            self.finish_scan(ws, true, "Replay events scanned and cached.");
            return;
        }
        if !scan.running && scan.started.elapsed() > Duration::from_secs(5) {
            self.finish_scan(
                ws,
                false,
                "Could not confirm the start of the replay; no cache saved.",
            );
            return;
        }
        if scan.running && scan.last_progress.elapsed() > Duration::from_secs(5) {
            self.finish_scan(ws, false, "Event scan stalled; no cache saved.");
            return;
        }
        if scan.running
            && scan.reached_end.is_none()
            && scan.last_progress.elapsed() > Duration::from_millis(750)
            && Instant::now() >= scan.resume_after
            && scan.resume_attempts < 3
        {
            // A delayed pause acknowledgement can follow the speed change. Retry only
            // unpause, with a bound; never restart or rewind a scan automatically.
            self.send(ws, "SetMatchPaused", json!({"bPaused":false}));
            if let Some(scan) = &mut self.scan {
                scan.resume_attempts += 1;
                scan.resume_after = Instant::now() + Duration::from_millis(750);
            }
            return;
        }
        let scan = self.scan.as_ref().unwrap();
        if !scan.running && scan.at_start && matches!(self.phase, Phase::Idle) {
            let speed_sent = self.send(ws, "SetGameSpeed", json!({"Speed":16.0}));
            let play_sent = self.send(ws, "SetMatchPaused", json!({"bPaused":false}));
            if !(speed_sent && play_sent) {
                self.finish_scan(ws, false, "Could not start event scan; no cache saved.");
                return;
            }
            if let Some(scan) = &mut self.scan {
                scan.running = true;
                scan.last_progress = Instant::now();
                scan.resume_after = Instant::now() + Duration::from_millis(750);
            }
            self.status = "Scanning replay events at 16×…".into();
        }
    }
    fn finish_scan(&mut self, ws: &WsStatsClient, complete: bool, message: &str) {
        let Some(mut scan) = self.scan.take() else {
            return;
        };
        self.status = message.into();
        self.scan_failed = !complete;
        if complete {
            scan.flush_events(scan.frame);
            scan.markers.sort_by_key(|m| m.frame);
            self.markers = scan.markers.clone();
            if let Err(error) = scan.key.save(scan.markers, scan.events) {
                self.scan_failed = true;
                self.status = format!("Events loaded, but cache could not be saved: {error}");
            }
        }
        // Always restore normal speed and pause at frame 1, including cancel/failure.
        self.prepare(ws);
    }
    fn apply(&mut self, command: &Command, ws: &WsStatsClient) {
        match command {
            Command::Pov { focus, .. } => {
                let mut data = serde_json::Map::new();
                if !focus.is_empty() {
                    data.insert("Focus".into(), json!(focus));
                }
                data.insert("Perspective".into(), json!("PlayerView"));
                self.send(ws, "ChangePOV", data.into());
            }
            Command::Speed(speed) => {
                self.send(ws, "SetGameSpeed", json!({"Speed": speed}));
                self.speed = *speed;
                if self.playing {
                    self.needs_resume = true;
                    self.resume_due = Some(Instant::now() + Duration::from_millis(120));
                    self.resume_check = None;
                }
            }
            Command::Hud(visible) => {
                self.send(ws, "SetHUDVisibility", json!({"bVisible": visible}));
            }
            _ => {}
        }
    }
    fn stop(&mut self, ws: &WsStatsClient) {
        self.playing = false;
        self.needs_resume = false;
        self.resume_due = None;
        self.resume_check = None;
        self.held_remaining = self
            .hold
            .take()
            .map(|(at, s)| (s - at.elapsed().as_secs_f64()).max(0.0));
        self.send(ws, "SetMatchPaused", json!({"bPaused": true}));
    }
    fn toggle(&mut self, ws: &WsStatsClient) {
        if !self.ready(ws) {
            return;
        }
        if self.playing {
            self.stop(ws);
            return;
        }
        self.playing = true;
        self.needs_resume = true;
        self.resume_due = Some(Instant::now() + Duration::from_millis(120));
        self.resume_check = None;
        if let Some(seconds) = self.held_remaining.take() {
            self.hold = Some((Instant::now(), seconds));
        }
        if !self.run_started {
            self.run_started = true;
            self.run_start = self.selected;
            self.fired.clear();
            self.pending.clear();
            self.apply(&Command::Speed(1.0), ws);
            self.send(ws, "SetHUDVisibility", json!({"bVisible": true}));
            self.restore_state(ws);
        }
        self.status = "Playing timeline.".into();
    }
    fn restore_state(&mut self, ws: &WsStatsClient) {
        let (mut pov, mut speed, mut hud) = (None, None, None);
        for command in self
            .keys
            .iter()
            .filter(|k| k.frame < self.selected)
            .flat_map(|k| &k.commands)
        {
            match command {
                Command::Pov { .. } => pov = Some(command.clone()),
                Command::Speed(_) => speed = Some(command.clone()),
                Command::Hud(_) => hud = Some(command.clone()),
                _ => {}
            }
        }
        for command in [pov, speed, hud].into_iter().flatten() {
            self.apply(&command, ws);
        }
    }
    fn scrub(&mut self, frame: i64, ws: &WsStatsClient) {
        if !self.ready(ws) {
            return;
        }
        self.stop(ws);
        self.run_started = false;
        self.hold = None;
        self.held_remaining = None;
        self.pending.clear();
        self.fired.clear();
        self.selected = frame.clamp(1, self.max_frame());
        self.frame = self.selected;
        self.selected_key = None;
        self.apply(&Command::Speed(1.0), ws);
        self.send(ws, "SetHUDVisibility", json!({"bVisible": true}));
        self.restore_state(ws);
        self.send(ws, "SetMatchPaused", json!({"bPaused":true}));
        self.send(ws, "SeekReplay", json!({"Frame": self.selected}));
        self.seek_target = Some((self.selected, Instant::now()));
    }
    fn refresh_files(&mut self) {
        let mut folders: Vec<_> = self.folder.iter().cloned().collect();
        if let Some(documents) = hebnix_sdk::utils::paths::documents_dir() {
            let root = documents.join("My Games/Rocket League/TAGame");
            folders.extend([root.join("DemosEpic"), root.join("Demos")]);
        }
        folders.sort();
        folders.dedup();
        self.files = folders
            .iter()
            .filter_map(|p| std::fs::read_dir(p).ok())
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("replay"))
            })
            .collect();
        self.files
            .sort_by_key(|p| std::cmp::Reverse(p.metadata().and_then(|m| m.modified()).ok()));
    }
    fn attach_current(&mut self, ws: &WsStatsClient, ctx: &egui::Context) {
        // The latest map URL identifies full replays even when Hebnix connects after ReplayCreated.
        // A live goal replay keeps the live match's map URL, so it cannot grant timeline control.
        let logged = current_replay_from_log();
        if !self.full_replay && logged.is_none() {
            return;
        }
        self.refresh_files();
        let logged_name = logged
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str());
        let event_name = self
            .replay_identity
            .as_ref()
            .map(|(name, _)| name.as_str())
            .unwrap_or("");
        let mut found = self
            .files
            .iter()
            .find(|path| {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
                logged_name.is_some_and(|n| name.eq_ignore_ascii_case(n))
                    || (!event_name.is_empty()
                        && (name.eq_ignore_ascii_case(event_name)
                            || stem.eq_ignore_ascii_case(event_name)))
            })
            .cloned();
        if found.is_none() {
            if let Some(path) = logged.filter(|p| p.is_absolute() && p.is_file()) {
                found = Some(path);
            }
        }
        if found.is_none() {
            if let Some((_, guid)) = &self.replay_identity {
                if !guid.is_empty() {
                    found = self
                        .files
                        .iter()
                        .find(|p| {
                            replay_metadata::read(p)
                                .is_ok_and(|m| m.ids.iter().any(|id| id.eq_ignore_ascii_case(guid)))
                        })
                        .cloned();
                }
            }
        }
        if let Some(path) = found {
            if self.select_file(path) {
                self.full_replay = true;
                self.live_match = false;
                if let Some(m) = &self.metadata {
                    self.replay_guids.extend(m.ids.iter().cloned());
                }
                self.start_analysis(ws, ctx);
            }
        }
    }
    fn hold_seconds(key: &Keyframe) -> f64 {
        key.commands
            .iter()
            .filter_map(|c| {
                if let Command::Pause(s) = c {
                    Some(s.clamp(0.1, 3600.0))
                } else {
                    None
                }
            })
            .sum()
    }
    fn position(&self, frame: i64) -> f64 {
        self.replay_time(frame)
            + self
                .keys
                .iter()
                .filter(|k| k.frame < frame)
                .map(Self::hold_seconds)
                .sum::<f64>()
    }
    fn duration(&self) -> f64 {
        self.position(self.max_frame())
            + self
                .keys
                .iter()
                .filter(|k| k.frame == self.max_frame())
                .map(Self::hold_seconds)
                .sum::<f64>()
    }
    fn frame_at(&self, seconds: f64) -> i64 {
        let mut pause = 0.0;
        for key in &self.keys {
            let start = self.replay_time(key.frame) + pause;
            let hold = Self::hold_seconds(key);
            if seconds < start {
                break;
            }
            if seconds <= start + hold && hold > 0.0 {
                return key.frame;
            }
            pause += hold;
        }
        let seconds = seconds - pause;
        (seconds * self.fps()).round() as i64 + 1
    }
    fn replay_time(&self, frame: i64) -> f64 {
        (frame - 1) as f64 / self.fps()
    }
    pub fn show(&mut self, ui: &mut egui::Ui, folder: Result<PathBuf, String>, ws: &WsStatsClient) {
        match folder {
            Ok(path) if self.folder.as_ref() != Some(&path) => {
                self.folder = Some(path);
                self.refresh_files();
            }
            Err(e) => {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            _ => {}
        }
        crate::i18n::layout::bordered_panel(ui, |ui| {
            ui.heading(crate::i18n::t("nav-replay-library"));
            ui.separator();
            if let Some(scan) = &self.scan {
                ui.add(
                    egui::ProgressBar::new(scan.frame as f32 / self.max_frame() as f32)
                        .text("Scanning replay events · 16×"),
                );
                if ui.button("Cancel scan").clicked() {
                    self.finish_scan(ws, false, "Event scan cancelled; no cache saved.");
                }
            }
            if self.scan_failed
                && ui
                    .add_enabled(self.ready(ws), egui::Button::new("Retry event scan"))
                    .clicked()
            {
                self.start_analysis(ws, ui.ctx());
            }
            ui.horizontal_wrapped(|ui| {
                let colour = if ws.connected() {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::GRAY
                };
                ui.colored_label(
                    colour,
                    if ws.connected() {
                        "Stats API connected"
                    } else {
                        "Stats API disconnected"
                    },
                );
                if ui.button("Detect current replay").clicked() {
                    self.last_attach_check = None;
                    self.refresh_files();
                }
                ui.label("Play/pause key:");
                let mut display = if self.capture_rx.is_some() {
                    "Press a key...".into()
                } else if self.keybind.is_empty() {
                    "Not set".into()
                } else {
                    self.keybind.to_uppercase()
                };
                ui.add(
                    egui::TextEdit::singleline(&mut display)
                        .interactive(false)
                        .desired_width(100.0),
                );
                if ui
                    .add_enabled(self.capture_rx.is_none(), egui::Button::new("Set"))
                    .clicked()
                {
                    self.capture_keybind(ui.ctx());
                }
                if ui
                    .add_enabled(
                        !self.keybind.is_empty() || self.capture_rx.is_some(),
                        egui::Button::new("Clear"),
                    )
                    .clicked()
                {
                    self.capture_rx = None;
                    self.capture_guard = None;
                    self.keybind.clear();
                    if let Some(hotkey) = &mut self.hotkey {
                        hotkey.clear();
                    }
                    self.hotkey_rx.try_iter().for_each(drop);
                    self.save_keybind();
                }
            });
            if self.metadata.is_none()
                && ws.connected()
                && self
                    .last_attach_check
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(2))
            {
                self.last_attach_check = Some(Instant::now());
                self.attach_current(ws, ui.ctx());
            }
            ui.label(&self.status);
        });
        ui.add_space(8.0);
        let ready = self.ready(ws);
        let canvas_height = if self.metadata.is_some() {
            205.0 + egui::Frame::group(ui.style()).total_margin().sum().y
        } else {
            0.0
        };
        let editor_height = (ui.available_height() - canvas_height).max(60.0);
        egui::ScrollArea::vertical().id_salt("replay_timeline_editor").max_height(editor_height).auto_shrink([false,false]).show(ui, |ui| {
            crate::i18n::layout::bordered_panel(ui, |ui| {
                ui.heading("Playback & Keyframes");
                ui.separator();
                if !self.full_replay || self.metadata.is_none() {
                    ui.heading("Waiting for a replay");
                    ui.label("Open a replay in Rocket League, then return here. Timeline controls become available while you are watching it.");
                    if self.full_replay { ui.label("Could not match this replay to a local file. Check your configured replay folder, then detect again."); }
                } else {
                    if let Some(meta) = &self.metadata { ui.strong(format!("{} · {} frames · {} fps", meta.title,meta.frames,meta.fps)); }
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(ready, egui::Button::new(if self.playing { "Pause" } else { "Play timeline" })).clicked() { self.toggle(ws); }
                        if frame_step_button(ui, ready, false).clicked() { self.scrub(self.selected-1,ws); }
                        if frame_step_button(ui, ready, true).clicked() { self.scrub(self.selected+1,ws); }
                        if ui.add_enabled(ready, egui::Button::new("Frame 1")).clicked() { self.scrub(1,ws); }
                        if ui.add_enabled(ready && !self.playing, egui::Button::new("+ Keyframe")).clicked() {
                            if !self.keys.iter().any(|k| k.frame == self.selected) { self.keys.push(Keyframe { frame:self.selected,commands:vec![] }); self.keys.sort_by_key(|k| k.frame); }
                            self.selected_key = Some(self.selected); self.save();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Keyframes:");
                        for key in &self.keys {
                            if ui.selectable_label(self.selected_key == Some(key.frame),format!("Frame {}",key.frame)).clicked() { self.selected_key = Some(key.frame); }
                        }
                    });
                    if let Some(frame) = self.selected_key {
                        self.edit_keyframe(ui, frame, ready && !self.playing);
                    } else { ui.label("Select a frame and add a keyframe, or select a diamond to edit its commands."); }
                }
            });
        });
        if self.metadata.is_some() {
            ui.add_space(4.0);
            crate::i18n::layout::bordered_panel(ui, |ui| self.canvas(ui, ws, ready));
        }
    }
    fn edit_keyframe(&mut self, ui: &mut egui::Ui, frame: i64, enabled: bool) {
        let Some(index) = self.keys.iter().position(|k| k.frame == frame) else {
            return;
        };
        let mut changed = false;
        let mut delete = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("Keyframe · Frame {frame}"));
                egui::ComboBox::from_id_salt("add_timeline_command")
                    .selected_text("+ Add command")
                    .show_ui(ui, |ui| {
                        for command in Command::defaults() {
                            if ui.button(command.label()).clicked() {
                                self.keys[index].commands.push(command);
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                delete = ui.button("Delete keyframe").clicked();
            });
            let mut remove = None;
            for (n, command) in self.keys[index].commands.iter_mut().enumerate() {
                ui.push_id(n, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(command.label());
                        match command {
                            Command::Pov { focus, perspective } => {
                                *perspective = "PlayerView".into();
                                egui::ComboBox::from_id_salt("focus")
                                    .selected_text(
                                        self.players
                                            .iter()
                                            .find(|(_, s)| s.to_string() == *focus)
                                            .map_or_else(
                                                || focus.clone(),
                                                |(name, _)| name.clone(),
                                            ),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (name, shortcut) in &self.players {
                                            changed |= ui
                                                .selectable_value(
                                                    focus,
                                                    shortcut.to_string(),
                                                    format!("{name} ({shortcut})"),
                                                )
                                                .changed();
                                        }
                                    });
                                ui.label("Player View");
                            }
                            Command::Speed(speed) => {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(speed)
                                            .range(0.0..=16.0)
                                            .speed(0.05)
                                            .suffix("×"),
                                    )
                                    .changed();
                                if *speed == 0.0 {
                                    ui.label("0 freezes replay progression");
                                }
                            }
                            Command::Hud(visible) => {
                                changed |= ui.checkbox(visible, "Visible").changed();
                            }
                            Command::Pause(seconds) => {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(seconds)
                                            .range(0.1..=3600.0)
                                            .speed(0.1)
                                            .suffix(" seconds"),
                                    )
                                    .changed();
                            }
                            Command::Seek(target) => {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(target)
                                            .range(
                                                1..=self.metadata.as_ref().map_or(1, |m| m.frames),
                                            )
                                            .prefix("Frame "),
                                    )
                                    .changed();
                            }
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(n);
                        }
                    });
                });
            }
            if let Some(n) = remove {
                self.keys[index].commands.remove(n);
                changed = true;
            }
        });
        if delete {
            self.keys.remove(index);
            self.selected_key = None;
            changed = true;
        }
        if changed {
            self.save();
        }
    }
    fn canvas(&mut self, ui: &mut egui::Ui, ws: &WsStatsClient, ready: bool) {
        if ready
            && self.capture_rx.is_none()
            && self.capture_guard.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let delta = ui.input(|i| {
                i.num_presses(egui::Key::ArrowRight) as i64
                    - i.num_presses(egui::Key::ArrowLeft) as i64
            });
            if delta != 0 {
                self.scrub(self.selected + delta, ws);
            }
        }
        let total = self.duration().max(1.0 / self.fps());
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Frame {} / {}", self.selected, self.max_frame()));
            ui.label(format!(
                "{} / {}",
                clock(self.position(self.selected)),
                clock(total)
            ));
            if ui.button("−").clicked() {
                self.zoom = (self.zoom / 1.6).max(1.0);
            }
            if ui.button("+").clicked() {
                self.zoom = (self.zoom * 1.6).min((self.max_frame() as f64 / 4.0).max(1.0));
            }
            if ui.button("Fit").clicked() {
                self.zoom = 1.0;
                self.view_start = 0.0;
            }
            if ui.button("Follow playhead").clicked() {
                self.view_start = self.position(self.selected) - total / self.zoom / 2.0;
            }
            ui.label(format!(
                "{:.1}× zoom · G goal · S save · blue/orange team · gold pause",
                self.zoom
            ));
        });
        let span = total / self.zoom;
        self.view_start = self.view_start.clamp(0.0, (total - span).max(0.0));
        if self.zoom > 1.0 {
            ui.add(
                egui::Slider::new(&mut self.view_start, 0.0..=(total - span).max(0.0))
                    .text("Pan")
                    .show_value(false),
            );
        }
        let height = ui.available_height().clamp(75.0, 155.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(1.0), height),
            egui::Sense::click_and_drag(),
        );
        let inner = rect.shrink2(Vec2::new(14.0, 8.0));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 5.0, Color32::from_rgb(20, 24, 33));
        let x = |s: f64| inner.left() + ((s - self.view_start) / span) as f32 * inner.width();
        let top = inner.top() + 20.0;
        let bottom = inner.bottom() - 20.0;
        painter.hline(inner.x_range(), top, Stroke::new(1.0, Color32::DARK_GRAY));
        painter.hline(
            inner.x_range(),
            bottom,
            Stroke::new(1.0, Color32::DARK_GRAY),
        );
        // Ticks are chosen in frame space, so deep zoom reveals individual frames.
        let visible_frames = span * self.fps();
        let wanted = (inner.width() / 85.0).max(1.0) as f64;
        let raw_step = (visible_frames / wanted).max(1.0);
        let base = 10_f64.powf(raw_step.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|v| v * base)
            .find(|v| *v >= raw_step)
            .unwrap_or(base * 10.0) as i64;
        let first = self.frame_at(self.view_start).max(1) / step * step;
        let last = self.frame_at(self.view_start + span).min(self.max_frame());
        let text = Color32::from_gray(185);
        for frame in (first..=last).step_by(step.max(1) as usize) {
            let frame = frame.max(1);
            let pos = self.position(frame);
            let px = x(pos);
            if px < inner.left() - 1.0 || px > inner.right() + 1.0 {
                continue;
            }
            painter.vline(px, top..=bottom, Stroke::new(1.0, Color32::from_gray(42)));
            painter.text(
                Pos2::new(px, inner.top()),
                egui::Align2::CENTER_TOP,
                frame.to_string(),
                egui::FontId::monospace(10.0),
                text,
            );
            painter.text(
                Pos2::new(px, inner.bottom()),
                egui::Align2::CENTER_BOTTOM,
                clock(pos),
                egui::FontId::monospace(10.0),
                text,
            );
        }
        for key in &self.keys {
            let start = self.position(key.frame);
            let hold = Self::hold_seconds(key);
            if hold > 0.0 {
                let a = x(start).max(inner.left());
                let b = x(start + hold).min(inner.right());
                if b >= a {
                    painter.rect_filled(
                        Rect::from_min_max(Pos2::new(a, top), Pos2::new(b, bottom)),
                        2.0,
                        Color32::from_rgba_unmultiplied(190, 140, 60, 85),
                    );
                }
            }
            let px = x(start);
            if px >= inner.left() && px <= inner.right() {
                let pos = Pos2::new(px, bottom - 10.0);
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos + Vec2::new(0.0, -6.0),
                        pos + Vec2::new(6.0, 0.0),
                        pos + Vec2::new(0.0, 6.0),
                        pos + Vec2::new(-6.0, 0.0),
                    ],
                    Color32::from_rgb(223, 192, 110),
                    Stroke::NONE,
                ));
                let hit = ui.interact(
                    Rect::from_center_size(pos, Vec2::splat(16.0)),
                    ui.id().with(("key", key.frame)),
                    egui::Sense::click(),
                );
                if hit.clicked() {
                    self.selected_key = Some(key.frame);
                }
                hit.on_hover_text(format!(
                    "Frame {} · {} commands · {:.1}s pause",
                    key.frame,
                    key.commands.len(),
                    hold
                ));
            }
        }
        let mut jump = None;
        for (n, marker) in self.markers.iter().enumerate() {
            let px = x(self.position(marker.frame));
            if px < inner.left() || px > inner.right() {
                continue;
            }
            let colour = match marker.team {
                0 => Color32::from_rgb(65, 155, 255),
                1 => Color32::from_rgb(255, 157, 52),
                _ => Color32::GRAY,
            };
            let y = if marker.team == 1 {
                top + 29.0
            } else {
                top + 11.0
            };
            let pos = Pos2::new(px, y);
            painter.circle_filled(pos, 7.0, colour);
            let icon = if marker.event == "Goal" {
                "G"
            } else if marker.event.to_lowercase().contains("save") {
                "S"
            } else {
                "•"
            };
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::monospace(9.0),
                Color32::from_rgb(15, 20, 27),
            );
            let hit = ui.interact(
                Rect::from_center_size(pos, Vec2::splat(12.0)),
                ui.id().with(("event", n)),
                egui::Sense::click(),
            );
            if hit.clicked() {
                jump = Some(marker.frame);
            }
            hit.on_hover_text(format!(
                "{} · {} · {} team · Frame {}",
                marker.event,
                marker.player,
                match marker.team {
                    0 => "Blue",
                    1 => "Orange",
                    _ => "Unknown",
                },
                marker.frame
            ));
        }
        let mut playhead = self.position(self.selected);
        if let Some((at, seconds)) = self.hold {
            playhead += at.elapsed().as_secs_f64().min(seconds);
        }
        let px = x(playhead);
        if (inner.left()..=inner.right()).contains(&px) {
            painter.vline(px, inner.y_range(), Stroke::new(2.0, Color32::WHITE));
        }
        if let Some(frame) = jump {
            self.scrub(frame, ws);
        } else if ready && (response.clicked() || response.dragged()) {
            if let Some(pos) = response.interact_pointer_pos() {
                let seconds = self.view_start
                    + ((pos.x - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64 * span;
                let frame = self.frame_at(seconds).clamp(1, self.max_frame());
                if frame != self.selected || response.clicked() {
                    self.scrub(frame, ws);
                }
            }
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let old_span = span;
                self.zoom = (self.zoom * (scroll as f64 * 0.005).exp())
                    .clamp(1.0, (self.max_frame() as f64 / 4.0).max(1.0));
                let fraction = response.hover_pos().map_or(0.5, |p| {
                    ((p.x - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64
                });
                self.view_start += fraction * (old_span - total / self.zoom);
            }
        }
    }
}
fn frame_step_button(ui: &mut egui::Ui, enabled: bool, forward: bool) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("Frame").min_size(Vec2::new(76.0, 20.0)),
    );
    let centre = Pos2::new(
        if forward {
            response.rect.right() - 11.0
        } else {
            response.rect.left() + 11.0
        },
        response.rect.center().y,
    );
    let direction = if forward { 1.0 } else { -1.0 };
    let colour = ui.style().interact(&response).fg_stroke.color;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            centre + Vec2::new(direction * 3.0, 0.0),
            centre + Vec2::new(-direction * 3.0, -4.0),
            centre + Vec2::new(-direction * 3.0, 4.0),
        ],
        colour,
        Stroke::NONE,
    ));
    ui.painter().line_segment(
        [
            centre + Vec2::new(direction * 5.0, -4.0),
            centre + Vec2::new(direction * 5.0, 4.0),
        ],
        Stroke::new(1.2, colour),
    );
    response.on_hover_text(if forward {
        "Next frame (Right Arrow)"
    } else {
        "Previous frame (Left Arrow)"
    })
}
fn clock(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{}:{:02}.{:03}",
        millis / 60000,
        (millis / 1000) % 60,
        millis % 1000
    )
}
fn current_replay_from_log() -> Option<PathBuf> {
    current_replay_map().flatten()
}
// Outer None means the log cannot establish a map. Some(None) confirms a non-replay map.
fn current_replay_map() -> Option<Option<PathBuf>> {
    let path = hebnix_sdk::utils::paths::documents_dir()?.join("My Games/Rocket League/TAGame/Logs/Launch.log");
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut window = 256 * 1024;
    let map = loop {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut tail = Vec::new();
        (&mut file).take(window).read_to_end(&mut tail).ok()?;
        let text = String::from_utf8_lossy(&tail);
        if let Some(map) = text.lines().rev().find_map(|line| {
            line.split_once("Log: LoadMap:")
                .map(|(_, map)| map.trim().to_owned())
        }) {
            break map;
        }
        // A long-running replay can push its LoadMap line outside the latest log tail.
        if start == 0 || window >= 64 * 1024 * 1024 {
            return None;
        }
        window *= 2;
    };
    if !map
        .split('?')
        .any(|part| part == "Game=TAGame.GameInfo_Replay_TA")
    {
        return Some(None);
    }
    let query = map.split_once('?')?.1.replace('?', "&");
    let replay = url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(key, value)| (key == "Replay").then(|| PathBuf::from(value.as_ref())))?;
    Some(Some(replay))
}
