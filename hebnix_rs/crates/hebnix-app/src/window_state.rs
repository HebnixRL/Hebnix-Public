//! Main-window identity and taskbar updates shared by Full and Lite.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_CLOAKED};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetLayeredWindowAttributes, GetWindowLongW, HWND_TOPMOST, IsIconic, IsWindow, IsWindowVisible,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, PostMessageW, SW_RESTORE, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetLayeredWindowAttributes, SetWindowLongW, SetWindowPos,
    ShowWindowAsync, WM_PAINT,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

// Store the address, rather than HWND's non-Send raw pointer. The window remains
// owned by eframe's UI thread; readers only use it with Win32 window APIs.
static MAIN_WINDOW: OnceLock<isize> = OnceLock::new();
static TASKBAR: OnceLock<Option<Sender<TaskbarUpdate>>> = OnceLock::new();
static TASKBAR_REQUESTED: AtomicU64 = AtomicU64::new(0);
static TASKBAR_COMPLETED: AtomicU64 = AtomicU64::new(0);

/// An opaque HWND swapchain bypasses DirectComposition for machines where the
/// transparent visual never appears. Keep the choice stable for every viewport.
pub fn safe_rendering() -> bool {
    static SAFE_RENDERING: OnceLock<bool> = OnceLock::new();
    *SAFE_RENDERING.get_or_init(|| std::env::args_os().any(|arg| arg == "--safe-rendering"))
}

struct TaskbarUpdate {
    address: isize,
    shown: bool,
    sequence: u64,
}

pub fn init(window: &impl HasWindowHandle) {
    if let Ok(handle) = window.window_handle()
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        let _ = MAIN_WINDOW.set(handle.hwnd.get());
    }
    TASKBAR.get_or_init(start_taskbar_worker);
}

pub fn main_window() -> Option<HWND> {
    let hwnd = HWND(*MAIN_WINDOW.get()? as *mut _);
    unsafe { IsWindow(Some(hwnd)).as_bool().then_some(hwnd) }
}

/// Winit can replace extended styles when applying mouse passthrough. Reapply
/// the complete hidden state afterwards, and remove legacy window layering on
/// Show so a stale alpha-zero surface cannot cover the rendered client area.
pub fn apply_window_visibility(hwnd: HWND, hidden: bool) {
    unsafe {
        let away=(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0) as i32;
        let style=GetWindowLongW(hwnd,GWL_EXSTYLE);
        if hidden {
            SetWindowLongW(hwnd,GWL_EXSTYLE,(style | away) & !(WS_EX_LAYERED.0 as i32));
        } else {
            // Wry/wgpu render through DirectComposition. Leaving WS_EX_LAYERED
            // installed after hiding can keep that surface invisible, even at
            // alpha 255. Restore normal composition and input together.
            SetWindowLongW(hwnd,GWL_EXSTYLE,style & !(away | WS_EX_LAYERED.0 as i32));
        }
        // Alpha-zero layered windows do not reliably hide a DirectComposition
        // host's non-client frame. Cloaking removes the complete window from
        // composition without destroying/minimizing its WebViews or swapchain.
        let cloak = windows::core::BOOL::from(hidden);
        if let Err(error) = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const _ as _, std::mem::size_of_val(&cloak) as u32) {
            tracing::warn!(%error, hidden, "could not cloak the main window");
            if hidden {
                SetWindowLongW(hwnd,GWL_EXSTYLE,style | WS_EX_LAYERED.0 as i32 | away);
                let _=SetLayeredWindowAttributes(hwnd,COLORREF(0),0,LWA_ALPHA);
            }
        }
        set_taskbar_button(hwnd,!hidden);
    }
}

pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, std::mem::size_of_val(&cloaked) as u32).is_ok() && cloaked != 0 }
}

/// Snapshot before queuing a toggle. A hidden/occluded parent cannot be relied
/// upon to process the UI frame that would otherwise make it visible again.
pub fn toggle_wake_target() -> Option<HWND> {
    let hwnd = main_window()?;
    unsafe {
        let mut alpha = 255;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();
        let transparent = GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)).is_ok()
            && flags.contains(LWA_ALPHA) && alpha == 0;
        (is_cloaked(hwnd) || transparent || IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool()).then_some(hwnd)
    }
}

pub fn wake_toggle_target(hwnd: HWND) {
    apply_window_visibility(hwnd, false);
    unsafe {
        // Neither call waits for the window's thread. Do not perform this for
        // Quit or generic background messages: only an explicit Show toggle.
        let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
    }
}

