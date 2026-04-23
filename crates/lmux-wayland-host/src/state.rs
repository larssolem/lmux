//! Smithay delegation state. Owns the global protocol state (compositor,
//! shm, xdg_shell, seat) and a map of live surfaces so the cockpit can
//! address them by a stable [`SurfaceId`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::Sender;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc, Modifier};
use smithay::backend::input::{Axis, AxisSource, ButtonState, KeyState, Keycode};
use smithay::delegate_compositor;
use smithay::delegate_cursor_shape;
use smithay::delegate_data_device;
use smithay::delegate_dmabuf;
use smithay::delegate_output;
use smithay::delegate_seat;
use smithay::delegate_shm;
use smithay::delegate_viewporter;
use smithay::delegate_xdg_shell;
use smithay::input::keyboard::{FilterResult, KeyboardHandle};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent, PointerHandle,
};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle};
use smithay::utils::{Serial, SERIAL_COUNTER};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    with_states, BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
    SurfaceAttributes, SurfaceData,
};
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::{
    get_dmabuf, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{with_buffer_contents, BufferAccessError, ShmHandler, ShmState};
use smithay::wayland::viewporter::ViewporterState;

use crate::{DmabufFrame, HostEvent};

/// Stable per-toplevel identifier. Monotonic within a host thread; does
/// not persist across cockpit restarts.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SurfaceId(pub u64);

impl std::fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sfc-{}", self.0)
    }
}

/// Per-toplevel bookkeeping: the surface handle plus the last title /
/// app_id we reported to the cockpit (so we can fire a TitleChanged /
/// AppIdChanged event on the next commit that mutates them).
pub(crate) struct TrackedToplevel {
    pub(crate) surface: ToplevelSurface,
    pub(crate) announced: bool,
    pub(crate) last_title: Option<String>,
    pub(crate) last_app_id: Option<String>,
    /// Set on first-commit when `surface.parent()` resolves to another
    /// known toplevel. Child toplevels render as overlay widgets on top of
    /// their parent's pane (same infra as xdg_popup) rather than being
    /// spliced into the layout — matches IntelliJ/JetBrains modal-dialog
    /// semantics where "Open File" lives on top of the welcome window.
    pub(crate) child_of: Option<SurfaceId>,
}

/// Per-popup bookkeeping. Popups hang off a parent surface (usually a
/// toplevel, sometimes another popup) and carry their own positioner-
/// resolved geometry. We mirror `TrackedToplevel.announced` so the first
/// commit is what fires `HostEvent::PopupCreated`.
pub(crate) struct TrackedPopup {
    pub(crate) surface: PopupSurface,
    pub(crate) parent: SurfaceId,
    /// Positioner-resolved rectangle in the parent surface's window-geometry
    /// coordinate space (x/y = offset from parent's top-left; w/h = popup
    /// size). Stashed at `new_popup` so the cockpit can place the overlay
    /// before the first frame arrives.
    pub(crate) geometry: (i32, i32, u32, u32),
    pub(crate) announced: bool,
}

/// Smithay state container. Every protocol handler hangs off this struct.
pub struct State {
    #[allow(dead_code)]
    pub(crate) display: DisplayHandle,
    pub(crate) compositor_state: CompositorState,
    pub(crate) shm_state: ShmState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) seat_state: SeatState<Self>,
    #[allow(dead_code)]
    pub(crate) seat: Seat<Self>,
    pub(crate) pointer: PointerHandle<Self>,
    pub(crate) keyboard: KeyboardHandle<Self>,
    #[allow(dead_code)]
    pub(crate) output: Output,
    pub(crate) data_device_state: DataDeviceState,
    #[allow(dead_code)]
    pub(crate) viewporter_state: ViewporterState,
    pub(crate) dmabuf_state: DmabufState,
    #[allow(dead_code)]
    pub(crate) dmabuf_global: DmabufGlobal,
    #[allow(dead_code)]
    pub(crate) cursor_shape_state: CursorShapeManagerState,
    pub(crate) event_tx: Sender<HostEvent>,
    pub(crate) toplevels: HashMap<SurfaceId, TrackedToplevel>,
    pub(crate) popups: HashMap<SurfaceId, TrackedPopup>,
    /// Surface a client has nominated as its cursor image via the legacy
    /// `wl_pointer.set_cursor(surface, hotspot_x, hotspot_y)` request.
    /// On the next commit of this surface we extract its buffer and emit
    /// `HostEvent::CursorBitmap` so the cockpit can build a GTK custom
    /// cursor from the pixels — JetBrains/AWT-Wayland uses this path for
    /// resize/I-beam cursors instead of wp_cursor_shape_device_v1.
    pub(crate) cursor_surface: Option<WlSurface>,
    /// SurfaceId that currently holds pointer focus, captured at the time
    /// the client set its cursor. Sent back with `CursorShape` /
    /// `CursorBitmap` so the cockpit scopes the cursor change to the
    /// satellite that requested it.
    pub(crate) cursor_focus: Option<SurfaceId>,
    /// Mirror of the smithay pointer's current focus surface, updated in
    /// `SeatHandler::focus_changed`. Used by `cursor_image` instead of
    /// `seat.get_pointer().current_focus()`, which deadlocks: smithay
    /// holds the pointer mutex while invoking `cursor_image` from inside
    /// `pointer.motion()`, and `current_focus()` re-acquires the same
    /// mutex. Caching here avoids the recursive lock.
    pub(crate) pointer_focus_surface: Option<WlSurface>,
    next_surface_id: AtomicU64,
    start_time: std::time::Instant,
}

