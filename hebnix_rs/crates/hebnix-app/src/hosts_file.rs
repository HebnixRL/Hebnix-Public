//! hosts file redirect, the game ignores the wininet proxy. needs admin, the
//! file is in System32.

use std::path::{Path, PathBuf};

// our lines end with this so we can find them later
pub const MARK: &str = "# hebnix spoofer";

// All Hebnix processes and the boot recovery task share this lock. Without
// it, a watchdog can overwrite a newer hosts snapshot during a restart.
pub(crate) struct HostsLock(windows::Win32::Foundation::HANDLE);

impl HostsLock {
    pub(crate) fn acquire() -> Result<Self, String> {
        use windows::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        let name: Vec<u16> = "Global\\HebnixHostsCleanup\0".encode_utf16().collect();
        let handle = unsafe { CreateMutexW(None, false, windows::core::PCWSTR(name.as_ptr())) }
            .map_err(|e| format!("Could not acquire hosts cleanup lock: {e}"))?;
        let result = unsafe { WaitForSingleObject(handle, 5000) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err("Hosts cleanup lock timed out".into());
        }
        Ok(Self(handle))
    }
}

impl Drop for HostsLock {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.0);
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub fn hosts_path() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    Path::new(&root).join(r"System32\drivers\etc\hosts")
}

pub fn is_writable() -> bool {
    // create, some debloat tools delete the hosts file entirely
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(hosts_path())
        .is_ok()
}

const NEEDS_ADMIN: &str = "the hosts file needs administrator, restart hebnix as admin";

// what windows ships, a missing file starts from this so it stays a normal hosts file
const HEADER: &str = "# Copyright (c) 1993-2009 Microsoft Corp.\r\n#\r\n# This is a sample HOSTS file used by Microsoft TCP/IP for Windows.\r\n#\r\n# This file contains the mappings of IP addresses to host names. Each\r\n# entry should be kept on an individual line. The IP address should\r\n# be placed in the first column followed by the corresponding host name.\r\n# The IP address and the host name should be separated by at least one\r\n# space.\r\n#\r\n# Additionally, comments (such as these) may be inserted on individual\r\n# lines or following the machine name denoted by a '#' symbol.\r\n#\r\n# For example:\r\n#\r\n#      102.54.94.97     rhino.acme.com          # source server\r\n#       38.25.63.10     x.acme.com              # x client host\r\n\r\n# localhost name resolution is handled within DNS itself.\r\n#\t127.0.0.1       localhost\r\n#\t::1             localhost\r\n";

// a missing hosts file reads as the standard header, windows works fine without it
fn read_plain(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HEADER.to_string()),
        other => other,
    }
}

// repairs the encoding first, callers hold HostsLock
fn read(path: &Path) -> Result<String, String> {
    repair(path).map_err(|e| e.message())?;
    read_plain(path).map_err(|e| format!("Cannot read hosts: {e}"))
}

struct Decoded {
    text: String,
    changed: bool,
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

// UTF-16 and a UTF-8 BOM become plain UTF-8, bytes that are not UTF-8 are read as Windows-1252
fn decode_hosts(bytes: &[u8]) -> Decoded {
    let legacy = |b: &[u8]| encoding_rs::WINDOWS_1252.decode_without_bom_handling(b).0.into_owned();
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Decoded { text: utf16(rest, u16::from_le_bytes), changed: true };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Decoded { text: utf16(rest, u16::from_be_bytes), changed: true };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text = std::str::from_utf8(rest).map_or_else(|_| legacy(rest), str::to_string);
        return Decoded { text, changed: true };
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Decoded { text: text.to_string(), changed: false },
        Err(_) => Decoded { text: legacy(bytes), changed: true },
    }
}

pub struct Repaired {
    pub backup: PathBuf,
}

pub enum RepairError {
    NeedsAdmin,
    Failed(String),
}

