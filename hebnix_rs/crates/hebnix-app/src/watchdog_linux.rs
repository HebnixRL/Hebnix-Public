//! cleanup watchdog: a *separate* subprocess (`hebnix-app --cleanup-watchdog
//! <owner-pid>`) that outlives the main app and undoes anything that needs
//! undoing (the Wine prefix's HTTP proxy setting, multiplayer state) if the
//! main process dies without cleaning up after itself.
//!
//! Since the owner is a real separate process (exec'd, not a thread), we
//! can't `waitpid` on it -- that only works for actual child processes, and
//! by the time this runs it's re-parented to init/systemd like any other
//! detached process. Poll `kill(pid, None)` (liveness probe, sends no
//! signal) instead, the standard /proc-less way to check if a pid is alive.

use std::sync::{Arc, Mutex, OnceLock};

use nix::sys::signal::kill;
use nix::unistd::Pid;

use crate::spoofer::SpooferManager;

// Windows restores the system proxy live and stops the proxy right away.
// Rocket League under Wine only reads the prefix proxy at launch, so a game
// that is still running needs the HTTP proxy until it exits: keep it alive
// in this process after the UI closes.
static LIVE_SPOOFER: OnceLock<Arc<SpooferManager>> = OnceLock::new();

pub fn handoff_live_spoofer(spoofer: Arc<SpooferManager>) -> bool {
    if !crate::spoofer::prefix_proxy_in_use(&crate::config::base_dir()) {
        return false;
    }
    spoofer.disable_spoofing();
    LIVE_SPOOFER.set(spoofer).is_ok() || LIVE_SPOOFER.get().is_some()
}

pub fn has_live_handoff() -> bool {
    LIVE_SPOOFER.get().is_some()
}

pub fn finish_live_handoff() {
    let Some(spoofer) = LIVE_SPOOFER.get() else {
        return;
    };
    tracing::info!("Hebnix UI closed; HTTP proxy stays up until Rocket League exits");
    while crate::spoofer::prefix_proxy_in_use(&crate::config::base_dir()) {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    spoofer.shutdown();
    tracing::info!("Rocket League closed; Wine prefix proxy restored");
}

pub const CLEANUP_WATCHDOG_ARG: &str = "--cleanup-watchdog";
static CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);

pub fn is_ready() -> bool {
    CHILD.lock().ok().is_some_and(|mut child| {
        child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    })
}

fn pid_alive(pid: u32) -> bool {
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

/// spawn the watchdog subprocess. Needs no privileges: the proxy setting it
/// restores lives in the user's own Wine prefix.
pub fn spawn() -> bool {
    use std::io::Read;

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
    if watchdog_owner_path()
        .parent()
        .is_some_and(|parent| std::fs::create_dir_all(parent).is_err())
        || std::fs::write(watchdog_owner_path(), owner.to_string()).is_err()
    {
        return false;
    }
    // the child acknowledges on this socket once it is watching, so a
    // watchdog that died on startup isn't mistaken for a running one
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
    let started = std::process::Command::new(&exe)
        .args([
            CLEANUP_WATCHDOG_ARG,
            &owner.to_string(),
            &address.to_string(),
            &token,
        ])
        .spawn();
    if let Ok(mut child) = started {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok((mut stream, _)) = ready.accept() {
                let _ = stream.set_nonblocking(false);
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
    if pid_alive(parent_pid) {
        let args: Vec<String> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == CLEANUP_WATCHDOG_ARG) {
            if let (Some(address), Some(token)) = (args.get(index + 2), args.get(index + 3)) {
                use std::io::Write;
                if let Ok(mut stream) = std::net::TcpStream::connect(address) {
                    let _ = stream.write_all(token.as_bytes());
                }
            }
        }
    }
    while pid_alive(parent_pid) {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if replacement_is_running(parent_pid) {
        return;
    }
    crate::spoofer::restore_when_prefix_free(&crate::config::base_dir());
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

pub(crate) fn replacement_is_running(parent_pid: u32) -> bool {
    let Some(owner) = watchdog_owner().filter(|owner| *owner != parent_pid) else {
        return false;
    };
    pid_alive(owner)
}
