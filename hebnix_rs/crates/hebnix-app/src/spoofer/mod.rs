// crates/hebnix-app/src/spoofer/mod.rs
//! name spoofer. our own CA (ca), our own mitm proxy (proxy), spoofs live in rules.

pub mod ca;
pub mod crl;
pub mod dns;
#[path = "http_linux.rs"]
mod http;
pub mod proxy;
pub mod rules;

pub use crate::recovery_tasks::cleanup_websocket_once;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;

use crate::messages::AppMsg;
use crate::spoofer::rules::{NameRule, Rule, TitleSpoofSettings};
use http::HttpProxy;

pub const PROXY_HOST: &str = "127.0.0.1";
pub const ACCOUNT_HOST: &str = "api.epicgames.dev";
const PROXY_PORTS: &[u16] = &[6969, 143, 6970, 6971, 6972, 6973, 0];
/// bind the first free port of `ports` on PROXY_HOST
pub(crate) fn bind_local(what: &str, ports: &[u16]) -> Result<std::net::TcpListener, String> {
    let mut last_error = None;
    for &port in ports {
        match std::net::TcpListener::bind((PROXY_HOST, port)) {
            Ok(listener) => {
                if let Ok(address) = listener.local_addr() {
                    tracing::info!("{what} listening on {address}");
                }
                return Ok(listener);
            }
            Err(error) => {
                tracing::debug!("{what}: {PROXY_HOST}:{port} unavailable: {error}");
                last_error = Some(error);
            }
        }
    }
    Err(format!(
        "Cannot bind {what}: {}",
        last_error.map_or_else(|| "no ports to try".to_string(), |e| e.to_string())
    ))
}

fn proxy_server(port: u16) -> String {
    format!("http={PROXY_HOST}:{port};https={PROXY_HOST}:{port}")
}

/// is `server` a proxy setting Hebnix wrote, on any port? Older versions
/// wrote a bare `127.0.0.1:8080`.
fn is_hebnix_proxy_server(server: &str) -> bool {
    let port = server
        .strip_prefix(&format!("http={PROXY_HOST}:"))
        .and_then(|rest| rest.split_once(';'))
        .and_then(|(port, _)| port.parse::<u16>().ok());
    port.is_some_and(|port| server == proxy_server(port))
        || server == format!("{PROXY_HOST}:8080")
}
pub const MAX_NAME_LENGTH: usize = 32;
pub const WEBSOCKET_UNAVAILABLE: &str = "WebSocket proxy features are unavailable.";

const INET_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

pub fn is_admin() -> bool {
    type BOOL = i32;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn IsUserAnAdmin() -> BOOL;
    }
    unsafe { IsUserAnAdmin() != 0 }
}

pub const SKIP_ELEVATE_ARG: &str = "--no-elevate";
pub fn disable_saved_http_features(base_dir: &Path) -> std::io::Result<()> {
    for (file, flags) in [
        ("spoofer_settings.json", &["spoofer_master", "spoofer_username_enabled"][..]),
        ("friends.json", &["spoofer_friends_enabled"][..]),
    ] {
        let path = base_dir.join(file);
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let mut settings: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if let Some(object) = settings.as_object_mut() {
            for flag in flags {
                object.insert((*flag).to_string(), serde_json::Value::Bool(false));
            }
        }
        let output = serde_json::to_vec_pretty(&settings)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(path, output)?;
    }
    Ok(())
}
/// The HTTP features use the per-user WinINet proxy and the user certificate
/// store, so they never need Hebnix to run as Administrator.
pub fn http_needs_elevated_relaunch() -> bool {
    false
}

/// Trust the proxy certificate. The caller has checked
/// `http_needs_elevated_relaunch`.
pub fn install_certificate(base_dir: &std::path::Path) -> Result<(), String> {
    ca::install(base_dir)
}