impl State {
    pub(crate) fn new(display: DisplayHandle, event_tx: Sender<HostEvent>) -> Self {
        let compositor_state = CompositorState::new::<Self>(&display);
        let shm_state = ShmState::new::<Self>(&display, vec![]);
        let xdg_shell_state = XdgShellState::new::<Self>(&display);
        let mut seat_state = SeatState::<Self>::new();
        // Advertise one seat named "lmux-cockpit". It gets a wl_seat
        // global so clients discover pointer + keyboard caps.
        let mut seat = seat_state.new_wl_seat(&display, "lmux-cockpit");
        let pointer = seat.add_pointer();
        // 600 ms repeat delay, 25 Hz rate matches GNOME/KDE defaults.
        // Default XkbConfig inherits the host's locale via xkbcommon
        // which is the right thing for almost every satellite.
        #[allow(clippy::expect_used)]
        let keyboard = seat
            .add_keyboard(Default::default(), 600, 25)
            .expect("seat.add_keyboard must succeed with default XkbConfig");
        // Advertise a single virtual output. Chromium/Electron/IntelliJ
        // refuse to render if no wl_output is visible — they need geometry
        // and scale to initialize. The mode is nominal; satellites get
        // resized via xdg_toplevel.configure from the pane slot anyway.
        let output = Output::new(
            "lmux-0".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "lmux".into(),
                model: "nested".into(),
            },
        );
        let mode = OutputMode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        output.create_global::<Self>(&display);
        // Advertise wl_data_device_manager. IntelliJ / JetBrains Runtime's
        // AWT Wayland toolkit hard-fails with
        // `Can't bind to the wl_data_device_manager interface` if this
        // global is missing. We keep the handlers as no-ops for now —
        // cross-satellite clipboard / DnD is v0.3.
        let data_device_state = DataDeviceState::new::<Self>(&display);
        // IntelliJ's AWT Wayland toolkit also requires wp_viewporter; without
        // it we get `Can't bind to the wp_viewporter interface`. Satellites
        // themselves manage viewporter state — we just need the global
        // advertised so the bind succeeds.
        let viewporter_state = ViewporterState::new::<Self>(&display);
        // Advertise linux-dmabuf. Chromium / Electron / Qt will prefer this
        // zero-copy path over wl_shm when available — clients hand us GPU
        // buffers by fd and we import them straight into a
        // `gdk::DmabufTexture` without any CPU memcpy. We only advertise
        // linear-modifier single-plane RGB formats for now; tiled modifiers
        // and YUV ride v0.3 (require matching GTK/GSK support).
        let mut dmabuf_state = DmabufState::new();
        let formats: Vec<smithay::backend::allocator::Format> = [
            Fourcc::Argb8888,
            Fourcc::Xrgb8888,
            Fourcc::Abgr8888,
            Fourcc::Xbgr8888,
        ]
        .into_iter()
        .flat_map(|fourcc| {
            [Modifier::Linear, Modifier::Invalid]
                .into_iter()
                .map(move |modifier| smithay::backend::allocator::Format {
                    code: fourcc,
                    modifier,
                })
        })
        .collect();
        let dmabuf_global = dmabuf_state.create_global::<Self>(&display, formats);
        // Advertise wp_cursor_shape_manager_v1 so clients (Chromium, GTK,
        // Qt, JetBrains) can request named cursor shapes. The shape arrives
        // via `SeatHandler::cursor_image` as `CursorImageStatus::Named` —
        // we forward the name to the cockpit, which applies it to the
        // satellite widget so the OS cursor actually changes.
        let cursor_shape_state = CursorShapeManagerState::new::<Self>(&display);
        Self {
            display,
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            seat,
            pointer,
            keyboard,
            output,
            data_device_state,
            viewporter_state,
            dmabuf_state,
            dmabuf_global,
            cursor_shape_state,
            event_tx,
            toplevels: HashMap::new(),
            popups: HashMap::new(),
            cursor_surface: None,
            cursor_focus: None,
            pointer_focus_surface: None,
            next_surface_id: AtomicU64::new(1),
            start_time: std::time::Instant::now(),
        }
    }

    fn now_ms(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }

    fn surface_for(&self, id: SurfaceId) -> Option<WlSurface> {
        if let Some(tt) = self.toplevels.get(&id) {
            return Some(tt.surface.wl_surface().clone());
        }
        if let Some(pp) = self.popups.get(&id) {
            return Some(pp.surface.wl_surface().clone());
        }
        None
    }

    /// Find the `SurfaceId` associated with a parent WlSurface. Called by
    /// `new_popup` to resolve the popup's parent into a stable id the
    /// cockpit already knows. Popups may nest (popup-of-popup, e.g. a
    /// submenu inside a menu), so we check both toplevels and other
    /// popups.
    fn surface_id_for_wl(&self, wl: &WlSurface) -> Option<SurfaceId> {
        for (id, tt) in &self.toplevels {
            if tt.surface.wl_surface() == wl {
                return Some(*id);
            }
        }
        for (id, pp) in &self.popups {
            if pp.surface.wl_surface() == wl {
                return Some(*id);
            }
        }
        None
    }

    pub(crate) fn reap_dead_popups(&mut self) {
        let dead: Vec<SurfaceId> = self
            .popups
            .iter()
            .filter(|(_, pp)| !pp.surface.alive())
            .map(|(id, _)| *id)
            .collect();
        for id in dead {
            tracing::info!(%id, "xdg_shell: reaping dead popup");
            self.popups.remove(&id);
            self.emit(HostEvent::PopupClosed { id });
        }
    }

    fn allocate_surface_id(&self) -> SurfaceId {
        SurfaceId(self.next_surface_id.fetch_add(1, Ordering::SeqCst))
    }

    fn emit(&self, event: HostEvent) {
        if let Err(err) = self.event_tx.send_blocking(event) {
            tracing::warn!(error = %err, "state: failed to post HostEvent");
        }
    }

    /// Poll every tracked toplevel and evict the ones whose client has
    /// disconnected. smithay's `XdgShellHandler::toplevel_destroyed` is
    /// only called when the client *cleanly* destroys the xdg_toplevel
    /// resource; hard disconnects (process crashed, socket closed mid-
    /// frame) tear the surface down via `wl_display.delete_id` without
    /// running the xdg-shell destructor. Without this reaper the cockpit
    /// keeps drawing a stale pane ("white window") because it never
    /// learns the client is gone. Called every host dispatch cycle.
    pub(crate) fn reap_dead_toplevels(&mut self) {
        let dead: Vec<(SurfaceId, Option<SurfaceId>)> = self
            .toplevels
            .iter()
            .filter(|(_, tt)| !tt.surface.alive())
            .map(|(id, tt)| (*id, tt.child_of))
            .collect();
        for (id, child_of) in dead {
            tracing::info!(%id, "xdg_shell: reaping dead toplevel");
            self.toplevels.remove(&id);
            if child_of.is_some() {
                self.emit(HostEvent::ChildToplevelClosed { id });
            } else {
                self.emit(HostEvent::ToplevelClosed { id });
            }
        }
    }

    /// Push a new size to a tracked toplevel via `xdg_toplevel.configure`.
    /// No-op if the id has already been destroyed.
    pub(crate) fn resize_toplevel(&mut self, id: SurfaceId, width: u32, height: u32) {
        let Some(tt) = self.toplevels.get(&id) else {
            return;
        };
        let size = smithay::utils::Size::from((width as i32, height as i32));
        tt.surface.with_pending_state(|st| {
            st.size = Some(size);
        });
        tt.surface.send_configure();
    }

    /// Send `xdg_toplevel.close`. The client is expected to tear down and
    /// destroy the toplevel, which eventually fires `ToplevelClosed`.
    pub(crate) fn close_toplevel(&mut self, id: SurfaceId) {
        let Some(tt) = self.toplevels.get(&id) else {
            return;
        };
        tt.surface.send_close();
    }

    pub(crate) fn pointer_motion(&mut self, id: SurfaceId, x: f64, y: f64) {
        let Some(surface) = self.surface_for(id) else {
            return;
        };
        let time = self.now_ms();
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.pointer.clone();
        let event = MotionEvent {
            location: (x, y).into(),
            serial,
            time,
        };
        // Origin (0, 0) since we treat each satellite as if its
        // surface occupies the whole compositor space — there's no
        // global layout to worry about.
        pointer.motion(self, Some((surface, (0.0, 0.0).into())), &event);
        pointer.frame(self);
    }

    pub(crate) fn pointer_leave(&mut self, _id: SurfaceId) {
        let time = self.now_ms();
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location: (0.0, 0.0).into(),
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    pub(crate) fn pointer_button(&mut self, _id: SurfaceId, button: u32, pressed: bool) {
        let time = self.now_ms();
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.pointer.clone();
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        pointer.button(
            self,
            &ButtonEvent {
                serial,
                time,
                button,
                state,
            },
        );
        pointer.frame(self);
    }

    pub(crate) fn pointer_axis(&mut self, _id: SurfaceId, dx: f64, dy: f64) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let time = self.now_ms();
        let pointer = self.pointer.clone();
        // 15px per notch is the libinput heuristic — matches what most
        // mutter/kwin builds pass through for a single wheel click.
        const STEP: f64 = 15.0;
        let frame = AxisFrame::new(time)
            .source(AxisSource::Wheel)
            .value(Axis::Horizontal, dx * STEP)
            .value(Axis::Vertical, dy * STEP);
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    pub(crate) fn key_input(&mut self, _id: SurfaceId, evdev_code: u32, pressed: bool) {
        let time = self.now_ms();
        let serial: Serial = SERIAL_COUNTER.next_serial();
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        // smithay/xkb keycodes are evdev + 8.
        let keycode = Keycode::from(evdev_code + 8);
        let keyboard = self.keyboard.clone();
        keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
            FilterResult::Forward
        });
    }

    pub(crate) fn keyboard_focus(&mut self, id: Option<SurfaceId>) {
        let target = id.and_then(|sid| self.surface_for(sid));
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, target, serial);
    }
}

