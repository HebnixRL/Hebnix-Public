#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod auto_upload_replays;
mod replay_metadata;
mod replay_scan;
mod replay_timeline;
mod replay_source;
mod replay_history_log;
#[cfg(target_os = "windows")]
mod replay_history_patch;
#[cfg(target_os = "linux")]
#[path = "replay_history_patch_linux.rs"]
mod replay_history_patch;
mod patcher;
mod ball {
    pub use crate::patcher::ball::*;
}
mod boost_patcher {
    pub use crate::patcher::boost_patcher::*;
}
mod car_patcher {
    pub use crate::patcher::car_patcher::*;
}
#[cfg(target_os = "linux")]
mod colours {
    pub use crate::patcher::colours::*;
}
mod config;
mod cosmetic_thumbnail {
    pub use crate::patcher::cosmetic_thumbnail::*;
}
mod cosmetic_upk {
    pub use crate::patcher::cosmetic_upk::*;
}
mod decal_patcher {
    pub use crate::patcher::decal_patcher::*;
}
mod deep_link;
mod discord_link;
mod discord_presence;
#[cfg(target_os = "windows")]
mod dpi_fix;
#[cfg(target_os = "linux")]
#[path = "dpi_fix_linux.rs"]
mod dpi_fix;
mod epic_connection;
#[cfg(target_os = "windows")]
mod hotkey;
#[cfg(target_os = "linux")]
#[path = "hotkey_linux.rs"]
mod hotkey;
#[cfg(target_os = "windows")]
mod hosts_file;
mod i18n;
mod item_cleanup;
mod messages;
#[cfg(target_os = "windows")]
mod monitor;
#[cfg(target_os = "linux")]
#[path = "monitor_linux.rs"]
mod monitor;
mod multiplayer_assets;
#[cfg(target_os = "windows")]
#[path = "multiplayer-lan/mod.rs"]
mod multiplayer_lan;
#[cfg(target_os = "linux")]
mod multiplayer_lan;
mod notifications;
#[cfg(target_os = "windows")]
mod overlay;
#[cfg(target_os = "linux")]
#[path = "overlay/mod_linux.rs"]
mod overlay;
mod owned_inventory;
mod veryimportantfile;
mod patch_core {
    pub use crate::patcher::patch_core::*;
}
mod plugins;
mod presets;
#[cfg(target_os = "windows")]
mod rl_launch;
#[cfg(target_os = "linux")]
#[path = "rl_launch_linux.rs"]
mod rl_launch;
#[cfg(target_os = "windows")]
mod recovery_tasks;
#[cfg(target_os = "linux")]
#[path = "recovery_tasks_linux.rs"]
mod recovery_tasks;
mod runtime_assets;
mod screen;
#[cfg(target_os = "windows")]
mod spoofer;
#[cfg(target_os = "linux")]
#[path = "spoofer/mod_linux.rs"]
mod spoofer;
mod statsapi_ini;
mod speed_patch {
    pub use crate::patcher::speed_patch::*;
}
mod swapper {
    pub use crate::patcher::swapper::*;
}
mod theme;
mod toast;
mod tray;
mod ui;
mod ui_work;
mod update;
mod maintenance;
mod upk_keys {
    pub use crate::patcher::upk_keys::*;
}
#[cfg(target_os = "windows")]
mod watchdog;
#[cfg(target_os = "linux")]
#[path = "watchdog_linux.rs"]
mod watchdog;
#[cfg(target_os = "windows")]
mod webview;
#[cfg(target_os = "linux")]
#[path = "webview_linux.rs"]
mod webview;
mod winutil;
#[cfg(target_os = "windows")]
mod window_state;
#[cfg(target_os = "linux")]
#[path = "window_state_linux.rs"]
mod window_state;
#[cfg(target_os = "windows")]
mod visibility;
#[cfg(target_os = "linux")]
#[path = "visibility_linux.rs"]
mod visibility;

use app::HebnixApp;

