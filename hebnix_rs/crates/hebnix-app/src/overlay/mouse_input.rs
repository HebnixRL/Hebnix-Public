//! Never put UI/COM work on the synchronous Windows mouse-hook path.
//! The hook owns a dedicated message pump; browser input runs on the window's
//! UI thread through PostMessage. Movement is coalesced so high polling rates
//! cannot build an ever-growing UI queue while Hebnix is busy.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostMessageW, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_APP,
};

const UPDATE_HOOK: u32 = WM_APP + 40;
const INPUT_BASE: u32 = WM_APP + 64;
const MOUSE_FIRST: u32 = 0x0200;
const MOUSE_LAST: u32 = 0x020e;

static HOOK_THREAD: OnceLock<Option<u32>> = OnceLock::new();
static MOVE: PendingMove = PendingMove::new();

struct PendingMove {
    point: AtomicU64,
    pending: AtomicU64,
    next: AtomicU64,
}

impl PendingMove {
    const fn new() -> Self {
        Self {
            point: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            next: AtomicU64::new(1),
        }
    }

    /// Only the hook thread publishes. A ticket identifies each movement batch.
    fn publish(&self, point: POINT) -> Option<u64> {
        self.point.store(pack_point(point), Ordering::Release);
        if self.pending.load(Ordering::Acquire) != 0 {
            return None;
        }
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        self.pending.store(ticket, Ordering::Release);
        Some(ticket)
    }

    fn take(&self, ticket: u64) -> Option<POINT> {
        // Clear before loading: a concurrent publish either appears in this
        // sample or posts another notification, so the final move is not lost.
        self.pending
            .compare_exchange(ticket, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| unpack_point(self.point.load(Ordering::Acquire)))
    }

    fn invalidate(&self) {
        self.pending.store(0, Ordering::Release);
    }
}

fn pack_point(point: POINT) -> u64 {
    point.x as u32 as u64 | ((point.y as u32 as u64) << 32)
}

fn unpack_point(packed: u64) -> POINT {
    POINT {
        x: packed as u32 as i32,
        y: (packed >> 32) as u32 as i32,
    }
}

pub(super) fn update_hook() {
    if !super::webview_clickable() {
        MOVE.invalidate();
    }
    let thread = HOOK_THREAD.get_or_init(|| {
        let (ready, receive) = std::sync::mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("overlay-mouse-hook".into())
            .spawn(move || unsafe {
                // PostThreadMessage requires the receiver's queue to exist first.
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                if ready.send(GetCurrentThreadId()).is_err() {
                    return;
                }
                let mut hook = None;
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    if message.message != UPDATE_HOOK {
                        continue;
                    }
                    if super::webview_clickable() {
                        if hook.is_none() {
                            match SetWindowsHookExW(
                                WH_MOUSE_LL,
                                Some(super::mouse_hook_proc),
                                None,
                                0,
                            ) {
                                Ok(value) => hook = Some(value),
                                Err(error) => {
                                    tracing::warn!(%error, "overlay mouse hook unavailable")
                                }
                            }
                        }
                    } else if let Some(value) = hook.take() {
                        let _ = UnhookWindowsHookEx(value);
                    }
                }
                if let Some(value) = hook {
                    let _ = UnhookWindowsHookEx(value);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "overlay mouse hook thread unavailable");
            return None;
        }
        receive.recv().ok()
    });
    if let Some(thread) = thread {
        unsafe {
            let _ = PostThreadMessageW(*thread, UPDATE_HOOK, WPARAM(0), LPARAM(0));
        }
    }
}

