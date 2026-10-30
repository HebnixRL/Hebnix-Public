use crate::config::{ReplayUploadCfg, RlLaunchCfg, RlLaunchMode};
use crate::messages::AppMsg;
use crossbeam_channel::Sender;
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HISTORY_FILE: &str = "replay_upload_history.json";
const QUEUE_FILE: &str = "replay_pending_matches.json";
const MAX_REPLAY_BYTES: u64 = 100 * 1024 * 1024;
const HISTORY_JOB: &str = "__replay_history_check__";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadHistoryEntry {
    pub match_guid: String,
    pub replay_id: String,
    pub location: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub warning: String,
    #[serde(default)]
    pub duplicate: bool,
    #[serde(default)]
    pub local_path: String,
    #[serde(default)]
    pub recorded_at: i64,
    #[serde(default)]
    pub player_id: String,
    #[serde(default)]
    pub source_hash: String,
}

#[derive(Debug)]
pub struct UploadResult {
    pub match_guid: String,
    pub result: Result<Option<UploadHistoryEntry>, String>,
}

#[derive(Debug, Clone)]
struct TrackedMatch {
    match_guid: String,
}

pub struct AutoUploadReplays {
    base_dir: PathBuf,
    running: bool,
    pending: HashSet<String>,
    history: Vec<UploadHistoryEntry>,
    status: String,
    generation: Arc<AtomicU64>,
    check_requested: bool,
    queued: HashSet<String>,
    watch_start: SystemTime,
    active_config: Option<ReplayUploadCfg>,
    refresh_client: Option<Arc<hebnix_sdk::stats::WsStatsClient>>,
}

