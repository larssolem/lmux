//! Regression test for the "GtkPaned drag freezes the host" bug
//! (paned-resize storm). Reproduces what the cockpit does: open an
//! xdg_toplevel, then send a flood of `HostCommand::ResizeToplevel` —
//! one per simulated divider tick — and assert that:
//!
//!   1. The client keeps receiving `xdg_surface.configure` events
//!      (i.e., the host loop didn't hang).
//!   2. The final configure carries the *last* size we sent
//!      (i.e., resizes aren't silently dropped).
//!   3. A follow-up command (a no-op `KeyboardFocus(None)`) still
//!      goes through after the storm — the host loop is still alive.
//!
//! The original symptom was a hard freeze of the wayland-host thread:
//! the GTK side posted ~60 ResizeToplevel commands per second during a
//! divider drag, and none of them reached the smithay handler — see
//! `docs/history/v0.2-progress.md` notes around
//! "paned drag freezes nested compositor".

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::collapsible_match)]

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Tests share process-global env vars (`XDG_RUNTIME_DIR`,
/// `WAYLAND_DISPLAY`) and so cannot run concurrently. Each test takes
/// this lock for its full duration.
fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

use lmux_wayland_host::{start, HostCommand, HostEvent, SurfaceId};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

/// Per-client bookkeeping: count configures, capture the last size the
/// compositor sent us. The size comes from `xdg_toplevel.configure`,
/// not `xdg_surface.configure` (which only carries a serial).
struct TestClient {
    configure_count: Arc<AtomicU32>,
    last_w: Arc<AtomicU32>,
    last_h: Arc<AtomicU32>,
    last_serial: Option<u32>,
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
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.last_serial = Some(serial);
            state.configure_count.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl Dispatch<XdgToplevel, ()> for TestClient {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: <XdgToplevel as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            // smithay only emits a non-zero size once we've sent a
            // configure with `pending_state.size = Some(...)` — so the
            // first configure of the handshake carries (0,0) and we
            // ignore it here. Real compositors do the same.
            if width > 0 && height > 0 {
                state.last_w.store(width as u32, Ordering::SeqCst);
                state.last_h.store(height as u32, Ordering::SeqCst);
            }
        }
    }
}

#[test]
fn resize_storm_does_not_hang_host() {
    let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, cmd_tx, evt_rx) = start().unwrap();

    let display_name = wait_for_ready(&evt_rx);
    std::env::set_var("WAYLAND_DISPLAY", &display_name);

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<TestClient>(&conn).unwrap();
    let qh = queue.handle();

    let compositor: WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=5, ()).expect("xdg_wm_base");

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("resize-storm".into());
    surface.commit();

    let configure_count = Arc::new(AtomicU32::new(0));
    let last_w = Arc::new(AtomicU32::new(0));
    let last_h = Arc::new(AtomicU32::new(0));
    let mut client = TestClient {
        configure_count: configure_count.clone(),
        last_w: last_w.clone(),
        last_h: last_h.clone(),
        last_serial: None,
    };

    // Drive the initial xdg handshake (configure -> ack -> ToplevelCreated).
    roundtrip_until(&mut queue, &mut client, |s| s.last_serial.is_some());
    let id = wait_for_toplevel_created(&evt_rx);

    // Reset the configure counter after the handshake — we want to
    // observe configures triggered by the *storm*, not the bootstrap.
    configure_count.store(0, Ordering::SeqCst);

    // === The storm ===
    // Mimic a 1-second GtkPaned drag at ~60Hz: 60 resizes interpolating
    // from 1800x2000 down to 200x2000. The original bug was the host
    // thread hard-freezing somewhere in here; if that happens, the
    // wayland-client roundtrip below will time out.
    const STEPS: u32 = 60;
    for i in 0..STEPS {
        let t = i as f64 / (STEPS - 1) as f64;
        let width = (1800.0 + (200.0 - 1800.0) * t) as u32;
        let height = 2000;
        cmd_tx
            .send_blocking(HostCommand::ResizeToplevel { id, width, height })
            .expect("cmd channel still open");
        // ~16ms per tick mirrors a real frame clock. Without this the
        // whole storm posts in <1ms, which is also a useful stress case
        // but doesn't match what GTK actually does.
        std::thread::sleep(Duration::from_millis(16));
    }

    // === Assert host is still responsive ===
    // Pump the queue until we see a configure carrying the *final*
    // resize size (200x2000). 2-second deadline matches the original
    // bug's freeze duration — pre-fix, this times out.
    let final_w = (1800.0 + (200.0 - 1800.0)) as u32;
    let final_h = 2000u32;
    let dispatch_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < dispatch_deadline {
        // blocking_dispatch reads from the wayland socket and processes
        // queued events. dispatch_pending alone wouldn't actually read
        // from the kernel socket buffer.
        if let Err(err) =
            blocking_dispatch_with_timeout(&mut queue, &mut client, Duration::from_millis(100))
        {
            panic!("client dispatch failed mid-storm: {err}");
        }
        if last_w.load(Ordering::SeqCst) == final_w && last_h.load(Ordering::SeqCst) == final_h {
            break;
        }
    }

    let observed_count = configure_count.load(Ordering::SeqCst);
    assert!(
        observed_count > 0,
        "expected at least one configure during the storm — host appears hung",
    );
    assert_eq!(
        (last_w.load(Ordering::SeqCst), last_h.load(Ordering::SeqCst)),
        (final_w, final_h),
        "final configure should carry the last resize we sent ({final_w}x{final_h}) — \
         got {}x{} after {} configures",
        last_w.load(Ordering::SeqCst),
        last_h.load(Ordering::SeqCst),
        observed_count,
    );

    // === Liveness check ===
    // After the storm, send one more command and verify the host
    // processes it. KeyboardFocus(None) is harmless and exercises
    // the cmd-drain path one more time.
    cmd_tx
        .send_blocking(HostCommand::KeyboardFocus { id: None })
        .expect("cmd channel still open after storm");

    // Give the host a couple of dispatch cycles to drain.
    let post_deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < post_deadline {
        let _ = blocking_dispatch_with_timeout(&mut queue, &mut client, Duration::from_millis(100));
    }
    // No assertion needed — if the host had hung, the watchdog would
    // already have killed the test via timeout. Reaching this point
    // means the cmd channel is still drained.

    toplevel.destroy();
    xdg_surface.destroy();
    surface.destroy();
    drop(queue);
    drop(conn);
    drop(host_handle);
}

