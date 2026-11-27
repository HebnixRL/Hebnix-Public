//! xinput (xbox controller) state.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use windows::Win32::Foundation::ERROR_DEVICE_NOT_CONNECTED;
use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};

/// How long an empty controller slot is left alone before it's asked again.
///
/// XInputGetState on a slot with no controller is not a cheap miss: XInput
/// re-enumerates devices (cfgmgr32/devobj) on every call. Hebnix asks all four
/// slots several times per frame (plugin binds, pad navigation, gamepad icons,
/// the Lua API), so with one controller or none that was ~5% of a CPU core.
/// Microsoft's guidance is to poll only connected slots every frame and
/// re-check empty ones occasionally. A controller plugged into an empty slot
/// is picked up within this interval.
const EMPTY_SLOT_RECHECK_MS: u64 = 2000;

/// Per slot: milliseconds since `clock_ms`'s start before the slot may be
/// probed again (0 = probe now).
static NEXT_PROBE_MS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

fn clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn probe_due(next_probe_ms: u64, now_ms: u64) -> bool {
    now_ms >= next_probe_ms
}

/// When to probe a slot next, given what XInputGetState returned now.
fn next_probe_ms(result: u32, now_ms: u64) -> u64 {
    if result == ERROR_DEVICE_NOT_CONNECTED.0 {
        now_ms + EMPTY_SLOT_RECHECK_MS
    } else {
        0
    }
}

// Button masks

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

/// typed wrapper around the raw xinput state
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

/// xinput state for controller user_index (0-3), None if not connected.
/// A slot found empty is not asked again for EMPTY_SLOT_RECHECK_MS.
pub fn get_xinput_state(user_index: u32) -> Option<XInputState> {
    let slot = NEXT_PROBE_MS.get(user_index as usize);
    let now = clock_ms();
    if slot.is_some_and(|slot| !probe_due(slot.load(Ordering::Relaxed), now)) {
        return None;
    }
    let mut state = XINPUT_STATE::default();
    let result = unsafe { XInputGetState(user_index, &mut state) };
    if let Some(slot) = slot {
        slot.store(next_probe_ms(result, now), Ordering::Relaxed);
    }
    if result != 0 {
        return None;
    }
    let g = state.Gamepad;
    Some(XInputState {
        packet_number: state.dwPacketNumber,
        buttons: g.wButtons.0,
        left_trigger: g.bLeftTrigger,
        right_trigger: g.bRightTrigger,
        thumb_lx: g.sThumbLX,
        thumb_ly: g.sThumbLY,
        thumb_rx: g.sThumbRX,
        thumb_ry: g.sThumbRY,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_slot_is_left_alone_until_the_recheck() {
        let now = 10_000;
        let next = next_probe_ms(ERROR_DEVICE_NOT_CONNECTED.0, now);
        assert!(!probe_due(next, now));
        assert!(!probe_due(next, now + EMPTY_SLOT_RECHECK_MS - 1));
        assert!(probe_due(next, now + EMPTY_SLOT_RECHECK_MS));
    }

    #[test]
    fn a_connected_slot_or_another_error_is_asked_every_time() {
        assert_eq!(next_probe_ms(0, 10_000), 0);
        assert!(probe_due(0, 10_000));
        // only "not connected" backs off; anything else is retried at once
        assert_eq!(next_probe_ms(5, 10_000), 0);
    }

    #[test]
    fn slots_out_of_range_are_not_tracked() {
        assert!(NEXT_PROBE_MS.get(4).is_none());
    }
}