/// Per-client compositor state. Smithay requires one of these per client
/// so its compositor handler can look up per-client bookkeeping.
#[derive(Default)]
pub(crate) struct ClientCompositorState {
    pub(crate) compositor: CompositorClientState,
}

impl smithay::reexports::wayland_server::backend::ClientData for ClientCompositorState {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}

// --- CompositorHandler -----------------------------------------------------

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    #[allow(clippy::expect_used)]
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientCompositorState>()
            .expect("client missing ClientCompositorState — insert_client passed the wrong data")
            .compositor
    }

    fn commit(&mut self, surface: &WlSurface) {
        // Cursor surfaces commit out-of-band relative to toplevel/popup
        // surfaces. Catch them first — the surface has no xdg role, so
        // the regular toplevel/popup matching below would silently drop
        // the buffer (and with it, JetBrains' resize cursor).
        if self.cursor_surface.as_ref() == Some(surface) {
            let hotspot = with_states(surface, |data: &SurfaceData| {
                data.data_map
                    .get::<smithay::input::pointer::CursorImageSurfaceData>()
                    .and_then(|m| m.lock().ok().map(|a| a.hotspot))
                    .unwrap_or_default()
            });
            let surface_id = self.cursor_focus;
            // Cursor surfaces are RGBA — most cursor pixels are
            // transparent (the bit *around* the arrow). Stripping alpha
            // would render the transparent pixels as opaque black,
            // giving the user a black-rectangle cursor. Read RGBA
            // (premultiplied per wayland) and ship that to GTK.
            match read_committed_cursor(surface) {
                Some(bitmap) => {
                    tracing::debug!(
                        ?surface_id,
                        w = bitmap.width,
                        h = bitmap.height,
                        hx = hotspot.x,
                        hy = hotspot.y,
                        "cursor_surface: shm bitmap committed"
                    );
                    let _ = self.event_tx.send_blocking(HostEvent::CursorBitmap {
                        surface_id,
                        width: bitmap.width,
                        height: bitmap.height,
                        rgba: bitmap.rgba,
                        hotspot_x: hotspot.x,
                        hotspot_y: hotspot.y,
                    });
                }
                None => {
                    tracing::debug!(?surface_id, "cursor_surface: commit without usable buffer");
                }
            }
            return;
        }

        // Route popups first — they have their own bookkeeping (geometry,
        // parent) and fire dedicated `PopupCreated` / `PopupClosed` events
        // so the cockpit can overlay them on the parent pane.
        let popup_id = self
            .popups
            .iter()
            .find_map(|(id, pp)| (pp.surface.wl_surface() == surface).then_some(*id));
        if let Some(id) = popup_id {
            let mut announced_now = false;
            if let Some(pp) = self.popups.get_mut(&id) {
                if !pp.announced {
                    pp.announced = true;
                    announced_now = true;
                    let (x, y, width, height) = pp.geometry;
                    let parent = pp.parent;
                    tracing::info!(%id, %parent, x, y, w = width, h = height,
                        "xdg_shell: popup first commit → PopupCreated");
                    let _ = self.event_tx.send_blocking(HostEvent::PopupCreated {
                        id,
                        parent,
                        x,
                        y,
                        width,
                        height,
                    });
                }
            }
            // Drain the committed frame the same way as toplevels.
            match read_committed_frame(id, surface) {
                Some(CommittedFrame::Shm(FrameCopy { width, height, rgb })) => {
                    tracing::debug!(%id, w = width, h = height, "xdg_shell: popup shm frame");
                    let _ = self.event_tx.send_blocking(HostEvent::FrameReady {
                        id,
                        width,
                        height,
                        rgb,
                    });
                }
                Some(CommittedFrame::Dmabuf(frame)) => {
                    tracing::debug!(%id, "xdg_shell: popup dmabuf frame");
                    let _ = self.event_tx.send_blocking(HostEvent::DmabufFrame(frame));
                }
                None => {
                    if !announced_now {
                        tracing::debug!(%id, "xdg_shell: popup commit with no buffer");
                    }
                }
            }
            return;
        }

        // Commit is the earliest point at which the client has finished
        // sending set_title / set_app_id, so we defer the
        // ToplevelCreated event until the first commit for a tracked
        // toplevel. Subsequent commits fire a ToplevelTitleChanged /
        // ToplevelAppIdChanged if the strings actually changed.
        let matched = self
            .toplevels
            .iter()
            .find_map(|(id, tt)| (tt.surface.wl_surface() == surface).then_some(*id));
        let Some(id) = matched else {
            return;
        };

        let (title, app_id) = read_toplevel_identity_from_data(surface);

        // Resolve parent at first commit; done once because xdg_toplevel
        // set_parent is a sticky property. Needs to happen before we
        // borrow `tracked` mutably (parent lookup borrows `self` too).
        let first_commit_parent = {
            let tracked = match self.toplevels.get(&id) {
                Some(t) => t,
                None => return,
            };
            if tracked.announced || tracked.child_of.is_some() {
                None
            } else {
                tracked
                    .surface
                    .parent()
                    .and_then(|wl| self.surface_id_for_wl(&wl))
            }
        };
        if let Some(parent_id) = first_commit_parent {
            if let Some(t) = self.toplevels.get_mut(&id) {
                t.child_of = Some(parent_id);
            }
            tracing::info!(%id, %parent_id, "xdg_shell: toplevel is child of another toplevel");
        }

        // Drain the committed buffer up-front so we have frame dimensions
        // available when we decide which creation event to emit.
        let committed = read_committed_frame(id, surface);

        let Some(tracked) = self.toplevels.get_mut(&id) else {
            return;
        };
        let child_of = tracked.child_of;

        let first_commit = !tracked.announced;
        if first_commit {
            if let Some(parent_id) = child_of {
                // Defer announcing until we have a frame — need pixel
                // dimensions to position the overlay.
                let (w, h) = match &committed {
                    Some(CommittedFrame::Shm(FrameCopy { width, height, .. })) => (*width, *height),
                    Some(CommittedFrame::Dmabuf(f)) => (f.width, f.height),
                    None => (0, 0),
                };
                if w > 0 && h > 0 {
                    tracked.announced = true;
                    tracked.last_title = title.clone();
                    tracked.last_app_id = app_id.clone();
                    let _ = self
                        .event_tx
                        .send_blocking(HostEvent::ChildToplevelCreated {
                            id,
                            parent: parent_id,
                            title,
                            app_id,
                            width: w,
                            height: h,
                        });
                }
            } else {
                tracked.announced = true;
                tracked.last_title = title.clone();
                tracked.last_app_id = app_id.clone();
                let _ =
                    self.event_tx
                        .send_blocking(HostEvent::ToplevelCreated { id, title, app_id });
            }
        } else {
            if title != tracked.last_title {
                tracked.last_title = title.clone();
                let _ = self
                    .event_tx
                    .send_blocking(HostEvent::ToplevelTitleChanged { id, title });
            }
            if app_id != tracked.last_app_id {
                tracked.last_app_id = app_id.clone();
                let _ = self
                    .event_tx
                    .send_blocking(HostEvent::ToplevelAppIdChanged { id, app_id });
            }
        }

        // Forward the frame (if any). For child toplevels the cockpit
        // routes these into the popup-overlay pipeline via `popup_to_pane`.
        match committed {
            Some(CommittedFrame::Shm(FrameCopy { width, height, rgb })) => {
                let _ = self.event_tx.send_blocking(HostEvent::FrameReady {
                    id,
                    width,
                    height,
                    rgb,
                });
            }
            Some(CommittedFrame::Dmabuf(frame)) => {
                let _ = self.event_tx.send_blocking(HostEvent::DmabufFrame(frame));
            }
            None => {}
        }
    }
}