impl AutoUploadReplays {
    pub fn new(base_dir: &Path) -> Self {
        let history = std::fs::read(base_dir.join(HISTORY_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let state: Value = std::fs::read(base_dir.join(QUEUE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let queued = serde_json::from_value(if state.is_array() {
            state.clone()
        } else {
            state["matches"].clone()
        })
        .unwrap_or_default();
        let watch_start = state["capture_started_at"]
            .as_u64()
            .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
            .unwrap_or_else(SystemTime::now);
        Self {
            base_dir: base_dir.to_path_buf(),
            running: false,
            pending: HashSet::new(),
            history,
            status: "Disabled".into(),
            generation: Arc::new(AtomicU64::new(0)),
            check_requested: false,
            queued,
            watch_start,
            active_config: None,
            refresh_client: None,
        }
    }
    pub fn running(&self) -> bool {
        self.running
    }
    pub fn with_refresh_client(mut self, client: Arc<hebnix_sdk::stats::WsStatsClient>) -> Self {
        self.refresh_client = Some(client);
        self
    }
    pub fn sync(&mut self, enabled: bool, _game_running: bool, _capture_enabled: bool) {
        if enabled {
            if !self.running {
                self.running = true;
                self.check_requested = false;
                self.status = "Waiting for a match to end or be left".into();
            }
        } else if self.running {
            self.stop();
        } else {
            self.status = "Disabled".into();
        }
    }
    pub fn stop(&mut self) {
        self.running = false;
        self.pending.clear();
        self.check_requested = false;
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.status = "Disabled".into();
    }
    pub fn needs_match_identity(&self, _guid: Option<&str>) -> bool {
        false
    }
    pub fn track_match(&mut self, _guid: &str, _player_id: Option<String>) {}

    pub fn check(
        &mut self,
        config: &ReplayUploadCfg,
        game_path: &Path,
        launch: &RlLaunchCfg,
        tx: Sender<AppMsg>,
        ctx: egui::Context,
        force: bool,
    ) {
        if force {
            self.check_requested = true;
        }
        if !self.check_requested {
            return;
        }
        if !hebnix_sdk::process::is_rocket_league_running() {
            self.check_requested = false;
            self.status = "Waiting for Rocket League and a completed or left match".into();
            return;
        }
        if !self.running
            || !self.pending.is_empty()
            || (!config.upload_enabled && !config.save_locally)
        {
            return;
        }
        self.check_requested = false;
        let local_dir = if config.save_locally {
            match configured_replay_directory(config, game_path, launch) {
                Ok(path) => Some(path),
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        } else {
            None
        };
        if config.upload_enabled && config.api_key.trim().is_empty() && local_dir.is_none() {
            self.status = "Enter a ballchasing.com API key".into();
            return;
        }
        if let Err(error) = self.save_queue() {
            self.status = format!("Could not persist replay queue: {error}");
            return;
        }
        let job_id = format!("{HISTORY_JOB}{}", self.generation.load(Ordering::Acquire));
        self.pending.insert(job_id.clone());
        self.status = "Checking replay history".into();
        let game_path = game_path.to_path_buf();
        let queued = self.queued.clone();
        let watch_start = self.watch_start;
        let source_directory = configured_replay_directory(config, &game_path, launch).ok();
        self.active_config = Some(config.clone());
        let config = config.clone();
        let history = self.history.clone();
        let generation = Arc::clone(&self.generation);
        let expected_generation = generation.load(Ordering::Acquire);
        let worker_id = job_id.clone();
        // The local Stats API bridge dispatches through the existing game
        // session. It never opens a second PsyNet connection.
        let requested = self.refresh_client.as_ref().is_some_and(|client| {
            client.connected()
                && client
                    .send_command(hebnix_sdk::stats::WsCommand::new("HebnixMatchHistory"))
                    .is_ok()
        });
        let spawn = std::thread::Builder::new()
            .name("replay-direct-delivery".into())
            .spawn(move || {
                if requested {
                    // Give the asynchronous history RPC time to complete.
                    // Unavailable replays remain queued until the next match event or manual check.
                    for _ in 0..30 {
                        if generation.load(Ordering::Acquire) != expected_generation {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                let local = || {
                    recover_local(
                        &config,
                        local_dir.clone(),
                        source_directory.as_deref(),
                        watch_start,
                        &history,
                        Arc::clone(&generation),
                        expected_generation,
                    )
                };
                let result = recover_next(
                    &config,
                    &game_path,
                    local_dir.clone(),
                    &history,
                    &queued,
                    Some(
                        watch_start
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64,
                    ),
                    Arc::clone(&generation),
                    expected_generation,
                )
                .or_else(|error| match local() {
                    Ok(Some(entry)) => Ok(Some(entry)),
                    _ => Err(error),
                });
                let _ = tx.send(AppMsg::ReplayUploadFinished(UploadResult {
                    match_guid: worker_id,
                    result,
                }));
                ctx.request_repaint();
            });
        if let Err(error) = spawn {
            self.pending.remove(&job_id);
            self.status = format!("Could not start replay worker: {error}");
        }
    }
    pub fn submit_finished(
        &mut self,
        event_guid: Option<&str>,
        config: &ReplayUploadCfg,
        game_path: &Path,
        launch: &RlLaunchCfg,
        tx: Sender<AppMsg>,
        ctx: egui::Context,
    ) -> Option<String> {
        if self.running {
            if let Some(guid) = event_guid
                .map(normalize_guid)
                .filter(|g| g.len() == 32 && g.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                self.queued.insert(guid);
                if let Err(error) = self.save_queue() {
                    self.status = format!("Could not persist pending replay: {error}");
                }
            }
            self.check(config, game_path, launch, tx, ctx, true);
        }
        None
    }
    pub fn finish_upload(&mut self, result: UploadResult) {
        if !self.pending.remove(&result.match_guid) {
            return;
        }
        match result.result {
            Ok(Some(entry)) => {
                self.status = format!("{} replay {}", replay_outcome(&entry), entry.match_guid);
                self.history
                    .retain(|item| item.match_guid != entry.match_guid);
                let delivered = self.active_config.as_ref().is_some_and(|config| {
                    completed(&entry, config, Path::new(&entry.local_path).parent())
                });
                let guid = entry.match_guid.clone();
                self.history.insert(0, entry);
                // The UI is limited to 20 rows, but the deduplication ledger persists.
                if let Err(error) = self.save_history() {
                    self.status
                        .push_str(&format!(" · Could not save replay history: {error}"));
                } else if delivered {
                    self.queued.remove(&guid);
                    let _ = self.save_queue();
                }
            }
            Ok(None) => {
                self.status = if self.queued.is_empty() {
                    "Up to date · waiting for a match to end or be left".into()
                } else {
                    format!(
                        "{} match(es) queued · waiting for the next match event or manual check",
                        self.queued.len()
                    )
                };
            }
            Err(error) => {
                self.status = format!("{error} · check again after a match ends or is left")
            }
        }
    }
    fn save_history(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.history).map_err(|e| e.to_string())?;
        let temporary = self.base_dir.join("replay_upload_history.tmp");
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        // Windows rename cannot replace an existing file; MoveFileExW can.
        replace_file(&temporary, &self.base_dir.join(HISTORY_FILE))
    }
    fn save_queue(&self) -> Result<(), String> {
        let temporary = self.base_dir.join("replay_pending_matches.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&serde_json::json!({"matches":self.queued,"capture_started_at":self.watch_start.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()})).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        replace_file(&temporary, &self.base_dir.join(QUEUE_FILE))
    }
    pub fn show_status(&self, ui: &mut egui::Ui) {
        crate::theme::page_heading(ui, "Replays");
        ui.label("Saves and uploads completed games automatically using the live history bridge.");
        ui.small("Checks run after match-end and match-left events while Rocket League is open.");
        ui.add_space(8.0);
        ui.label(&self.status);
        if !self.pending.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking or delivering replay");
            });
        }
    }
    pub fn show_history(&self, ui: &mut egui::Ui) {
        ui.heading("Last 20 replays");
        if self.history.is_empty() {
            ui.label("No replays saved or uploaded yet.");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("replay_upload_history")
            .show(ui, |ui| {
                for entry in self.history.iter().take(20) {
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(&entry.match_guid);
                        ui.label(replay_outcome(entry));
                        if !entry.location.is_empty() {
                            ui.hyperlink_to("Open on ballchasing.com", &entry.location);
                        }
                    });
                    if !entry.local_path.is_empty() {
                        ui.small(format!("Saved to {}", entry.local_path));
                    }
                    if !entry.title.is_empty() {
                        ui.label(&entry.title);
                    }
                    if !entry.warning.is_empty() {
                        ui.colored_label(egui::Color32::YELLOW, &entry.warning);
                    }
                    ui.separator();
                }
            });
    }
}

#[cfg(target_os = "linux")]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            windows::core::PCWSTR(from.as_ptr()),
            windows::core::PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| e.to_string())
}

fn completed(
    entry: &UploadHistoryEntry,
    config: &ReplayUploadCfg,
    local_dir: Option<&Path>,
) -> bool {
    (!config.upload_enabled || !entry.replay_id.is_empty())
        && (!config.save_locally
            || (!entry.local_path.is_empty()
                && Path::new(&entry.local_path).is_file()
                && local_dir.is_some_and(|dir| Path::new(&entry.local_path).parent() == Some(dir))))
}

fn next_match<'a>(
    value: &'a Value,
    config: &ReplayUploadCfg,
    local_dir: Option<&Path>,
    history: &[UploadHistoryEntry],
    queued: &HashSet<String>,
    capture_since: Option<i64>,
) -> Option<&'a Value> {
    let player = value["LocalPlayerID"].as_str().filter(|p| !p.is_empty())?;
    let mut matches: Vec<_> = value
        .get("Matches")?
        .as_array()?
        .iter()
        .filter(|e| {
            e["Match"]["MatchGUID"].as_str().is_some()
                && e["ReplayUrl"].as_str().is_some_and(|u| !u.is_empty())
                && participated(e, player)
        })
        .collect();
    matches.sort_by_key(|e| e["Match"]["RecordStartTimestamp"].as_i64().unwrap_or(0));
    let newest = matches
        .last()?
        .get("Match")?
        .get("RecordStartTimestamp")?
        .as_i64()?;
    let checkpoint = history
        .iter()
        .filter(|e| e.player_id == player)
        .map(|e| e.recorded_at)
        .max()
        .filter(|t| *t > 0)
        .unwrap_or_else(|| {
            capture_since
                .map(|since| since.min(newest))
                .unwrap_or(newest)
        });
    matches.into_iter().find(|e| {
        (e["Match"]["RecordStartTimestamp"].as_i64().unwrap_or(0) >= checkpoint
            || queued.contains(&normalize_guid(
                e["Match"]["MatchGUID"].as_str().unwrap_or(""),
            )))
            && !history.iter().any(|done| {
                normalize_guid(&done.match_guid)
                    == normalize_guid(e["Match"]["MatchGUID"].as_str().unwrap_or(""))
                    && completed(done, config, local_dir)
            })
    })
}

