use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PLUGIN_PREFIX: &str = "hebnix://install/plugin/";
const THEME_PREFIX: &str = "hebnix://install/theme/";
const MAX_THEME_DOWNLOAD: u64 = 50 * 1024 * 1024;

/// Filesystem polling belongs to a worker, never to egui's per-frame callback.
pub struct PendingInstalls {
    batches: crossbeam_channel::Receiver<(Vec<String>, Vec<String>)>,
    _stop: std::sync::mpsc::Sender<()>,
}

impl PendingInstalls {
    pub fn start(base_dir: &Path, ctx: &eframe::egui::Context) -> Self {
        let base_dir = base_dir.to_path_buf();
        let ctx = ctx.clone();
        let (tx, batches) = crossbeam_channel::unbounded();
        let (stop, stopped) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("install-links".into())
            .spawn(move || loop {
                let plugins = take_pending_plugin_ids(&base_dir);
                let themes = take_pending_theme_ids(&base_dir);
                if !plugins.is_empty() || !themes.is_empty() {
                    if tx.send((plugins, themes)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                match stopped.recv_timeout(Duration::from_millis(500)) {
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            })
            .expect("failed to spawn install link worker");
        Self { batches, _stop: stop }
    }

    pub fn take(&self) -> Option<(Vec<String>, Vec<String>)> {
        self.batches.try_recv().ok()
    }
}

pub fn register_and_queue_from_args(base_dir: &Path) {
    if let Err(error) = register_protocol() {
        tracing::warn!("failed to register hebnix:// protocol: {error}");
    }
    for argument in std::env::args().skip(1) {
        let queued = if let Some(id) = id_from_url(&argument, PLUGIN_PREFIX) {
            queue_install(base_dir, "plugin", id)
        } else if let Some(id) = id_from_url(&argument, THEME_PREFIX) {
            queue_install(base_dir, "theme", id)
        } else {
            continue;
        };
        if let Err(error) = queued {
            tracing::error!("failed to queue install link '{argument}': {error}");
        }
    }
}

#[cfg(target_os = "windows")]
fn register_protocol() -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let executable = executable
        .to_str()
        .ok_or_else(|| "application path is not valid Unicode".to_string())?;
    let classes = RegKey::predef(HKEY_CURRENT_USER);
    let (protocol, _) = classes
        .create_subkey("Software\\Classes\\hebnix")
        .map_err(|error| error.to_string())?;
    protocol
        .set_value("", &"URL:Hebnix Installer")
        .map_err(|error| error.to_string())?;
    protocol
        .set_value("URL Protocol", &"")
        .map_err(|error| error.to_string())?;
    let (command, _) = protocol
        .create_subkey("shell\\open\\command")
        .map_err(|error| error.to_string())?;
    command
        .set_value("", &format!("\"{executable}\" \"%1\""))
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(target_os = "linux")]
/// Register this binary as the `hebnix://` handler for the current user
/// (a .desktop entry + xdg-mime default). Idempotent: only rewrites the
/// entry when the executable path changed.
fn register_protocol() -> Result<(), String> {
    // an AppImage's current_exe is a throwaway mount; register the image itself
    let executable = match std::env::var_os("APPIMAGE").filter(|value| !value.is_empty()) {
        Some(image) => std::path::PathBuf::from(image),
        None => std::env::current_exe().map_err(|error| error.to_string())?,
    };
    let executable = executable
        .to_str()
        .ok_or_else(|| "application path is not valid Unicode".to_string())?;
    let applications = dirs::data_dir()
        .ok_or_else(|| "no XDG data directory".to_string())?
        .join("applications");
    let entry = applications.join("hebnix-url-handler.desktop");
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Hebnix Installer\nExec=\"{}\" %u\n\
         NoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/hebnix;\n",
        executable.replace('\\', "\\\\").replace('"', "\\\"")
    );
    if std::fs::read_to_string(&entry).ok().as_deref() == Some(contents.as_str()) {
        return Ok(());
    }
    std::fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
    std::fs::write(&entry, contents).map_err(|error| error.to_string())?;
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", "hebnix-url-handler.desktop", "x-scheme-handler/hebnix"])
        .status();
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&applications)
        .status();
    Ok(())
}

fn id_from_url<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let id = value.strip_prefix(prefix)?.trim_end_matches('/');
    (!id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(id)
}

fn pending_dir(base_dir: &Path, kind: &str) -> PathBuf {
    base_dir.join(format!("pending-{kind}-installs"))
}

fn queue_install(base_dir: &Path, kind: &str, id: &str) -> Result<(), String> {
    let directory = pending_dir(base_dir, kind);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    std::fs::write(
        directory.join(format!("{}-{stamp}.pending", std::process::id())),
        id,
    )
    .map_err(|error| error.to_string())
}

