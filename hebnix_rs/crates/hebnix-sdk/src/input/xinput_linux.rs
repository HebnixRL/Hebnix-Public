//! xbox/generic controller state, backed by `gilrs` (evdev under the hood).
//!
//! Keeps the same XINPUT_* button masks + XInputState shape as the windows
//! version so `bindings.rs` doesn't need to change: gilrs buttons are mapped
//! onto the xinput bitmask on the fly.

use gilrs::{Axis, Button};

use super::gilrs_hub;

// Button masks (same values as the windows XINPUT_GAMEPAD_* constants)

pub const XINPUT_DPAD_UP: u16 = 0x0001;
pub const XINPUT_DPAD_DOWN: u16 = 0x0002;
pub const XINPUT_DPAD_LEFT: u16 = 0x0004;
pub const XINPUT_DPAD_RIGHT: u16 = 0x0008;
pub const XINPUT_START: u16 = 0x0010;
/// "Back" / "View"
pub const XINPUT_SELECT: u16 = 0x0020;
/// Left Stick click
pub const XINPUT_LS: u16 = 0x0040;
/// Right Stick click
pub const XINPUT_RS: u16 = 0x0080;
/// Left Bumper
pub const XINPUT_LB: u16 = 0x0100;
/// Right Bumper
pub const XINPUT_RB: u16 = 0x0200;
pub const XINPUT_A: u16 = 0x1000;
pub const XINPUT_B: u16 = 0x2000;
pub const XINPUT_X: u16 = 0x4000;
pub const XINPUT_Y: u16 = 0x8000;

pub const XINPUT_BUTTON_DISPLAY: [(u16, &str); 14] = [
    (XINPUT_DPAD_UP, "D-Pad Up"),
    (XINPUT_DPAD_DOWN, "D-Pad Down"),
    (XINPUT_DPAD_LEFT, "D-Pad Left"),
    (XINPUT_DPAD_RIGHT, "D-Pad Right"),
    (XINPUT_START, "Start"),
    (XINPUT_SELECT, "Select"),
    (XINPUT_LS, "L-Stick"),
    (XINPUT_RS, "R-Stick"),
    (XINPUT_LB, "LB"),
    (XINPUT_RB, "RB"),
    (XINPUT_A, "A"),
    (XINPUT_B, "B"),
    (XINPUT_X, "X"),
    (XINPUT_Y, "Y"),
];

/// typed wrapper around the raw xinput-shaped state
#[derive(Debug, Clone, Copy, Default)]
pub struct XInputState {
    pub packet_number: u32,
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

impl XInputState {
    pub fn is_pressed(&self, button_mask: u16) -> bool {
        (self.buttons & button_mask) == button_mask
    }
}

const GILRS_BUTTONS: [(Button, u16); 14] = [
    (Button::DPadUp, XINPUT_DPAD_UP),
    (Button::DPadDown, XINPUT_DPAD_DOWN),
    (Button::DPadLeft, XINPUT_DPAD_LEFT),
    (Button::DPadRight, XINPUT_DPAD_RIGHT),
    (Button::Start, XINPUT_START),
    (Button::Select, XINPUT_SELECT),
    (Button::LeftThumb, XINPUT_LS),
    (Button::RightThumb, XINPUT_RS),
    (Button::LeftTrigger, XINPUT_LB),
    (Button::RightTrigger, XINPUT_RB),
    (Button::South, XINPUT_A),
    (Button::East, XINPUT_B),
    (Button::West, XINPUT_X),
    (Button::North, XINPUT_Y),
];

fn axis_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn trigger_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * u8::MAX as f32) as u8
}

/// Trigger travel in 0.0..=1.0. Pads gilrs maps from the kernel driver (the
/// 8BitDo Ultimate 2, for one) report a trigger as a `LeftZ`/`RightZ` axis
/// from -1.0 (released) to 1.0 (fully pulled); clamping that to 0..1 dropped
/// the first half of the pull. Pads from the SDL database report it as an
/// analog `LeftTrigger2`/`RightTrigger2` button instead. An axis that hasn't
/// moved yet has no data and counts as released.
fn trigger_travel(gamepad: &gilrs::Gamepad, axis: Axis, button: Button) -> f32 {
    let from_axis = gamepad
        .axis_data(axis)
        .map_or(0.0, |data| (data.value() + 1.0) / 2.0);
    let from_button = gamepad.button_data(button).map_or(0.0, |data| data.value());
    from_axis.max(from_button)
}

/// xinput-shaped state for controller user_index (0-3, by connection order),
/// None if not connected.
pub fn get_xinput_state(user_index: u32) -> Option<XInputState> {
    let mut guard = gilrs_hub::handle().lock().unwrap();
    let gilrs = guard.as_mut()?;
    // drain pending events so is_pressed()/value() reflect the latest state
    while gilrs.next_event().is_some() {}

    let (_id, gamepad) = gilrs
        .gamepads()
        .nth(user_index as usize)?;

    let mut buttons = 0u16;
    for (btn, mask) in GILRS_BUTTONS {
        if gamepad.is_pressed(btn) {
            buttons |= mask;
        }
    }

    Some(XInputState {
        packet_number: 0,
        buttons,
        left_trigger: trigger_u8(trigger_travel(&gamepad, Axis::LeftZ, Button::LeftTrigger2)),
        right_trigger: trigger_u8(trigger_travel(&gamepad, Axis::RightZ, Button::RightTrigger2)),
        thumb_lx: axis_i16(gamepad.value(Axis::LeftStickX)),
        thumb_ly: axis_i16(gamepad.value(Axis::LeftStickY)),
        thumb_rx: axis_i16(gamepad.value(Axis::RightStickX)),
        thumb_ry: axis_i16(gamepad.value(Axis::RightStickY)),
    })
}
