//! Recover only the verified Hebnix process holding this operation's OS lock.
use std::path::Path;
use windows::Win32::{Foundation::*, System::{RestartManager::*, Threading::*}};
use windows::core::{PCWSTR, PWSTR};

struct Session(u32);
impl Drop for Session { fn drop(&mut self) { unsafe { let _ = RmEndSession(self.0); } } }
struct Process(HANDLE);
impl Drop for Process { fn drop(&mut self) { unsafe { let _ = CloseHandle(self.0); } } }

fn normalized(path: &Path) -> String {
    path.to_string_lossy().trim_start_matches(r"\\?\").to_ascii_lowercase()
}
fn verified_owner(pid: u32, expected: FILETIME, actual: FILETIME, image: &Path, current: &Path) -> bool {
    pid != std::process::id() && pid != 0
        && expected.dwLowDateTime == actual.dwLowDateTime && expected.dwHighDateTime == actual.dwHighDateTime
        && normalized(image) == normalized(current)
}

pub(super) fn release_previous_worker(path: &Path) -> Result<bool, String> {
    // Never stop RL (or any unrelated process), even when it owns a package.
    if hebnix_sdk::process::is_rocket_league_running() { return Err("Rocket League is using the game files. Waiting for Rocket League to close.".into()); }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    unsafe {
        let mut session = 0;
        let mut key = [0u16; 33];
        let result = RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr()));
        if result != ERROR_SUCCESS { return Err(format!("Could not identify the previous patch worker: {}", result.0)); }
        let session = Session(session);
        let resource: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        let result = RmRegisterResources(session.0, Some(&[PCWSTR(resource.as_ptr())]), None, None);
        if result != ERROR_SUCCESS { return Err(format!("Could not inspect the patch operation lock: {}", result.0)); }
        let (mut needed, mut count, mut reasons) = (0, 0, 0);
        let result = RmGetList(session.0, &mut needed, &mut count, None, &mut reasons);
        if result == ERROR_SUCCESS { return Ok(false); }
        if result != ERROR_MORE_DATA || needed > 64 { return Err("Could not identify a recoverable Hebnix patch worker".into()); }
        let mut owners = vec![RM_PROCESS_INFO::default(); needed as usize];
        count = needed;
        let result = RmGetList(session.0, &mut needed, &mut count, Some(owners.as_mut_ptr()), &mut reasons);
        if result != ERROR_SUCCESS { return Err("The previous patch worker changed while recovery was starting".into()); }
        let mut stopped = false;
        for owner in owners.into_iter().take(count as usize) {
            let pid = owner.Process.dwProcessId;
            if pid == std::process::id() { continue; }
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) else { continue };
            let process = Process(handle);
            let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user).is_err() { continue; }
            let mut name = vec![0u16; 32768];
            let mut size = name.len() as u32;
            if QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, PWSTR(name.as_mut_ptr()), &mut size).is_err() { continue; }
            let image = std::path::PathBuf::from(String::from_utf16_lossy(&name[..size as usize]));
            if !verified_owner(pid, owner.Process.ProcessStartTime, created, &image, &current) { continue; }
            if hebnix_sdk::process::is_rocket_league_running() { return Err("Rocket League is using the game files. Waiting for Rocket League to close.".into()); }
            tracing::info!(pid, "stopping previous Hebnix patch worker; recovery journal retained");
            TerminateProcess(process.0, 0).map_err(|e| format!("Could not stop the previous Hebnix patch worker: {e}"))?;
            if WaitForSingleObject(process.0, 5000) == WAIT_TIMEOUT { return Err("Waiting for the previous Hebnix patch worker to exit".into()); }
            stopped = true;
        }
        Ok(stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_rejects_rl_unrelated_processes_and_reused_pids() {
        let created = FILETIME { dwLowDateTime: 7, dwHighDateTime: 1 };
        let current = Path::new(r"C:\Hebnix\Hebnix.exe");
        let pid = std::process::id().wrapping_add(1);
        assert!(verified_owner(pid, created, created, Path::new(r"c:\hebnix\HEBNIX.exe"), current));
        assert!(!verified_owner(pid, created, created, Path::new(r"C:\RL\RocketLeague.exe"), current));
        assert!(!verified_owner(std::process::id(), created, created, current, current));
        assert!(!verified_owner(pid, created, FILETIME::default(), current, current));
    }
}
