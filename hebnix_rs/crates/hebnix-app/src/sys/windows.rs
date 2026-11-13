//! windows bits: single-instance mutex, run-at-startup registry, focus handoff,
//! minimize hook, killing RL.

use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, Ordering};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    AttachThreadInput, CreateMutexW, GetCurrentProcessId, GetCurrentThreadId,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU, keybd_event,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST,
    IsIconic, IsWindow, IsWindowVisible, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetForegroundWindow, SetWindowPos, ShowWindow, SwitchToThisWindow,
};
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// grab the single-instance mutex. None if another instance already holds it.
/// handle is leaked on purpose for the process lifetime.
pub fn acquire_single_instance() -> Option<HANDLE> {
    let name = wide("Global\\hebnix_SingleInstanceMutex_v1");
    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))
            .map_err(|error| tracing::error!(%error, "could not open the single-instance guard"))
            .ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            tracing::warn!("another Hebnix instance is already running; this launch will exit without initializing graphics");
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            return None;
        }
        Some(handle)
    }
}

/// Our main window, registered from eframe rather than searched by title.
/// FindWindowW can wait on same-process windows on other threads.
pub fn main_window_hwnd() -> Option<HWND> {
    crate::window_state::main_window()
}

/// Pin/unpin the main window over everything (incl. the game) without
/// activating it. A topmost Hebnix is only useful while Hebnix or Rocket
/// League owns the foreground; never let it cover an unrelated application.
pub fn set_main_window_topmost(topmost: bool) {
    let topmost =
        topmost && (foreground_window_is_ours() || hebnix_sdk::process::is_rocket_league_focused());
    if let Some(hwnd) = main_window_hwnd() {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(if topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                }),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// a second instance calls this before exiting to surface the already-running
/// window, so launching twice isn't a silent no-op.
pub fn focus_existing_instance() {
    unsafe {
        let name = wide("Hebnix");
        if let Ok(hwnd) = FindWindowW(None, PCWSTR(name.as_ptr())) {
            if !hwnd.is_invalid() {
                if !IsWindowVisible(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

// focus handoff

static MAIN_HIDDEN: AtomicBool = AtomicBool::new(false);
static CAME_FROM_GAME: AtomicBool = AtomicBool::new(false);

fn window_is_ours(hwnd: HWND) -> bool {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

pub fn foreground_window_is_ours() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        !hwnd.is_invalid() && window_is_ours(hwnd)
    }
}

pub fn main_window_hidden() -> bool {
    MAIN_HIDDEN.load(Ordering::Relaxed)
}

struct DialogParent(HWND);

impl HasWindowHandle for DialogParent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(hwnd);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

impl HasDisplayHandle for DialogParent {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

pub fn parent_file_dialog(dialog: rfd::FileDialog) -> rfd::FileDialog {
    main_window_hwnd()
        .map(|hwnd| dialog.clone().set_parent(&DialogParent(hwnd)))
        .unwrap_or(dialog)
}

pub fn parent_message_dialog(dialog: rfd::MessageDialog) -> rfd::MessageDialog {
    main_window_hwnd()
        .map(|hwnd| dialog.clone().set_parent(&DialogParent(hwnd)))
        .unwrap_or(dialog)
}

fn focus_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // Windows only lets the process that got the last input event take the foreground,
        // and pad presses don't count. A tap of Alt is an input event from us, which is why
        // this works right after you click the panel and breaks once you click into the game.
        keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // a pad press isn't input Windows counts, so the lock above refuses. Sharing the
        // foreground thread's input state lifts it for this one call.
        let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let own_thread = GetCurrentThreadId();
        if foreground_thread != 0
            && foreground_thread != own_thread
            && AttachThreadInput(own_thread, foreground_thread, true).as_bool()
        {
            let focused = SetForegroundWindow(hwnd).as_bool();
            let _ = AttachThreadInput(own_thread, foreground_thread, false);
            if focused {
                return true;
            }
        }
        SwitchToThisWindow(hwnd, true); // alt-tab's path
        GetForegroundWindow() == hwnd
    }
}

/// note which program we came from. ours doesnt count, the monitor calls this
/// on its tick so it tracks whatever you were last actually in.
pub fn note_foreground() {
    if foreground_window_is_ours() {
        return;
    }
    CAME_FROM_GAME.store(
        hebnix_sdk::process::is_rocket_league_focused(),
        Ordering::Relaxed,
    );
}

/// hand the foreground back, but only when the game is what we came from.
pub fn restore_foreground() -> bool {
    // alt-tabbed off and hid it from there, whoever holds focus keeps it
    if !foreground_window_is_ours() {
        return false;
    }
    if !CAME_FROM_GAME.swap(false, Ordering::Relaxed) {
        return false; // one shot
    }
    let Some(hwnd) = hebnix_sdk::process::rocket_league_hwnd() else {
        return false;
    };
    // a minimised game stays minimised, thats the user's call
    if unsafe { IsIconic(hwnd).as_bool() } {
        return false;
    }
    focus_window(hwnd)
}

pub fn focus_main_window() -> bool {
    match main_window_hwnd() {
        Some(hwnd) => focus_window(hwnd),
        None => false,
    }
}

// minimize hook so that instead of minimizing it triggers the
// hide function since minimization cause bugs

const WM_SYSCOMMAND: u32 = 0x0112;
const SC_MINIMIZE: usize = 0xF020;

static MINIMIZE_REQUESTED: AtomicBool = AtomicBool::new(false);
static REPAINT_CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

pub fn take_minimize_request() -> bool {
    MINIMIZE_REQUESTED.swap(false, Ordering::Relaxed)
}

fn wake_ui() {
    if let Some(ctx) = REPAINT_CTX.get() {
        ctx.request_repaint();
    }
}

unsafe extern "system" fn minimize_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        // low 4 bits are reserved
        if msg == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_MINIMIZE {
            MINIMIZE_REQUESTED.store(true, Ordering::Relaxed);
            wake_ui();
            return LRESULT(0);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

pub fn install_minimize_hook(hwnd: HWND, ctx: &eframe::egui::Context) {
    let _ = REPAINT_CTX.set(ctx.clone());
    unsafe {
        if !SetWindowSubclass(hwnd, Some(minimize_proc), 0x4842_584D /* "HBXM" */, 0).as_bool() {
            tracing::warn!("minimize hook not installed, the button will still minimize");
        }
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Hebnix";

/// is hebnix set to start with windows
pub fn is_startup_enabled() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

/// toggle start-with-windows via the HKCU Run key
pub fn set_startup_enabled(enabled: bool) -> std::io::Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY)?;
    if enabled {
        let exe = std::env::current_exe()?;
        key.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()))?;
    } else {
        let _ = key.delete_value(RUN_VALUE);
    }
    Ok(())
}

/// force-kill RocketLeague.exe (console quit command)
pub fn kill_rocket_league() -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("taskkill")
        .args(["/F", "/IM", "RocketLeague.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .and_then(|output| {
            if output.status.success() || !hebnix_sdk::process::is_rocket_league_running() {
                return Ok(());
            }

            // Rocket League can be elevated independently of Hebnix.  A
            // normal user process cannot terminate an elevated process, so
            // retry via Windows' on-demand UAC prompt instead of requiring
            // the whole app to be launched as administrator.
            if !crate::spoofer::is_admin() && kill_rocket_league_elevated()? {
                return Ok(());
            }

            Err(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ))
        })
}

fn kill_rocket_league_elevated() -> std::io::Result<bool> {
    #[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "$process = Start-Process -FilePath 'taskkill.exe' -ArgumentList '/F','/IM','RocketLeague.exe' -Verb RunAs -Wait -PassThru -ErrorAction Stop; exit $process.ExitCode",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    Ok(output.status.success() || !hebnix_sdk::process::is_rocket_league_running())
}

/// Remove Rocket League's embedded-browser cache, if it exists, including
/// when Windows redirects Documents to OneDrive or another drive/share.
pub fn clear_rocket_league_web_cache() -> std::io::Result<()> {
    let documents = hebnix_sdk::utils::paths::documents_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Documents folder is unavailable",
        )
    })?;
    if !documents.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Documents folder must be an absolute path",
        ));
    }
    let web_cache = documents.join(r"My Games\Rocket League\TAGame\Cache\WebCache");

    match std::fs::remove_dir_all(web_cache) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

