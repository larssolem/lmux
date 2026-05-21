//! Wire-level test for Task #11: GTK → HostCommand → wl_seat → client.
//!
//! Stands up the nested compositor, connects a client that binds
//! `wl_seat`, creates + commits a toplevel, then posts
//! `HostCommand::KeyboardFocus` + `KeyInput` and asserts the client's
//! `wl_keyboard` sees the matching key event. Also posts pointer
//! motion + button and asserts `wl_pointer` sees `enter` + `button`.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lmux_wayland_host::{start, HostCommand, HostEvent};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{Event as KbEvent, WlKeyboard};
use wayland_client::protocol::wl_pointer::{Event as PtrEvent, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

#[derive(Default, Debug)]
struct Sink {
    key_codes: Vec<u32>,
    key_pressed: Vec<bool>,
    pointer_entered: bool,
    pointer_buttons: Vec<(u32, bool)>,
}

struct TestClient {
    configured: bool,
    sink: Arc<Mutex<Sink>>,
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

noop_dispatch!(WlCompositor, ());
noop_dispatch!(WlSurface, ());
noop_dispatch!(XdgToplevel, ());

impl Dispatch<WlSeat, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        event: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Capabilities arrive as a bitfield — we don't branch on them
        // in this test (we already know the server has kb+pointer);
        // we get the handles via explicit get_pointer/get_keyboard.
        if let wl_seat::Event::Capabilities { .. } = event {
            // ignored
        }
    }
}

impl Dispatch<WlKeyboard, ()> for TestClient {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: <WlKeyboard as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let KbEvent::Key {
            key, state: kstate, ..
        } = event
        {
            let pressed = matches!(
                kstate,
                wayland_client::WEnum::Value(
                    wayland_client::protocol::wl_keyboard::KeyState::Pressed
                ),
            );
            let mut s = state.sink.lock().unwrap();
            s.key_codes.push(key);
            s.key_pressed.push(pressed);
        }
    }
}

impl Dispatch<WlPointer, ()> for TestClient {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: <WlPointer as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut s = state.sink.lock().unwrap();
        match event {
            PtrEvent::Enter { .. } => s.pointer_entered = true,
            PtrEvent::Button {
                button,
                state: bstate,
                ..
            } => {
                let pressed = matches!(
                    bstate,
                    wayland_client::WEnum::Value(
                        wayland_client::protocol::wl_pointer::ButtonState::Pressed,
                    ),
                );
                s.pointer_buttons.push((button, pressed));
            }
            _ => {}
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
fn seat_commands_route_to_client_keyboard_and_pointer() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, cmd_tx, evt_rx) = start().unwrap();
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
    let seat: WlSeat = globals.bind(&qh, 1..=9, ()).unwrap();

    // The seat advertises pointer + keyboard after bind because the
    // host created both in State::new. We can grab the handles right
    // away without waiting for a Capabilities event — wayland-client
    // permits requesting caps that the server will then honor.
    let keyboard = seat.get_keyboard(&qh, ());
    let pointer = seat.get_pointer(&qh, ());

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("input-test".into());
    surface.commit();

    let sink = Arc::new(Mutex::new(Sink::default()));
    let mut client_state = TestClient {
        configured: false,
        sink: sink.clone(),
    };
    roundtrip_until(&mut queue, &mut client_state, |st| st.configured);

    // At this point the server has tracked the toplevel but not yet
    // emitted ToplevelCreated (deferred to first commit).
    let (surface_id, _, _) = wait_for_event(&evt_rx, |e| match e {
        HostEvent::ToplevelCreated { id, title, app_id } => Some((id, title, app_id)),
        _ => None,
    });

    // --- Keyboard focus + key ---------------------------------------
    cmd_tx
        .send_blocking(HostCommand::KeyboardFocus {
            id: Some(surface_id),
        })
        .unwrap();
    // evdev KEY_A is 30 — picked because it produces a visible,
    // non-modifier keysym so a regression (sending 0, wrong offset,
    // etc.) stands out in the assertion.
    cmd_tx
        .send_blocking(HostCommand::KeyInput {
            id: surface_id,
            evdev_code: 30,
            pressed: true,
        })
        .unwrap();
    cmd_tx
        .send_blocking(HostCommand::KeyInput {
            id: surface_id,
            evdev_code: 30,
            pressed: false,
        })
        .unwrap();

    // --- Pointer motion + button ------------------------------------
    cmd_tx
        .send_blocking(HostCommand::PointerMotion {
            id: surface_id,
            x: 12.5,
            y: 7.0,
        })
        .unwrap();
    // BTN_LEFT = 0x110
    cmd_tx
        .send_blocking(HostCommand::PointerButton {
            id: surface_id,
            button: 0x110,
            pressed: true,
        })
        .unwrap();
    cmd_tx
        .send_blocking(HostCommand::PointerButton {
            id: surface_id,
            button: 0x110,
            pressed: false,
        })
        .unwrap();

    // Pump until the client sees both sides of the handshake.
    roundtrip_until(&mut queue, &mut client_state, |_| {
        let s = sink.lock().unwrap();
        s.key_codes.len() >= 2 && s.pointer_entered && s.pointer_buttons.len() >= 2
    });

    let s = sink.lock().unwrap();
    assert_eq!(s.key_codes, vec![30, 30], "client saw evdev keycodes as-is");
    assert_eq!(s.key_pressed, vec![true, false], "press then release");
    assert!(s.pointer_entered, "pointer entered surface");
    assert_eq!(
        s.pointer_buttons,
        vec![(0x110, true), (0x110, false)],
        "press+release of BTN_LEFT",
    );
    drop(s);

    // Drop the pointer + keyboard before the connection goes away,
    // otherwise smithay logs warnings about dangling focus.
    drop(pointer);
    drop(keyboard);
    drop(toplevel);
    drop(xdg_surface);
    drop(surface);
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

fn roundtrip_until(
    queue: &mut EventQueue<TestClient>,
    state: &mut TestClient,
    mut cond: impl FnMut(&TestClient) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        queue.flush().unwrap();
        queue.dispatch_pending(state).unwrap_or_default();
        if cond(state) {
            return;
        }
        // Block for new events until timeout slice elapses.
        let _ = queue.blocking_dispatch(state);
    }
    panic!("client roundtrip never satisfied predicate");
}
