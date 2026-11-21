//! wlr-layer-shell overlay surface + wl_shm software rendering.
//!
//! A fullscreen, click-through `Layer::Overlay` surface (empty input region,
//! `KeyboardInteractivity::None`) rendered with tiny-skia into wl_shm
//! buffers.
//!
//! The layer surface only exists while something is drawn: hiding destroys
//! it and the next drawn frame creates a new one on the monitor Rocket
//! League is on (matched by the game window's position), the same way the
//! HTML overlay's GTK layer surface comes and goes. Created once and left
//! to the compositor, it stayed on whichever monitor was active when Hebnix
//! started, and unmapping/remapping one surface trips over compositors that
//! reset its state on unmap (KWin).
//!
//! Works over RL in both Borderless Windowed and the game's own real
//! fullscreen (confirmed live on Hyprland) -- the `Overlay` layer is above
//! `top` by design, so it isn't subject to whatever the compositor does to
//! hide bars/panels during fullscreen.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::registry_handlers;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm};
use wayland_client::protocol::{wl_output, wl_shm, wl_surface};
use wayland_client::{Connection, EventQueue, QueueHandle};

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    width: u32,
    height: u32,
    configured: bool,
    closed: bool,
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            self.width = w;
            self.height = h;
            self.configured = true;
        } else if w == 0 && h == 0 {
            // with all anchors set, compositor should give us full size and keep previous size if we have one
            if self.width > 0 && self.height > 0 {
                self.configured = true;
            }
        } else {
            if w > 0 && h > 0 {
                self.width = w;
                self.height = h;
                self.configured = true;
            } else if self.width > 0 && self.height > 0 {
                self.configured = true;
            }
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_layer!(State);
delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

pub struct WaylandOverlay {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    compositor: CompositorState,
    layer_shell: LayerShell,
    /// the mapped layer surface, None while hidden (see module docs)
    layer: Option<LayerSurface>,
    /// the monitor `layer` was created on; None when left to the compositor
    output: Option<wl_output::WlOutput>,
    /// Rocket League's window, (left, top, right, bottom) in the
    /// compositor's logical desktop coordinates
    target: Option<(i32, i32, i32, i32)>,
    /// buffers presented so far, reused round-robin once the compositor
    /// releases them (see `present`). Capped at a handful so a burst of
    /// frames outrunning the compositor's release rate can't grow the
    /// wl_shm pool without bound -- it used to allocate a brand new buffer
    /// on every single frame, which measurably leaked into the multiple
    /// gigabytes range over a session with sustained overlay activity.
    buffers: Vec<Buffer>,
}

impl WaylandOverlay {
    pub fn new() -> Result<Self, String> {
        let conn = Connection::connect_to_env()
            .map_err(|e| format!("wayland connect failed: {e}"))?;
        let (globals, mut queue) = wayland_client::globals::registry_queue_init(&conn)
            .map_err(|e| format!("wayland registry init failed: {e}"))?;
        let qh = queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|e| format!("wlr-layer-shell not available (compositor isn't wlroots-based, or doesn't support it): {e}"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
        let pool = SlotPool::new(1920 * 1080 * 4, &shm).map_err(|e| format!("shm pool: {e}"))?;

        let mut state = State {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            shm,
            pool,
            width: 0,
            height: 0,
            configured: false,
            closed: false,
        };
        // fetch the monitors' names and positions, needed to place the
        // surface on the game's monitor
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland output query failed: {e}"))?;

        Ok(WaylandOverlay {
            conn,
            queue,
            qh,
            state,
            compositor,
            layer_shell,
            layer: None,
            output: None,
            target: None,
            buffers: Vec::new(),
        })
    }

    /// where Rocket League's window is, so the overlay goes on its monitor.
    /// None (unknown) keeps whatever monitor the surface is already on.
    pub fn set_target(&mut self, rect: Option<(i32, i32, i32, i32)>) {
        self.target = rect;
    }

    /// (output, logical width, logical height) of the monitor holding the
    /// centre of the game window
    fn target_output(&self) -> Option<(wl_output::WlOutput, u32, u32)> {
        let (left, top, right, bottom) = self.target?;
        let (cx, cy) = ((left + right) / 2, (top + bottom) / 2);
        self.state.output_state.outputs().find_map(|output| {
            let info = self.state.output_state.info(&output)?;
            let (x, y) = info.logical_position.unwrap_or(info.location);
            let (w, h) = info.logical_size.or_else(|| {
                // no xdg-output: physical mode size over the integer scale
                let mode = info.modes.iter().find(|m| m.current)?;
                let scale = info.scale_factor.max(1);
                Some((mode.dimensions.0 / scale, mode.dimensions.1 / scale))
            })?;
            let inside = (x..x + w).contains(&cx) && (y..y + h).contains(&cy);
            (inside && w > 0 && h > 0).then_some((output, w as u32, h as u32))
        })
    }

    /// create, set up and commit a fresh layer surface (no buffer yet; the
    /// compositor answers the commit with the configure `present` waits for)
    fn create_layer(&mut self) {
        let output = self.target_output().map(|(output, _, _)| output);
        let surface = self.compositor.create_surface(&self.qh);

        // click-through: empty input region so pointer/keyboard events pass
        // straight through to whatever's underneath.
        if let Ok(region) = Region::new(&self.compositor) {
            surface.set_input_region(Some(region.wl_region()));
        }

        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some("hebnix-overlay"),
            output.as_ref(),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0); // 0,0 with all anchors = compositor picks full output size
        layer.commit();

        self.state.configured = false;
        self.layer = Some(layer);
        self.output = output;
    }

    /// destroy the layer surface (and the buffers that were attached to it)
    fn destroy_layer(&mut self) {
        self.buffers.clear();
        self.layer = None;
        self.output = None;
        self.state.configured = false;
        let _ = self.conn.flush();
    }

    /// read and dispatch whatever the compositor has sent, without blocking.
    /// Marks the connection closed if it died.
    fn pump(&mut self) {
        let _ = self.conn.flush();
        // `dispatch_pending` alone only processes events already read
        // into the queue's internal buffer -- it does not itself read
        // the socket. Without also reading here, events the compositor
        // sends later (critically, wl_buffer.release) never arrive, so
        // `present`'s buffer-reuse check sees every buffer as
        // permanently "still active" and silently drops every frame
        // after the first few.
        //
        // `ReadEventsGuard::read()` may only be called when the fd is
        // actually readable -- calling it unconditionally desyncs
        // fd-bearing messages on the socket. A protocol error surfaces
        // through `read()`'s Result, not `dispatch_pending`'s, so both
        // must be checked, or a dead connection looks alive forever and
        // the auto-recreate path never runs.
        let mut dead = false;
        if let Some(guard) = self.queue.prepare_read() {
            use std::os::fd::AsFd;
            let readable = nix::poll::poll(
                &mut [nix::poll::PollFd::new(self.queue.as_fd(), nix::poll::PollFlags::POLLIN)],
                nix::poll::PollTimeout::ZERO,
            )
            .map(|n| n > 0)
            .unwrap_or(false);
            if readable {
                if let Err(e) = guard.read() {
                    tracing::warn!("overlay: wayland read failed, will recreate: {e}");
                    dead = true;
                }
            }
        }
        if let Err(e) = self.queue.dispatch_pending(&mut self.state) {
            tracing::warn!("overlay: wayland dispatch failed, will recreate: {e}");
            dead = true;
        }
        if dead {
            // connection is dead (protocol error, compositor hung up,
            // etc). Mark closed so `NativeOverlay` (mod_linux.rs) tears
            // down and recreates the backend instead of retrying a broken
            // connection forever.
            self.state.closed = true;
        }
    }

    /// pump pending wayland events (non-blocking) and return the size to
    /// draw at: the mapped surface's, else the game monitor's. None if the
    /// connection died or no size is known yet.
    pub fn poll_size(&mut self) -> Option<(u32, u32)> {
        self.pump();
        if self.state.closed {
            return None;
        }
        let target = self.target_output();
        // the game moved to another monitor: start over there
        if let (Some(_), Some((want, _, _))) = (&self.layer, &target)
            && self.output.as_ref() != Some(want)
        {
            self.destroy_layer();
        }
        if self.layer.is_some() && self.state.configured {
            return Some((self.state.width, self.state.height));
        }
        if let Some((_, w, h)) = target {
            return Some((w, h));
        }
        if self.state.width > 0 && self.state.height > 0 {
            return Some((self.state.width, self.state.height));
        }
        // nothing known about the monitors: map a surface once to learn the
        // size (it stays mapped only if the frame draws something)
        if self.layer.is_none() {
            self.create_layer();
            if let Err(e) = self.queue.roundtrip(&mut self.state) {
                tracing::warn!("overlay: wayland roundtrip failed, will recreate: {e}");
                self.state.closed = true;
                return None;
            }
        }
        (self.state.configured && self.state.width > 0 && self.state.height > 0)
            .then_some((self.state.width, self.state.height))
    }

    /// paint `pixmap` into an shm buffer and commit it, mapping the surface
    /// first if it's hidden. A frame drawn at a size other than the one the
    /// compositor configured is skipped (the next one uses the right size).
    /// Reuses a buffer the compositor has already released rather than
    /// allocating a fresh one every call. Returns whether it was committed.
    pub fn present(&mut self, pixmap: &tiny_skia::Pixmap) -> bool {
        if self.state.closed {
            return false;
        }
        if self.layer.is_none() {
            self.create_layer();
        }
        if !self.state.configured {
            if let Err(e) = self.queue.roundtrip(&mut self.state) {
                tracing::warn!("overlay: wayland roundtrip failed, will recreate: {e}");
                self.state.closed = true;
                return false;
            }
            if !self.state.configured {
                return false;
            }
        }
        let width = self.state.width;
        let height = self.state.height;
        if width == 0 || height == 0 || (pixmap.width(), pixmap.height()) != (width, height) {
            return false;
        }
        let Some(layer) = self.layer.as_ref() else { return false };
        let stride = width as i32 * 4;

        // drop buffers left over from a stale resolution (monitor change).
        self.buffers.retain(|b| b.height() == height as i32 && b.stride() == stride);

        let idx = self.buffers.iter().position(|b| !b.slot().has_active_buffers());
        let buffer = match idx {
            Some(i) => &self.buffers[i],
            None if self.buffers.len() < 3 => {
                let created = self.state.pool.create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                );
                let (buffer, _canvas) = match created {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("overlay: create_buffer failed: {e}");
                        return false;
                    }
                };
                self.buffers.push(buffer);
                self.buffers.last().unwrap()
            }
            // compositor hasn't released any of our buffers yet -- drop
            // this frame rather than growing the pool further.
            None => return false,
        };

        let Some(canvas) = buffer.canvas(&mut self.state.pool) else {
            return false;
        };

        // tiny-skia stores premultiplied RGBA8 (R,G,B,A byte order);
        // wl_shm Argb8888 wants premultiplied little-endian 0xAARRGGBB, i.e.
        // byte order B,G,R,A.
        let src = pixmap.data();
        for (dst, src) in canvas.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if let Err(e) = buffer.attach_to(surface) {
            tracing::warn!("overlay: buffer attach failed: {e}");
            return false;
        }
        layer.commit();
        let _ = self.conn.flush();
        true
    }

    /// true while a layer surface is mapped
    pub fn is_shown(&self) -> bool {
        self.layer.is_some()
    }

    /// true once the connection has died (protocol error, compositor
    /// hangup) or the layer surface was closed by the compositor.
    pub fn is_closed(&self) -> bool {
        self.state.closed
    }

    /// hide the overlay by destroying the layer surface. Unmapping it with
    /// a null buffer instead resets its state on KWin, and a surface kept
    /// around would stay on the monitor it was created on.
    pub fn hide(&mut self) {
        if self.layer.is_some() {
            self.destroy_layer();
        }
    }
}