const EPIC_LAUNCH_URI: &str = "com.epicgames.launcher://apps/9773aa1aa54f4f7b80e44bef04986cea%3A530145df28a24424923f5828cc9031a1%3ASugar?action=launch&silent=true";

// The monitor updates SettingsCfg.rl_path from the running process before a
// launch command is handled. Steam installs can live in any library, so use
// the Steam-owned directory markers rather than a fixed drive/path.
fn is_steam_path(game_path: &std::path::Path) -> bool {
    let path = game_path.to_string_lossy().to_ascii_lowercase();
    path.contains("steamapps") || path.contains("steam\\common") || path.contains("steam/library")
}

/// steam://... or com.epicgames.launcher://... for the modes that just need
/// a URI opened via `cmd /C start` - everything except Heroic, which spawns
/// its own binary directly (see rl_launch::heroic_launch).
fn simple_launch_uri(cfg: &crate::config::RlLaunchCfg, game_path: &std::path::Path) -> String {
    use crate::config::RlLaunchMode;
    match cfg.mode {
        // a non-Steam shortcut still gets a plain restart THROUGH Steam
        // (steam overlay/input/presence all work here) - only Workshop
        // LAN's -multihome relaunch has to bypass Steam for this mode,
        // since steam://run's argument override refuses non-Steam
        // shortcuts outright.
        RlLaunchMode::SteamNative | RlLaunchMode::SteamShortcutToHeroic => {
            format!("steam://rungameid/{}", cfg.steam_id)
        }
        RlLaunchMode::EpicDirect => EPIC_LAUNCH_URI.to_string(),
        RlLaunchMode::Unconfigured | RlLaunchMode::HeroicDirect => {
            if is_steam_path(game_path) {
                "steam://rungameid/252950".to_string()
            } else {
                EPIC_LAUNCH_URI.to_string()
            }
        }
    }
}

