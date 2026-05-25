//! Full wire-level test: spin up the nested compositor, connect a real
//! wayland-client, walk the xdg_shell handshake up through
//! `xdg_toplevel.set_title`, and assert that `HostEvent::ToplevelCreated`
//! lands on the cockpit channel with the title we sent.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::collapsible_match)]

use std::time::{Duration, Instant};

use lmux_wayland_host::{start, HostEvent, SurfaceId};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

/// Client-side dispatch sink. The protocol doesn't require per-object
/// state for this test — we only drive the handshake and exit — so
/// every `Dispatch` impl is a no-op.
struct TestClient {
    configured: bool,
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

noop_dispatch!(XdgToplevel, ());

#[test]
fn client_can_create_xdg_toplevel_and_emit_event() {
    // Sandbox XDG_RUNTIME_DIR so parallel test processes don't stomp
    // each other's sockets.
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, _cmd_tx, evt_rx) = start().unwrap();

    // Wait for HostEvent::Ready so we know the socket is accepting.
    let display_name = wait_for_ready(&evt_rx);

    // Connect a client against our socket (fall back to WAYLAND_DISPLAY
    // env var, which is how real clients find it).
    std::env::set_var("WAYLAND_DISPLAY", &display_name);
    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<TestClient>(&conn).unwrap();
    let qh = queue.handle();

    let compositor: WlCompositor = globals
        .bind(&qh, 1..=6, ())
        .expect("compositor global present");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=5, ()).expect("xdg_wm_base present");

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("lmux-test-title".into());
    toplevel.set_app_id("no.jpro.lmux-test".into());
    // The xdg-shell contract is: client must commit the surface after
    // get_toplevel + role setters, then wait for the compositor's first
    // xdg_surface.configure before attaching a buffer.
    surface.commit();

    let mut client_state = TestClient { configured: false };

    // Pump the queue until we've received the first Configure + acked it.
    roundtrip_until(&mut queue, &mut client_state, |st| st.configured);

    // Drain HostEvents — we expect a Ready (already consumed) and now a
    // ToplevelCreated with the title/app_id we set.
    let (id, title, app_id) = wait_for_toplevel_created(&evt_rx);
    assert_eq!(title.as_deref(), Some("lmux-test-title"));
    assert_eq!(app_id.as_deref(), Some("no.jpro.lmux-test"));

    // Tear down the toplevel from the client side and assert we get the
    // matching ToplevelClosed event.
    toplevel.destroy();
    xdg_surface.destroy();
    surface.destroy();
    drop(queue);
    drop(conn);

    let closed_id = wait_for_toplevel_closed(&evt_rx);
    assert_eq!(closed_id, id);

    drop(host_handle);
}

fn wait_for_ready(evt_rx: &async_channel::Receiver<HostEvent>) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(HostEvent::Ready { display_name }) = evt_rx.try_recv() {
            return display_name;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("host never emitted HostEvent::Ready");
}

fn wait_for_toplevel_created(
    evt_rx: &async_channel::Receiver<HostEvent>,
) -> (SurfaceId, Option<String>, Option<String>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(evt) = evt_rx.try_recv() {
            if let HostEvent::ToplevelCreated { id, title, app_id } = evt {
                return (id, title, app_id);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("host never emitted HostEvent::ToplevelCreated");
}

fn wait_for_toplevel_closed(evt_rx: &async_channel::Receiver<HostEvent>) -> SurfaceId {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(evt) = evt_rx.try_recv() {
            if let HostEvent::ToplevelClosed { id } = evt {
                return id;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("host never emitted HostEvent::ToplevelClosed");
}

fn roundtrip_until(
    queue: &mut EventQueue<TestClient>,
    state: &mut TestClient,
    mut cond: impl FnMut(&TestClient) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        queue.blocking_dispatch(state).unwrap();
        if cond(state) {
            return;
        }
    }
    panic!("client roundtrip never satisfied predicate");
}