impl RepairError {
    pub fn message(&self) -> String {
        match self {
            Self::NeedsAdmin => NEEDS_ADMIN.to_string(),
            Self::Failed(text) => text.clone(),
        }
    }
}

// never overwrites an earlier backup, that one may hold the real original
fn backup_path(path: &Path) -> PathBuf {
    let first = path.with_file_name("hosts.hebnix.bak");
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| path.with_file_name(format!("hosts.hebnix.{n}.bak")))
        .find(|candidate| !candidate.exists())
        .expect("endless range")
}

/// Converts a UTF-16 or non-UTF-8 hosts file to UTF-8 without BOM. The original goes to
/// hosts.hebnix.bak first. Returns None when the file is fine or missing.
fn repair(path: &Path) -> Result<Option<Repaired>, RepairError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(RepairError::NeedsAdmin);
        }
        Err(e) => return Err(RepairError::Failed(format!("Cannot read hosts: {e}"))),
    };
    let decoded = decode_hosts(&bytes);
    if !decoded.changed {
        return Ok(None);
    }
    let backup = backup_path(path);
    std::fs::write(&backup, &bytes).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            RepairError::NeedsAdmin
        } else {
            RepairError::Failed(format!(
                "Could not back up the hosts file to {}: {e}. The hosts file was not changed.",
                backup.display()
            ))
        }
    })?;
    let failed = |why: String| {
        RepairError::Failed(format!(
            "Could not convert the hosts file to UTF-8 ({why}). Your original is saved at {}.",
            backup.display()
        ))
    };
    write_bytes(path, decoded.text.as_bytes()).map_err(failed)?;
    let intact = std::fs::read(path)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .is_some_and(|text| text.lines().eq(decoded.text.lines()));
    if !intact {
        let _ = write_bytes(path, &bytes);
        return Err(failed("the converted file did not match".into()));
    }
    Ok(Some(Repaired { backup }))
}

static STARTUP_REPAIR: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();

/// Runs once at launch. Without administrator it stays quiet, the real error comes when a
/// redirect is written.
pub fn repair_at_startup() {
    let Ok(_guard) = HostsLock::acquire() else {
        return;
    };
    let result = match repair(&hosts_path()) {
        Ok(Some(repaired)) => Ok(repaired.backup),
        Ok(None) => return,
        Err(RepairError::NeedsAdmin) => {
            tracing::warn!("hosts file is not UTF-8 and needs administrator to repair");
            return;
        }
        Err(RepairError::Failed(text)) => Err(text),
    };
    let _ = STARTUP_REPAIR.set(result);
}

/// The startup repair outcome, Ok is the backup path.
pub fn startup_repair() -> Option<&'static Result<PathBuf, String>> {
    STARTUP_REPAIR.get()
}

fn line_for(host: &str) -> String {
    format!("127.0.0.1 {host} {MARK}")
}

/// Ensure exactly one Hebnix redirect per requested host. Leave the file
/// untouched when it already has the desired entries.
pub fn set_redirects(hosts: &[&str]) -> Result<(), String> {
    let _guard = HostsLock::acquire()?;
    let path = hosts_path();
    let content = read(&path)?;
    if redirects_match(&content, hosts) {
        return Ok(());
    }

    let out = with_redirects(&content, hosts);
    write(&path, &out)?;
    let verified =
        std::fs::read_to_string(&path).map_err(|e| format!("cant verify hosts redirects: {e}"))?;
    if !redirects_match(&verified, hosts) {
        return Err("Hebnix hosts redirects did not match the requested hosts".into());
    }
    flush_dns();
    Ok(())
}

fn redirects_match(content: &str, hosts: &[&str]) -> bool {
    let ours: Vec<&str> = content.lines().filter(|line| is_ours(line)).collect();
    ours.len() == hosts.len()
        && hosts.iter().all(|host| {
            ours.iter()
                .filter(|line| line.trim().eq_ignore_ascii_case(&line_for(host)))
                .count()
                == 1
        })
}