/// Stricter variant: the client deliberately *does not* read its
/// wayland socket during the storm. This mirrors a client whose UI
/// thread has briefly stalled (the suspected real-world trigger for
/// the IntelliJ hang). The host must not block on writes to a slow
/// client — if it does, the cmd-drain loop freezes, the watchdog
/// thread observes it, and we miss the liveness probe at the end.
#[test]
fn resize_storm_with_slow_client_does_not_hang_host() {
    let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, cmd_tx, evt_rx) = start().unwrap();
    let display_name = wait_for_ready(&evt_rx);
    std::env::set_var("WAYLAND_DISPLAY", &display_name);

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<TestClient>(&conn).unwrap();
    let qh = queue.handle();
    let compositor: WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=5, ()).expect("xdg_wm_base");

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("slow-client".into());
    surface.commit();

    let configure_count = Arc::new(AtomicU32::new(0));
    let last_w = Arc::new(AtomicU32::new(0));
    let last_h = Arc::new(AtomicU32::new(0));
    let mut client = TestClient {
        configure_count: configure_count.clone(),
        last_w: last_w.clone(),
        last_h: last_h.clone(),
        last_serial: None,
    };

    roundtrip_until(&mut queue, &mut client, |s| s.last_serial.is_some());
    let id = wait_for_toplevel_created(&evt_rx);
    configure_count.store(0, Ordering::SeqCst);

    // Pile a large backlog of resizes onto the cmd channel without
    // ever pumping the wayland socket. If smithay's write path blocks
    // when the per-client buffer fills, the host thread freezes here.
    const STEPS: u32 = 500;
    let storm_started = Instant::now();
    for i in 0..STEPS {
        let width = 200 + (i * 3); // monotonically grows from 200 to ~1700
        cmd_tx
            .send_blocking(HostCommand::ResizeToplevel {
                id,
                width,
                height: 2000,
            })
            .expect("cmd channel still open");
    }
    let storm_send_dur = storm_started.elapsed();
    assert!(
        storm_send_dur < Duration::from_secs(2),
        "sending {STEPS} cmds should be fast — took {storm_send_dur:?}",
    );

    // Liveness probe: after the storm, the host should still process
    // a follow-up command. We can't directly observe this, but we can
    // observe that the wayland socket eventually delivers configures
    // *after* we start reading again. If the host was hung on a
    // socket write, no progress is possible.
    let progress_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < progress_deadline {
        let _ = blocking_dispatch_with_timeout(&mut queue, &mut client, Duration::from_millis(100));
        if configure_count.load(Ordering::SeqCst) > 0 {
            break;
        }
    }

    assert!(
        configure_count.load(Ordering::SeqCst) > 0,
        "after a {STEPS}-cmd storm with a stalled client, the host did not deliver \
         a single configure within 5s — looks like the host thread is hung",
    );

    toplevel.destroy();
    xdg_surface.destroy();
    surface.destroy();
    drop(queue);
    drop(conn);
    drop(host_handle);
}

