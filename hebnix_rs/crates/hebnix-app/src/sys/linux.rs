//! linux bits: single-instance lock, run-at-startup (.desktop autostart),
//! focus handoff, kill/restart RL, epic multihome ini editing.
//!
//! There's no portable Wayland equivalent of Win32's window-management APIs
//! (SetForegroundWindow, topmost z-order, SetLayeredWindowAttributes, etc) --
//! those are compositor-side concerns on Wayland, not something a client can
//! do to itself or to other windows. The handful of functions that did that
//! on Windows (`set_main_window_topmost`, `focus_main_window`,
//! `set_main_window_invisible`'s visual layering, the minimize subclass hook)
//! are kept as best-effort no-ops here so callers don't need to change; if
//! Hyprland's IPC grows the right requests later this is the place to wire
//! them in (see `hebnix_sdk::process::window` for the existing Hyprland IPC
//! client pattern).

use std::sync::atomic::{AtomicBool, Ordering};

/// linux has no HWND; kept as an opaque unit so call sites (`if let
/// Some(hwnd) = winutil::main_window_hwnd()`) don't need to change.
pub type WindowHandle = ();

fn runtime_dir() -> std::path::PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

fn lock_path(name: &str) -> std::path::PathBuf {
    runtime_dir().join(format!("hebnix_{name}.lock"))
}

/// flock-based single-instance guard. the fd is leaked (kept open) for the
/// process lifetime -- the lock releases automatically when the process
/// exits or dies, same "don't bother cleaning up" behavior as the windows
/// named-mutex version.
pub struct SingleInstanceLock {
    _file: std::fs::File,
}

fn acquire_lock(name: &str) -> Option<SingleInstanceLock> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let path = lock_path(name);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&path)
        .ok()?;
    match nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockExclusiveNonblock) {
        Ok(()) => Some(SingleInstanceLock { _file: file }),
        Err(_) => None, // already held by another instance
    }
}

pub fn acquire_single_instance() -> Option<SingleInstanceLock> {
    acquire_lock("single_instance")
}

/// a second instance calls this before exiting. We have no reliable
/// client-side way to raise another process's window on Wayland, so this
/// just logs -- the user has to alt-tab/click the taskbar entry themselves.
pub fn focus_existing_instance() {
    if hebnix_sdk::process::raise_other_instance(std::process::id()) {
        return;
    }
    tracing::info!("hebnix is already running (another instance holds the single-instance lock)");
}

/// no HWND concept on Wayland; always None (logged once) so callers fall
/// back to their "not found at startup" path.
pub fn main_window_hwnd() -> Option<WindowHandle> {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::debug!(
            "winutil: no client-side window handle on Wayland, minimize-hook/dpi-fix are no-ops"
        );
    });
    None
}

/// Windows parents file dialogs to the Hebnix window; there is no equivalent
/// to attach to here, so the dialog is returned unchanged.
pub fn parent_file_dialog(dialog: rfd::FileDialog) -> rfd::FileDialog {
    dialog
}

/// X11 / Sway / i3 only (`_NET_WM_STATE_ABOVE`, floating); elsewhere the
/// compositor decides stacking and this stays a no-op
pub fn set_main_window_topmost(topmost: bool) {
    if hebnix_sdk::process::can_track_own_window() {
        hebnix_sdk::process::set_own_always_on_top(std::process::id(), topmost);
    }
}

pub fn install_minimize_hook(_hwnd: WindowHandle, _ctx: &eframe::egui::Context) {}

// focus handoff

static MAIN_HIDDEN: AtomicBool = AtomicBool::new(false);
static CAME_FROM_GAME: AtomicBool = AtomicBool::new(false);

/// best-effort: on X11, Sway and i3 we ask the window manager whether we're
/// the active window; elsewhere (no reliable signal) assume not.
pub fn foreground_window_is_ours() -> bool {
    // only where the window manager can say so cheaply; Hyprland/KDE/niri
    // keep the old constant false, their callers rely on it
    hebnix_sdk::process::can_track_own_window()
        && hebnix_sdk::process::window::is_pid_focused(std::process::id())
}

/// is Hebnix's own window the focused one, asked of the compositor.
/// `foreground_window_is_ours` above stays a constant false because other
/// callers depend on it, so this is the real answer for anything that has to
/// know (it shells out to the compositor - don't call it in a tight loop).
pub fn hebnix_window_focused() -> bool {
    hebnix_sdk::process::window::is_pid_focused(std::process::id())
}

pub fn main_window_hidden() -> bool {
    MAIN_HIDDEN.load(Ordering::Relaxed)
}

pub fn note_foreground() {
    if foreground_window_is_ours() {
        return;
    }
    CAME_FROM_GAME.store(
        hebnix_sdk::process::is_rocket_league_focused(),
        Ordering::Relaxed,
    );
}

/// hand focus back to RL when Hebnix was opened from the game. A Wayland
/// client can't focus another app's window itself, so the compositor does it
/// (Hyprland, KWin and niri; elsewhere this does nothing).
pub fn restore_foreground() -> bool {
    CAME_FROM_GAME.load(Ordering::Relaxed)
        && hebnix_sdk::process::rocket_league_hwnd().is_some()
        && hebnix_sdk::process::focus_rocket_league()
}

/// Hyprland-only: pops our window on top of a fullscreened RL, on RL's own
/// workspace (mirrors BakkesMod's overlay-toggle behavior on Windows). The
/// real OS-level show is done separately via ViewportCommand::Visible from
/// the caller; this just handles raising + focus.
pub fn focus_main_window() -> bool {
    hebnix_sdk::process::focus_own_window_over_game(std::process::id())
}