/// Change one owned hostname without changing any other Hebnix or user entries.
pub fn set_host_redirect(host: &str, enabled: bool) -> Result<(), String> {
    let _guard = HostsLock::acquire()?;
    let path = hosts_path();
    let content = read(&path)?;
    let out = with_host_redirect(&content, host, enabled)?;
    if out == content {
        return Ok(());
    }
    write(&path, &out)?;
    let verified = read(&path)?;
    if with_host_redirect(&verified, host, enabled)? != verified {
        return Err(format!("Could not verify the hosts entry for {host}"));
    }
    flush_dns();
    Ok(())
}

fn maps_host(line: &str, host: &str) -> bool {
    line.split('#')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip(1)
        .any(|name| name.eq_ignore_ascii_case(host))
}

fn with_host_redirect(content: &str, host: &str, enabled: bool) -> Result<String, String> {
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err("Invalid redirect hostname".into());
    }
    if enabled
        && content
            .lines()
            .any(|line| maps_host(line, host) && !is_ours(line))
    {
        return Err(format!(
            "An existing hosts entry for {host} must be resolved before enabling the account proxy"
        ));
    }
    let owned: Vec<_> = content
        .lines()
        .filter(|line| is_ours(line) && maps_host(line, host))
        .collect();
    if (!enabled && owned.is_empty())
        || (enabled && owned.len() == 1 && owned[0].trim().eq_ignore_ascii_case(&line_for(host)))
    {
        return Ok(content.into());
    }
    let mut out: String = content
        .split_inclusive('\n')
        .filter(|line| !(is_ours(line) && maps_host(line, host)))
        .collect();
    if enabled {
        let newline = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(newline);
        }
        out.push_str(&line_for(host));
        out.push_str(newline);
    }
    Ok(out)
}

pub fn has_redirects() -> bool {
    read_plain(&hosts_path())
        .map(|content| content.lines().any(is_ours))
        .unwrap_or(false)
}

/// drops our lines, whatever host they were for
pub fn clear() -> Result<(), String> {
    let _guard = HostsLock::acquire()?;
    let path = hosts_path();
    let content = read(&path)?;

    if !content.lines().any(is_ours) {
        return Ok(());
    }

    let out = without_redirects(&content);
    write(&path, &out)?;
    let verified =
        std::fs::read_to_string(&path).map_err(|e| format!("cant verify hosts cleanup: {e}"))?;
    if verified.lines().any(is_ours) {
        return Err("Hebnix redirects remain in the hosts file after cleanup".into());
    }
    flush_dns();
    Ok(())
}

fn is_ours(line: &str) -> bool {
    let Some((mapping, comment)) = line.rsplit_once('#') else {
        return false;
    };
    let mut fields = mapping.split_whitespace();
    if fields.next() != Some("127.0.0.1") || fields.next().is_none() {
        return false;
    }
    matches!(
        comment.trim().to_ascii_lowercase().as_str(),
        "hebnix spoofer" | "hebnix"
    )
}

fn without_redirects(content: &str) -> String {
    content
        .split_inclusive('\n')
        .filter(|line| !is_ours(line.trim_end_matches(['\r', '\n'])))
        .collect()
}

fn with_redirects(content: &str, hosts: &[&str]) -> String {
    let mut out = without_redirects(content);
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(newline);
    }
    for host in hosts {
        out.push_str(&line_for(host));
        out.push_str(newline);
    }
    out
}

fn write(path: &Path, content: &str) -> Result<(), String> {
    write_bytes(path, content.as_bytes())
}