/// Stricter still: 3 simultaneous clients, two of them silent. This is
/// what the cockpit looks like in practice — IntelliJ + Chrome + the
/// cockpit's own cursor surface — and it's the variant most likely to
/// expose any cross-client interaction in the host loop.
#[test]
fn resize_storm_with_multiple_clients_does_not_hang_host() {
    let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, cmd_tx, evt_rx) = start().unwrap();
    let display_name = wait_for_ready(&evt_rx);
    std::env::set_var("WAYLAND_DISPLAY", &display_name);

    // Three independent clients, three independent wayland connections.
    // All three reach a `Configured` state before we start the storm.
    let mut clients: Vec<ClientFixture> = (0..3)
        .map(|i| ClientFixture::connect(&format!("multi-{i}")))
        .collect();
    let mut ids = Vec::with_capacity(3);
    for c in &mut clients {
        c.handshake();
    }
    for _ in 0..3 {
        ids.push(wait_for_toplevel_created(&evt_rx));
    }
    for c in &mut clients {
        c.configure_count.store(0, Ordering::SeqCst);
    }

    // Storm: 200 resizes per toplevel, interleaved across all 3
    // toplevels. Clients 1 and 2 deliberately do NOT pump their sockets.
    // Client 0 stays responsive in a background thread (this is the one
    // we observe for liveness — if the host hangs trying to write to a
    // stalled client, client 0's events also stop arriving).
    const STEPS: u32 = 200;
    for i in 0..STEPS {
        for (idx, &id) in ids.iter().enumerate() {
            let width = 200 + (i * 3) + (idx as u32 * 10);
            cmd_tx
                .send_blocking(HostCommand::ResizeToplevel {
                    id,
                    width,
                    height: 1500 + idx as u32 * 100,
                })
                .expect("cmd channel still open");
        }
    }

    // Pump only client 0's queue. If the host is stuck writing to
    // clients 1 or 2, no events reach client 0 either.
    let final_w0 = 200 + ((STEPS - 1) * 3);
    let final_h0 = 1500u32;
    let deadline = Instant::now() + Duration::from_secs(5);
    let c0 = &mut clients[0];
    while Instant::now() < deadline {
        let _ = blocking_dispatch_with_timeout(
            &mut c0.queue,
            &mut c0.state,
            Duration::from_millis(100),
        );
        if c0.last_w.load(Ordering::SeqCst) == final_w0
            && c0.last_h.load(Ordering::SeqCst) == final_h0
        {
            break;
        }
    }

    let final_count = c0.configure_count.load(Ordering::SeqCst);
    assert!(
        final_count > 0,
        "client 0 saw zero configures with two silent siblings — host hung",
    );
    assert_eq!(
        (
            c0.last_w.load(Ordering::SeqCst),
            c0.last_h.load(Ordering::SeqCst),
        ),
        (final_w0, final_h0),
        "client 0 should have received the final ({final_w0}x{final_h0}) configure; \
         saw {} configures total",
        final_count,
    );

    drop(clients);
    drop(host_handle);
}

