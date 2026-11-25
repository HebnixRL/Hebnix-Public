//! HTML plugin overlays (`plugin.overlay_page` + `hebnix.overlay.send`),
//! linux-port backend.
//!
//! Windows renders this through a WebView2 composition controller bound
//! into the same DirectComposition visual tree as the game overlay. There's
//! no Wayland equivalent of that composition API, so this takes a different
//! (and honestly simpler) route: a dedicated click-through
//! `gtk-layer-shell` window on the `Overlay` layer, hosting one
//! `webkit2gtk::WebView` whose page is a small generated shell containing
//! one `<iframe>` per plugin that sets `overlay_page`. `hebnix.overlay.send`
//! delivers data by running JS that does
//! `iframe.contentWindow.postMessage(data, '*')`, matching how the plugin
//! pages already listen (`window.addEventListener('message', ...)`) since
//! that's the same mechanism Windows' host page uses per plugin's own
//! `window.addEventListener('message')` in the page itself.
//!
//! GTK/WebKit's GLib main loop is never run as its own `gtk::main()` -- this
//! app already owns the process's main loop via winit/eframe, so instead
//! `WebviewOverlay::pump()` is called once a frame from the main tick and
//! drains pending GLib main-context events non-blockingly
//! (`gtk::main_iteration_do(false)`), which is what actually drives WebKit
//! painting, JS execution and the layer-shell surface's own Wayland events.

use std::path::{Path, PathBuf};

#[allow(unused_imports)]
use gdk;
use gtk::prelude::*;
use gtk_layer_shell::LayerShell;
use webkit2gtk::WebViewExt;

/// (slug, overlay_page filename, plugin's own assets dir)
pub type PageLayer = (String, String, PathBuf);

pub struct WebviewOverlay {
    window: Option<gtk::Window>,
    webview: Option<webkit2gtk::WebView>,
    active: Vec<PageLayer>,
    visible: bool,
    last_error: Option<String>,
    /// GPU mode: when the overlay was first shown, until it survived 5 s
    gpu_shown_since: Option<std::time::Instant>,
    gpu_confirmed: bool,
}

/// layer-shell only exists on Wayland; on X11 (i3, Cinnamon, XFCE, ...) the
/// overlay is a plain override-redirect window instead
fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
}

/// X11: (x, y, width, height) of the monitor Rocket League is on, else the
/// primary monitor
fn x11_target() -> (i32, i32, i32, i32) {
    if let Some((l, t, r, b)) = hebnix_sdk::process::rocket_league_monitor_rect() {
        if r > l && b > t {
            return (l, t, r - l, b - t);
        }
    }
    gdk::Display::default()
        .and_then(|display| display.primary_monitor().or_else(|| display.monitor(0)))
        .map(|monitor| {
            let area = monitor.geometry();
            (area.x(), area.y(), area.width(), area.height())
        })
        .unwrap_or((0, 0, 1920, 1080))
}

fn file_url(path: &Path) -> String {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("file://{}", path.display()))
}

/// `overlay_page` may carry a query string or fragment (`page.html?v=2`,
/// used by plugins for cache busting). Only the path part is a filename, so
/// joining the whole string and percent-encoding it turned `?` into `%3F`
/// and the page never loaded.
fn page_url(assets_dir: &Path, page: &str) -> String {
    let split = page.find(['?', '#']).unwrap_or(page.len());
    let (name, suffix) = page.split_at(split);
    format!("{}{suffix}", file_url(&assets_dir.join(name)))
}

