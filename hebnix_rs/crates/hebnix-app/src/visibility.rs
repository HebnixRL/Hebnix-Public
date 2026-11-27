//! Admit one visibility request, with bounded native/render/shell settlement.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use eframe::egui::{Context, ViewportId};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetLayeredWindowAttributes, GetWindowLongW, IsIconic, IsWindowVisible,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    id: u64,
    started_at: Instant,
    hidden: bool,
    started_frame: u64,
    drawn_frame: Option<u64>,
    applied: bool,
    taskbar: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Queued,
    Changing(Pending),
}

struct Gate {
    phase: Phase,
    generation: u64,
}

impl Gate {
    const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            generation: 0,
        }
    }

    fn queue(&mut self) -> bool {
        if self.phase != Phase::Idle {
            return false;
        }
        self.phase = Phase::Queued;
        true
    }

    fn begin(&mut self, hidden: bool, frame: u64) -> bool {
        if matches!(self.phase, Phase::Changing(_)) {
            return false;
        }
        self.generation += 1;
        self.phase = Phase::Changing(Pending {
            id: self.generation,
            started_at: Instant::now(),
            hidden,
            started_frame: frame,
            drawn_frame: None,
            applied: false,
            taskbar: None,
        });
        true
    }

    fn cancel_queued(&mut self) {
        if self.phase == Phase::Queued {
            self.phase = Phase::Idle;
        }
    }

    fn settling(&self, frame: u64) -> Option<Pending> {
        let Phase::Changing(pending) = self.phase else {
            return None;
        };
        (pending.applied && frame > pending.started_frame).then_some(pending)
    }

    fn candidate(&self, frame: u64) -> Option<Pending> {
        self.settling(frame).filter(|pending| {
            pending.hidden || pending.drawn_frame.is_some_and(|drawn| frame > drawn)
        })
    }

    fn complete(&mut self, pending: Pending, taskbar_done: u64, native: bool) -> bool {
        if self.phase != Phase::Changing(pending)
            || !native
            || !pending
                .taskbar
                .is_some_and(|request| taskbar_done >= request)
        {
            return false;
        }
        self.phase = Phase::Idle;
        true
    }

    fn expire(&mut self, now: Instant) -> Option<Pending> {
        let Phase::Changing(pending) = self.phase else {
            return None;
        };
        if now.saturating_duration_since(pending.started_at) < Duration::from_secs(3) {
            return None;
        }
        // A missing render, unavailable layered-window attributes, or a stalled
        // Explorer must not permanently disable both the tray and the hotkey.
        self.phase = Phase::Idle;
        Some(pending)
    }
}

static GATE: Mutex<Gate> = Mutex::new(Gate::new());

pub fn queue_toggle() -> bool {
    let Ok(mut gate) = GATE.try_lock() else {
        tracing::warn!("show/hide request could not acquire the visibility gate");
        return false;
    };
    // Also recover here: a previous transition may have stopped before its
    // completion worker started (for example while dispatching plugin events).
    if let Some(pending) = gate.expire(Instant::now()) {
        tracing::warn!(hidden = pending.hidden, "show/hide recovered a stale visibility transition");
    }
    let accepted = gate.queue();
    tracing::info!(accepted, phase = ?gate.phase, "show/hide delivery gate");
    accepted
}

pub fn toggle_waiting_for_ui() -> bool {
    GATE.lock().unwrap().phase == Phase::Queued
}

pub fn cancel_queued_toggle() {
    GATE.lock().unwrap().cancel_queued();
}

pub fn begin_change(ctx: &Context, hidden: bool) -> bool {
    GATE.lock()
        .unwrap()
        .begin(hidden, ctx.cumulative_frame_nr_for(ViewportId::ROOT))
}

pub fn applied(ctx: &Context) {
    let id = {
        let mut gate = GATE.lock().unwrap();
        let Phase::Changing(pending) = &mut gate.phase else {
            return;
        };
        pending.applied = true;
        pending.id
    };
    crate::window_state::wake_ui(ctx);
    let ctx = ctx.clone();
    // One small checker per accepted transition; a burst cannot spawn more.
    // Normally the native/frame/shell checks settle the transition. The deadline
    // also runs here so recovery works when Windows stops delivering UI frames.
    std::thread::Builder::new()
        .name("visibility-completion".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(16));
                let (current, expired) = {
                    let mut gate = GATE.lock().unwrap();
                    let expired = gate.expire(Instant::now());
                    (gate.phase, expired)
                };
                if let Some(pending) = expired {
                    tracing::warn!(
                        hidden = pending.hidden,
                        ui_frame_drawn = pending.drawn_frame.is_some(),
                        taskbar_request = ?pending.taskbar,
                        "window visibility did not settle within 3 seconds; releasing controls for recovery"
                    );
                    crate::window_state::wake_ui(&ctx);
                }
                if !matches!(current, Phase::Changing(pending) if pending.id == id) {
                    break;
                }
                crate::window_state::wake_ui(&ctx);
            }
        })
        .expect("could not start visibility completion checker");
}

pub fn note_ui_frame(ctx: &Context) {
    let mut gate = GATE.lock().unwrap();
    if let Phase::Changing(pending) = &mut gate.phase {
        if !pending.hidden && pending.drawn_frame.is_none() {
            pending.drawn_frame = Some(ctx.cumulative_frame_nr_for(ViewportId::ROOT));
        }
    }
}

