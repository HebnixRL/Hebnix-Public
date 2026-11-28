use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, Ordering};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST,
    IsIconic, IsWindow, SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetForegroundWindow, SetWindowPos, ShowWindow, SwitchToThisWindow,
};
use windows::core::PCWSTR;

static HIDDEN: AtomicBool = AtomicBool::new(false);
static CAME_FROM_GAME: AtomicBool = AtomicBool::new(false);

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main_window() -> Option<HWND> {
    crate::window_state::main_window()
}

pub fn acquire_single_instance() -> Option<HANDLE> {
    let name = wide("Global\\hebnix_LiteSingleInstanceMutex_v1");
    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))
            .map_err(|error| tracing::error!(%error, "could not open the single-instance guard"))
            .ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            tracing::warn!("another Hebnix Lite instance is already running; this launch will exit without initializing graphics");
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            None
        } else {
            Some(handle)
        }
    }
}

pub fn focus_existing_instance() {
    // Only the second-instance path needs to find another process's window.
    let title = wide("Hebnix Lite");
    if let Ok(window) = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) } {
        let _ = focus_window(window);
    }
}

pub fn main_window_hwnd() -> Option<HWND> {
    main_window()
}

pub fn foreground_window_is_ours() -> bool {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return false;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(window, Some(&mut process_id));
        process_id == GetCurrentProcessId()
    }
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
    main_window()
        .map(|hwnd| dialog.clone().set_parent(&DialogParent(hwnd)))
        .unwrap_or(dialog)
}

pub fn parent_message_dialog(dialog: rfd::MessageDialog) -> rfd::MessageDialog {
    main_window()
        .map(|hwnd| dialog.clone().set_parent(&DialogParent(hwnd)))
        .unwrap_or(dialog)
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

pub fn set_main_window_topmost(topmost: bool) {
    let topmost =
        topmost && (foreground_window_is_ours() || hebnix_sdk::process::is_rocket_league_focused());
    if let Some(window) = main_window() {
        unsafe {
            let _ = SetWindowPos(
                window,
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

fn focus_window(window: HWND) -> bool {
    unsafe {
        if !IsWindow(Some(window)).as_bool() {
            return false;
        }
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        if SetForegroundWindow(window).as_bool() {
            return true;
        }
        SwitchToThisWindow(window, true);
        GetForegroundWindow() == window
    }
}

pub fn focus_main_window() {
    if let Some(window) = main_window() {
        let _ = focus_window(window);
    }
}

/// hand the foreground back, but only when the game is what we came from.
pub fn focus_rocket_league() {
    // alt-tabbed off and hid it from there, whoever holds focus keeps it
    if !foreground_window_is_ours() {
        return;
    }
    if !CAME_FROM_GAME.swap(false, Ordering::Relaxed) {
        return; // one shot
    }
    if let Some(window) = hebnix_sdk::process::rocket_league_hwnd() {
        unsafe {
            // a minimised game stays minimised, thats the user's call
            if IsIconic(window).as_bool() {
                return;
            }
        }
        let _ = focus_window(window);
    }
}

pub fn install_minimize_hook(_: HWND, _: &eframe::egui::Context) {}

pub fn main_window_hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

/// Keep Lite's focus policy in sync with the shared native visibility state.
pub fn set_main_window_invisible(invisible: bool) {
    HIDDEN.store(invisible, Ordering::Relaxed);
    if let Some(window) = main_window() {
        crate::window_state::apply_window_visibility(window, invisible);
    }
}

pub fn start_rocket_league(game_path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let path = game_path.to_string_lossy().to_ascii_lowercase();
    let launch = if path.contains("steamapps")
        || path.contains("steam\\common")
        || path.contains("steam/library")
    {
        "steam://rungameid/252950"
    } else {
        "com.epicgames.launcher://apps/9773aa1aa54f4f7b80e44bef04986cea%3A530145df28a24424923f5828cc9031a1%3ASugar?action=launch&silent=true"
    };
    std::process::Command::new("cmd")
        .args(["/C", "start", "", launch])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}
pub fn restart_rocket_league(game_path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "RocketLeague.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    for _ in 0..60 {
        let running = hebnix_sdk::process::is_rocket_league_running();
        if !running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let path = game_path.to_string_lossy().to_ascii_lowercase();
    let launch = if path.contains("steamapps")
        || path.contains("steam\\common")
        || path.contains("steam/library")
    {
        "steam://rungameid/252950"
    } else {
        "com.epicgames.launcher://apps/9773aa1aa54f4f7b80e44bef04986cea%3A530145df28a24424923f5828cc9031a1%3ASugar?action=launch&silent=true"
    };
    std::process::Command::new("cmd")
        .args(["/C", "start", "", launch])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Hebnix Lite";

pub fn is_startup_enabled() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

pub fn kill_rocket_league() -> std::io::Result<()> {
    hebnix_sdk::process::window::close_rocket_league()
}

pub fn clear_rocket_league_web_cache() -> std::io::Result<()> {
    let documents = hebnix_sdk::utils::paths::documents_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Documents folder is unavailable",
        )
    })?;
    let web_cache = documents.join(r"My Games\Rocket League\TAGame\Cache\WebCache");
    match std::fs::remove_dir_all(web_cache) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn set_startup_enabled(enabled: bool) -> std::io::Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY)?;
    if enabled {
        key.set_value(
            RUN_VALUE,
            &format!("\"{}\"", std::env::current_exe()?.display()),
        )?;
    } else {
        let _ = key.delete_value(RUN_VALUE);
    }
    Ok(())
}