/// Remove the proxy certificate. The caller has checked
/// `http_needs_elevated_relaunch`.
pub fn uninstall_certificate(_base_dir: &std::path::Path) -> Result<(), String> {
    ca::uninstall()
}

pub fn spawn_elevated_relaunch() -> bool {
    use std::os::windows::process::CommandExt;

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    if !crate::maintenance::elevation_location_ready(&exe) {
        return false;
    }
    let exe = exe.to_string_lossy().replace('\'', "''"); // ps single quote escape
    let pid = std::process::id();
    let renderer_args = if crate::window_state::safe_rendering() {
        " -ArgumentList '--safe-rendering'"
    } else {
        ""
    };
    let fallback_args = if crate::window_state::safe_rendering() {
        format!("{SKIP_ELEVATE_ARG} --safe-rendering")
    } else {
        SKIP_ELEVATE_ARG.to_string()
    };
    let script = format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         try {{ Start-Process -FilePath '{exe}'{renderer_args} -Verb RunAs -ErrorAction Stop }} \
         catch {{ Start-Process -FilePath '{exe}' -ArgumentList '{fallback_args}' }}"
    );
    let Some(system_root) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let powershell = PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let started = std::process::Command::new(powershell)
        .env_remove("HEBNIX_RELOCATION_ACK")
        .env_remove("HEBNIX_UPDATE_ACK")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
        .is_ok();
    if started {
        crate::maintenance::acknowledge_update();
    }
    started
}

fn marker_path(base_dir: &Path) -> PathBuf {
    ca::dir(base_dir).join("proxy_backup.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProxyBackup {
    proxy_enable: Option<u32>,
    proxy_server: Option<String>,
    #[serde(default)]
    proxy_override: Option<String>,
}

fn read_current_proxy() -> ProxyBackup {
    use winreg::RegKey;
    let hkcu = RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    match hkcu.open_subkey(INET_KEY) {
        Ok(k) => ProxyBackup {
            proxy_enable: k.get_value("ProxyEnable").ok(),
            proxy_server: k.get_value("ProxyServer").ok(),
            proxy_override: k.get_value("ProxyOverride").ok(),
        },
        Err(_) => ProxyBackup {
            proxy_enable: None,
            proxy_server: None,
            proxy_override: None,
        },
    }
}

fn apply_proxy(state: &ProxyBackup) -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::*;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (settings, _) = hkcu
        .create_subkey(INET_KEY)
        .map_err(|e| format!("open inet key: {e}"))?;

    for (name, value) in [
        ("ProxyServer", &state.proxy_server),
        ("ProxyOverride", &state.proxy_override),
    ] {
        match value {
            Some(v) => settings
                .set_value(name, v)
                .map_err(|e| format!("set {name}: {e}"))?,
            None => {
                let _ = settings.delete_value(name);
            }
        }
    }

    settings
        .set_value("ProxyEnable", &state.proxy_enable.unwrap_or(0))
        .map_err(|e| format!("set ProxyEnable: {e}"))?;
    refresh_wininet();
    Ok(())
}

fn restore_legacy_hebnix_proxy(base_dir: &Path) {
    let marker = marker_path(base_dir);
    if !marker.is_file() {
        return;
    }
    let current = read_current_proxy();
    let legacy_hebnix_proxy = current
        .proxy_server
        .as_deref()
        .is_some_and(is_hebnix_proxy_server);
    if legacy_hebnix_proxy {
        if let Some(backup) = std::fs::read(&marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ProxyBackup>(&bytes).ok())
        {
            if let Err(error) = apply_proxy(&backup) {
                tracing::error!("Could not restore HTTP proxy settings: {error}");
                return;
            }
        } else {
            tracing::error!("Cannot read the HTTP proxy backup; keeping it for recovery");
            return;
        }
    }
    let _ = std::fs::remove_file(&marker);
}

