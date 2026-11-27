//! RL window focus + geometry (windows only).
//!
//! id by process id, not window title. GetWindowText can hang forever on
//! same-process windows owned by non-pumping threads (medal/discord overlay
//! hooks inject those everywhere), and OpenProcess on the game is unreliable
//! once eac locks it. GetWindowThreadProcessId dodges both.

use std::sync::OnceLock;
use std::time::Duration;

use crate::background_cache::BackgroundCache;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsWindowVisible,
};

/// Focus checks run on every UI frame and on the monitor thread. Neither may
/// wait for a process scan, including when the cache expires or is first used.
fn cached_rl_pid() -> Option<u32> {
    static CACHE: OnceLock<BackgroundCache<Option<u32>>> = OnceLock::new();
    *CACHE
        .get_or_init(|| {
            BackgroundCache::new("rl-focus", None, Duration::from_millis(500), move |_| {
                // Toolhelp does not need to open the protected game process.
                // Use the same PID source as close/detection, not a richer
                // sysinfo scan that can lose the name after EAC locks it.
                Some(super::detector::find_process("rocketleague.exe"))
            })
        })
        .snapshot()
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid
}

struct EnumState {
    pid: u32,
    result: Option<HWND>,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    unsafe {
        let state = &mut *(lparam.0 as *mut EnumState);
        if window_pid(hwnd) == state.pid
            && IsWindowVisible(hwnd).as_bool()
            // main window = the visible one with a caption. GetWindowTextLengthW
            // reads a cached length, never sends messages, so it can't hang.
            && GetWindowTextLengthW(hwnd) > 0
        {
            let mut class = [0u16; 128];
            let length = GetClassNameW(hwnd, &mut class) as usize;
            let unreal = String::from_utf16_lossy(&class[..length]).contains("Unreal");
            if state.result.is_none() || unreal { state.result = Some(hwnd); }
            // Injected overlays can have a caption and the game's PID. Prefer
            // the Unreal viewport rather than closing the first overlay found.
            if unreal { return false.into(); }
        }
        true.into()
    }
}

/// RL's main window handle by pid
pub fn rocket_league_hwnd() -> Option<HWND> {
    let pid = cached_rl_pid()?;
    let mut state = EnumState { pid, result: None };
    unsafe {
        // EnumWindows errors when the callback stops early, not a failure here
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut state as *mut EnumState as isize));
    }
    state.result
}

/// Ask an active live hook to quit internally, then try the game's viewport.
/// EAC can deny PROCESS_TERMINATE even for the same user.
/// Never elevate, and only force-terminate the PID selected at the start.
pub fn close_rocket_league() -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::time::Instant;
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

    let Some(pid) = super::detector::find_process("rocketleague.exe") else { return Ok(()); };
    let request = game_quit_request(pid);
    if request.is_some() {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if super::detector::find_process("rocketleague.exe") != Some(pid) { return Ok(()); }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    // Do not use the focus cache here: it may still contain the previous game.
    let mut state = EnumState { pid, result: None };
    unsafe { let _ = EnumWindows(Some(enum_cb), LPARAM(&mut state as *mut EnumState as isize)); }
    if let Some(hwnd) = state.result {
        if unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                if super::detector::find_process("rocketleague.exe") != Some(pid) { return Ok(()); }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let output = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .creation_flags(0x0800_0000)
        .output()?;
    if output.status.success() || super::detector::find_process("rocketleague.exe") != Some(pid) {
        return Ok(());
    }
    Err(std::io::Error::other(format!("Rocket League did not close: {}",
        String::from_utf8_lossy(&output.stderr).trim())))
}

struct QuitRequest(std::path::PathBuf);
impl Drop for QuitRequest {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}
fn game_quit_request(pid: u32) -> Option<QuitRequest> {
    let directory = crate::utils::paths::base_dir().join("state/live-cosmetics");
    let report = directory.join("runtime.ini");
    let metadata = std::fs::metadata(&report).ok()?;
    if metadata.len() > 8192 || metadata.modified().ok()?.elapsed().ok()? > Duration::from_secs(4) { return None; }
    let text = std::fs::read_to_string(report).ok()?;
    let session = quit_session(&text)?;
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let path = directory.join("quit.request");
    std::fs::write(&path, format!("quit:{session}:{pid}:{nonce}")).ok()?;
    Some(QuitRequest(path))
}
fn quit_session(text: &str) -> Option<&str> {
    if text.len() > 8192 { return None; }
    if !text.replace('\r', "").ends_with("HbxLiveRuntime.end=1\n") { return None; }
    let revision: u32 = text.lines().find_map(|line| line.strip_prefix("HbxLiveRuntime.version="))?.parse().ok()?;
    if revision < 19 { return None; }
    let session = text.lines().find_map(|line| line.strip_prefix("session="))?;
    if session.is_empty() || session.len() > 40 || session.contains(['\r', '\n']) { return None; }
    Some(session)
}

#[cfg(test)]
mod quit_tests {
    use super::quit_session;
    #[test]
    fn quit_requires_a_complete_report_from_a_hook_that_supports_it() {
        assert_eq!(quit_session("HbxLiveRuntime.version=19\nsession=2026.11.27-19.00.00\nHbxLiveRuntime.end=1\n"), Some("2026.11.27-19.00.00"));
        assert!(quit_session("HbxLiveRuntime.version=18\nsession=old\nHbxLiveRuntime.end=1\n").is_none());
        assert!(quit_session("HbxLiveRuntime.version=19\nsession=partial\n").is_none());
        assert!(quit_session("HbxLiveRuntime.version=19\nsession=\nHbxLiveRuntime.end=1\n").is_none());
    }
}

/// does the RL window have focus. compares the foreground window's pid to
/// RocketLeague.exe, no title reads, no handle opens.
pub fn is_rocket_league_focused() -> bool {
    let Some(pid) = cached_rl_pid() else {
        return false;
    };
    unsafe {
        let fg = GetForegroundWindow();
        !fg.is_invalid() && window_pid(fg) == pid
    }
}

/// (left, top, right, bottom) pixel rect of the RL window
pub fn get_rocket_league_window_rect() -> Option<(i32, i32, i32, i32)> {
    let hwnd = rocket_league_hwnd()?;
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        if rect.right > rect.left && rect.bottom > rect.top {
            Some((rect.left, rect.top, rect.right, rect.bottom))
        } else {
            None
        }
    }
}

/// pixel size of the monitor RL is on, falls back to the primary. what plugin
/// windows size themselves against when they ask in percent.
pub fn rocket_league_monitor_size() -> (i32, i32) {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    unsafe {
        if let Some(hwnd) = rocket_league_hwnd() {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let r = info.rcMonitor;
                if r.right > r.left && r.bottom > r.top {
                    return (r.right - r.left, r.bottom - r.top);
                }
            }
        }
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    }
}

/// is the cursor inside the RL window
pub fn is_cursor_inside_rl_window() -> bool {
    let Some((left, top, right, bottom)) = get_rocket_league_window_rect() else {
        return false;
    };
    unsafe {
        let mut pt = windows::Win32::Foundation::POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return false;
        }
        left <= pt.x && pt.x <= right && top <= pt.y && pt.y <= bottom
    }
}
