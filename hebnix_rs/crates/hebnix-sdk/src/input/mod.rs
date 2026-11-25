//! keyboard, xinput (xbox) + generic dinput (ps4/ps5/8bitdo/etc, via gilrs) input detection.

pub mod bindings;
#[cfg(target_os = "windows")]
pub mod dinput;
#[cfg(target_os = "linux")]
#[path = "dinput_linux.rs"]
pub mod dinput;
#[cfg(target_os = "windows")]
pub mod game_actions;
#[cfg(target_os = "linux")]
#[path = "game_actions_linux.rs"]
pub mod game_actions;
#[cfg(target_os = "linux")]
mod gilrs_hub;
#[cfg(target_os = "windows")]
pub mod keyboard;
#[cfg(target_os = "linux")]
#[path = "keyboard_linux.rs"]
pub mod keyboard;
#[cfg(target_os = "windows")]
pub mod xinput;
#[cfg(target_os = "linux")]
#[path = "xinput_linux.rs"]
pub mod xinput;

pub use bindings::{
    ControllerType, HotkeyBind, bind_to_string, detect_any_hotkey, dinput_name_to_xinput_mask,
    get_button_display, init_controllers, is_bind_pressed, is_hotkey_pressed,
    resolve_bind_string, xinput_mask_to_dinput_name,
};
pub use dinput::{
    DINPUT_BUTTONS, Layout as DInputLayout, PsGen, current_dinput_layout, current_ps_gen,
    get_dinput_inputs, get_dinput_raw_inputs, is_dinput_connected, is_dinput_raw_pressed,
};
pub use game_actions::{
    action_binds, chat_channel_bind, clear_action_bind_cache, is_action_pressed, ui_scale,
};
pub use keyboard::{detect_hotkey, is_key_pressed, tap_key, type_text, type_text_while};
#[cfg(target_os = "windows")]
pub use keyboard::name_to_vk;
pub use xinput::{XINPUT_BUTTON_DISPLAY, XInputState, get_xinput_state};