fn write_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    // Publish a complete file instead of truncating hosts in place. A killed
    // process must not erase the user's unrelated entries halfway through.
    let temporary = path.with_file_name(format!(
        ".hebnix-hosts-{}-{}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACE_FILE_FLAGS, ReplaceFileW,
        };
        let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            if path.exists() {
                ReplaceFileW(
                    windows::core::PCWSTR(target.as_ptr()),
                    windows::core::PCWSTR(source.as_ptr()),
                    None,
                    REPLACE_FILE_FLAGS(0),
                    None,
                    None,
                )
            } else {
                MoveFileExW(
                    windows::core::PCWSTR(source.as_ptr()),
                    windows::core::PCWSTR(target.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
        }
        .map_err(std::io::Error::other)
    })();
    let _ = std::fs::remove_file(&temporary);
    result.map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            NEEDS_ADMIN.to_string()
        } else {
            format!("cant write the hosts file: {e}")
        }
    })
}

pub fn flush_dns() {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("ipconfig")
        .arg("/flushdns")
        .creation_flags(0x08000000)
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hebnix-hosts-test-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn utf16_bytes(text: &str, bom: [u8; 2], big_endian: bool) -> Vec<u8> {
        let mut out = bom.to_vec();
        for unit in text.encode_utf16() {
            out.extend(if big_endian { unit.to_be_bytes() } else { unit.to_le_bytes() });
        }
        out
    }

    const ENTRIES: &str = "127.0.0.1 localhost\r\n10.0.0.5 nas # caf\u{e9}\r\n";

    #[test]
    fn utf16_files_become_utf8_and_keep_every_entry() {
        for (bom, big) in [([0xFF, 0xFE], false), ([0xFE, 0xFF], true)] {
            let dir = dir();
            let hosts = dir.join("hosts");
            std::fs::write(&hosts, utf16_bytes(ENTRIES, bom, big)).unwrap();
            let repaired = repair(&hosts).ok().unwrap().unwrap();
            assert_eq!(std::fs::read_to_string(&hosts).unwrap(), ENTRIES);
            assert_eq!(repaired.backup, dir.join("hosts.hebnix.bak"));
            assert_eq!(std::fs::read(&repaired.backup).unwrap(), utf16_bytes(ENTRIES, bom, big));
        }
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let dir = dir();
        let hosts = dir.join("hosts");
        std::fs::write(&hosts, [&[0xEF, 0xBB, 0xBF][..], b"1.1.1.1 a\r\n"].concat()).unwrap();
        assert!(repair(&hosts).ok().unwrap().is_some());
        assert_eq!(std::fs::read(&hosts).unwrap(), b"1.1.1.1 a\r\n");
    }

    #[test]
    fn legacy_bytes_are_read_as_windows_1252() {
        let dir = dir();
        let hosts = dir.join("hosts");
        std::fs::write(&hosts, b"1.1.1.1 a # caf\xe9\r\n").unwrap();
        assert!(repair(&hosts).ok().unwrap().is_some());
        assert_eq!(std::fs::read_to_string(&hosts).unwrap(), "1.1.1.1 a # caf\u{e9}\r\n");
    }

    #[test]
    fn a_good_file_is_left_alone() {
        let dir = dir();
        let hosts = dir.join("hosts");
        std::fs::write(&hosts, ENTRIES).unwrap();
        assert!(repair(&hosts).ok().unwrap().is_none());
        assert!(!dir.join("hosts.hebnix.bak").exists());
    }

    #[test]
    fn a_missing_file_reads_as_the_standard_header() {
        let dir = dir();
        let hosts = dir.join("hosts");
        assert!(repair(&hosts).ok().unwrap().is_none());
        assert_eq!(read(&hosts).unwrap(), HEADER);
    }

    #[test]
    fn an_earlier_backup_is_never_overwritten() {
        let dir = dir();
        let hosts = dir.join("hosts");
        std::fs::write(dir.join("hosts.hebnix.bak"), b"first").unwrap();
        std::fs::write(&hosts, utf16_bytes(ENTRIES, [0xFF, 0xFE], false)).unwrap();
        let repaired = repair(&hosts).ok().unwrap().unwrap();
        assert_eq!(repaired.backup, dir.join("hosts.hebnix.2.bak"));
        assert_eq!(std::fs::read(dir.join("hosts.hebnix.bak")).unwrap(), b"first");
    }
}