fn participated(entry: &Value, player: &str) -> bool {
    entry["Match"]["Players"].as_array().is_some_and(|players| {
        players.iter().any(|p| {
            p["PlayerID"].as_str() == Some(player)
                && matches!(p["LastTeam"].as_i64(), Some(0 | 1))
                && p["SecondsPlayed"]
                    .as_f64()
                    .is_some_and(|seconds| seconds > 0.0)
        })
    })
}

fn recover_next(
    config: &ReplayUploadCfg,
    game_path: &Path,
    local_dir: Option<PathBuf>,
    history: &[UploadHistoryEntry],
    queued: &HashSet<String>,
    capture_since: Option<i64>,
    generation: Arc<AtomicU64>,
    expected_generation: u64,
) -> Result<Option<UploadHistoryEntry>, String> {
    if generation.load(Ordering::Acquire) != expected_generation {
        return Err("Stopped".into());
    }
    let response = crate::replay_source::match_history(game_path, None)?;
    let Some(entry) = next_match(
        &response,
        config,
        local_dir.as_deref(),
        history,
        queued,
        capture_since,
    ) else {
        return Ok(None);
    };
    let guid = normalize_guid(
        entry["Match"]["MatchGUID"]
            .as_str()
            .ok_or("Replay has no match GUID")?,
    );
    let job = UploadJob {
        tracked: TrackedMatch { match_guid: guid },
        upload_enabled: config.upload_enabled,
        local_dir,
        api_key: config.api_key.trim().into(),
        naming_template: config.naming_template.trim().into(),
        visibility: config.visibility.clone(),
        group_id: config.group_id.trim().into(),
        generation,
        expected_generation,
    };
    let title = render_replay_title(&job.naming_template, entry);
    let mut result = download_and_upload(
        &job,
        entry["ReplayUrl"]
            .as_str()
            .ok_or("Replay has no download URL")?,
        &title,
    )?;
    result.recorded_at = entry["Match"]["RecordStartTimestamp"].as_i64().unwrap_or(0);
    result.player_id = response["LocalPlayerID"].as_str().unwrap_or("").into();
    Ok(Some(result))
}