fn take_pending_ids(base_dir: &Path, kind: &str, prefix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(pending_dir(base_dir, kind)) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("pending"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let id = std::fs::read_to_string(&path).ok();
            let _ = std::fs::remove_file(path);
            id.filter(|value| id_from_url(&format!("{prefix}{value}"), prefix).is_some())
        })
        .collect()
}

pub fn take_pending_plugin_ids(base_dir: &Path) -> Vec<String> {
    take_pending_ids(base_dir, "plugin", PLUGIN_PREFIX)
}

pub fn take_pending_theme_ids(base_dir: &Path) -> Vec<String> {
    take_pending_ids(base_dir, "theme", THEME_PREFIX)
}

fn catalog_matches(entry: &serde_json::Value, id: &str) -> bool {
    entry.get("plugin_id").and_then(serde_json::Value::as_str) == Some(id)
        || entry.get("id").is_some_and(|value| {
            value.as_str() == Some(id)
                || value.as_u64().is_some_and(|number| number.to_string() == id)
        })
}

fn catalog_item(endpoint: &str, id: &str) -> Result<serde_json::Value, String> {
    let catalog: serde_json::Value = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .build()
        .get(endpoint)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|error| error.to_string())?
        .into_json()
        .map_err(|error| error.to_string())?;
    catalog
        .as_array()
        .and_then(|entries| {
            entries.iter().find(|entry| catalog_matches(entry, id))
        })
        .cloned()
        .ok_or_else(|| format!("Item ID {id} was not found in the Hebnix catalog."))
}

pub fn plugin_identity(id: &str) -> Result<(String, String), String> {
    let plugin = catalog_item("https://api.hebnix.com/plugins", id)?;
    let name = plugin
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(id);
    let author = plugin
        .get("author")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unknown");
    Ok((name.to_string(), author.to_string()))
}

pub fn install_theme(
    id: &str,
    themes_dir: &Path,
    fonts_dir: &Path,
) -> Result<(String, String), String> {
    let item = catalog_item("https://api.hebnix.com/themes", id)?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !item_type.eq_ignore_ascii_case("theme") {
        return Err(format!("Item ID {id} is not a theme"));
    }
    let name = item
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(id)
        .to_string();
    let author = item
        .get("author")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let source_name = item
        .get("file_path")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("theme.toml");

    let response = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .build()
        .get(&format!("https://api.hebnix.com/download/item/{id}"))
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_THEME_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_THEME_DOWNLOAD {
        return Err("Theme download exceeds the 50 MB limit".to_string());
    }

    std::fs::create_dir_all(themes_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(fonts_dir).map_err(|error| error.to_string())?;
    if bytes.starts_with(b"PK\x03\x04") {
        install_theme_zip(&bytes, themes_dir, fonts_dir)?;
    } else {
        validate_theme(&bytes)?;
        let filename = safe_filename(source_name, "toml")?;
        std::fs::write(themes_dir.join(filename), bytes).map_err(|error| error.to_string())?;
    }
    Ok((name, author))
}

pub(crate) fn import_theme_zip(path: &Path, themes_dir: &Path, fonts_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_THEME_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_THEME_DOWNLOAD {
        return Err("Theme archive exceeds the 50 MB limit".into());
    }
    install_theme_zip(&bytes, themes_dir, fonts_dir)
}

fn install_theme_zip(bytes: &[u8], themes_dir: &Path, fonts_dir: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| error.to_string())?;
    let mut files = Vec::new();
    let mut theme_count = 0;
    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_dir() || entry.size() > MAX_THEME_DOWNLOAD {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let asset_destination = if path.starts_with("assets") && matches!(extension, "png" | "jpg" | "jpeg" | "webp") {
            let asset = path.to_string_lossy().replace('\\', "/");
            crate::theme::validate_asset_path(&asset)?;
            Some(themes_dir.join(&path))
        } else { None };
        let destination = if asset_destination.is_some() {
            themes_dir
        } else if extension.eq_ignore_ascii_case("toml") {
            theme_count += 1;
            themes_dir
        } else if extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
            || (extension.eq_ignore_ascii_case("txt") && filename.ends_with("-OFL.txt")) {
            fonts_dir
        } else {
            continue;
        };
        let mut contents = Vec::new();
        entry
            .by_ref()
            .take(MAX_THEME_DOWNLOAD + 1)
            .read_to_end(&mut contents)
            .map_err(|error| error.to_string())?;
        total_bytes += contents.len() as u64;
        if total_bytes > MAX_THEME_DOWNLOAD { return Err("Expanded theme archive exceeds 50 MB".into()); }
        if contents.len() as u64 > MAX_THEME_DOWNLOAD {
            return Err(format!("Theme archive entry '{filename}' is too large"));
        }
        if extension.eq_ignore_ascii_case("toml") {
            validate_theme(&contents)?;
        }
        let mut path = asset_destination.unwrap_or(destination.join(safe_filename(filename, extension)?));
        if extension.eq_ignore_ascii_case("toml") {
            path.set_extension("toml");
        }
        files.push((path, contents));
    }
    if theme_count == 0 {
        return Err("Theme archive does not contain a TOML theme file".to_string());
    }
    for (path, contents) in &files {
        if path.extension().is_some_and(|e| e == "toml") {
            let text = std::str::from_utf8(contents).map_err(|e| e.to_string())?;
            let theme = crate::theme::parse_theme(text)?;
            if let Some(asset) = theme.assets.background {
                if !files.iter().any(|(p,_)| *p == themes_dir.join(&asset)) {
                    return Err(format!("Theme archive is missing background: {asset}"));
                }
            }
        }
    }
    for (path, contents) in files {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        std::fs::write(path, contents).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn validate_theme(bytes: &[u8]) -> Result<(), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "Theme TOML is not valid UTF-8".to_string())?;
    crate::theme::parse_theme(text)
        .map(|_| ())
        .map_err(|error| format!("Invalid theme TOML: {error}"))
}

