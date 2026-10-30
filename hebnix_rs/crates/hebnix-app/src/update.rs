use serde_json::Value;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub setup_url: String,
}

#[derive(Debug, Clone)]
pub struct ChangelogEntry {
    pub version: String,
    pub changelog: String,
    pub release_date: String,
}

#[derive(Debug, Clone)]
pub struct ApiInfo {
    pub update: Option<UpdateInfo>,
    pub newest_changelog: Option<ChangelogEntry>,
}
fn is_newer_version(current: &str, remote: &str) -> bool {
    let curr_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();
    let rem_parts: Vec<u32> = remote.split('.').filter_map(|s| s.parse().ok()).collect();

    for i in 0..std::cmp::max(curr_parts.len(), rem_parts.len()) {
        let c = curr_parts.get(i).unwrap_or(&0);
        let r = rem_parts.get(i).unwrap_or(&0);
        if r > c {
            return true;
        }
        if r < c {
            return false;
        }
    }
    false
}

pub fn fetch_info(current_version: &str, lite: bool) -> Result<ApiInfo, String> {
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let resp = agent
        .get("https://api.hebnix.com/info")
        .set(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| format!("Network error while checking for updates: {e}"))?;

    let json: Value = resp
        .into_json()
        .map_err(|e| format!("Invalid JSON response: {e}"))?;
    parse_info(&json, current_version, lite)
}

fn parse_info(json: &Value, current_version: &str, lite: bool) -> Result<ApiInfo, String> {
    let latest_version = json
        .get("latest_version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let setup_url = json
        .get(if lite { "lite_setup_url" } else { "setup_url" })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if latest_version.is_empty() {
        return Err("API response missing latest_version".to_string());
    }

    let newest_changelog = json
        .get("changelog_history")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .map(|entry| ChangelogEntry {
            version: entry
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            changelog: entry
                .get("changelog")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            release_date: entry
                .get("release_date")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });

    let update = is_newer_version(current_version, &latest_version).then(|| UpdateInfo {
        version: latest_version,
        setup_url,
    });
    Ok(ApiInfo {
        update,
        newest_changelog,
    })
}

pub fn render_changelog(ui: &mut eframe::egui::Ui, entry: &ChangelogEntry) {
    ui.heading(crate::i18n::t_args(
        "changelog-heading",
        &[("version", entry.version.as_str().into())],
    ));
    if !entry.release_date.is_empty() {
        ui.label(
            eframe::egui::RichText::new(&entry.release_date)
                .small()
                .weak(),
        );
    }
    ui.add_space(8.0);

    for line in entry
        .changelog
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let trimmed = line.trim();
        // the [Added]/[Fixed]/[Removed] tags come from the server and stay as
        // they are, only the label shown for them is translated
        let (prefix, body, colour) = if let Some(body) = trimmed.strip_prefix("[Added]") {
            (
                crate::i18n::t("changelog-added"),
                body.trim().trim_start_matches(':').trim(),
                eframe::egui::Color32::from_rgb(46, 204, 113),
            )
        } else if let Some(body) = trimmed.strip_prefix("[Fixed]") {
            (
                crate::i18n::t("changelog-fixed"),
                body.trim().trim_start_matches(':').trim(),
                eframe::egui::Color32::from_rgb(52, 152, 219),
            )
        } else if let Some(body) = trimmed.strip_prefix("[Removed]") {
            (
                crate::i18n::t("changelog-removed"),
                body.trim().trim_start_matches(':').trim(),
                eframe::egui::Color32::from_rgb(231, 76, 60),
            )
        } else {
            (String::new(), trimmed, ui.visuals().text_color())
        };
        ui.horizontal_wrapped(|ui| {
            if !prefix.is_empty() {
                ui.label(eframe::egui::RichText::new(&prefix).strong().color(colour));
            }
            ui.label(body);
        });
    }
}

/// Shared with the popup; the worker never blocks the UI thread.
#[derive(Debug, Default, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub ready: bool,
}
pub type SharedProgress = std::sync::Arc<std::sync::Mutex<DownloadProgress>>;

pub fn render_progress(ui: &mut eframe::egui::Ui, progress: &SharedProgress) {
    let state = progress.lock().unwrap().clone();
    let fraction = state
        .total
        .filter(|n| *n > 0)
        .map(|n| (state.downloaded as f32 / n as f32).min(1.0));
    let text = if state.ready {
        crate::i18n::t("update-restarting")
    } else if let Some(total) = state.total {
        format!(
            "{:.1} / {:.1} MB",
            state.downloaded as f64 / 1048576.0,
            total as f64 / 1048576.0
        )
    } else {
        format!("{:.1} MB", state.downloaded as f64 / 1048576.0)
    };
    let bar = eframe::egui::ProgressBar::new(fraction.unwrap_or(0.0)).text(text);
    ui.add(if fraction.is_none() {
        bar.animate(true)
    } else {
        bar
    });
}