pub fn start_rocket_league(
    cfg: &crate::config::RlLaunchCfg,
    game_path: &std::path::Path,
) -> std::io::Result<()> {
    use crate::config::RlLaunchMode;
    #[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if cfg.mode == RlLaunchMode::HeroicDirect {
        return crate::rl_launch::heroic_launch(cfg, None).map_err(std::io::Error::other);
    }

    let launch = simple_launch_uri(cfg, game_path);
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &launch])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

pub fn restart_rocket_league(
    cfg: &crate::config::RlLaunchCfg,
    game_path: &std::path::Path,
) -> std::io::Result<()> {
    use crate::config::RlLaunchMode;
    #[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = kill_rocket_league();
    for _ in 0..60 {
        let running = std::process::Command::new("tasklist")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains("rocketleague.exe")
            })
            .unwrap_or(false);
        if !running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    if cfg.mode == RlLaunchMode::HeroicDirect {
        return crate::rl_launch::heroic_launch(cfg, None).map_err(std::io::Error::other);
    }

    let launch = simple_launch_uri(cfg, game_path);
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &launch])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

pub fn restart_rocket_league_multihome(
    cfg: &crate::config::RlLaunchCfg,
    game_path: &std::path::Path,
    address: &str,
) -> Result<(), String> {
    use crate::config::RlLaunchMode;
    #[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    kill_rocket_league().map_err(|error| error.to_string())?;
    for _ in 0..60 {
        if !hebnix_sdk::process::is_rocket_league_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    if matches!(
        cfg.mode,
        RlLaunchMode::SteamShortcutToHeroic | RlLaunchMode::HeroicDirect
    ) {
        return crate::rl_launch::heroic_launch(cfg, Some(address));
    }

    let is_steam = match cfg.mode {
        RlLaunchMode::SteamNative => true,
        RlLaunchMode::EpicDirect => false,
        RlLaunchMode::Unconfigured => is_steam_path(game_path),
        RlLaunchMode::SteamShortcutToHeroic | RlLaunchMode::HeroicDirect => unreachable!(),
    };
    if is_steam {
        let steam_id = if cfg.mode == RlLaunchMode::SteamNative {
            cfg.steam_id.as_str()
        } else {
            "252950"
        };
        let launch = format!("steam://run/{steam_id}//-multihome%3D{address}/");
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &launch])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    apply_epic_multihome(address)?;
    restart_epic_launcher_for_multihome()?;
    std::process::Command::new("cmd")
        .args(["/C", "start", "", EPIC_LAUNCH_URI])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn stop_epic_launcher() -> Result<(), String> {
    restart_epic_launcher_for_multihome()
}

fn restart_epic_launcher_for_multihome() -> Result<(), String> {
    #[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "EpicGamesLauncher.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    for _ in 0..40 {
        let running = std::process::Command::new("tasklist")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains("epicgameslauncher.exe")
            })
            .unwrap_or(false);
        if !running {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(
        "Epic Games Launcher did not close so its launch options could not be refreshed"
            .to_string(),
    )
}

