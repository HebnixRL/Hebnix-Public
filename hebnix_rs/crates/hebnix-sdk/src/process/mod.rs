//! RL process detection + window helpers.

#[cfg(target_os = "windows")]
pub mod detector;
#[cfg(target_os = "linux")]
#[path = "detector_linux.rs"]
pub mod detector;
#[cfg(target_os = "linux")]
pub mod i3ipc;
#[cfg(target_os = "windows")]
pub mod window;
#[cfg(target_os = "linux")]
#[path = "window_linux.rs"]
pub mod window;
#[cfg(target_os = "linux")]
pub mod wine_prefix;
#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "windows")]
pub use detector::{
    RlPlatform, RlProcessInfo, detect_platform, find_process, find_rocket_league,
    get_save_data_path, is_rocket_league_running,
};
#[cfg(target_os = "windows")]
pub use window::{
    get_rocket_league_window_rect, is_cursor_inside_rl_window, is_rocket_league_focused,
    rocket_league_hwnd, rocket_league_monitor_size,
};

#[cfg(target_os = "linux")]
pub use detector::{
    RlPlatform, RlProcessInfo, detect_platform, find_process, find_rocket_league,
    get_save_data_path, is_rocket_league_running, save_data_dir_in,
};
#[cfg(target_os = "linux")]
pub use wine_prefix::{
    candidate_documents_dirs, documents_dir_for_exe, prefix_in_use, rocket_league_documents_dir,
    rocket_league_prefixes, running_game_prefixes,
    set_documents_override, steam_library_steamapps_dirs, wine_prefixes,
};
#[cfg(target_os = "linux")]
pub use window::{
    can_track_own_window, exempt_own_window_decorations, focus_own_window_over_game, get_rocket_league_window_rect,
    cursor_in_rl_window, cursor_position, hyprland_layer_visible, is_cursor_inside_rl_window,
    is_rocket_league_focused,
    place_own_window, raise_other_instance, reassert_own_window_decorations, rocket_league_hwnd,
    rocket_league_monitor_rect, rocket_league_monitor_size, set_own_always_on_top,
    unminimize_own_window,
};
