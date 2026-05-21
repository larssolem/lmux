//! Wire-level test for the wl_shm → RGB frame pipeline (Task #9).
//!
//! Stands up the nested compositor, connects a real wayland-client,
//! walks the xdg_shell handshake, creates a wl_shm pool + XRGB8888
//! buffer with a known pixel pattern, attaches + commits it, and
//! asserts that `HostEvent::FrameReady` arrives with the same pixels
//! (byte-swizzled ARGB→RGB) at the correct dimensions.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use lmux_wayland_host::{start, HostEvent, SurfaceId};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::WlCallback;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

struct TestClient {
    configured: bool,
    buffer_released: bool,
    frame_done: bool,
}

impl Dispatch<WlRegistry, GlobalListContents> for TestClient {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! noop_dispatch {
    ($proxy:ty, $udata:ty) => {
        impl Dispatch<$proxy, $udata> for TestClient {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as wayland_client::Proxy>::Event,
                _: &$udata,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}

noop_dispatch!(WlCompositor, ());
noop_dispatch!(WlSurface, ());
noop_dispatch!(WlShm, ());
noop_dispatch!(WlShmPool, ());
noop_dispatch!(XdgToplevel, ());

impl Dispatch<WlBuffer, ()> for TestClient {
    fn event(
        state: &mut Self,
        _: &WlBuffer,
        event: <WlBuffer as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_buffer::Event::Release = event {
            state.buffer_released = true;
        }
    }
}

impl Dispatch<WlCallback, ()> for TestClient {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: <WlCallback as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_callback::Event::Done { .. } = event {
            state.frame_done = true;
        }
    }
}

impl Dispatch<XdgWmBase, ()> for TestClient {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: <XdgWmBase as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for TestClient {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: <XdgSurface as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_protocols::xdg::shell::client::xdg_surface::Event::Configure { serial } =
            event
        {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

#[test]
fn shm_buffer_commit_emits_frame_ready_and_releases_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, _cmd_tx, evt_rx) = start().unwrap();
    let display_name = wait_for_event(&evt_rx, |e| match e {
        HostEvent::Ready { display_name } => Some(display_name),
        _ => None,
    });
    std::env::set_var("WAYLAND_DISPLAY", &display_name);

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<TestClient>(&conn).unwrap();
    let qh = queue.handle();

    let compositor: WlCompositor = globals.bind(&qh, 1..=6, ()).unwrap();
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=5, ()).unwrap();
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).unwrap();

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("shm-test".into());
    surface.commit();

    let mut client_state = TestClient {
        configured: false,
        buffer_released: false,
        frame_done: false,
    };
    roundtrip_until(&mut queue, &mut client_state, |st| st.configured);

    // Build a 4x3 XRGB8888 buffer. Stride = 16, total = 48 bytes.
    // Row-major; each pixel has a unique memory pattern so any
    // stride/row-major bug is visible in the assertion.
    const W: i32 = 4;
    const H: i32 = 3;
    const STRIDE: i32 = W * 4;
    let mut pattern_argb = Vec::with_capacity((STRIDE * H) as usize);
    for y in 0..H {
        for x in 0..W {
            // XRGB8888, little-endian → memory order B, G, R, X.
            let r = (10 + x * 20) as u8;
            let g = (30 + y * 40) as u8;
            let b = (50 + x * 5 + y * 3) as u8;
            pattern_argb.extend_from_slice(&[b, g, r, 0xFF]);
        }
    }

    let shm_file = tempfile::tempfile().unwrap();
    shm_file.set_len((STRIDE * H) as u64).unwrap();
    let mut f = shm_file;
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(&pattern_argb).unwrap();
    f.flush().unwrap();

    let pool = shm.create_pool(f.as_fd(), STRIDE * H, &qh, ());
    let buffer = pool.create_buffer(0, W, H, STRIDE, Format::Xrgb8888, &qh, ());
    let frame_cb = surface.frame(&qh, ());
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, W, H);
    surface.commit();
    // Force the queued requests onto the wire. wayland-client batches
    // within a dispatch round so without an explicit flush the
    // server wouldn't see the commit until the client next blocks.
    queue.flush().unwrap();

    // First we'll see ToplevelCreated (triggered by the pre-buffer
    // commit that acked configure), then FrameReady for this commit.
    let (created_id, _title, _app_id) = wait_for_event(&evt_rx, |e| match e {
        HostEvent::ToplevelCreated { id, title, app_id } => Some((id, title, app_id)),
        _ => None,
    });

    let (fw, fh, rgb) = wait_for_event(&evt_rx, |e| match e {
        HostEvent::FrameReady {
            id,
            width,
            height,
            rgb,
        } if id == created_id => Some((width, height, rgb)),
        _ => None,
    });
    assert_eq!((fw, fh), (W as u32, H as u32), "frame dimensions");
    assert_eq!(rgb.len(), (W * H * 3) as usize, "rgb tight-packed length");

    // Spot-check pixels at (0,0), (3,0), (0,2) — covers first row,
    // row-end, and last row so an off-by-one in stride or row order
    // fails loudly.
    let pix = |x: i32, y: i32| {
        let off = ((y * W + x) * 3) as usize;
        (rgb[off], rgb[off + 1], rgb[off + 2])
    };
    assert_eq!(pix(0, 0), (10, 30, 50), "top-left");
    assert_eq!(pix(3, 0), (10 + 60, 30, 50 + 15), "top-right");
    assert_eq!(pix(0, 2), (10, 30 + 80, 50 + 6), "bottom-left");

    // The server MUST have released the buffer back — otherwise the
    // client would wedge forever waiting to redraw the next frame.
    roundtrip_until(&mut queue, &mut client_state, |st| {
        st.buffer_released && st.frame_done
    });
    assert!(client_state.buffer_released, "server released wl_buffer");
    assert!(client_state.frame_done, "server fired wl_callback.done");

    toplevel.destroy();
    xdg_surface.destroy();
    surface.destroy();
    drop(frame_cb);
    drop(buffer);
    drop(pool);
    drop(queue);
    drop(conn);

    drop(host_handle);
}

fn wait_for_event<T>(
    evt_rx: &async_channel::Receiver<HostEvent>,
    mut pick: impl FnMut(HostEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(evt) = evt_rx.try_recv() {
            if let Some(v) = pick(evt) {
                return v;
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    panic!("wait_for_event timed out");
}

#[allow(dead_code)]
fn _proof_surface_id_is_used(_: SurfaceId) {}

fn roundtrip_until(
    queue: &mut EventQueue<TestClient>,
    state: &mut TestClient,
    mut cond: impl FnMut(&TestClient) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        queue.blocking_dispatch(state).unwrap();
        if cond(state) {
            return;
        }
    }
    panic!("client roundtrip never satisfied predicate");
}