pub(super) fn post_input(hwnd: HWND, msg: u32, point: POINT, mouse_data: u32) -> bool {
    // These are the events supported by WebView2's input forwarding. Never
    // suppress an unsupported button event in the application underneath.
    if !matches!(msg, 0x0200..=0x020a | 0x020e) {
        return false;
    }
    let packed = if msg == MOUSE_FIRST {
        let Some(ticket) = MOVE.publish(point) else {
            return true;
        };
        ticket
    } else {
        // A later move must be queued after this button/wheel, not folded into
        // an earlier notification that would leave the browser at an old point.
        MOVE.invalidate();
        pack_point(point)
    };
    let posted = unsafe {
        PostMessageW(
            Some(hwnd),
            INPUT_BASE + msg - MOUSE_FIRST,
            WPARAM(mouse_data as usize),
            LPARAM(packed as isize),
        )
        .is_ok()
    };
    if !posted && msg == MOUSE_FIRST {
        MOVE.invalidate();
    }
    posted
}

pub(super) fn dispatch(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    if !(INPUT_BASE..=INPUT_BASE + MOUSE_LAST - MOUSE_FIRST).contains(&msg) {
        return false;
    }
    let mouse_msg = MOUSE_FIRST + msg - INPUT_BASE;
    let point = if mouse_msg == MOUSE_FIRST {
        let Some(point) = MOVE.take(lparam.0 as u64) else {
            return true;
        };
        point
    } else {
        unpack_point(lparam.0 as u64)
    };
    if super::webview_clickable() {
        crate::webview::host::forward_mouse_screen_message(hwnd, mouse_msg, point, wparam.0 as u32);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_burst_has_one_notification_and_delivers_final_position() {
        let pending = PendingMove::new();
        let ticket = pending.publish(POINT { x: -2560, y: -1440 }).unwrap();
        for x in 0..8000 {
            assert!(pending.publish(POINT { x, y: -x }).is_none());
        }
        let point = pending.take(ticket).unwrap();
        assert_eq!((point.x, point.y), (7999, -7999));
        let ticket = pending
            .publish(POINT {
                x: i32::MIN,
                y: i32::MAX,
            })
            .unwrap();
        let point = pending.take(ticket).unwrap();
        assert_eq!((point.x, point.y), (i32::MIN, i32::MAX));
    }

    #[test]
    fn stalled_ui_does_not_block_input_or_accumulate_mouse_moves() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, PM_REMOVE,
        };
        use windows::core::{PCWSTR, w};

        // A message-only window has no visible UI and we deliberately do not
        // pump its thread while the producer submits a high-polling-rate burst.
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                PCWSTR::null(),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap()
        };
        struct WindowGuard(HWND);
        impl Drop for WindowGuard {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let _window = WindowGuard(hwnd);
        let raw = hwnd.0 as isize;
        let (done, result) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            let target = HWND(raw as *mut _);
            for x in 0..8000 {
                assert!(post_input(target, MOUSE_FIRST, POINT { x, y: -x }, 0));
            }
            // Button positions and wheel deltas must remain individual events.
            assert!(post_input(target, 0x0201, POINT { x: -200, y: 300 }, 0));
            assert!(post_input(
                target,
                0x020a,
                POINT { x: -200, y: 300 },
                120 << 16
            ));
            assert!(post_input(
                target,
                MOUSE_FIRST,
                POINT { x: -8000, y: 8000 },
                0
            ));
            done.send(()).unwrap();
        });
        result
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        producer.join().unwrap();
        let mut messages = Vec::new();
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(
                &mut message,
                Some(hwnd),
                INPUT_BASE,
                INPUT_BASE + MOUSE_LAST - MOUSE_FIRST,
                PM_REMOVE,
            )
            .as_bool()
            {
                messages.push(message);
            }
        }
        assert_eq!(messages.len(), 4, "move, button, wheel, subsequent move");
        // Buttons invalidate preceding move samples. A subsequent move must
        // get its own notification and stale tickets must not consume it.
        assert!(MOVE.take(messages[0].lParam.0 as u64).is_none());
        let point = MOVE.take(messages[3].lParam.0 as u64).unwrap();
        assert_eq!((point.x, point.y), (-8000, 8000));
        assert_eq!(messages[1].message, INPUT_BASE + 1);
        let click = unpack_point(messages[1].lParam.0 as u64);
        assert_eq!((click.x, click.y), (-200, 300));
        assert_eq!(messages[2].wParam.0, 120 << 16);
    }
}
