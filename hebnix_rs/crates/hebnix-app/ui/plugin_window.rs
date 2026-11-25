use super::HebnixApp;
use crate::{
    modern_ui::{Navigation, PLUGIN_WINDOW_PAGE},
    plugins::lua_api::WindowState,
};
use eframe::egui;
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashSet, num::NonZeroIsize};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};
use wry::{
    Rect, WebView,
    dpi::{PhysicalPosition, PhysicalSize},
};

struct Host(HWND);

impl HasWindowHandle for Host {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = Win32WindowHandle::new(
            NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?,
        );
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    ManageReady,
    PluginWindowInput { id: String, value: Value },
    PluginWindowExpand { id: String, open: bool },
    PluginWindowClose,
    PluginWindowDrag,
}

pub(super) struct PluginWindow {
    view: WebView,
    host: Host,
    receiver: crossbeam_channel::Receiver<String>,
    ready: bool,
    visible: bool,
    dimensions: (u32, u32),
    expanded: HashSet<String>,
    inputs: HashSet<String>,
    rendered: Option<String>,
}

impl PluginWindow {
    fn new(state: &WindowState, ctx: &egui::Context) -> Result<Self, String> {
        let (x, y) = state.pos.unwrap_or((80.0, 80.0));
        let scale = ctx.pixels_per_point();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("STATIC"),
                PCWSTR::null(),
                WS_POPUP | WS_CLIPCHILDREN,
                (x * scale).round() as i32,
                (y * scale).round() as i32,
                260,
                160,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|error| error.to_string())?;
        let host = Host(hwnd);
        let (sender, receiver) = crossbeam_channel::unbounded();
        let view = Navigation::build_view(&host, ctx, PLUGIN_WINDOW_PAGE, String::new(), sender)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            view,
            host,
            receiver,
            ready: false,
            visible: false,
            dimensions: (0, 0),
            expanded: HashSet::new(),
            inputs: HashSet::new(),
            rendered: None,
        })
    }

    fn show(&mut self, model: &Value, size: (u32, u32)) -> Result<(), String> {
        if !self.ready {
            return Ok(());
        }
        if self.dimensions != size {
            unsafe {
                SetWindowPos(
                    self.host.0,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    size.0 as i32,
                    size.1 as i32,
                    SWP_NOMOVE | SWP_NOACTIVATE,
                )
            }
            .map_err(|error| error.to_string())?;
            self.view
                .set_bounds(Rect {
                    position: PhysicalPosition::new(0, 0).into(),
                    size: PhysicalSize::new(size.0, size.1).into(),
                })
                .map_err(|error| error.to_string())?;
            self.dimensions = size;
        }
        let payload = model.to_string();
        if self.rendered.as_ref() != Some(&payload) {
            self.view
                .evaluate_script(&format!("window.showManager({payload})"))
                .map_err(|error| error.to_string())?;
            self.rendered = Some(payload);
        }
        if !self.visible {
            self.view
                .set_visible(true)
                .map_err(|error| error.to_string())?;
            let _ = unsafe { ShowWindow(self.host.0, SW_SHOWNOACTIVATE) };
            self.visible = true;
        }
        Ok(())
    }

    fn hide(&mut self) {
        if self.visible {
            let _ = unsafe { ShowWindow(self.host.0, SW_HIDE) };
            self.visible = false;
        }
    }
}

impl HebnixApp {
    pub(super) fn render_wry_plugin_windows(&mut self, ctx: &egui::Context) {
        let (monitor_width, monitor_height) = self.plugin_monitor_size();
        let scale = ctx.pixels_per_point();
        let focus = hebnix_sdk::process::is_rocket_league_focused()
            || crate::winutil::foreground_window_is_ours();
        let windows = self
            .plugin_mgr
            .plugins
            .iter()
            .filter_map(|plugin| {
                let runtime = plugin
                    .enabled
                    .then_some(plugin.runtime.as_ref())
                    .flatten()?;
                let state = runtime.host.window.borrow().clone();
                state.open.then(|| (plugin.slug.clone(), state))
            })
            .collect::<Vec<_>>();
        self.wry_plugin_windows
            .retain(|slug, _| windows.iter().any(|(open, _)| open == slug));
        for (slug, state) in windows {
            if !state.shown(focus) {
                if let Some(window) = self.wry_plugin_windows.get_mut(&slug) {
                    window.hide();
                }
                continue;
            }
            let window = match self.wry_plugin_windows.entry(slug.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    match PluginWindow::new(&state, ctx) {
                        Ok(window) => entry.insert(window),
                        Err(error) => {
                            self.plugin_mgr.close_window(&slug);
                            self.console
                                .write(format!("[Plugins] Could not open {slug}: {error}"));
                            continue;
                        }
                    }
                }
            };
            let mut input = None;
            let mut close = false;
            while let Ok(message) = window.receiver.try_recv() {
                match serde_json::from_str(&message) {
                    Ok(Message::ManageReady) => window.ready = true,
                    Ok(Message::PluginWindowClose) if state.close_button => close = true,
                    Ok(Message::PluginWindowDrag) => {
                        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
                        unsafe {
                            let _ = ReleaseCapture();
                            SendMessageW(
                                window.host.0,
                                WM_NCLBUTTONDOWN,
                                Some(WPARAM(HTCAPTION as usize)),
                                Some(LPARAM(0)),
                            );
                        }
                    }
                    Ok(Message::PluginWindowInput { id, value }) if window.inputs.contains(&id) => {
                        input = Some((id, value))
                    }
                    Ok(Message::PluginWindowExpand { id, open }) if window.inputs.contains(&id) => {
                        if open {
                            window.expanded.insert(id);
                        } else {
                            window.expanded.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
            if close {
                self.plugin_mgr.close_window(&slug);
                window.hide();
                continue;
            }
            let (widgets, error) =
                match self
                    .plugin_mgr
                    .web_window(&slug, ctx, input, window.expanded.clone())
                {
                    Ok(frame) => {
                        window.inputs = frame.inputs;
                        (frame.widgets, None)
                    }
                    Err(error) => {
                        window.inputs.clear();
                        (Vec::new(), Some(error))
                    }
                };
            let model = json!({
                "page": "plugin-window", "selected": slug, "title": state.title, "close": state.close_button,
                "opacity": state.opacity, "widgets": widgets, "error": error,
            });
            let dpi_scale = unsafe { GetDpiForWindow(window.host.0) }.max(96) as f32 / 96.0;
            let size = (
                (state.width.resolve(monitor_width, dpi_scale) * dpi_scale)
                    .round()
                    .max(1.0) as u32,
                (state.height.resolve(monitor_height, dpi_scale) * dpi_scale)
                    .round()
                    .max(1.0) as u32,
            );
            if let Err(error) = window.show(&model, size) {
                window.hide();
                self.plugin_mgr.close_window(&slug);
                self.console
                    .write(format!("[Plugins] Could not display {slug}: {error}"));
            }
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(window.host.0, &mut rect) }.is_ok() {
                let (x, y) = (rect.left as f32 / scale, rect.top as f32 / scale);
                if state
                    .last_pos
                    .is_none_or(|(old_x, old_y)| (old_x - x).abs() > 1.0 || (old_y - y).abs() > 1.0)
                {
                    self.plugin_mgr.set_window_pos(&slug, x, y);
                }
            }
        }
        if self
            .wry_plugin_windows
            .values()
            .any(|window| window.visible)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
}