/// Output of a single successful surface commit: either an RGB8 copy of
/// the shm buffer or a dmabuf fd the GTK side will import zero-copy.
enum CommittedFrame {
    Shm(FrameCopy),
    Dmabuf(DmabufFrame),
}

/// RGB8 copy of a freshly-committed shm buffer. Stride is `width * 3`.
struct FrameCopy {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

/// Pull the committed buffer off `surface` and turn it into either an
/// RGB8 copy (shm path) or a dmabuf-fd payload (GPU path). Also releases
/// the source `wl_buffer` and fires any pending `wl_callback` frame
/// callbacks so the client can schedule its next draw.
///
/// Returns `None` if this commit didn't attach a new buffer, the buffer
/// was detached, or the buffer format is one we can't consume.
fn read_committed_frame(id: SurfaceId, surface: &WlSurface) -> Option<CommittedFrame> {
    with_states(surface, |data: &SurfaceData| {
        let mut guard = data.cached_state.get::<SurfaceAttributes>();
        let attrs: &mut SurfaceAttributes = guard.current();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u32)
            .unwrap_or(0);
        let callbacks = std::mem::take(&mut attrs.frame_callbacks);
        for cb in callbacks {
            cb.done(now_ms);
        }

        // `take()` so we don't re-process the same buffer on a
        // subsequent commit that didn't actually attach anything
        // (the buffer field is aggregated from commit to commit).
        let assignment = attrs.buffer.take()?;
        let buffer = match assignment {
            BufferAssignment::NewBuffer(b) => b,
            BufferAssignment::Removed => return None,
        };

        // Fast path: dmabuf. If the client attached a GPU buffer we skip
        // the CPU readback entirely and hand the fd to GTK.
        let frame = if let Ok(dmabuf) = get_dmabuf(&buffer) {
            dmabuf_to_frame(id, dmabuf).map(CommittedFrame::Dmabuf)
        } else {
            match with_buffer_contents(&buffer, copy_shm_to_rgb) {
                Ok(opt) => opt.map(CommittedFrame::Shm),
                Err(BufferAccessError::NotManaged) => {
                    tracing::debug!("frame: non-shm, non-dmabuf buffer attached — ignoring");
                    None
                }
                Err(err) => {
                    tracing::warn!(error = %err, "frame: shm buffer access failed");
                    None
                }
            }
        };

        // Always release — even if we failed to copy — so the client
        // doesn't deadlock waiting for its buffer back.
        buffer.release();

        frame
    })
}