/// Wake controls even when Windows suppresses paints for an alpha-zero window.
/// Its WS_VISIBLE flag remains set, so eframe's invisible-window fallback does
/// not apply. Post to the owning UI thread without revealing or activating it.
pub fn wake_ui(ctx: &eframe::egui::Context) {
    ctx.request_repaint_of(eframe::egui::ViewportId::ROOT);
    if let Some(hwnd) = main_window() {
        unsafe {
            if let Err(error) = post_control_wake(hwnd) {
                tracing::warn!(%error, "could not wake main window for a control request");
            }
        }
    }
}

unsafe fn post_control_wake(hwnd: HWND) -> windows::core::Result<()> {
    use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_INTERNALPAINT, RDW_INVALIDATE};
    unsafe {
        // Mark a real update region as well as posting the fallback paint.
        // Merely posting WM_PAINT can be consumed without a lasting redraw
        // when Winit is buffering events while the game owns the foreground.
        let _ = RedrawWindow(Some(hwnd), None, None, RDW_INTERNALPAINT | RDW_INVALIDATE);
        PostMessageW(Some(hwnd), WM_PAINT, WPARAM(0), LPARAM(0))
    }
}

pub fn set_taskbar_button(hwnd: HWND, shown: bool) -> u64 {
    let sequence = TASKBAR_REQUESTED.fetch_add(1, Ordering::AcqRel) + 1;
    let update = TaskbarUpdate {
        address: hwnd.0 as isize,
        shown,
        sequence,
    };
    if !TASKBAR
        .get()
        .and_then(Option::as_ref)
        .is_some_and(|tx| tx.send(update).is_ok())
    {
        TASKBAR_COMPLETED.fetch_max(sequence, Ordering::Release);
    }
    sequence
}

pub fn completed_taskbar_request() -> u64 {
    TASKBAR_COMPLETED.load(Ordering::Acquire)
}

fn start_taskbar_worker() -> Option<Sender<TaskbarUpdate>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("taskbar-updates".into())
        .spawn(move || unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                drop(rx);
                TASKBAR_COMPLETED
                    .fetch_max(TASKBAR_REQUESTED.load(Ordering::Acquire), Ordering::Release);
                tracing::warn!("could not initialize taskbar worker COM apartment");
                return;
            }
            {
                // Create and use the COM interface on this thread. Explorer can
                // stall these calls; visibility and focus must not wait for it.
                let mut list: Option<ITaskbarList> = None;
                taskbar_updates(rx, |update: TaskbarUpdate| {
                    let hwnd = HWND(update.address as *mut _);
                    if !IsWindow(Some(hwnd)).as_bool() {
                        TASKBAR_COMPLETED.fetch_max(update.sequence, Ordering::Release);
                        return;
                    }
                    if list.is_none() {
                        list = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                            .ok()
                            .filter(|list: &ITaskbarList| list.HrInit().is_ok());
                    }
                    if let Some(taskbar) = &list {
                        let result = if update.shown {
                            taskbar.AddTab(hwnd)
                        } else {
                            taskbar.DeleteTab(hwnd)
                        };
                        if result.is_err() {
                            list = None;
                        }
                    }
                    TASKBAR_COMPLETED.fetch_max(update.sequence, Ordering::Release);
                });
            }
            CoUninitialize();
        })
        .ok()?;
    Some(tx)
}