static MINIMIZE_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn take_minimize_request() -> bool {
    MINIMIZE_REQUESTED.swap(false, Ordering::Relaxed)
}

pub fn take_show_request() -> bool {
    SHOW_REQUESTED.swap(false, Ordering::Relaxed)
}

/// "invisible" on windows dropped the layered-window alpha to 0; there's no
/// client-side equivalent on Wayland (a hidden toplevel is just unmapped by
/// the compositor when minimized). We track the intent so the rest of the
/// app's hidden-state logic (monitor.rs, overlay topmost decisions) keeps
/// working, we just can't actually hide the window ourselves here -- the
/// app's own egui viewport visibility (`ViewportCommand::Visible`) is what
/// should be driven from this flag at the call site.
pub fn set_main_window_invisible(invisible: bool) {
    MAIN_HIDDEN.store(invisible, Ordering::Relaxed);
}

/// Hyprland is the desktop the Linux defaults were tuned on (GPU-rendered
/// overlays, plain hide on the hotkey); everything else gets the KDE Plasma
/// defaults (software overlays, Minimize to Tray)
pub fn is_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|value| !value.is_empty())
}

// "Minimize to Tray" setting, mirrored here so the hotkey and tray threads
// can read it without going through the app
static MINIMIZE_TO_TRAY: AtomicBool = AtomicBool::new(false);

pub fn set_minimize_to_tray(enabled: bool) {
    MINIMIZE_TO_TRAY.store(enabled, Ordering::Relaxed);
}

pub fn minimize_to_tray() -> bool {
    MINIMIZE_TO_TRAY.load(Ordering::Relaxed)
}

/// called from the hotkey/tray threads before they ask the app to show the
/// window: a window minimized on Wayland stops getting redraws, so the app
/// would never see that request until the window is back. Only acts while
/// minimized to tray.
pub fn wake_minimized_main_window() {
    if minimize_to_tray() && main_window_hidden() {
        hebnix_sdk::process::unminimize_own_window();
    }
}

// run-at-startup via XDG autostart

fn autostart_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("autostart")
        .join("hebnix.desktop")
}

pub fn is_startup_enabled() -> bool {
    autostart_path().is_file()
}

pub fn set_startup_enabled(enabled: bool) -> std::io::Result<()> {
    let path = autostart_path();
    if enabled {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let exe = std::env::current_exe()?;
        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Hebnix\n\
             Comment=Rocket League companion tool\n\
             Exec=\"{}\"\n\
             Icon=hebnix\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&path, contents)
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

// kill / restart RL

pub fn kill_rocket_league() -> std::io::Result<()> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    for (_, proc_) in sys.processes() {
        if proc_
            .name()
            .to_string_lossy()
            .to_lowercase()
            .contains("rocketleague")
        {
            proc_.kill_with(Signal::Kill);
        }
    }
    Ok(())
}

pub fn start_rocket_league(
    launch_cfg: &crate::config::RlLaunchCfg,
    _game_path: &std::path::Path,
) -> std::io::Result<()> {
    crate::rl_launch::restart(launch_cfg).map_err(std::io::Error::other)
}

pub fn restart_rocket_league(
    launch_cfg: &crate::config::RlLaunchCfg,
    _game_path: &std::path::Path,
) -> std::io::Result<()> {
    kill_rocket_league()?;
    for _ in 0..60 {
        if !hebnix_sdk::process::is_rocket_league_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    crate::rl_launch::restart(launch_cfg).map_err(std::io::Error::other)
}

/// Workshop multiplayer "restart, but leave out -multihome" (experimental
/// option, keeps normal online play working)
pub fn restart_rocket_league_without_multihome(
    launch_cfg: &crate::config::RlLaunchCfg,
    _game_path: &std::path::Path,
) -> Result<(), String> {
    kill_rocket_league().map_err(|error| error.to_string())?;
    for _ in 0..60 {
        if !hebnix_sdk::process::is_rocket_league_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    crate::rl_launch::restart_for_multiplayer_without_multihome(launch_cfg)
}

/// Workshop LAN "restart with -multihome=<address>" -- see rl_launch.rs for
/// how this actually gets dispatched depending on the configured setup.
pub fn restart_rocket_league_multihome(
    launch_cfg: &crate::config::RlLaunchCfg,
    _game_path: &std::path::Path,
    address: &str,
) -> Result<(), String> {
    kill_rocket_league().map_err(|error| error.to_string())?;
    for _ in 0..60 {
        if !hebnix_sdk::process::is_rocket_league_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    crate::rl_launch::restart_multihome(launch_cfg, address)
}

/// Remove Rocket League's embedded-browser cache from every known Wine
/// prefix's Documents folder (and the host's own, for native layouts).
pub fn clear_rocket_league_web_cache() -> std::io::Result<()> {
    let rel = std::path::Path::new("My Games/Rocket League/TAGame/Cache/WebCache");
    let mut documents = hebnix_sdk::process::candidate_documents_dirs();
    if let Some(host_docs) = dirs::document_dir() {
        documents.push(host_docs);
    }
    let mut first_error = None;
    for docs in documents {
        match std::fs::remove_dir_all(docs.join(rel)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => first_error = first_error.or(Some(error)),
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub fn clear_rocket_league_multihome() -> Result<(), String> {
    // no-op: apply_epic_multihome's ini edits are Windows Epic-launcher-path
    // specific and multihome LAN isn't wired up on Linux (see
    // multiplayer_lan/mod.rs), so there's nothing to clear.
    Ok(())
}