/// Turn a single-plane linear-modifier dmabuf into a `DmabufFrame` payload
/// we can ship over to the cockpit. Multi-plane / non-linear dmabufs are
/// rejected for now — GTK's `DmabufTextureBuilder` handles linear RGB
/// fine, and YUV / tiled modifiers need a renderer-side import pass we
/// haven't built yet.
fn dmabuf_to_frame(id: SurfaceId, dmabuf: &Dmabuf) -> Option<DmabufFrame> {
    if dmabuf.num_planes() != 1 {
        tracing::debug!(
            planes = dmabuf.num_planes(),
            "dmabuf: multi-plane buffer not supported yet — falling through"
        );
        return None;
    }
    let size = dmabuf.size();
    if size.w <= 0 || size.h <= 0 {
        return None;
    }
    let fourcc: u32 = dmabuf.format().code as u32;
    let modifier: u64 = Into::<u64>::into(dmabuf.format().modifier);
    let fd = dmabuf.handles().next()?.try_clone_to_owned().ok()?;
    let stride = dmabuf.strides().next()?;
    let offset = dmabuf.offsets().next()?;
    Some(DmabufFrame {
        id,
        width: size.w as u32,
        height: size.h as u32,
        fourcc,
        modifier,
        fd,
        stride,
        offset,
    })
}

