//! Nested Wayland compositor hosted inside the lmux cockpit (ADR-0018).
//!
//! Each GUI satellite the user launches from Ctrl+B l is a Wayland client
//! that connects to *this* compositor (via `WAYLAND_DISPLAY=lmux-<pid>`);
//! its surfaces land as GTK widgets in the cockpit's pane tree, treated
//! identically to terminal panes for focus, workspace, and anchor-switch
//! semantics.
//!
//! This crate is MVP scaffolding (Task #7 in the v0.2 nested-compositor
//! plan): it binds the socket, spawns the compositor event loop on its
//! own OS thread, and advertises the minimal globals (`wl_compositor` +
//! `wl_shm`) so a wayland-client probe can connect + list the registry.
//!
//! xdg_shell handlers, wl_shm → RGB pipeline, input routing, and GTK-side
//! integration all ship as follow-up tasks against this type's public
//! command/event channels.
//!
//! # Thread model
//!
//! * One dedicated OS thread owns the `calloop::EventLoop` and drives the
//!   smithay [`Display`]. All protocol dispatch happens there.
//! * The cockpit (GTK main thread) talks to the host via:
//!   - [`HostEvent`] on the async-channel receiver returned from [`start`],
//!     for "something happened in compositor-land" (toplevel created,
//!     frame ready, etc.).
//!   - [`HostCommand`] on the async-channel sender, for "cockpit wants
//!     something done" (resize a toplevel, close it, move focus).
//!
//! The compositor thread polls the command receiver via a `calloop`
//! async-channel source so GTK → compositor is non-blocking.

use std::ffi::OsString;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::thread;

use thiserror::Error;

#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
mod state;

#[cfg(target_os = "linux")]
pub use host::HostHandle;
#[cfg(target_os = "linux")]
pub use state::SurfaceId;

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SurfaceId(pub u64);

#[cfg(not(target_os = "linux"))]
impl std::fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sfc-{}", self.0)
    }
}

#[cfg(not(target_os = "linux"))]
pub struct HostHandle;

#[cfg(not(target_os = "linux"))]
impl HostHandle {
    pub fn request_shutdown(&self) {}
}

/// Payload for [`HostEvent::DmabufFrame`]. Kept as its own struct because
/// `OwnedFd` doesn't implement `Clone`, so we can't let the variant ride
/// inside a `#[derive(Clone)]` enum. Each DmabufFrame owns its fd outright;
/// on drop the fd is closed, so downstream code must feed it to GTK before
/// dropping.
#[derive(Debug)]
pub struct DmabufFrame {
    pub id: SurfaceId,
    pub width: u32,
    pub height: u32,
    /// DRM fourcc (e.g. `DRM_FORMAT_XRGB8888` = 0x34325258).
    pub fourcc: u32,
    /// DRM format modifier (e.g. `DRM_FORMAT_MOD_LINEAR` = 0).
    pub modifier: u64,
    /// Single-plane dmabuf fd. Caller owns it until passed to GTK.
    pub fd: OwnedFd,
    pub stride: u32,
    pub offset: u32,
}

