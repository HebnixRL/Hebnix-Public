//! RL process detection + steam/epic platform id (by files in the game dir).

use std::path::{Path, PathBuf};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::utils::constants::{SAVE_PATH_EPIC, SAVE_PATH_STEAM};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RlPlatform {
    Steam,
    Epic,
    Unknown,
}

impl RlPlatform {
    pub fn as_str(&self) -> &'static str {
        match self {
            RlPlatform::Steam => "steam",
            RlPlatform::Epic => "epic",
            RlPlatform::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RlProcessInfo {
    pub pid: u32,
    pub exe_path: PathBuf,
    pub root_dir: PathBuf,
    pub platform: RlPlatform,
    pub save_data_path: PathBuf,
}

/// walk up from exe_path to the game root (the folder with Binaries/ and
/// Engine/ or TAGame/)
fn find_root_dir(exe_path: &Path) -> Option<PathBuf> {
    let mut current = exe_path.parent()?.to_path_buf();
    for _ in 0..10 {
        let bins = current.join("Binaries");
        let engine = current.join("Engine");
        let tagame = current.join("TAGame");
        if bins.is_dir() && (engine.is_dir() || tagame.is_dir()) {
            return Some(current);
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => break,
        }
    }
    None
}

/// steam vs epic from the root dir: appinfo.vdf = steam, .egstore = epic, else
/// sniff the path for steamapps/steamlibrary or epic games.
pub fn detect_platform(root_dir: &Path) -> RlPlatform {
    if root_dir.join("appinfo.vdf").exists() {
        return RlPlatform::Steam;
    }
    if root_dir.join(".egstore").is_dir() {
        return RlPlatform::Epic;
    }
    let root_str = root_dir.to_string_lossy().to_lowercase();
    if root_str.contains("steamapps") || root_str.contains("steamlibrary") {
        return RlPlatform::Steam;
    }
    if root_str.contains("epic games") || root_str.contains("epicgames") {
        return RlPlatform::Epic;
    }
    RlPlatform::Unknown
}

fn documents_dir() -> PathBuf {
    crate::utils::paths::documents_dir().unwrap_or_else(|| PathBuf::from("Documents"))
}

/// DBE_Production/ under a Documents folder: the platform's own when known,
/// else whichever of Steam's and Epic's has the newest .save
pub fn save_data_dir_in(docs: &Path, platform: RlPlatform) -> PathBuf {
    match platform {
        RlPlatform::Steam => docs.join(SAVE_PATH_STEAM),
        RlPlatform::Epic => docs.join(SAVE_PATH_EPIC),
        RlPlatform::Unknown => [SAVE_PATH_STEAM, SAVE_PATH_EPIC]
            .into_iter()
            .map(|rel| docs.join(rel))
            .filter_map(|dir| Some((newest_save_time(&dir)?, dir)))
            .max_by_key(|(modified, _)| *modified)
            .map(|(_, dir)| dir)
            .unwrap_or_else(|| docs.join(SAVE_PATH_STEAM)),
    }
}

fn newest_save_time(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("save"))
        })
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
}

/// full path to DBE_Production/ for the platform
pub fn get_save_data_path(platform: RlPlatform) -> PathBuf {
    if let Some(docs) = super::wine_prefix::rocket_league_documents_dir() {
        return save_data_dir_in(&docs, platform);
    }
    let docs = documents_dir();
    match platform {
        RlPlatform::Steam => docs.join(SAVE_PATH_STEAM),
        RlPlatform::Epic => docs.join(SAVE_PATH_EPIC),
        RlPlatform::Unknown => {
            for rel in [SAVE_PATH_STEAM, SAVE_PATH_EPIC] {
                let candidate = docs.join(rel);
                if candidate.is_dir() {
                    return candidate;
                }
            }
            docs.join(SAVE_PATH_STEAM) // best guess
        }
    }
}

/// name-only check for a running RocketLeague.exe. works even when the exe
/// path is unreadable (eac protects it), unlike find_rocket_league.
/// Cached for half a second: this walks all of /proc, and the app asks
/// several times per frame, which starved the UI thread (and with it the
/// plugin overlays) on desktops with many processes. Anything that must
/// know the game is closed right now uses `rocket_league_running_checked`.
pub fn is_rocket_league_running() -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, running)) = *cache {
        if at.elapsed() < Duration::from_millis(500) {
            return running;
        }
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let running = sys.processes().values().any(|p| {
        p.name()
            .to_string_lossy()
            .to_lowercase()
            .contains("rocketleague")
    });
    *cache = Some((Instant::now(), running));
    running
}

/// Fallible name-only enumeration for operations that require confirmed closure.
/// An enumeration failure is unknown, never evidence that the game is closed.
pub fn rocket_league_running_checked() -> Result<bool, String> {
    // linux-port: walk /proc directly. RL runs under Wine/Proton, where comm
    // is the (truncated) exe name, e.g. "RocketLeague.ex".
    let entries =
        std::fs::read_dir("/proc").map_err(|e| format!("could not list processes: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not list processes: {e}"))?;
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // a process can exit between the listing and this read; that's not
        // a failure, it's just not running anymore
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        if comm.trim().to_lowercase().contains("rocketleague") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// find the running RL process + its info. None if not running or the exe
/// path isn't readable (eac), use is_rocket_league_running for plain liveness.
pub fn find_rocket_league() -> Option<RlProcessInfo> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always),
    );

    for (pid, proc_) in sys.processes() {
        let name = proc_.name().to_string_lossy().to_lowercase();
        if !name.contains("rocketleague") {
            continue;
        }
        let Some(exe) = proc_.exe() else {
            continue;
        };
        if !exe.exists() {
            continue;
        }
        let Some(root) = find_root_dir(exe) else {
            continue;
        };
        let platform = detect_platform(&root);
        // if the running exe lives inside a Wine/Proton prefix (it always
        // does - RL has no native Linux client anymore), use that exact
        // prefix's Documents dir rather than guessing between however many
        // prefixes happen to be installed.
        // the game is often installed outside its prefix (Heroic, a games
        // drive), so the exe path alone can't say which prefix it runs in
        let save_path = super::wine_prefix::documents_dir_for_exe(exe)
            .or_else(super::wine_prefix::rocket_league_documents_dir)
            .map(|docs| save_data_dir_in(&docs, platform))
            .unwrap_or_else(|| get_save_data_path(platform));

        return Some(RlProcessInfo {
            pid: pid.as_u32(),
            exe_path: exe.to_path_buf(),
            root_dir: root,
            platform,
            save_data_path: save_path,
        });
    }
    None
}

/// pid of the first process whose exe name contains `name` (case-insensitive).
/// Wine/Proton games show up under their Windows exe name, which the kernel
/// truncates to 15 bytes, so the full exe path's file name is checked too.
pub fn find_process(name: &str) -> Option<u32> {
    let needle = name.to_ascii_lowercase();
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
    );
    sys.processes()
        .iter()
        .find(|(_, process)| {
            process.name().to_string_lossy().to_lowercase().contains(&needle)
                || process
                    .exe()
                    .and_then(|exe| exe.file_name())
                    .is_some_and(|exe| exe.to_string_lossy().to_lowercase().contains(&needle))
        })
        .map(|(pid, _)| pid.as_u32())
}