/// RGBA bitmap pulled off a cursor surface commit. Stride is `width * 4`,
/// premultiplied alpha (matches wayland Argb8888 + GTK
/// `MemoryFormat::A8r8g8b8Premultiplied`'s expectations after channel
/// reorder).
struct CursorBitmapCopy {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Read the freshly-committed wl_shm buffer of a cursor surface and copy
/// it as RGBA (premultiplied). Releases the source `wl_buffer` and fires
/// any frame callbacks the same way `read_committed_frame` does for
/// toplevels — cursor surfaces still expect callback delivery.
///
/// Returns `None` if the surface has no buffer attached, the buffer is
/// dmabuf (cursor-via-dmabuf is rare and unsupported here), or the shm
/// format isn't ARGB/XRGB.
fn read_committed_cursor(surface: &WlSurface) -> Option<CursorBitmapCopy> {
    with_states(surface, |data: &SurfaceData| {
        let mut guard = data.cached_state.get::<SurfaceAttributes>();
        let attrs: &mut SurfaceAttributes = guard.current();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u32)
            .unwrap_or(0);
        let callbacks = std::mem::take(&mut attrs.frame_callbacks);
        for cb in callbacks {
            cb.done(now_ms);
        }

        let buffer = match attrs.buffer.take() {
            Some(BufferAssignment::NewBuffer(b)) => b,
            _ => return None,
        };

        let result = with_buffer_contents(&buffer, copy_shm_to_rgba);
        // Release the source buffer so the client can reuse it for the
        // next cursor frame.
        buffer.release();
        match result {
            Ok(opt) => opt,
            Err(BufferAccessError::NotManaged) => None,
            Err(err) => {
                tracing::debug!(?err, "cursor_surface: with_buffer_contents failed");
                None
            }
        }
    })
}