fn recover_local(
    config: &ReplayUploadCfg,
    local_dir: Option<PathBuf>,
    source_directory: Option<&Path>,
    since: SystemTime,
    history: &[UploadHistoryEntry],
    generation: Arc<AtomicU64>,
    expected_generation: u64,
) -> Result<Option<UploadHistoryEntry>, String> {
    use std::io::Read;
    let mut directories = Vec::new();
    if source_directory.is_none()
        && let Some(documents) = hebnix_sdk::utils::paths::documents_dir()
    {
        directories.push(replay_directory_in(&documents, true));
        directories.push(replay_directory_in(&documents, false));
    }
    if let Some(path) = source_directory {
        directories.push(path.to_path_buf());
    }
    directories.sort();
    directories.dedup();
    let mut files = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("replay"))
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if !metadata.is_file()
                || metadata.len() > MAX_REPLAY_BYTES
                || modified < since
                || SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default()
                    < Duration::from_secs(3)
            {
                continue;
            }
            files.push((modified, path, metadata.len()));
        }
    }
    files.sort_by_key(|entry| entry.0);
    for (modified, path, size) in files {
        if generation.load(Ordering::Acquire) != expected_generation {
            return Err("Stopped".into());
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file
            .take(MAX_REPLAY_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || validate_replay_bytes(&bytes).is_err()
        {
            continue;
        }
        let Ok(after) = path.metadata() else {
            continue;
        };
        if after.len() != size || after.modified().ok() != Some(modified) {
            continue;
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        if history.iter().any(|done| {
            (done.source_hash == hash || Path::new(&done.local_path) == path)
                && completed(done, config, local_dir.as_deref())
        }) {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Saved replay");
        let guid = if !stem.is_empty()
            && stem.len() <= 128
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            stem.to_ascii_uppercase()
        } else {
            format!("LOCAL_{}", &hash[..24])
        };
        let job = UploadJob {
            tracked: TrackedMatch { match_guid: guid },
            upload_enabled: config.upload_enabled,
            local_dir,
            api_key: config.api_key.trim().into(),
            naming_template: config.naming_template.clone(),
            visibility: config.visibility.clone(),
            group_id: config.group_id.trim().into(),
            generation,
            expected_generation,
        };
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| e.to_string())?;
        let title: String = stem.chars().take(200).collect();
        return deliver_replay(&job, &bytes, &title, || {
            upload_replay(&client, &job, &bytes, &title)
        })
        .map(Some);
    }
    Ok(None)
}

struct UploadJob {
    tracked: TrackedMatch,
    upload_enabled: bool,
    local_dir: Option<PathBuf>,
    api_key: String,
    naming_template: String,
    visibility: String,
    group_id: String,
    generation: Arc<AtomicU64>,
    expected_generation: u64,
}

impl UploadJob {
    fn cancelled(&self) -> bool {
        self.generation.load(Ordering::Acquire) != self.expected_generation
    }
}

fn download_and_upload(
    job: &UploadJob,
    replay_url: &str,
    title: &str,
) -> Result<UploadHistoryEntry, String> {
    if job.cancelled() {
        return Err("Stopped".into());
    }
    let replay_url = validate_replay_url(replay_url)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(replay_url)
        .send()
        .map_err(|error| format!("could not download replay: {error}"))?
        .error_for_status()
        .map_err(|error| format!("could not download replay: {error}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_REPLAY_BYTES)
    {
        return Err("replay download was unexpectedly large".to_string());
    }
    use std::io::Read;
    let mut replay = Vec::new();
    response
        .take(MAX_REPLAY_BYTES + 1)
        .read_to_end(&mut replay)
        .map_err(|error| format!("could not read replay: {error}"))?;
    if replay.len() as u64 > MAX_REPLAY_BYTES {
        return Err("replay download was unexpectedly large".to_string());
    }
    validate_replay_bytes(&replay)?;
    if job.cancelled() {
        return Err("stopped".to_string());
    }

    deliver_replay(job, &replay, title, || {
        upload_replay(&client, job, &replay, title)
    })
}

// The local destination writes the downloaded bytes directly. It never calls
// Epic's replay upload service or the game's cloud-save APIs.
fn deliver_replay(
    job: &UploadJob,
    replay: &[u8],
    title: &str,
    upload: impl FnOnce() -> Result<UploadHistoryEntry, String>,
) -> Result<UploadHistoryEntry, String> {
    if job.cancelled() {
        return Err("stopped".to_string());
    }
    let local_result = job
        .local_dir
        .as_ref()
        .map(|directory| save_local_replay(directory, &job.tracked.match_guid, replay));
    let mut entry = UploadHistoryEntry {
        match_guid: job.tracked.match_guid.clone(),
        replay_id: String::new(),
        location: String::new(),
        title: title.to_string(),
        warning: String::new(),
        duplicate: false,
        local_path: String::new(),
        recorded_at: 0,
        player_id: String::new(),
        source_hash: String::new(),
    };
    if job.upload_enabled {
        if job.cancelled() {
            return Err("stopped".to_string());
        }
        match upload() {
            Ok(uploaded) => entry = uploaded,
            Err(error) if matches!(&local_result, Some(Ok(_))) => {
                entry.warning = format!("Replay saved locally, but upload failed: {error}");
            }
            Err(error) => return Err(error),
        }
    }
    match local_result {
        Some(Ok(path)) => entry.local_path = path.to_string_lossy().into_owned(),
        Some(Err(error)) if !entry.replay_id.is_empty() => {
            if !entry.warning.is_empty() {
                entry.warning.push('\n');
            }
            entry
                .warning
                .push_str(&format!("Replay uploaded, but local save failed: {error}"));
        }
        Some(Err(error)) => return Err(error),
        None => {}
    }
    entry.source_hash = format!("{:x}", Sha256::digest(replay));
    Ok(entry)
}

fn upload_replay(
    client: &reqwest::blocking::Client,
    job: &UploadJob,
    replay: &[u8],
    title: &str,
) -> Result<UploadHistoryEntry, String> {
    let mut url = reqwest::Url::parse("https://ballchasing.com/api/v2/upload")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("visibility", visibility(&job.visibility));
    if !job.group_id.is_empty() {
        url.query_pairs_mut().append_pair("group", &job.group_id);
    }
    let part = reqwest::blocking::multipart::Part::bytes(replay.to_vec())
        .file_name(format!("{}.replay", job.tracked.match_guid))
        .mime_str("application/octet-stream")
        .map_err(|error| error.to_string())?;
    let response = client
        .post(url)
        .header(reqwest::header::AUTHORIZATION, &job.api_key)
        .multipart(reqwest::blocking::multipart::Form::new().part("file", part))
        .send()
        .map_err(|error| format!("ballchasing upload failed: {error}"))?;
    let status = response.status();
    let response_text = response
        .text()
        .map_err(|error| format!("could not read ballchasing response ({status}): {error}"))?;
    let body: Value = serde_json::from_str(&response_text)
        .map_err(|error| format!("ballchasing returned invalid JSON ({status}): {error}"))?;
    if status.as_u16() != 201 && status.as_u16() != 409 {
        let detail = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request rejected");
        return Err(format!("ballchasing returned {status}: {detail}"));
    }
    let replay_id = body
        .get("id")
        .and_then(Value::as_str)
        .ok_or("ballchasing response did not include a replay id")?
        .to_string();
    let location = body
        .get("location")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://ballchasing.com/replay/{replay_id}"));
    let warning = if title.is_empty() || status.as_u16() == 409 {
        String::new()
    } else {
        patch_replay_title(&client, &job.api_key, &replay_id, title)
            .err()
            .map(|error| format!("Replay uploaded, but its name could not be set: {error}"))
            .unwrap_or_default()
    };
    Ok(UploadHistoryEntry {
        match_guid: job.tracked.match_guid.clone(),
        replay_id,
        location,
        title: title.to_string(),
        warning,
        duplicate: status.as_u16() == 409,
        local_path: String::new(),
        recorded_at: 0,
        player_id: String::new(),
        source_hash: String::new(),
    })
}

pub fn replay_directory(game_path: &Path, launch: &RlLaunchCfg) -> Result<PathBuf, String> {
    use hebnix_sdk::process::{RlPlatform, detect_platform};
    let steam = match detect_platform(game_path) {
        RlPlatform::Steam => true,
        RlPlatform::Epic => false,
        RlPlatform::Unknown => launch.mode == RlLaunchMode::SteamNative,
    };
    let documents = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .filter(|path| path.is_absolute())
        .ok_or("Documents folder is unavailable")?;
    Ok(replay_directory_in(&documents, steam))
}

pub fn configured_replay_directory(
    config: &ReplayUploadCfg,
    game_path: &Path,
    launch: &RlLaunchCfg,
) -> Result<PathBuf, String> {
    let directory = config.save_directory.trim();
    if directory.is_empty() {
        return replay_directory(game_path, launch);
    }
    let path = PathBuf::from(directory);
    if !path.is_absolute() {
        return Err("Choose an absolute replay save folder".into());
    }
    Ok(path)
}

fn replay_directory_in(documents: &Path, steam: bool) -> PathBuf {
    documents
        .join("My Games")
        .join("Rocket League")
        .join("TAGame")
        .join(if steam { "Demos" } else { "DemosEpic" })
}

fn validate_replay_bytes(replay: &[u8]) -> Result<(), String> {
    let invalid =
        || "The replay download is incomplete or is not a Rocket League replay".to_string();
    let header_len = replay
        .get(..4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        .ok_or_else(invalid)?;
    let header_end = header_len.checked_add(8).ok_or_else(invalid)?;
    let body_start = header_end.checked_add(8).ok_or_else(invalid)?;
    let header = replay.get(8..header_end).ok_or_else(invalid)?;
    if !header
        .windows(b"TAGame.Replay".len())
        .any(|w| w == b"TAGame.Replay")
    {
        return Err(invalid());
    }
    let body_len = replay
        .get(header_end..header_end + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        .ok_or_else(invalid)?;
    if body_len == 0 || body_start.checked_add(body_len) != Some(replay.len()) {
        return Err(invalid());
    }
    Ok(())
}

fn save_local_replay(directory: &Path, match_guid: &str, replay: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    if match_guid.is_empty()
        || match_guid.len() > 128
        || !match_guid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("match history returned an invalid match GUID".into());
    }
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("could not create replay folder: {e}"))?;
    let path = directory.join(format!("{match_guid}.replay"));
    if path.exists() {
        return if std::fs::read(&path).ok().as_deref() == Some(replay) {
            Ok(path)
        } else {
            Err(format!(
                "a different replay already exists at {}",
                path.display()
            ))
        };
    }
    // Rocket League only sees the .replay name after the complete file is synced.
    let temporary = directory.join(format!(
        "{match_guid}.{:016x}.partial",
        rand::random::<u64>()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| format!("could not save replay: {e}"))?;
    let written = file.write_all(replay).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("could not save replay: {error}"));
    }
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        if std::fs::read(&path).ok().as_deref() != Some(replay) {
            return Err(format!("could not finish saving replay: {error}"));
        }
    }
    Ok(path)
}

fn replay_outcome(entry: &UploadHistoryEntry) -> &'static str {
    match (
        !entry.local_path.is_empty(),
        !entry.replay_id.is_empty(),
        entry.duplicate,
    ) {
        (true, true, true) => "Saved locally · already uploaded",
        (true, true, false) => "Saved locally · uploaded",
        (true, false, _) => "Saved locally",
        (false, _, true) => "Already uploaded",
        _ => "Uploaded",
    }
}