fn native_matches(hidden: bool) -> bool {
    let Some(hwnd) = crate::window_state::main_window() else {
        return false;
    };
    unsafe {
        if crate::window_state::is_cloaked(hwnd) { return hidden; }
        if hidden && !IsWindowVisible(hwnd).as_bool() { return true; }
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let away = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0;
        if style & away != if hidden { away } else { 0 } {
            return false;
        }
        if (style & WS_EX_TRANSPARENT.0 != 0) != hidden {
            return false;
        }
        if !hidden && (!IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool()) {
            return false;
        }
        let alpha = if style & WS_EX_LAYERED.0 == 0 {
            255
        } else {
            let mut alpha = 255;
            let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();
            if GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)).is_err() {
                return false;
            }
            if flags.contains(LWA_ALPHA) {
                alpha
            } else {
                255
            }
        };
        alpha == if hidden { 0 } else { 255 }
    }
}

/// Called before consuming controls in each root logic frame. A newer frame
/// guarantees the previous viewport commands and render submission have run.
pub fn poll(ctx: &Context) {
    let frame = ctx.cumulative_frame_nr_for(ViewportId::ROOT);
    let Some(pending) = GATE.lock().unwrap().settling(frame) else {
        return;
    };
    if let Some(hwnd) = crate::window_state::main_window() {
        // Restore native visibility before waiting for a drawn frame. An
        // alpha-zero window can be occluded and never get that first frame.
        crate::window_state::apply_window_visibility(hwnd, pending.hidden);
    }
    let Some(mut pending) = GATE.lock().unwrap().candidate(frame) else {
        return;
    };
    if !native_matches(pending.hidden) {
        return;
    }
    if pending.taskbar.is_none() {
        let Some(hwnd) = crate::window_state::main_window() else {
            return;
        };
        let request = crate::window_state::set_taskbar_button(hwnd, !pending.hidden);
        pending.taskbar = Some(request);
        let mut gate = GATE.lock().unwrap();
        if let Phase::Changing(current) = &mut gate.phase {
            if current.id == pending.id {
                current.taskbar = Some(request);
            }
        }
    }
    let taskbar_done = crate::window_state::completed_taskbar_request();
    if !pending
        .taskbar
        .is_some_and(|request| taskbar_done >= request)
    {
        return;
    }
    // Do not wait for DwmFlush on the UI thread: controls must remain responsive
    // even when the compositor is not advancing.
    if GATE
        .lock()
        .unwrap()
        .complete(pending, taskbar_done, native_matches(pending.hidden))
    {
        tracing::debug!(
            hidden = pending.hidden,
            "window visibility transition complete"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_queued_requests_release_the_gate_but_cannot_abort_a_transition() {
        let mut gate = Gate::new();
        assert!(gate.queue());
        gate.cancel_queued();
        assert!(gate.queue());
        assert!(gate.begin(true, 1));
        gate.cancel_queued();
        assert!(!gate.queue());
    }

    #[test]
    fn spam_is_discarded_until_every_completion_barrier_is_met() {
        for hidden in [true, false] {
            let mut gate = Gate::new();
            assert!(gate.queue());
            for _ in 0..1000 {
                assert!(!gate.queue());
            }
            assert!(gate.begin(hidden, 10));
            for _ in 0..1000 {
                assert!(!gate.queue());
                assert!(!gate.begin(!hidden, 10));
            }
            assert!(gate.candidate(11).is_none());
            let Phase::Changing(pending) = &mut gate.phase else {
                unreachable!()
            };
            pending.applied = true;
            pending.drawn_frame = Some(11);
            pending.taskbar = Some(7);
            assert!(gate.candidate(10).is_none());
            if !hidden {
                assert!(gate.candidate(11).is_none());
            }
            let pending = gate.candidate(12).unwrap();
            assert!(!gate.complete(pending, 6, true));
            assert!(!gate.complete(pending, 7, false));
            assert!(!gate.queue());
            assert!(gate.complete(pending, 7, true));
            assert!(gate.queue());
            assert!(
                !gate.queue(),
                "ignored spam must not be replayed after completion"
            );
        }
    }

    #[test]
    fn stalled_startup_or_toggle_releases_controls_without_replaying_requests() {
        for hidden in [true, false] {
            for drawn in [false, true] {
                let mut gate = Gate::new();
                assert!(gate.begin(hidden, 0));
                let Phase::Changing(mut pending) = gate.phase else {
                    unreachable!()
                };
                pending.applied = true;
                pending.drawn_frame = drawn.then_some(1);
                // Cover both a missing first frame and a stalled shell update.
                pending.taskbar = drawn.then_some(1);
                gate.phase = Phase::Changing(pending);
                assert_eq!(
                    gate.settling(1),
                    Some(pending),
                    "native recovery must run without a drawn UI frame"
                );
                if !hidden && !drawn {
                    assert!(gate.candidate(1).is_none());
                }
                assert!(
                    gate.expire(pending.started_at + Duration::from_secs(2))
                        .is_none()
                );
                assert!(!gate.queue());
                assert_eq!(
                    gate.expire(pending.started_at + Duration::from_secs(3)),
                    Some(pending)
                );
                assert_eq!(gate.phase, Phase::Idle);
                assert!(!gate.complete(pending, 1, true));
                assert!(gate.queue());
                assert!(!gate.queue());
                assert!(gate.begin(!hidden, 2));
            }
        }
    }
}