fn taskbar_updates<T>(rx: Receiver<T>, mut apply: impl FnMut(T)) {
    while let Ok(update) = rx.recv() {
        // A slow shell must not replay obsolete states after rapid toggles.
        let latest = rx.try_iter().last().unwrap_or(update);
        apply(latest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn toggle_wake_restores_a_transparent_parent_before_ui_delivery() {
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP, WS_VISIBLE};
        use windows::core::w;
        unsafe {
            let window = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("STATIC"), w!("Toggle wake check"),
                WS_POPUP | WS_VISIBLE, -32000, -32000, 1, 1, None, None, None, None).unwrap();
            apply_window_visibility(window, true);
            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
            assert!(is_cloaked(window) || GetWindowLongW(window, GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0 != 0);
            wake_toggle_target(window);
            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
            assert!(!is_cloaked(window));
            let flags = GetWindowLongW(window, GWL_EXSTYLE) as u32;
            assert_eq!(flags & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0), 0);
            assert!(IsWindowVisible(window).as_bool());
            DestroyWindow(window).unwrap();
        }
    }

    #[test]
    fn showing_clears_hidden_layering_after_a_native_style_rewrite() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetLayeredWindowAttributes,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS, WS_EX_APPWINDOW, WS_EX_NOREDIRECTIONBITMAP,
            WS_VISIBLE,
        };
        use windows::core::w;

        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        unsafe {
            let preserved = WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP;
            let window = Window(
                CreateWindowExW(
                    preserved | WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("Visibility recovery check"),
                    WS_VISIBLE,
                    -32000,
                    -32000,
                    1,
                    1,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap(),
            );
            for _ in 0..2 {
                apply_window_visibility(window.0, true);
                // Model winit rebuilding styles while applying mouse passthrough.
                SetWindowLongW(
                    window.0,
                    GWL_EXSTYLE,
                    (preserved.0 | WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as i32,
                );
                apply_window_visibility(window.0, true);
                let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                let mut alpha = 255;
                let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();
                if !is_cloaked(window.0) {
                    GetLayeredWindowAttributes(window.0, None, Some(&mut alpha), Some(&mut flags)).unwrap();
                    assert_eq!(alpha, 0);
                    assert!(flags.contains(LWA_ALPHA));
                }
                apply_window_visibility(window.0, false);
                let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                assert!(!is_cloaked(window.0));
                let style = GetWindowLongW(window.0, GWL_EXSTYLE) as u32;
                let hidden =
                    WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0;
                assert_eq!(
                    style & hidden,
                    0,
                    "Show must remove alpha-zero layering and passthrough"
                );
                assert_eq!(style & preserved.0, preserved.0);
                assert!(IsWindowVisible(window.0).as_bool());
            }
        }
    }

    #[test]
    fn native_control_wake_reaches_a_hidden_window_without_showing_it() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, IsWindowVisible, MSG, PM_REMOVE, PeekMessageW,
            WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        use windows::core::w;

        unsafe {
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Hidden control wake check"),
                WINDOW_STYLE::default(),
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            assert!(!IsWindowVisible(window).as_bool());
            for _ in 0..2 {
                post_control_wake(window).unwrap();
                let mut message = MSG::default();
                assert!(
                    PeekMessageW(&mut message, Some(window), WM_PAINT, WM_PAINT, PM_REMOVE)
                        .as_bool()
                );
                assert_eq!(message.hwnd, window);
            }
            assert!(
                !IsWindowVisible(window).as_bool(),
                "Quit must not flash the main window"
            );
            DestroyWindow(window).unwrap();
        }
    }

    #[test]
    fn cached_window_lookup_does_not_wait_for_its_window_thread() {
        use raw_window_handle::{HandleError, Win32WindowHandle, WindowHandle};
        use std::num::NonZeroIsize;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        use windows::core::w;

        struct TestWindow(isize);
        impl HasWindowHandle for TestWindow {
            fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
                let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0).unwrap());
                Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
            }
        }

        let (window_tx, window_rx) = crossbeam_channel::bounded(1);
        let (resume_tx, resume_rx) = crossbeam_channel::bounded(1);
        let owner = std::thread::spawn(move || unsafe {
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Changed main window title"),
                WINDOW_STYLE::default(),
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            window_tx.send(window.0 as isize).unwrap();
            // Deliberately do not pump messages, like a stalled plugin thread.
            resume_rx.recv().unwrap();
            DestroyWindow(window).unwrap();
        });
        let address = window_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        init(&TestWindow(address));
        let (lookup_tx, lookup_rx) = crossbeam_channel::bounded(1);
        let reader = std::thread::spawn(move || {
            let _ = lookup_tx.send(main_window().map(|window| window.0 as isize));
        });
        let result = lookup_rx.recv_timeout(Duration::from_secs(1));
        resume_tx.send(()).unwrap();
        owner.join().unwrap();
        reader.join().unwrap();
        assert_eq!(result.unwrap(), Some(address));
        assert!(
            main_window().is_none(),
            "destroyed handles must be rejected"
        );
    }

    #[test]
    fn stalled_taskbar_does_not_block_toggles_and_applies_latest_state() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (started_tx, started_rx) = crossbeam_channel::bounded(1);
        let (resume_tx, resume_rx) = crossbeam_channel::bounded(1);
        let (applied_tx, applied_rx) = crossbeam_channel::unbounded();
        let worker = std::thread::spawn(move || {
            let mut first = true;
            taskbar_updates(rx, |shown| {
                if first {
                    first = false;
                    started_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                }
                applied_tx.send(shown).unwrap();
            });
        });
        tx.send(false).unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        for shown in [true, false, true] {
            tx.send(shown).unwrap();
        }
        drop(tx);
        resume_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(applied_rx.try_iter().collect::<Vec<_>>(), [false, true]);
    }
}