fn load_window_icon(base_dir: &std::path::Path) -> Option<eframe::egui::IconData> {
    let bytes =
        std::fs::read(base_dir.join("hebnix.ico")).unwrap_or_else(|_| tray::EMBEDDED_ICON.to_vec());
    let img = image::load_from_memory(&bytes).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(eframe::egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

fn open_fresh_log(base_dir: &std::path::Path) -> std::io::Result<std::fs::File> {
    let log_path = base_dir.join("hebnix.log");
    let old_log_path = base_dir.join("hebnix.log.old");

    match std::fs::remove_file(&old_log_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match std::fs::rename(&log_path, &old_log_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
}

fn setup_logging(base_dir: &std::path::Path) {
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into());

    // sync writer, nothing buffered so the log survives a hard crash.
    let log_file = open_fresh_log(base_dir).map_err(|error| {
        eprintln!("failed to refresh hebnix.log: {error}");
    });

    match log_file {
        Ok(file) => {
            let file = std::sync::Mutex::new(file);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(file.and(std::io::stdout))
                .init();
        }
        Err(()) => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
}

fn setup_panic_hook(base_dir: &std::path::Path) {
    let crash_path = base_dir.join("crash.txt");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let thread = std::thread::current();
        let text = format!(
            "=== PANIC (unix time {ts}, thread {:?}) ===\n{info}\n\nbacktrace:\n{backtrace}\n\n",
            thread.name().unwrap_or("<unnamed>")
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_path)
        {
            use std::io::Write;
            let _ = f.write_all(text.as_bytes());
        }
        tracing::error!("PANIC: {info}");
        default_hook(info);
    }));
}

#[cfg(target_os = "windows")]
fn dcomp_wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::wgpu;

    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        let desc = &mut setup.instance_descriptor;
        desc.backends = wgpu::Backends::DX12;
        desc.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
        desc.backend_options.dx12.presentation_system = if window_state::safe_rendering() {
            wgpu::Dx12SwapchainKind::DxgiFromHwnd
        } else {
            wgpu::Dx12SwapchainKind::DxgiFromVisual
        };
    }
    options
}

#[cfg(target_os = "linux")]
fn main() -> eframe::Result {
    unsafe {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()) {
            std::env::set_var("GDK_BACKEND", "wayland");
        }
    }

    if let Some(code) = spoofer::maybe_handle_privileged_cli() {
        std::process::exit(code);
    }

    let base_dir = config::base_dir();
    if let Some(parent_pid) = watchdog::parent_pid() {
        watchdog::run(parent_pid);
        return Ok(());
    }
    setup_logging(&base_dir);
    deep_link::register_and_queue_from_args(&base_dir);
    setup_panic_hook(&base_dir);
    tracing::info!("Hebnix {} starting", app::APP_VERSION);
    multiplayer_lan::raise_net_admin_ambient();

    let cfg = config::Config::load(&base_dir);
    i18n::init(&cfg.settings.language, Some(base_dir.join("locales")));

    let Some(_lock) = winutil::acquire_single_instance() else {
        winutil::focus_existing_instance();
        return Ok(());
    };

    spoofer::restore_if_crashed(&base_dir);
    if let Err(error) = spoofer::cleanup_websocket_once(&base_dir) {
        tracing::warn!("WebSocket proxy cleanup: {error}");
    }
    let _ = watchdog::spawn();

    hebnix_sdk::process::exempt_own_window_decorations();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Hebnix")
        .with_inner_size([
            (cfg.window.width as f32).max(app::MIN_WIDTH),
            (cfg.window.height as f32).max(app::MIN_HEIGHT),
        ])
        .with_min_inner_size([app::MIN_WIDTH, app::MIN_HEIGHT])
        .with_transparent(true)
        .with_decorations(false)
        .with_visible(!cfg.settings.start_in_tray);
    #[cfg(target_os = "linux")]
    if let (Some(x), Some(y)) = (cfg.window.x, cfg.window.y) {
        viewport = viewport.with_position([x as f32, y as f32]);
    }
    if let Some(icon) = load_window_icon(&base_dir) {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let result = eframe::run_native(
        "Hebnix",
        options,
        Box::new(|cc| Ok(Box::new(HebnixApp::new(cc)))),
    );
    watchdog::finish_live_handoff();
    result
}