/// Stage a straight EXE beside the current executable (same volume and portable
/// root). The caller starts maintenance and performs normal application cleanup.
pub fn download_update(
    setup_url: &str,
    progress: &SharedProgress,
) -> Result<std::path::PathBuf, String> {
    use std::io::{Read, Write};
    if setup_url.is_empty() {
        return Err(
            "The update service has not supplied a direct EXE URL for this edition of Hebnix."
                .into(),
        );
    }
    #[cfg(not(windows))]
    return Err("Direct EXE updates are available on Windows only.".into());
    #[allow(unreachable_code)]
    {
        let url = url::Url::parse(setup_url).map_err(|e| format!("Invalid update URL: {e}"))?;
        if url.scheme() != "https" {
            return Err("The update URL must use HTTPS.".into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let directory = exe.parent().ok_or("Executable has no parent directory")?;
        let staged = directory.join(format!(".hebnix-update-{:016x}.exe", rand::random::<u64>()));
        let result = (|| -> Result<(), String> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&staged)
                .map_err(|e| format!("Cannot stage the update in {}: {e}", directory.display()))?;
            let response = ureq::AgentBuilder::new()
                .try_proxy_from_env(false)
                .build()
                .get(url.as_str())
                .set("Accept-Encoding", "identity")
                .timeout(Duration::from_secs(600))
                .call()
                .map_err(|e| format!("Failed to download update: {e}"))?;
            let total = response
                .header("Content-Length")
                .and_then(|s| s.parse::<u64>().ok());
            *progress.lock().unwrap() = DownloadProgress {
                total,
                ..Default::default()
            };
            let mut reader = response.into_reader();
            let mut buffer = [0u8; 65536];
            let mut downloaded = 0u64;
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|e| format!("Download interrupted: {e}"))?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])
                    .map_err(|e| e.to_string())?;
                downloaded += count as u64;
                progress.lock().unwrap().downloaded = downloaded;
            }
            if total.is_some_and(|n| n != downloaded) {
                return Err("Incomplete update download.".into());
            }
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            validate_executable(&staged)?;
            progress.lock().unwrap().ready = true;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&staged);
            return Err(error);
        }
        Ok(staged)
    }
}

/// Reject HTML, ZIPs, truncated files and executables for a different CPU.
fn validate_executable(path: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos)
        .map_err(|_| "Update is not a complete Windows executable")?;
    if &dos[..2] != b"MZ" {
        return Err("Update is not a Windows EXE (ZIP installers are no longer supported).".into());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut pe = [0u8; 24];
    file.read_exact(&mut pe)
        .map_err(|_| "Update has an incomplete PE header")?;
    let machine = u16::from_le_bytes([pe[4], pe[5]]);
    let expected = if cfg!(target_arch = "aarch64") {
        0xaa64
    } else {
        0x8664
    };
    let characteristics = u16::from_le_bytes([pe[22], pe[23]]);
    if &pe[..4] != b"PE\0\0" || machine != expected || characteristics & 0x2002 != 0x0002 {
        return Err("Update is not a compatible Windows application.".into());
    }
    let sections = u16::from_le_bytes([pe[6], pe[7]]) as u64;
    let optional_size = u16::from_le_bytes([pe[20], pe[21]]) as u64;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let section_start = offset + 24 + optional_size;
    if sections == 0 || sections > 96 || optional_size < 112 || section_start + sections * 40 > size
    {
        return Err("Update has an incomplete executable header.".into());
    }
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if magic != [0x0b, 0x02] {
        return Err("Update must be a 64-bit Windows EXE.".into());
    }
    file.seek(SeekFrom::Start(section_start))
        .map_err(|e| e.to_string())?;
    for _ in 0..sections {
        let mut section = [0u8; 40];
        file.read_exact(&mut section).map_err(|e| e.to_string())?;
        let raw_size = u32::from_le_bytes(section[16..20].try_into().unwrap()) as u64;
        let raw_start = u32::from_le_bytes(section[20..24].try_into().unwrap()) as u64;
        if raw_size > 0 && (raw_start < section_start || raw_start + raw_size > size) {
            return Err("Update has an incomplete executable section.".into());
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn update_urls_keep_the_full_and_lite_editions_separate() {
        let json = serde_json::json!({"latest_version": "9.0", "setup_url": "https://example.com/hebnix.exe", "lite_setup_url": "https://example.com/hebnix-lite.exe"});
        assert_eq!(
            parse_info(&json, "1.0", false)
                .unwrap()
                .update
                .unwrap()
                .setup_url,
            "https://example.com/hebnix.exe"
        );
        assert_eq!(
            parse_info(&json, "1.0", true)
                .unwrap()
                .update
                .unwrap()
                .setup_url,
            "https://example.com/hebnix-lite.exe"
        );
        let json = serde_json::json!({"latest_version": "9.0", "setup_url": "https://example.com/hebnix.exe"});
        assert!(
            parse_info(&json, "1.0", true)
                .unwrap()
                .update
                .unwrap()
                .setup_url
                .is_empty()
        );
    }

    #[test]
    fn rejects_archives_html_and_truncated_executables() {
        let path =
            std::env::temp_dir().join(format!("hebnix-exe-check-{:016x}", rand::random::<u64>()));
        for data in [
            b"PK\x03\x04".as_slice(),
            b"<html>download failed</html>",
            &[0u8; 128],
        ] {
            std::fs::write(&path, data).unwrap();
            assert!(validate_executable(&path).is_err());
        }
        let executable = std::env::current_exe().unwrap();
        assert!(validate_executable(&executable).is_ok());
        let bytes = std::fs::read(executable).unwrap();
        // All headers remain present, but the final raw section is incomplete.
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(validate_executable(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