pub fn restore_if_crashed(base_dir: &Path) {
    if marker_path(base_dir).is_file() {
        tracing::warn!("stale HTTP proxy marker, restoring system proxy");
        restore_legacy_hebnix_proxy(base_dir);
    }
    // Older versions redirected the account host through the hosts file.
    if is_admin() {
        if let Err(error) = crate::hosts_file::set_host_redirect(ACCOUNT_HOST, false) {
            tracing::warn!("Could not clean up the account HTTP redirect: {error}");
        }
    }
}

fn refresh_wininet() {
    use windows::Win32::Networking::WinInet::{
        INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
    };
    unsafe {
        let _ = InternetSetOptionW(None, INTERNET_OPTION_SETTINGS_CHANGED, None, 0);
        let _ = InternetSetOptionW(None, INTERNET_OPTION_REFRESH, None, 0);
    }
}

pub struct SpooferManager {
    base_dir: PathBuf,
    tx: Sender<AppMsg>,
    spoofed_name: Arc<Mutex<String>>,
    pub spoofed_friends: Arc<Mutex<HashMap<String, String>>>,
    pub discovered_friends: Arc<Mutex<HashMap<String, String>>>,
    http_proxy: Mutex<Option<HttpProxy>>,
    http_active: Arc<AtomicBool>,
    redirect_active: AtomicBool,
    game_present: Arc<AtomicBool>,
    item_cleanup: crate::item_cleanup::Cleaner,
    crl: Mutex<Option<crl::CrlServer>>,
    runtime_lock: Mutex<()>,
    certificate_blocked: AtomicBool,
    closing: AtomicBool,
    lifecycle_started: AtomicBool,
    requested_http: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxyOptions {
    pub account: bool,
    pub psynet: bool,
    pub replays: bool,
}

impl ProxyOptions {
    pub fn http_only(self) -> Self {
        Self {
            account: self.account,
            psynet: false,
            replays: false,
        }
    }

    fn for_runtime(self, game: bool, closing: bool, blocked: bool) -> Self {
        if game && !closing && !blocked {
            self.http_only()
        } else {
            Self::default()
        }
    }
}

impl SpooferManager {
    pub fn new(base_dir: PathBuf, tx: Sender<AppMsg>) -> Self {
        let item_cleanup = crate::item_cleanup::Cleaner::new(&base_dir);
        Self {
            base_dir,
            tx,
            spoofed_name: Arc::new(Mutex::new(String::new())),
            spoofed_friends: Arc::new(Mutex::new(HashMap::new())),
            discovered_friends: Arc::new(Mutex::new(HashMap::new())),
            http_proxy: Mutex::new(None),
            http_active: Arc::new(AtomicBool::new(false)),
            redirect_active: AtomicBool::new(false),
            game_present: Arc::new(AtomicBool::new(
                hebnix_sdk::process::is_rocket_league_running(),
            )),
            item_cleanup,
            crl: Mutex::new(None),
            runtime_lock: Mutex::new(()),
            certificate_blocked: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            lifecycle_started: AtomicBool::new(false),
            requested_http: AtomicBool::new(false),
        }
    }

    pub fn game_running(&self) -> bool {
        self.game_present.load(Ordering::Acquire)
    }
    pub fn configure(&self, options: ProxyOptions) {
        self.requested_http
            .store(options.http_only().account, Ordering::Release);
    }
    pub fn http_running(&self) -> bool {
        self.http_active.load(Ordering::Acquire)
            && self
                .http_proxy
                .lock()
                .is_ok_and(|proxy| proxy.as_ref().is_some_and(HttpProxy::is_running))
    }
    pub fn account_redirect_active(&self) -> bool {
        self.redirect_active.load(Ordering::Acquire)
    }
    pub fn socket_running(&self) -> bool {
        false
    }
    pub fn rlapi_running(&self) -> bool {
        false
    }
    pub fn relay_running(&self) -> bool {
        false
    }
    pub fn enable_rlapi(&self) -> Result<(), String> {
        Err(WEBSOCKET_UNAVAILABLE.into())
    }
    pub fn disable_rlapi(&self) {
        hebnix_sdk::rlapi::session::shared_game_session().reset();
    }
    pub fn cleanup_idle_rlapi(&self) {
        self.disable_rlapi();
    }
    pub fn update_ranks(&self, _ranks: HashMap<i32, (i32, f64)>) {}
    pub fn set_titles(&self, _titles: Vec<TitleSpoofSettings>) {}
    pub fn set_title_enabled(&self, _enabled: bool) {}