/// Replace the window's input region with the union of the rectangles the
/// plugin pages report for their interactive elements; empty means fully
/// click-through.
fn apply_input_region(window: &gtk::Window, regions: &std::collections::HashMap<String, [f64; 4]>) {
    let region = cairo::Region::create();
    for [left, top, right, bottom] in regions.values() {
        let rect = cairo::RectangleInt::new(
            left.floor() as i32,
            top.floor() as i32,
            (right.ceil() - left.floor()).max(0.0) as i32,
            (bottom.ceil() - top.floor()).max(0.0) as i32,
        );
        let _ = region.union_rectangle(&rect);
    }
    window.input_shape_combine_region(Some(&region));
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

impl WebviewOverlay {
    pub fn new() -> Self {
        Self {
            window: None,
            webview: None,
            active: Vec::new(),
            visible: false,
            last_error: None,
            gpu_shown_since: None,
            gpu_confirmed: false,
        }
    }

    /// drains pending GLib/GTK main-context work (WebKit painting, JS
    /// callbacks, layer-shell surface events). Cheap no-op when idle. Call
    /// once a frame regardless of whether a webview currently exists --
    /// also what actually services the tray icon's DBus menu.
    pub fn pump() {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
    }

    /// true once at least one plugin's html overlay is loaded and ticking.
    pub fn is_active(&self) -> bool {
        self.webview.is_some()
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    fn create_window(&mut self) -> Result<(), String> {
        static GTK_INIT: std::sync::Once = std::sync::Once::new();
        let mut init_err = None;
        GTK_INIT.call_once(|| {
            if let Err(e) = gtk::init() {
                init_err = Some(e.to_string());
            }
        });
        if let Some(e) = init_err {
            return Err(format!("gtk::init() failed: {e}"));
        }

        let wayland = is_wayland_session();
        if !wayland && !hebnix_sdk::process::x11::compositor_running() {
            return Err("the HTML overlay needs a compositing manager (Cinnamon/Muffin has one; \
                        i3 needs picom or similar)"
                .to_string());
        }
        // X11: an override-redirect popup, so no window manager tiles, decorates or focuses it
        let window = gtk::Window::new(if wayland {
            gtk::WindowType::Toplevel
        } else {
            gtk::WindowType::Popup
        });
        window.set_decorated(false);
        window.set_app_paintable(true);
        if let Some(screen) = gtk::prelude::GtkWindowExt::screen(&window) {
            if let Some(visual) = screen.rgba_visual() {
                window.set_visual(Some(&visual));
            }
        }

        if wayland {
            window.init_layer_shell();
            window.set_layer(gtk_layer_shell::Layer::Overlay);
            window.set_namespace("hebnix-html-overlay");
            for edge in [
                gtk_layer_shell::Edge::Top,
                gtk_layer_shell::Edge::Bottom,
                gtk_layer_shell::Edge::Left,
                gtk_layer_shell::Edge::Right,
            ] {
                window.set_anchor(edge, true);
            }
            window.set_exclusive_zone(-1);
            // pure HUD, never grabs keyboard/pointer -- plugin pages aren't
            // interactive, same click-through intent as the draw overlay.
            window.set_keyboard_interactivity(false);
            window.set_keyboard_mode(gtk_layer_shell::KeyboardMode::None);
            // set size explicitly to avoid width 0 requests
            window.set_size_request(1, 1);
        } else {
            window.set_keep_above(true);
            window.set_accept_focus(false);
            window.set_focus_on_map(false);
            window.set_skip_taskbar_hint(true);
            window.set_skip_pager_hint(true);
            let (x, y, w, h) = x11_target();
            window.move_(x, y);
            window.set_default_size(w, h);
        }
        let empty_region = cairo::Region::create();
        window.input_shape_combine_region(Some(&empty_region));

        // WEBKIT_DISABLE_COMPOSITING_MODE is set once at process start in
        // main() -- must happen before any other thread exists, so it can't
        // safely be set here (this runs long after startup, other threads
        // already running).
        let webview = webkit2gtk::WebView::new();
        tracing::info!(
            software = render_mode::software(),
            "html overlay: rendering mode (software when forced in Settings or after a GPU crash)"
        );
        // WebKit's GPU-accelerated compositing path (DMA-BUF + explicit
        // sync, wp_linux_drm_syncobj_surface_v1) hits a driver bug on at
        // least this system's NVIDIA/Wayland combo -- confirmed live: it
        // throws a fatal Wayland protocol error ("Missing acquire
        // timeline") that GTK treats as unrecoverable and aborts the whole
        // process on. This HUD is plain CSS/DOM with no video/canvas load,
        // so there's nothing worth GPU-compositing here anyway -- force
        // software rendering to sidestep the driver bug entirely.
        if let Some(settings) = webkit2gtk::WebViewExt::settings(&webview) {
            webkit2gtk::SettingsExt::set_hardware_acceleration_policy(
                &settings,
                if render_mode::software() {
                    webkit2gtk::HardwareAccelerationPolicy::Never
                } else {
                    webkit2gtk::HardwareAccelerationPolicy::Always
                },
            );
            // the host shell is loaded via load_html with a synthetic base
            // URI, and its iframes point at real file:// paths -- without
            // these, WebKit's local-file security policy silently blocks
            // that cross-origin navigation (host page not itself file://
            // loaded), and the iframe just never loads with no error
            // reported anywhere obvious.
            webkit2gtk::SettingsExt::set_allow_file_access_from_file_urls(&settings, true);
            webkit2gtk::SettingsExt::set_allow_universal_access_from_file_urls(&settings, true);
            // plugin page JS errors/console.log land in our own stdout/log
            // instead of vanishing into the (invisible, headless) web
            // process -- this is the main debugging tool for page content
            // issues once the layer-shell surface itself is confirmed up.
            webkit2gtk::SettingsExt::set_enable_write_console_messages_to_stdout(&settings, true);
        }
        webview.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));

        webview.connect_load_changed(|_, event| {
            tracing::info!(?event, "html overlay: webview load event");
        });
        webview.connect_load_failed(|_, event, uri, error| {
            tracing::warn!(?event, uri, %error, "html overlay: webview load failed");
            false // let webkit show its own error page too
        });

        // Plugin pages that set `plugin.clickable` report the rectangle of
        // their interactive elements to the host page
        // (`{__hebnixPointerHit:true, region:{left,top,right,bottom}|null}`,
        // same message Windows' host consumes). The window stays fully
        // click-through except for those rectangles.
        if let Some(manager) = webkit2gtk::WebViewExt::user_content_manager(&webview) {
            use javascriptcore::ValueExt;
            use webkit2gtk::UserContentManagerExt;
            manager.register_script_message_handler("hebnixptr");
            let regions = std::rc::Rc::new(std::cell::RefCell::new(
                std::collections::HashMap::<String, [f64; 4]>::new(),
            ));
            let region_window = window.clone();
            manager.connect_script_message_received(Some("hebnixptr"), move |_, result| {
                let Some(text) = result.js_value().map(|value| value.to_str().to_string()) else {
                    return;
                };
                let Ok(message) = serde_json::from_str::<serde_json::Value>(&text) else {
                    return;
                };
                let Some(slug) = message.get("slug").and_then(|v| v.as_str()) else {
                    return;
                };
                let mut regions = regions.borrow_mut();
                match message.get("region").filter(|r| r.is_object()) {
                    Some(r) => {
                        let side = |key: &str| r.get(key).and_then(|v| v.as_f64());
                        if let (Some(l), Some(t), Some(rt), Some(b)) =
                            (side("left"), side("top"), side("right"), side("bottom"))
                        {
                            regions.insert(slug.to_string(), [l, t, rt, b]);
                        }
                    }
                    None => {
                        regions.remove(slug);
                    }
                }
                apply_input_region(&region_window, &regions);
            });
        }

        window.add(&webview);
        window.show_all();
        // show_all() actually maps/shows the window -- keep `visible` in
        // sync with that or the very next hide() call (if RL isn't
        // focused) sees `!self.visible` and wrongly no-ops, leaving this
        // stuck on screen regardless of focus until some *other* true ->
        // false transition happens to fix the bookkeeping. Confirmed live:
        // this was the actual cause of the overlay showing while Hebnix's
        // own window (not Rocket League) had focus.
        self.visible = true;

        tracing::info!(
            is_layer_window = wayland && window.is_layer_window(),
            wayland,
            "html overlay: window created"
        );

        self.window = Some(window);
        self.webview = Some(webview);
        Ok(())
    }

    /// (re)build the host page if the active plugin set changed since the
    /// last call. Cheap no-op otherwise -- this runs every frame.
    ///
    /// The window/webview, once created, is kept alive for the rest of the
    /// process's life -- an earlier version destroyed and recreated it on
    /// every transition to/from an empty page set (e.g. toggling the
    /// plugin, or any full plugin-runtime reload for an unrelated reason
    /// like a Steam/Epic transition), which left the *old* surface's last
    /// frame visible on screen for a moment (a compositor close animation,
    /// or possibly the GTK object just not being torn down synchronously)
    /// while the *new* one was already up -- looking exactly like a stray
    /// duplicate overlay. Confirmed live by the user. An empty page set now
    /// just hides the existing window and clears its content instead.
    pub fn sync_pages(&mut self, pages: &[PageLayer]) {
        if pages.is_empty() {
            if !self.active.is_empty() {
                tracing::info!("html overlay: no active page plugins, clearing host page");
                self.active.clear();
                if let Some(webview) = &self.webview {
                    webview.load_html("<!doctype html><html><body></body></html>", None);
                }
                self.hide();
            }
            return;
        }
        if pages == self.active.as_slice() {
            return;
        }
        tracing::info!(
            slugs = ?pages.iter().map(|(slug, ..)| slug.as_str()).collect::<Vec<_>>(),
            "html overlay: active plugin page set changed, rebuilding host page"
        );
        self.active = pages.to_vec();

        if self.webview.is_none() {
            if let Err(e) = self.create_window() {
                tracing::warn!(error = %e, "html overlay: failed to create layer-shell window");
                self.last_error = Some(e);
                return;
            }
        }
        let Some(webview) = &self.webview else { return };

        let mut iframes = String::new();
        for (slug, page, assets_dir) in pages {
            let src = page_url(assets_dir, page);
            tracing::info!(slug, src, "html overlay: iframe src");
            iframes.push_str(&format!(
                "<iframe id=\"frame-{id}\" src=\"{src}\" \
                 style=\"position:absolute;inset:0;width:100%;height:100%;border:0;\"></iframe>\n",
                id = html_escape(slug),
            ));
        }
        let html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
             html,body{{margin:0;padding:0;background:transparent;overflow:hidden;}}\
             iframe{{pointer-events:none;background:transparent;}}\
             </style></head><body>{iframes}<script>\
             window.addEventListener('message', function(e) {{\
               var d = e.data;\
               if (!d || !d.__hebnixPointerHit) return;\
               var frames = document.querySelectorAll('iframe');\
               for (var i = 0; i < frames.length; i++) {{\
                 if (frames[i].contentWindow === e.source) {{\
                   window.webkit.messageHandlers.hebnixptr.postMessage(JSON.stringify(\
                     {{slug: frames[i].id, region: d.region || null}}));\
                   return;\
                 }}\
               }}\
             }});\
             {repaint}\
             window.__hebnixDeliver = function(slug, data) {{\
               var el = document.getElementById('frame-' + slug);\
               if (el && el.contentWindow) el.contentWindow.postMessage(data, '*');\
               else console.error('hebnix overlay: no iframe for ' + slug);\
             }};\
             </script></body></html>",
            repaint = if render_mode::software() {
                // webkit2gtk 2.54.2's software path only repaints the changed
                // area into an otherwise blank buffer, so everything but the
                // moving part vanished (Controller Overlay). Nudging the root's
                // opacity every frame makes it repaint the whole page.
                "(function f(t){document.body.style.opacity=(t&1)?'0.999':'1';\
                 requestAnimationFrame(function(){f(t+1);});})(0);"
            } else {
                ""
            },
        );
        tracing::debug!(html, "html overlay: host page");
        // real file:// base (not a synthetic scheme) so the file:// iframe
        // navigations below aren't cross-origin even without the
        // allow_*_access_from_file_urls settings above -- belt and
        // suspenders, since either alone should be enough.
        webview.load_html(&html, Some("file:///"));
    }

    /// push `data` (already-serialized JSON) into the named plugin's iframe.
    /// `slug` must be run through the same `html_escape` used for the
    /// iframe's own id in `sync_pages`, or the lookup silently misses.
    pub fn deliver(&self, slug: &str, data: &serde_json::Value) {
        let Some(webview) = &self.webview else { return };
        let script = format!(
            "window.__hebnixDeliver && window.__hebnixDeliver({}, {})",
            json_escape(&html_escape(slug)),
            data
        );
        tracing::trace!(slug, %data, "html overlay: deliver");
        webview.run_javascript(&script, gtk::gio::Cancellable::NONE, |result| {
            if let Err(e) = result {
                tracing::warn!(error = %e, "html overlay: deliver run_javascript failed");
            }
        });
    }

    pub fn show(&mut self) {
        if self.visible {
            return;
        }
        if let Some(window) = &self.window {
            tracing::info!("html overlay: show (Rocket League focused)");
            if !render_mode::software() && !self.gpu_confirmed && self.gpu_shown_since.is_none() {
                render_mode::gpu_starting();
                self.gpu_shown_since = Some(std::time::Instant::now());
            }
            if is_wayland_session() {
                // restore full click-through input region
                let empty_region = cairo::Region::create();
                window.input_shape_combine_region(Some(&empty_region));
                window.move_(0, 0);
                window.show_all();
            } else {
                // follow the game's monitor
                let (x, y, w, h) = x11_target();
                window.move_(x, y);
                window.resize(w, h);
                window.show_all();
            }
            self.visible = true;
        }
    }

    /// Call once a frame while shown: once the GPU-rendered overlay has been on
    /// screen for a few seconds without taking the process down, clear the
    /// crash marker so the next start keeps GPU rendering.
    pub fn tick_visible(&mut self) {
        if let Some(since) = self.gpu_shown_since {
            if self.visible && since.elapsed() >= std::time::Duration::from_secs(5) {
                render_mode::gpu_ok();
                self.gpu_shown_since = None;
                self.gpu_confirmed = true;
            }
        }
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        if let Some(window) = &self.window {
            tracing::info!("html overlay: hide (Rocket League not focused)");
            if is_wayland_session() {
                // move off-screen and clear input region to be click-through
                window.move_(-20000, -20000);
                let empty_region = cairo::Region::create();
                window.input_shape_combine_region(Some(&empty_region));
            } else {
                window.hide();
            }
        }
        self.visible = false;
    }
}

