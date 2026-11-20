//! wlr-layer-shell overlay surface + wl_shm software rendering.
//!
//! Creates a fullscreen, click-through `Layer::Overlay` surface (empty input
//! region, `KeyboardInteractivity::None`) and renders into it with tiny-skia
//! into a wl_shm buffer, committing a new buffer each frame.
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
    layer: LayerSurface,
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
    visible: bool,
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
        let (globals, queue) = wayland_client::globals::registry_queue_init(&conn)
            .map_err(|e| format!("wayland registry init failed: {e}"))?;
        let qh = queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|e| format!("wlr-layer-shell not available (compositor isn't wlroots-based, or doesn't support it): {e}"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;

        let surface = compositor.create_surface(&qh);

        // click-through: empty input region so pointer/keyboard events pass
        // straight through to whatever's underneath.
        if let Ok(region) = Region::new(&compositor) {
            surface.set_input_region(Some(region.wl_region()));
        }

        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("hebnix-overlay"),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0); // 0,0 with all anchors = compositor picks full output size
        layer.commit();

        let pool = SlotPool::new(1920 * 1080 * 4, &shm).map_err(|e| format!("shm pool: {e}"))?;

        let state = State {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            shm,
            layer,
            pool,
            width: 0,
            height: 0,
            configured: false,
            closed: false,
        };

        Ok(WaylandOverlay {
            conn,
            queue,
            qh,
            state,
            visible: false,
            buffers: Vec::new(),
        })
    }

    /// pump pending wayland events (non-blocking) and return the current
    /// surface size, or None if not configured yet or the layer got closed.
    pub fn poll_size(&mut self) -> Option<(u32, u32)> {
        let _ = self.conn.flush();
        if !self.state.configured {
            // re-mapping after hide(): wlr-layer-shell wants a commit with
            // no buffer, then a fresh configure, before a buffer may be
            // attached again.
            if !self.visible {
                self.state.layer.set_size(0, 0);
                self.state.layer.commit();
            }
            let _ = self.queue.roundtrip(&mut self.state);
        } else {
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
            // fd-bearing messages on the socket. This compositor still
            // reports a recurring (so far harmless-looking, but treated as
            // fatal below so we recover instead of assuming) EPROTO on our
            // layer surface even with the readiness check in place; the
            // error surfaces through `read()`'s Result, not
            // `dispatch_pending`'s, so both must be checked -- missing the
            // `read()` side here previously meant the connection silently
            // stayed "alive" from this function's point of view long after
            // it had actually died, and the auto-recreate path never ran.
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
                // etc). Stop hammering it every tick -- mark closed so
                // `Overlay` (mod.rs) knows to tear down and recreate the
                // backend instead of retrying a broken connection forever.
                self.state.closed = true;
            }
        }
        if self.state.closed {
            return None;
        }
        if !self.state.configured || self.state.width == 0 || self.state.height == 0 {
            return None;
        }
        Some((self.state.width, self.state.height))
    }

    /// paint `pixmap` (must match the current surface size) into an shm
    /// buffer and commit it. Reuses a buffer the compositor has already
    /// released rather than allocating a fresh one every call.
    pub fn present(&mut self, pixmap: &tiny_skia::Pixmap) {
        // if not configured or zero size, try to get size first
        if !self.state.configured || self.state.width == 0 || self.state.height == 0 {
            let _ = self.poll_size();
        }
        let width = self.state.width;
        let height = self.state.height;
        if width == 0 || height == 0 {
            return;
        }
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
                        return;
                    }
                };
                self.buffers.push(buffer);
                self.buffers.last().unwrap()
            }
            // compositor hasn't released any of our buffers yet -- drop
            // this frame rather than growing the pool further.
            None => return,
        };

        let Some(canvas) = buffer.canvas(&mut self.state.pool) else {
            return;
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

        let surface = self.state.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if let Err(e) = buffer.attach_to(surface) {
            tracing::warn!("overlay: buffer attach failed: {e}");
            return;
        }
        self.state.layer.commit();
        self.visible = true;
        let _ = self.conn.flush();
    }

    /// true once the connection has died (protocol error, compositor
    /// hangup) or the layer surface was closed by the compositor.
    pub fn is_closed(&self) -> bool {
        self.state.closed
    }

    /// hide the surface
    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        let surface = self.state.layer.wl_surface();
        surface.attach(None, 0, 0);
        surface.commit();
        self.visible = false;
        // a null buffer unmaps the layer surface; attaching a buffer again
        // before the compositor's next configure is a protocol error (KWin
        // enforces it, Hyprland lets it slide), so poll_size() must re-map first
        self.state.configured = false;
        let _ = self.conn.flush();
    }
}