    pub fn set_username(&self, name: &str) {
        if let Ok(mut guard) = self.spoofed_name.lock() {
            *guard = name.chars().take(MAX_NAME_LENGTH).collect();
        }
    }
    pub fn update_friends(&self, spoofs: HashMap<String, String>) {
        if let Ok(mut guard) = self.spoofed_friends.lock() {
            *guard = spoofs;
        }
    }
    pub fn item_status(&self) -> crate::item_cleanup::ItemStatus {
        self.item_cleanup.status()
    }
    pub fn begin_item_clear(&self) -> Result<(), String> {
        self.item_cleanup.begin_clear()
    }
    pub fn cancel_item_clear(&self) -> Result<(), String> {
        self.item_cleanup.cancel_clear()
    }
    pub fn finish_item_clear(&self) -> Result<bool, String> {
        self.item_cleanup.finish_clear()
    }

    fn start_http(&self) -> Result<(), String> {
        if self.http_running() {
            if !crate::watchdog::is_ready() && !crate::watchdog::spawn() {
                return Err("HTTP proxy recovery watchdog stopped.".into());
            }
            self.http_active.store(true, Ordering::Release);
            return Ok(());
        }
        if !ca::is_current_installed(&self.base_dir) {
            return Err(
                "Install the Hebnix certificate in Settings > System to enable HTTP proxying."
                    .into(),
            );
        }
        let ca = Arc::new(ca::ensure(&self.base_dir)?);
        let mut crl = self.crl.lock().map_err(|_| "CRL lock poisoned")?;
        if crl.is_none() {
            *crl = Some(crl::CrlServer::start(ca.crl_der()?, self.tx.clone())?);
        }
        drop(crl);
        let rules: Vec<Box<dyn Rule>> = vec![
            Box::new(NameRule::new(Arc::clone(&self.spoofed_name))),
            Box::new(rules::FriendsRule::new(
                Arc::clone(&self.spoofed_friends),
                Arc::clone(&self.discovered_friends),
            )),
        ];
        let rules = Arc::new(
            rules
                .into_iter()
                .map(|inner| {
                    Box::new(rules::EnabledRule {
                        inner,
                        http: Arc::clone(&self.http_active),
                        socket: Arc::new(AtomicBool::new(false)),
                        game: Arc::clone(&self.game_present),
                    }) as Box<dyn Rule>
                })
                .collect(),
        );
        let proxy = HttpProxy::start(ca, rules, self.tx.clone(), PROXY_PORTS)?;
        // Recovery must be ready before the system proxy points at this listener.
        if !crate::watchdog::spawn() {
            proxy.stop();
            return Err("Could not start HTTP proxy recovery watchdog.".into());
        }
        // A marker left by a crash already holds the user's original settings.
        let marker = marker_path(&self.base_dir);
        let current = read_current_proxy();
        let ours = current.proxy_server.as_deref().is_some_and(is_hebnix_proxy_server);
        if !(ours && marker.is_file()) {
            let backup = serde_json::to_vec(&current).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(marker.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&marker, backup).map_err(|e| format!("write proxy backup: {e}"))?;
        }
        *self
            .http_proxy
            .lock()
            .map_err(|_| "HTTP proxy lock poisoned")? = Some(proxy);
        let port = self
            .http_proxy
            .lock()
            .ok()
            .and_then(|p| p.as_ref().map(HttpProxy::port))
            .ok_or("HTTP proxy stopped")?;
        self.http_active.store(true, Ordering::Release);
        self.redirect_active.store(true, Ordering::Release);
        apply_proxy(&ProxyBackup {
            proxy_enable: Some(1),
            proxy_server: Some(proxy_server(port)),
            proxy_override: Some("127.0.0.1;localhost;<local>".into()),
        })?;
        let _ = self.tx.send(AppMsg::Log(format!(
            "[Spoofer] Account HTTPS proxy ready on {PROXY_HOST}:{port} for {ACCOUNT_HOST}. Restart Rocket League to refresh its cached display name."
        )));
        Ok(())
    }