fn patch_replay_title(
    client: &reqwest::blocking::Client,
    api_key: &str,
    replay_id: &str,
    title: &str,
) -> Result<(), String> {
    let response = client
        .patch(format!("https://ballchasing.com/api/replays/{replay_id}"))
        .header(reqwest::header::AUTHORIZATION, api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({ "title": title }).to_string())
        .send()
        .map_err(|error| error.to_string())?;
    if response.status().as_u16() == 204 {
        Ok(())
    } else {
        Err(format!("ballchasing returned {}", response.status()))
    }
}

fn render_replay_title(template: &str, entry: &Value) -> String {
    let details = entry.get("Match").unwrap_or(&Value::Null);
    let timestamp = details
        .get("RecordStartTimestamp")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let winning_team = match details.get("WinningTeam").and_then(Value::as_i64) {
        Some(0) => "Blue",
        Some(1) => "Orange",
        _ => "Unknown",
    };
    let playlist = details
        .get("Playlist")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let match_guid = details
        .get("MatchGUID")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let map = details
        .get("MapName")
        .and_then(Value::as_str)
        .unwrap_or("Unknown Map");
    let (date, time, time24) = local_times(timestamp);
    template
        .replace("{date}", &date)
        .replace("{time24}", &time24)
        .replace("{time}", &time)
        .replace("{winning_team}", winning_team)
        .replace("{gamemode}", &gamemode_name(playlist))
        .replace("{map}", map)
        .replace("{match_guid}", match_guid)
        .trim()
        .chars()
        .take(200)
        .collect()
}