fn copy_shm_to_rgba(
    ptr: *const u8,
    len: usize,
    meta: smithay::wayland::shm::BufferData,
) -> Option<CursorBitmapCopy> {
    if meta.width <= 0 || meta.height <= 0 || meta.stride <= 0 {
        return None;
    }
    let width = meta.width as usize;
    let height = meta.height as usize;
    let stride = meta.stride as usize;
    let required = stride
        .checked_mul(height)
        .and_then(|v| v.checked_add(meta.offset.max(0) as usize))?;
    if required > len {
        return None;
    }
    let row_bytes = width.checked_mul(4)?;
    if row_bytes > stride {
        return None;
    }
    match meta.format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => {}
        other => {
            tracing::debug!(?other, "cursor: unsupported shm format");
            return None;
        }
    }
    let xrgb = matches!(meta.format, wl_shm::Format::Xrgb8888);
    let mut rgba = Vec::with_capacity(width * height * 4);
    let base = unsafe { ptr.add(meta.offset.max(0) as usize) };
    for y in 0..height {
        let row = unsafe { base.add(y * stride) };
        for x in 0..width {
            let px = unsafe { row.add(x * 4) };
            let b = unsafe { *px };
            let g = unsafe { *px.add(1) };
            let r = unsafe { *px.add(2) };
            // XRGB8888: alpha byte is meaningless padding — force opaque
            // so we don't accidentally emit garbage transparency.
            let a = if xrgb { 0xFF } else { unsafe { *px.add(3) } };
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }
    }
    Some(CursorBitmapCopy {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

/// Convert a wl_shm mapping into a tightly-packed RGB8 Vec. Supports the
/// two protocol-mandatory formats (Argb8888, Xrgb8888) which in wayland
/// are little-endian — i.e. the bytes in memory are `B, G, R, A/X`.
/// Any other format returns None; follow-up work can extend this as
/// real satellite clients surface new formats.
///
/// Safety: `ptr` is valid for `len` bytes per smithay's shm contract
/// while the `with_buffer_contents` closure runs.
fn copy_shm_to_rgb(
    ptr: *const u8,
    len: usize,
    meta: smithay::wayland::shm::BufferData,
) -> Option<FrameCopy> {
    if meta.width <= 0 || meta.height <= 0 || meta.stride <= 0 {
        return None;
    }
    let width = meta.width as usize;
    let height = meta.height as usize;
    let stride = meta.stride as usize;

    let required = stride
        .checked_mul(height)
        .and_then(|v| v.checked_add(meta.offset.max(0) as usize))?;
    if required > len {
        tracing::warn!(
            width,
            height,
            stride,
            offset = meta.offset,
            len,
            "frame: shm buffer advertises more pixels than its mmap holds",
        );
        return None;
    }

    let row_bytes = width.checked_mul(4)?;
    if row_bytes > stride {
        return None;
    }

    match meta.format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => {}
        other => {
            tracing::debug!(?other, "frame: unsupported shm format — ignoring");
            return None;
        }
    }

    let mut rgb = Vec::with_capacity(width * height * 3);
    let base = unsafe { ptr.add(meta.offset.max(0) as usize) };
    for y in 0..height {
        let row = unsafe { base.add(y * stride) };
        for x in 0..width {
            let px = unsafe { row.add(x * 4) };
            // Little-endian ARGB/XRGB → memory order B,G,R,A.
            let b = unsafe { *px };
            let g = unsafe { *px.add(1) };
            let r = unsafe { *px.add(2) };
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }
    }

    Some(FrameCopy {
        width: width as u32,
        height: height as u32,
        rgb,
    })
}

// --- BufferHandler ---------------------------------------------------------

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {
        // Task #9 will track pending frames; at MVP we hold no buffer refs.
    }
}

// --- ShmHandler ------------------------------------------------------------

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

// --- OutputHandler ---------------------------------------------------------

impl OutputHandler for State {}
delegate_output!(State);

// --- DataDevice (clipboard / DnD stubs) ------------------------------------
// The global itself has to be advertised for IntelliJ's AWT Wayland toolkit
// to even initialize. Actual selection / DnD forwarding between satellites
// is deferred to v0.3.

impl SelectionHandler for State {
    type SelectionUserData = ();
}
impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}
impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}
delegate_data_device!(State);

// --- Viewporter ------------------------------------------------------------

delegate_viewporter!(State);

// --- Dmabuf ----------------------------------------------------------------

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        // We don't have a GPU-side import probe (no EGL/Vulkan context in
        // the host thread). Accepting unconditionally is fine because the
        // GTK side validates at `gdk::DmabufTextureBuilder::build` time —
        // if the import fails there we drop the frame. Refusing here
        // would just force the client into the wl_shm fallback.
        let _ = notifier.successful::<State>();
    }
}
delegate_dmabuf!(State);

// --- SeatHandler (stub, populated by Task #11) -----------------------------

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, focused: Option<&WlSurface>) {
        tracing::trace!(
            focused_some = focused.is_some(),
            "SeatHandler::focus_changed",
        );
        // Cache focus here (cheap, no smithay locks involved) so
        // `cursor_image` can read it without re-locking the pointer
        // mutex that smithay already holds when it invokes us.
        self.pointer_focus_surface = focused.cloned();
        // Task #11 will route focus changes back into GTK.
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        // Use the cached focus instead of `seat.get_pointer().current_focus()`.
        // The latter deadlocks when smithay invokes us from inside
        // `pointer.motion()` — it already holds the pointer mutex that
        // current_focus() needs.
        let surface_id = self
            .pointer_focus_surface
            .as_ref()
            .and_then(|wl| self.surface_id_for_wl(wl));
        self.cursor_focus = surface_id;
        match image {
            CursorImageStatus::Named(icon) => {
                self.cursor_surface = None;
                tracing::debug!(?surface_id, shape = icon.name(), "cursor_image: named");
                self.emit(HostEvent::CursorShape {
                    surface_id,
                    name: icon.name().to_string(),
                });
            }
            CursorImageStatus::Hidden => {
                self.cursor_surface = None;
                tracing::debug!(?surface_id, "cursor_image: hidden");
                self.emit(HostEvent::CursorShape {
                    surface_id,
                    name: "none".to_string(),
                });
            }
            CursorImageStatus::Surface(wl) => {
                tracing::debug!(?surface_id, "cursor_image: surface — awaiting commit");
                self.cursor_surface = Some(wl);
                // The actual pixels arrive on the next commit of this
                // surface; commit() handles the extraction + emit.
            }
        }
    }
}

impl smithay::wayland::tablet_manager::TabletSeatHandler for State {}