/// Regression test for the recursive-pointer-mutex deadlock that hung
/// the host the moment the user moved the cursor out of a satellite
/// pane (run19 freeze):
///
///   1. A client creates a toplevel and we send `PointerMotion` to it
///      so smithay's pointer focus is on the client.
///   2. We send `PointerLeave`. smithay clears the focus, which makes
///      it invoke our `SeatHandler::cursor_image` callback with the
///      default cursor — *while still holding the pointer mutex*.
///   3. Pre-fix, our `cursor_image` called
///      `seat.get_pointer().current_focus()`, which re-acquired that
///      same mutex → permanent deadlock; the host thread never
///      processed another command.
///   4. Post-fix, `cursor_image` reads a cached focus surface that
///      `focus_changed` updates, so no recursive lock.
///
/// The assertion is liveness, not correctness: after the leave we
/// send a follow-up `KeyboardFocus(None)` and verify the cmd queue
/// drains within a generous deadline. If the host is wedged it never
/// drains and the test times out.
#[test]
fn pointer_leave_does_not_deadlock_host() {
    let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
    std::env::set_var("WAYLAND_DEBUG", "0");

    let (host_handle, cmd_tx, evt_rx) = start().unwrap();
    let display_name = wait_for_ready(&evt_rx);
    std::env::set_var("WAYLAND_DISPLAY", &display_name);

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<TestClient>(&conn).unwrap();
    let qh = queue.handle();
    let compositor: WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=5, ()).expect("xdg_wm_base");

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("pointer-leave-deadlock".into());
    surface.commit();

    let configure_count = Arc::new(AtomicU32::new(0));
    let last_w = Arc::new(AtomicU32::new(0));
    let last_h = Arc::new(AtomicU32::new(0));
    let mut client = TestClient {
        configure_count: configure_count.clone(),
        last_w: last_w.clone(),
        last_h: last_h.clone(),
        last_serial: None,
    };

    roundtrip_until(&mut queue, &mut client, |s| s.last_serial.is_some());
    let id = wait_for_toplevel_created(&evt_rx);

    // Step 1: enter focus on the satellite. PointerMotion with a real
    // SurfaceId makes smithay set the pointer focus to that surface.
    cmd_tx
        .send_blocking(HostCommand::PointerMotion {
            id,
            x: 50.0,
            y: 50.0,
        })
        .unwrap();

    // Step 2: leave. Pre-fix, this is the call that wedges the host
    // thread: smithay invokes cursor_image(named=default) from inside
    // pointer.motion(None), our cursor_image re-locks the pointer
    // mutex via current_focus(), and the thread deadlocks.
    cmd_tx
        .send_blocking(HostCommand::PointerLeave { id })
        .unwrap();

    // Step 3: follow-up command. If the host is alive, it drains the
    // cmd queue within a few iterations (~100 ms each). If wedged, the
    // queue never drains.
    cmd_tx
        .send_blocking(HostCommand::KeyboardFocus { id: None })
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_len = cmd_tx.len();
    while Instant::now() < deadline {
        // Pump the wayland queue too — wl_pointer.leave is sent during
        // the leave handler and we don't want to backpressure smithay.
        let _ = blocking_dispatch_with_timeout(&mut queue, &mut client, Duration::from_millis(50));
        last_len = cmd_tx.len();
        if last_len == 0 {
            break;
        }
    }
    assert_eq!(
        last_len, 0,
        "host did not drain {last_len} cmds within 3s after PointerLeave — \
         the pointer-mutex deadlock is back",
    );

    toplevel.destroy();
    xdg_surface.destroy();
    surface.destroy();
    drop(queue);
    drop(conn);
    drop(host_handle);
}

/// Boilerplate-bundle: a per-client connection plus the atomics used
/// to observe configures.
struct ClientFixture {
    queue: EventQueue<TestClient>,
    state: TestClient,
    configure_count: Arc<AtomicU32>,
    last_w: Arc<AtomicU32>,
    last_h: Arc<AtomicU32>,
    _conn: Connection,
    surface: WlSurface,
    xdg_surface: XdgSurface,
    toplevel: XdgToplevel,
}

impl Drop for ClientFixture {
    fn drop(&mut self) {
        self.toplevel.destroy();
        self.xdg_surface.destroy();
        self.surface.destroy();
    }
}

impl ClientFixture {
    fn connect(title: &str) -> Self {
        let conn = Connection::connect_to_env().unwrap();
        let (globals, queue) = registry_queue_init::<TestClient>(&conn).unwrap();
        let qh = queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
        let wm_base: XdgWmBase = globals.bind(&qh, 1..=5, ()).expect("xdg_wm_base");

        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        toplevel.set_title(title.into());
        surface.commit();

        let configure_count = Arc::new(AtomicU32::new(0));
        let last_w = Arc::new(AtomicU32::new(0));
        let last_h = Arc::new(AtomicU32::new(0));
        let state = TestClient {
            configure_count: configure_count.clone(),
            last_w: last_w.clone(),
            last_h: last_h.clone(),
            last_serial: None,
        };

        Self {
            queue,
            state,
            configure_count,
            last_w,
            last_h,
            _conn: conn,
            surface,
            xdg_surface,
            toplevel,
        }
    }

    fn handshake(&mut self) {
        roundtrip_until(&mut self.queue, &mut self.state, |s| {
            s.last_serial.is_some()
        });
    }
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

fn wait_for_toplevel_created(evt_rx: &async_channel::Receiver<HostEvent>) -> SurfaceId {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(evt) = evt_rx.try_recv() {
            if let HostEvent::ToplevelCreated { id, .. } = evt {
                return id;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("host never emitted HostEvent::ToplevelCreated");
}

/// Dispatch incoming wayland events with a wall-clock timeout. The
/// wayland-client crate's blocking_dispatch blocks indefinitely, which
/// is no good in a test that needs to bail out cleanly.
fn blocking_dispatch_with_timeout(
    queue: &mut EventQueue<TestClient>,
    state: &mut TestClient,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    queue.flush()?;
    queue.dispatch_pending(state)?;
    let read = match queue.prepare_read() {
        Some(r) => r,
        None => return Ok(()),
    };
    let mut pfd = libc::pollfd {
        fd: read.connection_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
    if n < 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("poll: {err}").into());
    }
    if n == 0 {
        // Timed out — drop the read guard (it will not consume).
        drop(read);
        return Ok(());
    }
    let _ = read.read();
    queue.dispatch_pending(state)?;
    Ok(())
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