fn safe_filename<'a>(value: &'a str, expected_extension: &str) -> Result<&'a str, String> {
    let path = Path::new(value);
    let is_single_component = path.components().count() == 1 && path.file_name().is_some();
    let extension_matches = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension));
    (is_single_component && extension_matches && !value.contains([':', '\\', '/']))
        .then_some(value)
        .ok_or_else(|| format!("Unsafe theme filename '{value}'"))
}

#[cfg(test)]
mod theme_archive_tests {
    use super::*;

    #[test]
    fn matches_numeric_and_string_catalog_ids() {
        assert!(catalog_matches(&serde_json::json!({"id": 381}), "381"));
        assert!(catalog_matches(&serde_json::json!({"id": "381"}), "381"));
        assert!(catalog_matches(&serde_json::json!({"id": 5, "plugin_id": "abc"}), "abc"));
        assert!(catalog_matches(&serde_json::json!({"id": 381, "plugin_id": ""}), "381"));
        assert!(!catalog_matches(&serde_json::json!({"id": 381}), "382"));
    }

    #[test]
    fn install_worker_delivers_both_link_types_once_and_keeps_polling() {
        let base = std::env::temp_dir()
            .join(format!("hebnix-install-links-{}", rand::random::<u64>()));
        queue_install(&base, "plugin", "plugin-one").unwrap();
        queue_install(&base, "theme", "theme-one").unwrap();
        queue_install(&base, "plugin", "invalid/link").unwrap();
        let pending = PendingInstalls::start(&base, &eframe::egui::Context::default());
        assert_eq!(
            pending.batches.recv_timeout(Duration::from_secs(5)).unwrap(),
            (vec!["plugin-one".to_string()], vec!["theme-one".to_string()]),
        );
        assert!(pending.take().is_none());
        queue_install(&base, "plugin", "plugin-two").unwrap();
        assert_eq!(
            pending.batches.recv_timeout(Duration::from_secs(5)).unwrap(),
            (vec!["plugin-two".to_string()], Vec::new()),
        );
        assert!(pending.take().is_none());
        drop(pending);
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let resolved = base.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp) && resolved != temp);
        std::fs::remove_dir_all(resolved).unwrap();
    }

    #[test]
    #[ignore = "v2 example theme packages were never committed"]
    fn builder_theme_zip_installs_and_loads_with_background() {
        let base = std::env::temp_dir().join(format!("hebnix-theme-archive-{}",std::process::id()));
        let themes = base.join("themes");
        let fonts = base.join("fonts");
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::create_dir_all(&fonts).unwrap();
        for (bytes,name) in [
            (std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),"/../../examples/themes/packages/Carbon Studio.zip")).unwrap().as_slice(),"Carbon Studio"),
            (std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),"/../../examples/themes/packages/Ocean Workspace.zip")).unwrap().as_slice(),"Ocean Workspace"),
            (std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),"/../../examples/themes/packages/Crimson Arena.zip")).unwrap().as_slice(),"Crimson Arena"),
            (std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),"/../../examples/themes/packages/Ocean Studio Dark.zip")).unwrap().as_slice(),"Ocean Studio Dark"),
            (std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),"/../../examples/themes/packages/Hebnix 2.zip")).unwrap().as_slice(),"Hebnix 2"),
            (include_bytes!("../testdata/theme-background.zip").as_slice(),"Test Background"),
        ] {
            install_theme_zip(bytes,&themes,&fonts).unwrap();
            let ctx=eframe::egui::Context::default();
            crate::theme::apply_theme(&ctx,&themes,&fonts,name).unwrap();
            assert!(crate::theme::presentation(&ctx).is_some());
        }
        assert!(themes.join("assets/test/background.png").is_file());
        assert!(fonts.join("Barlow-Medium.ttf").is_file());
        assert!(std::fs::read_to_string(fonts.join("Barlow-Medium-OFL.txt")).unwrap().contains("SIL OPEN FONT LICENSE"));
        let temp=std::env::temp_dir().canonicalize().unwrap();
        let resolved=base.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp) && resolved!=temp);
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