delegate_cursor_shape!(State);

// --- XdgShellHandler -------------------------------------------------------

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let id = self.allocate_surface_id();
        tracing::info!(%id, "xdg_shell: new_toplevel");

        // Initial configure: let the client pick its own size. Task #10
        // upgrades this to pushing the pane slot's measured geometry.
        surface.with_pending_state(|st| {
            st.size = None;
        });
        surface.send_configure();

        self.toplevels.insert(
            id,
            TrackedToplevel {
                surface,
                announced: false,
                last_title: None,
                last_app_id: None,
                child_of: None,
            },
        );
        // Event emission deferred to the first commit — see
        // CompositorHandler::commit — so that title / app_id set by the
        // client between get_toplevel and commit make it into the event.
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        let id = self.allocate_surface_id();
        // Resolve the parent surface → the cockpit-known SurfaceId. Popups
        // without a mapped parent are spec-legal (e.g. some tooltips) but
        // rare; drop them so the cockpit isn't forced to invent a parent.
        let parent_wl = match surface.get_parent_surface() {
            Some(s) => s,
            None => {
                tracing::warn!(%id, "xdg_shell: new_popup without parent surface; dropping");
                return;
            }
        };
        let parent = match self.surface_id_for_wl(&parent_wl) {
            Some(p) => p,
            None => {
                tracing::warn!(%id, "xdg_shell: new_popup parent unknown; dropping");
                return;
            }
        };
        // Resolve positioner geometry into parent-surface coords. For
        // anchor-less positioners this collapses to the anchor-rect origin
        // plus offset, which is what IntelliJ + browser menus already give us.
        let geo = positioner.get_geometry();
        let width = geo.size.w.max(1) as u32;
        let height = geo.size.h.max(1) as u32;
        let geometry = (geo.loc.x, geo.loc.y, width, height);
        tracing::info!(%id, %parent, x = geo.loc.x, y = geo.loc.y, w = width, h = height,
            "xdg_shell: new_popup");

        // Configure the popup with the resolved geometry so it can map.
        surface.with_pending_state(|st| {
            st.geometry = geo;
            st.positioner = positioner;
        });
        if let Err(err) = surface.send_configure() {
            tracing::warn!(%id, error = %err, "xdg_shell: popup configure failed");
            return;
        }

        self.popups.insert(
            id,
            TrackedPopup {
                surface,
                parent,
                geometry,
                announced: false,
            },
        );
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let Some(id) = self.surface_id_for_wl(surface.wl_surface()) else {
            return;
        };
        let geo = positioner.get_geometry();
        let width = geo.size.w.max(1) as u32;
        let height = geo.size.h.max(1) as u32;
        let geometry = (geo.loc.x, geo.loc.y, width, height);
        surface.with_pending_state(|st| {
            st.geometry = geo;
            st.positioner = positioner;
        });
        // `send_repositioned` rides `send_configure` internally for v3+
        // clients, but the simple `send_configure` works fine here since
        // IntelliJ/Chrome redraw on the next ack either way.
        let _ = surface.send_configure();
        if let Some(pp) = self.popups.get_mut(&id) {
            pp.geometry = geometry;
        }
        self.emit(HostEvent::PopupRepositioned {
            id,
            x: geometry.0,
            y: geometry.1,
            width,
            height,
            token,
        });
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        let found = self
            .popups
            .iter()
            .find(|(_, pp)| pp.surface.wl_surface() == surface.wl_surface())
            .map(|(id, _)| *id);
        if let Some(id) = found {
            tracing::info!(%id, "xdg_shell: popup_destroyed");
            self.popups.remove(&id);
            self.emit(HostEvent::PopupClosed { id });
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let found = self
            .toplevels
            .iter()
            .find(|(_, tt)| tt.surface.wl_surface() == surface.wl_surface())
            .map(|(id, tt)| (*id, tt.child_of));
        if let Some((id, child_of)) = found {
            tracing::info!(%id, "xdg_shell: toplevel_destroyed");
            self.toplevels.remove(&id);
            if child_of.is_some() {
                self.emit(HostEvent::ChildToplevelClosed { id });
            } else {
                self.emit(HostEvent::ToplevelClosed { id });
            }
        }
    }
}

/// Read `title` and `app_id` from a toplevel surface's role data.
#[allow(dead_code)]
fn read_toplevel_identity(surface: &ToplevelSurface) -> (Option<String>, Option<String>) {
    read_toplevel_identity_from_data(surface.wl_surface())
}

fn read_toplevel_identity_from_data(wl: &WlSurface) -> (Option<String>, Option<String>) {
    with_states(wl, |data: &SurfaceData| {
        let Some(role) = data.data_map.get::<XdgToplevelSurfaceData>() else {
            return (None, None);
        };
        let Ok(guard) = role.lock() else {
            return (None, None);
        };
        let title: Option<String> = guard.title.clone().filter(|s: &String| !s.is_empty());
        let app_id: Option<String> = guard.app_id.clone().filter(|s: &String| !s.is_empty());
        (title, app_id)
    })
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_xdg_shell!(State);
