//! Remove Hebnix's account HTTP redirect and recover legacy proxy settings after an interrupted exit.
use crate::spoofer::SpooferManager;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};

pub fn handoff_live_spoofer(_spoofer: Arc<SpooferManager>) -> bool {
    false
}
pub fn has_live_handoff() -> bool {
    false
}
pub fn finish_live_handoff() {}
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

pub const CLEANUP_WATCHDOG_ARG: &str = "--cleanup-watchdog";
static CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);

pub fn is_ready() -> bool {
    CHILD.lock().ok().is_some_and(|mut child| {
        child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    })
}

pub fn spawn() -> bool {
    use std::io::Read;
    use std::os::windows::process::CommandExt;

    let Ok(mut child_slot) = CHILD.lock() else {
        return false;
    };
    if child_slot
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    {
        return true;
    }
    *child_slot = None;

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let owner = std::process::id();
    if std::fs::create_dir_all(watchdog_owner_path().parent().unwrap()).is_err()
        || std::fs::write(watchdog_owner_path(), owner.to_string()).is_err()
    {
        return false;
    }
    let Ok(ready) = std::net::TcpListener::bind("127.0.0.1:0") else {
        return false;
    };
    let Ok(address) = ready.local_addr() else {
        return false;
    };
    if ready.set_nonblocking(true).is_err() {
        return false;
    }
    let token = rand::random::<u64>().to_string();
    // Escape a launcher job when Windows allows it; otherwise use a detached
    // child so cleanup can outlive the Hebnix window.
    let start = |flags| {
        std::process::Command::new(&exe)
            .args([
                CLEANUP_WATCHDOG_ARG,
                &owner.to_string(),
                &address.to_string(),
                &token,
            ])
            .creation_flags(flags)
            .spawn()
    };
    if let Ok(mut child) = start(0x09000208).or_else(|_| start(0x08000208)) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok((mut stream, _)) = ready.accept() {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
                let mut received = vec![0; token.len()];
                if stream.read_exact(&mut received).is_ok()
                    && received == token.as_bytes()
                    && matches!(child.try_wait(), Ok(None))
                {
                    *child_slot = Some(child);
                    return true;
                }
            }
            if !matches!(child.try_wait(), Ok(None)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_file(watchdog_owner_path());
    false
}

/// Put the user's proxy back when Windows logs off or shuts down. A hidden
/// top-level window receives WM_ENDSESSION; killed processes get no other chance.
pub fn restore_on_session_end() {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_ENDSESSION, WNDCLASSW,
    };
    use windows::core::w;

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_ENDSESSION && wparam.0 != 0 {
            crate::spoofer::restore_if_crashed(&crate::config::base_dir());
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let _ = std::thread::Builder::new()
        .name("session-end-proxy-restore".into())
        .spawn(|| unsafe {
            let Ok(instance) = GetModuleHandleW(None) else {
                return;
            };
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: w!("HebnixSessionEnd"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let Ok(_window) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("HebnixSessionEnd"),
                w!("Hebnix"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            ) else {
                return;
            };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        });
}

pub fn parent_pid() -> Option<u32> {
    let mut args = std::env::args();
    while let Some(argument) = args.next() {
        if argument == CLEANUP_WATCHDOG_ARG {
            return args.next()?.parse().ok();
        }
    }
    None
}

pub fn run(parent_pid: u32) {
    restore_on_session_end();
    if let Ok(parent) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) } {
        let args: Vec<String> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == CLEANUP_WATCHDOG_ARG) {
            if let (Some(address), Some(token)) = (args.get(index + 2), args.get(index + 3)) {
                use std::io::Write;
                if let Ok(mut stream) = std::net::TcpStream::connect(address) {
                    let _ = stream.write_all(token.as_bytes());
                }
            }
        }
        unsafe {
            while WaitForSingleObject(parent, 250) == WAIT_TIMEOUT {}
            let _ = CloseHandle(parent);
        }
    }
    if replacement_is_running(parent_pid) {
        return;
    }
    crate::spoofer::restore_if_crashed(&crate::config::base_dir());
    let _ = crate::multiplayer_lan::cleanup_system_state();
    if watchdog_owner().is_some_and(|owner| owner == parent_pid) {
        let _ = std::fs::remove_file(watchdog_owner_path());
    }
}
fn watchdog_owner_path() -> std::path::PathBuf {
    crate::config::base_dir()
        .join("state")
        .join("watchdog_owner.pid")
}

fn watchdog_owner() -> Option<u32> {
    std::fs::read_to_string(watchdog_owner_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn replacement_is_running(parent_pid: u32) -> bool {
    let Some(owner) = watchdog_owner().filter(|owner| *owner != parent_pid) else {
        return false;
    };
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, owner) }) else {
        return false;
    };
    let running = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
    unsafe {
        let _ = CloseHandle(process);
    }
    running
}