    fn stop_http(&self) -> Result<(), String> {
        self.http_active.store(false, Ordering::Release);
        if self.redirect_active.load(Ordering::Acquire) {
            // If cleanup fails, keep forwarding through the listener until recovery succeeds.
            restore_legacy_hebnix_proxy(&self.base_dir);
            if marker_path(&self.base_dir).is_file() {
                return Err("could not restore the Windows proxy settings".into());
            }
            self.redirect_active.store(false, Ordering::Release);
        }
        if let Ok(mut proxy) = self.http_proxy.lock() {
            if let Some(proxy) = proxy.take() {
                proxy.stop();
            }
        }
        if let Ok(mut crl) = self.crl.lock() {
            if let Some(crl) = crl.take() {
                crl.stop();
            }
        }
        Ok(())
    }

    fn sync_proxy_options(&self) -> Result<(), String> {
        let _guard = self.runtime_lock.lock().unwrap_or_else(|e| e.into_inner());
        let options = ProxyOptions {
            account: self.requested_http.load(Ordering::Acquire),
            ..Default::default()
        }
        .for_runtime(
            self.game_running(),
            self.closing.load(Ordering::Acquire),
            self.certificate_blocked.load(Ordering::Acquire),
        );
        if options.account {
            if let Err(error) = self.start_http() {
                if let Err(cleanup) = self.stop_http() {
                    return Err(format!("{error}. Account redirect cleanup also failed: {cleanup}"));
                }
                return Err(error);
            }
        } else {
            self.stop_http()?;
        }
        Ok(())
    }
    pub fn disable_spoofing(&self) {
        self.closing.store(true, Ordering::Release);
        self.shutdown();
    }
    pub fn shutdown(&self) {
        let _guard = self.runtime_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.requested_http.store(false, Ordering::Release);
        self.disable_rlapi();
        if let Err(error) = self.stop_http() {
            tracing::error!("Account HTTP proxy cleanup failed: {error}");
            let _ = self.tx.send(AppMsg::Log(format!("[Spoofer] Account redirect cleanup failed: {error}")));
        }
    }
    pub fn block_for_certificate_removal(&self) {
        self.certificate_blocked.store(true, Ordering::Release);
        self.shutdown();
    }
    pub fn certificate_setup_ready(&self) {
        self.certificate_blocked.store(false, Ordering::Release);
    }
    pub fn start_lifecycle_monitor(self: &Arc<Self>) -> Result<(), String> {
        if self.lifecycle_started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let manager = Arc::downgrade(self);
        let result = std::thread::Builder::new()
            .name("proxy-lifecycle".into())
            .spawn(move || {
                let mut last_error = None;
                loop {
                    let Some(manager) = manager.upgrade() else {
                        break;
                    };
                    manager.game_present.store(
                        hebnix_sdk::process::is_rocket_league_running(),
                        Ordering::Release,
                    );
                    if manager.requested_http.load(Ordering::Acquire)
                        && !manager.certificate_blocked.load(Ordering::Acquire)
                        && !ca::is_current_installed(&manager.base_dir)
                    {
                        manager.block_for_certificate_removal();
                        let _ = manager.tx.send(AppMsg::SpooferCertificateLost);
                    }
                    match manager.sync_proxy_options() {
                        Err(error) => {
                            if last_error.as_ref() != Some(&error) {
                                let _ = manager.tx.send(AppMsg::Log(format!("[Spoofer] {error}")));
                            }
                            last_error = Some(error);
                        }
                        Ok(()) => last_error = None,
                    }
                    drop(manager);
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            });
        if let Err(error) = result {
            self.lifecycle_started.store(false, Ordering::Release);
            return Err(format!("Could not start HTTP proxy monitor: {error}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod proxy_tests {
    use super::ProxyOptions;
    #[test]
    fn busy_ports_fall_back_to_the_next_one() {
        let taken = std::net::TcpListener::bind((super::PROXY_HOST, 0)).unwrap();
        let busy = taken.local_addr().unwrap().port();
        let listener = super::bind_local("test", &[busy, 0]).unwrap();
        assert_ne!(listener.local_addr().unwrap().port(), busy);
        assert!(super::bind_local("test", &[busy]).is_err());
    }
    #[test]
    fn hebnix_proxy_settings_are_recognised_on_any_port() {
        for port in [6969, 143, 8080, 51234] {
            assert!(super::is_hebnix_proxy_server(&super::proxy_server(port)));
        }
        assert!(super::is_hebnix_proxy_server("127.0.0.1:8080"));
        for other in [
            "http=127.0.0.1:6969;https=127.0.0.1:7000",
            "http=10.0.0.1:6969;https=10.0.0.1:6969",
            "proxy.example.com:3128",
            "",
        ] {
            assert!(!super::is_hebnix_proxy_server(other), "{other}");
        }
    }
    #[test]
    fn declined_elevation_disables_http_features_but_preserves_customisation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("hebnix-uac-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let settings = serde_json::json!({
            "spoofer_master": false,
            "spoofer_username_enabled": true,
            "spoofer_username": "名前🚀",
            "spoofer_username_history": ["名前🚀", "Previous"],
            "spoofer_rank_enabled": true,
            "spoofer_titles": [{"text": "Local title"}],
        });
        let friends = serde_json::json!({
            "spoofer_friends_enabled": true,
            "friends": {"id": {"enabled": true, "spoofed_name": "Friend"}},
        });
        std::fs::write(directory.join("spoofer_settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
        std::fs::write(directory.join("friends.json"), serde_json::to_vec(&friends).unwrap()).unwrap();
        super::disable_saved_http_features(&directory).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.join("spoofer_settings.json")).unwrap()).unwrap();
        let saved_friends: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.join("friends.json")).unwrap()).unwrap();
        assert_eq!(saved["spoofer_username_enabled"], false);
        assert_eq!(saved["spoofer_master"], false);
        assert_eq!(saved_friends["spoofer_friends_enabled"], false);
        for key in ["spoofer_username", "spoofer_username_history", "spoofer_rank_enabled", "spoofer_titles"] {
            assert_eq!(saved[key], settings[key]);
        }
        assert_eq!(saved_friends["friends"], friends["friends"]);
        // The fallback is safe to repeat, including on an empty installation.
        super::disable_saved_http_features(&directory).unwrap();
        std::fs::remove_file(directory.join("spoofer_settings.json")).unwrap();
        std::fs::remove_file(directory.join("friends.json")).unwrap();
        super::disable_saved_http_features(&directory).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn websocket_consumers_cannot_start_any_proxy() {
        for bits in 0..8 {
            let requested = ProxyOptions {
                account: bits & 1 != 0,
                psynet: bits & 2 != 0,
                replays: bits & 4 != 0,
            };
            let options = requested.http_only();
            assert_eq!(options.account, requested.account);
            assert!(!options.psynet && !options.replays);
            for state in 0..8 {
                let effective =
                    requested.for_runtime(state & 1 != 0, state & 2 != 0, state & 4 != 0);
                assert_eq!(effective.account, requested.account && state == 1);
                assert!(!effective.psynet && !effective.replays);
            }
        }
    }
}