fn local_times(timestamp: i64) -> (String, String, String) {
    let utc = time::OffsetDateTime::from_unix_timestamp(timestamp)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let offset = time::UtcOffset::local_offset_at(utc).unwrap_or(time::UtcOffset::UTC);
    date_and_times(utc.to_offset(offset))
}

fn date_and_times(local: time::OffsetDateTime) -> (String, String, String) {
    let hour24 = local.hour();
    let minute = local.minute();
    let suffix = if hour24 < 12 { "AM" } else { "PM" };
    let hour12 = match hour24 % 12 {
        0 => 12,
        hour => hour,
    };
    (
        format!(
            "{:04}-{:02}-{:02}",
            local.year(),
            local.month() as u8,
            local.day()
        ),
        format!("{hour12}:{minute:02} {suffix}"),
        format!("{hour24:02}:{minute:02}"),
    )
}

fn gamemode_name(playlist: i64) -> String {
    hebnix_sdk::utils::playlists::display_name(playlist)
}

fn validate_replay_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| "PsyNet returned an invalid replay URL".to_string())?;
    let trusted_host = url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("api.rlpp.psynet.gg"));
    if !trusted_host
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("PsyNet returned an invalid replay URL".to_string());
    }
    Ok(url)
}

fn visibility(value: &str) -> &'static str {
    match value {
        "public" => "public",
        "unlisted" => "unlisted",
        _ => "private",
    }
}

fn normalize_guid(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}