fn html_escape(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

impl Default for WebviewOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod page_url_tests {
    use super::page_url;
    use std::path::Path;

    #[test]
    fn keeps_query_and_fragment_unescaped() {
        let url = page_url(Path::new("/p/My Plugin/assets"), "page.html?v=2#top");
        assert_eq!(url, "file:///p/My%20Plugin/assets/page.html?v=2#top");
    }

    #[test]
    fn plain_page_is_unchanged() {
        assert_eq!(page_url(Path::new("/a"), "x.html"), "file:///a/x.html");
    }
}

/// How the HTML overlay is rendered. GPU by default on Hyprland, software
/// elsewhere (the GPU-rendered overlay updates far too slowly on KDE Plasma).
/// Settings can force either one; `HEBNIX_WEBVIEW_SOFTWARE` forces software.
/// A GPU start that died while the overlay was coming up switches to software
/// (older NVIDIA drivers abort the whole process with a "Missing acquire
/// timeline" Wayland protocol error).
pub mod render_mode {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    static SOFTWARE: AtomicBool = AtomicBool::new(false);

    fn state_dir() -> PathBuf {
        crate::config::base_dir().join("state")
    }

    fn forced_marker() -> PathBuf {
        state_dir().join("webview-software")
    }

    fn pending_marker() -> PathBuf {
        state_dir().join("webview-gpu-pending")
    }

    /// set in Settings: GPU rendering where software is the default
    fn gpu_marker() -> PathBuf {
        state_dir().join("webview-gpu")
    }

    /// GPU rendering is only the default where it is known to keep up
    pub fn default_software() -> bool {
        !crate::winutil::is_hyprland()
    }

    /// the Settings choice; takes effect at the next start
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Choice {
        /// hardware on Hyprland, software elsewhere
        Auto,
        Hardware,
        Software,
    }

    /// Decide once, from `main()` before any other thread exists (it may set
    /// `WEBKIT_DISABLE_COMPOSITING_MODE`).
    ///
    /// # Safety
    /// Must run while the process is still single-threaded.
    pub unsafe fn init() {
        let software = if std::env::var_os("HEBNIX_WEBVIEW_SOFTWARE").is_some() {
            tracing::info!("html overlay: software rendering (HEBNIX_WEBVIEW_SOFTWARE)");
            true
        } else if forced_marker().exists() {
            tracing::info!("html overlay: software rendering (Settings or an earlier GPU crash)");
            true
        } else if pending_marker().exists() {
            tracing::warn!(
                "html overlay: the last start died while the GPU-rendered overlay was coming up, \
                 switching to software rendering (turn it back off in Settings to retry)"
            );
            let _ = std::fs::create_dir_all(state_dir());
            let _ = std::fs::write(forced_marker(), "set automatically: GPU overlay crashed\n");
            let _ = std::fs::remove_file(pending_marker());
            true
        } else if gpu_marker().exists() {
            false
        } else if default_software() {
            tracing::info!("html overlay: software rendering (default outside Hyprland)");
            true
        } else {
            false
        };
        SOFTWARE.store(software, Ordering::Relaxed);
        if software {
            unsafe { std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1") };
        }
    }

    pub fn software() -> bool {
        SOFTWARE.load(Ordering::Relaxed)
    }

    pub(super) fn gpu_starting() {
        let _ = std::fs::create_dir_all(state_dir());
        let _ = std::fs::write(pending_marker(), b"");
    }

    pub(super) fn gpu_ok() {
        let _ = std::fs::remove_file(pending_marker());
    }

    pub fn choice() -> Choice {
        if forced_marker().exists() {
            Choice::Software
        } else if gpu_marker().exists() {
            Choice::Hardware
        } else {
            Choice::Auto
        }
    }

    pub fn set_choice(choice: Choice) -> std::io::Result<()> {
        fn remove(path: PathBuf) -> std::io::Result<()> {
            match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            }
        }
        let _ = std::fs::remove_file(pending_marker());
        remove(forced_marker())?;
        remove(gpu_marker())?;
        let marker = match choice {
            Choice::Auto => return Ok(()),
            Choice::Hardware => gpu_marker(),
            Choice::Software => forced_marker(),
        };
        std::fs::create_dir_all(state_dir())?;
        std::fs::write(marker, "set in Settings\n")
    }

    /// what `choice` gives at the next start
    pub fn choice_is_software(choice: Choice) -> bool {
        match choice {
            Choice::Auto => default_software(),
            Choice::Hardware => false,
            Choice::Software => true,
        }
    }
}

pub mod runtime {
    /// The Linux HTML overlay is a webkit2gtk window (`WebviewOverlay`), so
    /// there is no WebView2 runtime to install or wait for.
    pub fn is_available() -> bool {
        true
    }

    pub fn ensure_present() {}
}

/// Same surface the Windows `WebviewHost` has, backed by `WebviewOverlay` so
/// `app.rs` drives both platforms through one code path.
pub mod host {
    use super::{PageLayer, WebviewOverlay};
    use std::path::PathBuf;

    pub struct WebviewHost {
        overlay: WebviewOverlay,
    }

    impl WebviewHost {
        pub fn new(_visual: ()) -> Self {
            Self {
                overlay: WebviewOverlay::new(),
            }
        }

        pub fn sync_pages(&mut self, pages: &[(String, Option<String>, PathBuf, bool)]) {
            let layers: Vec<PageLayer> = pages
                .iter()
                .filter_map(|(slug, page, assets_dir, _)| {
                    page.as_ref()
                        .map(|page| (slug.clone(), page.clone(), assets_dir.clone()))
                })
                .collect();
            self.overlay.sync_pages(&layers);
        }

        pub fn deliver(&self, slug: &str, data: serde_json::Value) -> Result<(), String> {
            if !self.overlay.is_active() {
                return Err("html overlay is not running".to_string());
            }
            self.overlay.deliver(slug, &data);
            Ok(())
        }

        pub fn status(&self) -> String {
            if self.overlay.is_active() {
                format!(
                    "webkit2gtk overlay, {} plugin page(s), {}",
                    self.overlay.active.len(),
                    if self.overlay.visible { "shown" } else { "hidden" }
                )
            } else {
                "webkit2gtk overlay not created".to_string()
            }
        }

        /// plugin pages are click-through HUDs
        pub fn wants_input(&self) -> bool {
            false
        }

        pub fn wants_overlay(&self) -> bool {
            self.overlay.is_active() && !self.overlay.active.is_empty()
        }

        pub fn is_ready(&self) -> bool {
            self.overlay.is_active()
        }

        pub fn tick(&mut self, _target: (), _size: (u32, u32), visible: bool) {
            if visible {
                self.overlay.show();
                self.overlay.tick_visible();
            } else {
                self.overlay.hide();
            }
        }

        pub fn take_error(&mut self) -> Option<String> {
            self.overlay.take_error()
        }
    }
}