pub fn clear_rocket_league_multihome() -> Result<(), String> {
    clear_epic_multihome()
}

fn apply_epic_multihome(address: &str) -> Result<(), String> {
    let config_root = dirs::data_local_dir()
        .ok_or_else(|| "could not find LocalAppData".to_string())?
        .join("EpicGamesLauncher")
        .join("Saved")
        .join("Config");
    let paths = [
        config_root
            .join("WindowsEditor")
            .join("GameUserSettings.ini"),
        config_root.join("Windows").join("GameUserSettings.ini"),
    ];
    let command_key = "9773aa1aa54f4f7b80e44bef04986cea:530145df28a24424923f5828cc9031a1:Sugar_AdditionalCommands";
    let enabled_key = "9773aa1aa54f4f7b80e44bef04986cea:530145df28a24424923f5828cc9031a1:Sugar_AdditionalCommandsEnabled";
    let mut changed = false;
    for path in paths.into_iter().filter(|path| path.is_file()) {
        let original = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
        let prefixes = lines
            .iter()
            .filter_map(|line| {
                line.strip_prefix('[')
                    .and_then(|line| line.strip_suffix("_Launcher]"))
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        for prefix in prefixes {
            let settings_section = format!("[{prefix}_Settings]");
            let settings = match lines.iter().position(|line| line == &settings_section) {
                Some(index) => index,
                None => {
                    lines.push(settings_section);
                    lines.len() - 1
                }
            };
            let end = lines
                .iter()
                .skip(settings + 1)
                .position(|line| line.starts_with('['))
                .map(|offset| settings + 1 + offset)
                .unwrap_or(lines.len());
            let mut command_found = false;
            let mut enabled_found = false;
            for line in &mut lines[settings + 1..end] {
                if let Some((key, value)) = line.split_once('=') {
                    if key == command_key {
                        let mut arguments = value
                            .split_whitespace()
                            .filter(|argument| !argument.starts_with("-multihome="))
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        arguments.push(format!("-multihome={address}"));
                        *line = format!("{command_key}={}", arguments.join(" "));
                        command_found = true;
                    } else if key == enabled_key {
                        *line = format!("{enabled_key}=True");
                        enabled_found = true;
                    }
                }
            }
            if !command_found {
                lines.insert(end, format!("{command_key}=-multihome={address}"));
            }
            if !enabled_found {
                let enabled_at = if command_found { end } else { end + 1 };
                lines.insert(enabled_at, format!("{enabled_key}=True"));
            }
            changed = true;
        }
        if lines.join("\n") != original.replace("\r\n", "\n") {
            std::fs::write(path, lines.join("\r\n")).map_err(|error| error.to_string())?;
        }
    }
    if changed {
        Ok(())
    } else {
        Err("Epic Games Launcher settings were not found".to_string())
    }
}

fn clear_epic_multihome() -> Result<(), String> {
    let backup = crate::config::base_dir()
        .join("state")
        .join("epic_multihome_backup.txt");
    if let Ok(contents) = std::fs::read_to_string(&backup) {
        if let Some((path, original)) = contents.split_once('\n') {
            std::fs::write(path, original).map_err(|error| error.to_string())?;
            let _ = std::fs::remove_file(&backup);
            // same reason as below - the launcher only reads this file on
            // startup, so it needs restarting for the reverted setting to
            // actually take effect instead of silently launching with the
            // stale -multihome argument next time
            restart_epic_launcher_for_multihome()?;
            return Ok(());
        }
    }
    let config_root = dirs::data_local_dir()
        .ok_or_else(|| "could not find LocalAppData".to_string())?
        .join("EpicGamesLauncher")
        .join("Saved")
        .join("Config");
    let paths = [
        config_root
            .join("WindowsEditor")
            .join("GameUserSettings.ini"),
        config_root.join("Windows").join("GameUserSettings.ini"),
    ];
    let mut any_changed = false;
    for path in paths.into_iter().filter(|path| path.is_file()) {
        let original = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
        let mut changed = false;
        for line in &mut lines {
            if let Some((key, value)) = line.split_once('=') {
                if key.ends_with(":Sugar_AdditionalCommands") {
                    // tailnet IPs aren't a fixed subnet Hebnix can pattern-match
                    // (headscale assigns them dynamically), so strip any
                    // -multihome argument outright -- Hebnix is the only thing
                    // that ever sets one.
                    let remaining = value
                        .split_whitespace()
                        .filter(|argument| !argument.starts_with("-multihome="))
                        .collect::<Vec<_>>()
                        .join(" ");
                    if remaining != value {
                        *line = format!("{key}={remaining}");
                        changed = true;
                    }
                }
            }
        }
        let commands_empty = lines.iter().all(|line| {
            line.split_once('=')
                .filter(|(key, _)| key.ends_with(":Sugar_AdditionalCommands"))
                .is_none_or(|(_, value)| value.trim().is_empty())
        });
        if commands_empty {
            for line in &mut lines {
                if let Some((key, value)) = line.split_once('=')
                    && key.ends_with(":Sugar_AdditionalCommandsEnabled")
                    && !value.eq_ignore_ascii_case("false")
                {
                    *line = format!("{key}=False");
                    changed = true;
                }
            }
        }
        if changed {
            std::fs::write(&path, lines.join("\r\n")).map_err(|error| error.to_string())?;
            any_changed = true;
        }
    }
    let _ = std::fs::remove_file(backup);
    if any_changed {
        // mirrors apply_epic_multihome's own restart -- the launcher only
        // reads GameUserSettings.ini on startup, so without this it keeps
        // running with the stale -multihome argument cached in memory and
        // launches Rocket League with it again next time regardless of
        // what's now on disk
        restart_epic_launcher_for_multihome()?;
    }
    Ok(())
}

/// hide by dropping the os alpha to 0. SW_HIDE crashes wgpu and takes the
/// plugin child windows with it. an alpha 0 window is still a normal one to the
/// os though, so NOACTIVATE stops it being handed the foreground when whatever
/// sat on top goes away, and TOOLWINDOW drops it from alt-tab.
pub fn set_main_window_invisible(invisible: bool) {
    MAIN_HIDDEN.store(invisible, Ordering::Relaxed);
    if let Some(hwnd) = main_window_hwnd() {
        crate::window_state::apply_window_visibility(hwnd, invisible);
    }
}