/// Public event surface sent from the compositor thread to the cockpit.
#[derive(Debug)]
pub enum HostEvent {
    /// Emitted exactly once after the socket has been bound and the event
    /// loop is dispatching. The cockpit uses this to delay launcher spawns
    /// until `WAYLAND_DISPLAY` actually resolves.
    Ready { display_name: String },
    /// A client just created an xdg_toplevel. The cockpit responds by
    /// allocating a `SatelliteWidget` (Task #10) for this surface id
    /// and attaching it as a pane child of the currently-active anchor.
    ToplevelCreated {
        id: SurfaceId,
        title: Option<String>,
        app_id: Option<String>,
    },
    /// Fired when the client mutates its toplevel title after the
    /// initial `ToplevelCreated`. Cockpit updates the pane label.
    ToplevelTitleChanged {
        id: SurfaceId,
        title: Option<String>,
    },
    /// Fired when the client mutates its app_id after the initial
    /// `ToplevelCreated`. Most clients set app_id exactly once, so this
    /// event is rare but the protocol allows it.
    ToplevelAppIdChanged {
        id: SurfaceId,
        app_id: Option<String>,
    },
    /// A toplevel was destroyed by its client (or the client went
    /// away). The cockpit removes the matching `SatelliteWidget` and
    /// collapses the pane slot.
    ToplevelClosed { id: SurfaceId },
    /// A new xdg_toplevel with a `set_parent` relationship committed for
    /// the first time. Render as an overlay on the parent pane (same
    /// infra as xdg_popup) rather than spliting the tree — matches the
    /// user expectation for modal/child dialogs (e.g. IntelliJ "Open
    /// File" floating on top of the welcome window).
    ChildToplevelCreated {
        id: SurfaceId,
        parent: SurfaceId,
        title: Option<String>,
        app_id: Option<String>,
        width: u32,
        height: u32,
    },
    /// Child toplevel destroyed. Cockpit removes the overlay.
    ChildToplevelClosed { id: SurfaceId },
    /// A client created an xdg_popup (menu / dropdown / tooltip). The
    /// cockpit overlays a floating Picture on top of the parent
    /// satellite's pane at `(x, y)` with `(width, height)` — coordinates
    /// are in the parent surface's window-geometry space.
    PopupCreated {
        id: SurfaceId,
        parent: SurfaceId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// A popup was repositioned (xdg_positioner reconstrain after parent
    /// resize, or submenu walk). Cockpit re-places the overlay widget.
    PopupRepositioned {
        id: SurfaceId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        /// Client-provided token so the client can correlate the configure
        /// it'll receive (currently unused by the cockpit, but faithful to
        /// the xdg_popup v3 protocol).
        token: u32,
    },
    /// A popup was destroyed (menu closed, client went away). Cockpit
    /// removes the overlay Picture.
    PopupClosed { id: SurfaceId },
    /// A client committed a new shm frame. `rgb` is tightly-packed
    /// RGB8 (no alpha, stride == `width * 3`) so the GTK side can
    /// feed it straight into a `gdk::MemoryTexture`. The host has
    /// already released the source `wl_buffer` back to the client by
    /// the time this event is sent, so the cockpit owns `rgb` outright.
    FrameReady {
        id: SurfaceId,
        width: u32,
        height: u32,
        rgb: Vec<u8>,
    },
    /// A client committed a new dmabuf frame. The GTK side imports
    /// `fd` into a `gdk::DmabufTexture` so the GPU holds the pixel
    /// data — no CPU memcpy, which is the difference between "Chrome
    /// crawls" and "Chrome feels native". Single-plane RGB formats
    /// only for now; multi-plane / YUV rides v0.3.
    DmabufFrame(DmabufFrame),
    /// A satellite client changed the pointer cursor shape via
    /// wp_cursor_shape_device_v1 (or legacy wl_pointer.set_cursor, which
    /// we coerce to a named fallback). `surface_id` names the surface
    /// that currently holds pointer focus; the cockpit scopes the cursor
    /// to that satellite widget so other panes keep their default. `name`
    /// is the freedesktop/CSS cursor name ("default", "pointer", "text",
    /// "ew-resize", …) that GTK's `set_cursor_from_name` takes directly.
    CursorShape {
        surface_id: Option<SurfaceId>,
        name: String,
    },
    /// A satellite client set its cursor to a custom bitmap surface via
    /// the legacy `wl_pointer.set_cursor(surface, hotspot_x, hotspot_y)`
    /// path — used by JetBrains/AWT-Wayland for resize/I-beam cursors
    /// when wp_cursor_shape_device_v1 isn't enough. `rgb` is tightly-packed
    /// RGB8 (stride == width*3), suitable for `gdk::MemoryTexture` →
    /// `gdk::Cursor::from_texture`. `surface_id` scopes the cursor to a
    /// single satellite the same way `CursorShape` does.
    CursorBitmap {
        surface_id: Option<SurfaceId>,
        width: u32,
        height: u32,
        /// Premultiplied RGBA8, stride == width * 4. Wayland Argb8888 is
        /// premultiplied per spec; XRGB8888 frames synthesize alpha = 0xFF.
        rgba: Vec<u8>,
        hotspot_x: i32,
        hotspot_y: i32,
    },
    /// Emitted when the event loop terminates (either clean shutdown or
    /// an unrecoverable protocol error). After this, no further events
    /// will arrive.
    Stopped,
}

/// Public command surface sent from the cockpit to the compositor thread.
///
/// MVP variants are stubs; follow-up tasks add `ResizeToplevel`,
/// `CloseToplevel`, `SetKeyboardFocus`, `PointerInput`, etc.
#[derive(Debug, Clone)]
pub enum HostCommand {
    /// Tell the compositor thread to exit its event loop. Idempotent.
    Shutdown,
    /// Push a new size to the tracked toplevel. The compositor sends an
    /// `xdg_toplevel.configure` with this size so the client re-allocates
    /// its buffer on the next frame. No-op if the id is unknown.
    ResizeToplevel {
        id: SurfaceId,
        width: u32,
        height: u32,
    },
    /// Ask the client to close the toplevel (sends `xdg_toplevel.close`).
    /// The client will follow up with `xdg_toplevel.destroy` which maps to
    /// a `HostEvent::ToplevelClosed`.
    CloseToplevel { id: SurfaceId },
    /// Pointer moved inside the satellite's GTK widget. `x`/`y` are
    /// surface-local pixel coordinates.
    PointerMotion { id: SurfaceId, x: f64, y: f64 },
    /// Pointer button pressed or released while over the satellite.
    /// `button` is a linux evdev code (`BTN_LEFT=0x110`, etc).
    PointerButton {
        id: SurfaceId,
        button: u32,
        pressed: bool,
    },
    /// Pointer left the satellite's GTK widget — clear focus so the
    /// client sees a proper leave event.
    PointerLeave { id: SurfaceId },
    /// Scroll wheel delta expressed in logical "steps" (one notch per
    /// integer). Positive y = scroll down, positive x = scroll right,
    /// matching GTK's convention. The compositor multiplies by 15 to
    /// produce the wayland axis value (the libinput convention is
    /// roughly 15 pixels per notch).
    PointerAxis { id: SurfaceId, dx: f64, dy: f64 },
    /// Raw key press or release. `keycode` is the evdev code (as
    /// delivered by GDK via `EventControllerKey::connect_key_pressed`
    /// + `keyval_to_keycode`). The compositor adds 8 internally to
    ///   match XKB's offset.
    KeyInput {
        id: SurfaceId,
        evdev_code: u32,
        pressed: bool,
    },
    /// Move keyboard focus to a satellite (or away from all of them
    /// when `id` is None). Fires `wl_keyboard.enter`/`leave`.
    KeyboardFocus { id: Option<SurfaceId> },
}

/// Anything that can go wrong setting up or running the host.
#[derive(Debug, Error)]
pub enum Error {
    #[error("XDG_RUNTIME_DIR is not set — cannot bind the wayland socket")]
    NoRuntimeDir,
    #[error("failed to create socket parent {0:?}: {1}")]
    SocketDirCreate(PathBuf, std::io::Error),
    #[error("failed to bind wayland socket at {0:?}: {1}")]
    SocketBind(OsString, std::io::Error),
    #[error("calloop event loop init failed: {0}")]
    EventLoopInit(String),
    #[error("host thread spawn failed: {0}")]
    ThreadSpawn(std::io::Error),
    #[error("nested Wayland host is only available on Linux")]
    UnsupportedPlatform,
}

/// Spawn the compositor on a dedicated OS thread and return the channel
/// pair + a [`HostHandle`] that lets the cockpit command it.
///
/// The returned [`HostHandle`] owns the thread's join handle and a
/// shutdown flag; dropping it posts [`HostCommand::Shutdown`] and joins.
#[cfg(target_os = "linux")]
pub fn start() -> Result<
    (
        HostHandle,
        async_channel::Sender<HostCommand>,
        async_channel::Receiver<HostEvent>,
    ),
    Error,
> {
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<HostCommand>();
    let (evt_tx, evt_rx) = async_channel::unbounded::<HostEvent>();
    let stop = Arc::new(AtomicBool::new(false));

    let thread_stop = stop.clone();
    let thread_cmd_rx = cmd_rx.clone();
    let thread_evt_tx = evt_tx.clone();
    let join = thread::Builder::new()
        .name("lmux-wayland-host".into())
        .spawn(move || {
            if let Err(err) = host::run(thread_cmd_rx, thread_evt_tx.clone(), thread_stop) {
                tracing::warn!(error = %err, "lmux-wayland-host: event loop terminated with error");
            }
            // Always post Stopped so GTK listeners can tear their own state down.
            let _ = thread_evt_tx.send_blocking(HostEvent::Stopped);
        })
        .map_err(Error::ThreadSpawn)?;

    Ok((
        HostHandle {
            join: Some(join),
            stop,
            cmd_tx: cmd_tx.clone(),
        },
        cmd_tx,
        evt_rx,
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn start() -> Result<
    (
        HostHandle,
        async_channel::Sender<HostCommand>,
        async_channel::Receiver<HostEvent>,
    ),
    Error,
> {
    Err(Error::UnsupportedPlatform)
}
