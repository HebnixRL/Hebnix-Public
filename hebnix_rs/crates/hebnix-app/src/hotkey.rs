//! non-consuming show/hide hotkey polling.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

fn normalize_key(name: &str) -> Option<String> {
    let key = name.trim().to_lowercase();
    let mut chars = key.chars();
    if let (Some(character), None) = (chars.next(), chars.next()) {
        if character.is_ascii_alphanumeric() {
            return Some(key);
        }
    }

    matches!(
        key.as_str(),
        "f1" | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
            | "tab"
            | "space"
            | "enter"
            | "return"
            | "backspace"
            | "esc"
            | "escape"
            | "insert"
            | "delete"
            | "home"
            | "end"
            | "page up"
            | "page down"
            | "up"
            | "down"
            | "left"
            | "right"
            | "`"
            | "-"
            | "="
            | "["
            | "]"
            | "\\"
            | ";"
            | "'"
            | ","
            | "."
            | "/"
    )
    .then_some(key)
}

pub struct ToggleHotkey {
    key: Arc<RwLock<Option<String>>>,
    stop: Arc<AtomicBool>,
}

impl ToggleHotkey {
    pub fn clear(&mut self) {
        if let Ok(mut key) = self.key.write() {
            *key = None;
        }
    }

    pub fn new() -> Option<Self> {
        Some(Self {
            key: Arc::new(RwLock::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn rebind(&mut self, key_name: &str) -> bool {
        let Some(key) = normalize_key(key_name) else {
            tracing::warn!("cannot map '{key_name}' to a hotkey");
            return false;
        };
        let Ok(mut current) = self.key.write() else {
            return false;
        };
        *current = Some(key);
        true
    }

    pub fn listen<F>(&self, mut on_press: F)
    where F: FnMut() + Send + 'static,
    {
        let key = Arc::clone(&self.key);
        let stop = Arc::clone(&self.stop);
        std::thread::Builder::new().name("hotkey-listener".into()).spawn(move || {
            use windows::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, UnhookWindowsHookEx, PeekMessageW, MSG, PM_REMOVE, WH_KEYBOARD_LL};
            let (tx, rx) = std::sync::mpsc::channel();
            if let Ok(mut slot) = FALLBACK.lock() { *slot = Some(tx); }
            let module = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }.ok();
            let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(selected_key_fallback), module.map(Into::into), 0) }.ok();
            let mut observed_key = None;
            let mut was_down = false;
            let mut last_press: Option<std::time::Instant> = None;
            while !stop.load(Ordering::Relaxed) {
                let current = key.read().ok().and_then(|value| value.clone());
                if current != observed_key {
                    FALLBACK_VK.store(current.as_deref().and_then(hebnix_sdk::input::name_to_vk).unwrap_or(0) as u32, Ordering::Relaxed);
                    FALLBACK_DOWN.store(false, Ordering::Relaxed);
                    observed_key = current.clone();
                    was_down = current.as_deref().is_some_and(hebnix_sdk::input::is_key_pressed);
                } else if let Some(current) = current.as_deref() {
                    let down = hebnix_sdk::input::is_key_pressed(current);
                    if down && !was_down && last_press.is_none_or(|last| last.elapsed() >= Duration::from_millis(150)) {
                        on_press(); last_press = Some(std::time::Instant::now());
                    }
                    was_down = down;
                }
                let mut message = MSG::default();
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {}
                // The hook only backs up a missed polling edge for the selected
                // key. Both paths forward the original key to the focused app.
                while rx.try_recv().is_ok() {
                    if last_press.is_none_or(|last| last.elapsed() >= Duration::from_millis(150)) {
                        on_press(); last_press = Some(std::time::Instant::now());
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            FALLBACK_VK.store(0, Ordering::Relaxed);
            if let Some(hook) = hook { let _ = unsafe { UnhookWindowsHookEx(hook) }; }
            if let Ok(mut slot) = FALLBACK.lock() { *slot = None; }
        }).ok();
    }
}

static FALLBACK_VK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static FALLBACK_DOWN: AtomicBool = AtomicBool::new(false);
static FALLBACK: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>> = std::sync::Mutex::new(None);
unsafe extern "system" fn selected_key_fallback(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_SYSKEYDOWN, WM_KEYUP, WM_SYSKEYUP};
    if code >= 0 {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if event.vkCode == FALLBACK_VK.load(Ordering::Relaxed) && event.vkCode != 0 {
            let message = wparam.0 as u32;
            if message == WM_KEYUP || message == WM_SYSKEYUP { FALLBACK_DOWN.store(false, Ordering::Relaxed); }
            else if (message == WM_KEYDOWN || message == WM_SYSKEYDOWN) && !FALLBACK_DOWN.swap(true, Ordering::Relaxed) {
                if let Ok(slot) = FALLBACK.try_lock() { if let Some(tx) = slot.as_ref() { let _ = tx.send(()); } }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

impl Drop for ToggleHotkey {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