#[cfg(target_os = "windows")]
fn main() -> eframe::Result {
    let base_dir = config::base_dir();
    if let Some(parent_pid) = watchdog::parent_pid() {
        watchdog::run(parent_pid);
        return Ok(());
    }
    setup_logging(&base_dir);
    deep_link::register_and_queue_from_args(&base_dir);
    if let Err(error) = runtime_assets::ensure_present(&base_dir) {
        tracing::warn!("failed to prepare Hebnix runtime assets: {error}");
    }
    if let Err(error) = multiplayer_assets::ensure_present(&base_dir) {
        tracing::warn!("failed to prepare Hebnix multiplayer assets: {error}");
    }
    setup_panic_hook(&base_dir);
    tracing::info!("Hebnix {} starting", app::APP_VERSION);
    tracing::info!(
        safe_rendering = window_state::safe_rendering(),
        "main window rendering mode"
    );

    let mut cfg = config::Config::load(&base_dir);
    tracing::info!(
        pid = std::process::id(),
        base_dir = %base_dir.display(),
        "startup configuration loaded"
    );
    i18n::init(&cfg.settings.language, Some(base_dir.join("locales")));
    tracing::info!("startup localization initialized");

    let skip_elevate = std::env::args().any(|a| a == spoofer::SKIP_ELEVATE_ARG);
    if skip_elevate && !spoofer::is_admin() {
        cfg.settings.run_as_admin = false;
        let _ = cfg.save(&base_dir);
        let _ = spoofer::disable_saved_http_features(&base_dir);
    }
    if cfg.settings.run_as_admin && !skip_elevate && !spoofer::is_admin() {
        tracing::info!("requesting elevated relaunch");
        if spoofer::spawn_elevated_relaunch() {
            return Ok(());
        }
        tracing::warn!("couldnt spawn the elevated relaunch helper");
    }

    let Some(_mutex) = winutil::acquire_single_instance() else {
        winutil::focus_existing_instance();
        return Ok(());
    };
    let shortcut_base = base_dir.clone();
    std::thread::spawn(move || maintenance::first_launch_shortcut(&shortcut_base));
    tracing::info!("startup single-instance guard acquired");

    spoofer::restore_if_crashed(&base_dir);
    tracing::info!("checking legacy WebSocket cleanup");
    if let Err(error) = spoofer::cleanup_websocket_once(&base_dir) {
        tracing::warn!("WebSocket proxy cleanup: {error}");
    }
    tracing::info!("legacy WebSocket cleanup check complete");
    let _ = watchdog::spawn();

    tracing::info!("checking WebView2 runtime");
    webview::runtime::ensure_present();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Hebnix")
        .with_inner_size([
            (cfg.window.width as f32).max(app::MIN_WIDTH),
            (cfg.window.height as f32).max(app::MIN_HEIGHT),
        ])
        .with_min_inner_size([app::MIN_WIDTH, app::MIN_HEIGHT])
        .with_transparent(!window_state::safe_rendering())
        .with_visible(!cfg.settings.start_in_tray || window_state::safe_rendering()
            || replay_history_patch::resume_operation().is_some()
            || replay_history_patch::admin_restart_canceled());
    if let Some(icon) = load_window_icon(&base_dir) {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        wgpu_options: dcomp_wgpu_options(),
        ..Default::default()
    };

    tracing::info!("starting main window renderer");
    let result = eframe::run_native(
        "Hebnix",
        options,
        Box::new(|cc| Ok(Box::new(HebnixApp::new(cc)))),
    );
    if let Err(error) = &result {
        tracing::error!(%error, "main window renderer failed");
    }
    watchdog::finish_live_handoff();
    result
}
