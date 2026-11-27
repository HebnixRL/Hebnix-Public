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

    // Focus restrictions belong to the UI handler, as before the Wry migration.
    pub fn set_restricted(&self, _restricted: bool) {}

    /// Keep the same non-consuming listener for both UI implementations.
    pub fn listen_show_hide<F>(&self, on_press: F)
    where F: FnMut() + Send + 'static,
    {
        start_key_fallback();
        self.listen_inner(on_press, true);
    }
    pub fn listen<F>(&self, on_press: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.listen_inner(on_press, false);
    }

    fn listen_inner<F>(&self, mut on_press: F, fallback: bool)
    where F: FnMut() + Send + 'static,
    {
        let key = Arc::clone(&self.key);
        let stop = Arc::clone(&self.stop);
        std::thread::Builder::new()
            .name("hotkey-listener".into())
            .spawn(move || {
                let mut observed_key = None;
                let mut was_down = false;
                let mut last_press: Option<std::time::Instant> = None;
                while !stop.load(Ordering::Relaxed) {
                    let current = key.read().ok().and_then(|value| value.clone());
                    if fallback {
                        FALLBACK_KEY.store(current.as_deref().and_then(hebnix_sdk::input::keyboard::name_to_vk).unwrap_or(0) as u32, Ordering::Relaxed);
                    }
                    let fallback_pressed = fallback && FALLBACK_PRESSED.swap(false, Ordering::AcqRel);
                    if current != observed_key {
                        if fallback { FALLBACK_DOWN.store(false, Ordering::Release); }
                        observed_key = current.clone();
                        was_down = current
                            .as_deref()
                            .is_some_and(hebnix_sdk::input::is_key_pressed);
                    } else if let Some(current) = current.as_deref() {
                        let is_down = hebnix_sdk::input::is_key_pressed(current);
                        if (is_down && !was_down || fallback_pressed)
                            && last_press.is_none_or(|at| at.elapsed() >= Duration::from_millis(150)) {
                            last_press = Some(std::time::Instant::now());
                            tracing::info!(key = current, fallback_pressed, "show/hide key received");
                            on_press();
                        }
                        was_down = is_down;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .ok();
    }
}

// GetAsyncKeyState can miss input while the game owns exclusive focus. This
// non-consuming fallback records ONLY the configured key and always forwards
// every event to Windows, including Space and keys unrelated to Show/Hide.
static FALLBACK_KEY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static FALLBACK_PRESSED: AtomicBool = AtomicBool::new(false);
static FALLBACK_DOWN: AtomicBool = AtomicBool::new(false);

fn record_selected_key(vk: u32, down: bool) {
    if vk == 0 || vk != FALLBACK_KEY.load(Ordering::Relaxed) { return; }
    if down {
        if !FALLBACK_DOWN.swap(true, Ordering::AcqRel) { FALLBACK_PRESSED.store(true, Ordering::Release); }
    } else { FALLBACK_DOWN.store(false, Ordering::Release); }
}

fn start_key_fallback() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| { let _ = std::thread::Builder::new().name("show-hide-key-fallback".into()).spawn(|| {
        use windows::Win32::UI::WindowsAndMessaging::*;
        unsafe extern "system" fn key_event(code: i32, message: windows::Win32::Foundation::WPARAM, data: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
            unsafe {
                if code >= 0 {
                    let event = &*(data.0 as *const KBDLLHOOKSTRUCT);
                    match message.0 as u32 {
                        WM_KEYDOWN | WM_SYSKEYDOWN => record_selected_key(event.vkCode, true),
                        WM_KEYUP | WM_SYSKEYUP => record_selected_key(event.vkCode, false),
                        _ => {}
                    }
                }
                CallNextHookEx(None, code, message, data)
            }
        }
        unsafe {
            let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok().map(Into::into);
            match SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_event), module, 0) {
                Ok(hook) => {
                    let mut message = MSG::default();
                    while GetMessageW(&mut message, None, 0, 0).0 > 0 { let _ = TranslateMessage(&message); DispatchMessageW(&message); }
                    let _ = UnhookWindowsHookEx(hook);
                }
                Err(error) => tracing::warn!(%error, "show/hide key fallback unavailable; polling remains active"),
            }
        }
    }); });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_tracks_only_the_selected_key_and_ignores_repeat() {
        FALLBACK_KEY.store(0x71, Ordering::Relaxed); // F2
        FALLBACK_DOWN.store(false, Ordering::Relaxed);
        FALLBACK_PRESSED.store(false, Ordering::Relaxed);
        record_selected_key(0x20, true); // Space remains unrelated.
        assert!(!FALLBACK_PRESSED.load(Ordering::Relaxed));
        record_selected_key(0x71, true);
        assert!(FALLBACK_PRESSED.swap(false, Ordering::Relaxed));
        record_selected_key(0x71, true);
        assert!(!FALLBACK_PRESSED.load(Ordering::Relaxed));
        record_selected_key(0x71, false);
        record_selected_key(0x71, true);
        assert!(FALLBACK_PRESSED.swap(false, Ordering::Relaxed));
        FALLBACK_KEY.store(0, Ordering::Relaxed);
        FALLBACK_DOWN.store(false, Ordering::Relaxed);
    }
}

impl Drop for ToggleHotkey {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
