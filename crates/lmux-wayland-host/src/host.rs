//! Compositor event loop. Runs on its own OS thread (spawned by
//! [`crate::start`]); owns the smithay `Display` + the calloop
//! `EventLoop` + the listening wayland socket.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};
use smithay::reexports::wayland_server::{Display, ListeningSocket};

use crate::state::{ClientCompositorState, State};
use crate::{Error, HostCommand, HostEvent};

/// Owned join-handle for the compositor thread. Dropping it signals the
/// compositor to shut down and waits for the thread to exit.
pub struct HostHandle {
    pub(crate) join: Option<JoinHandle<()>>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) cmd_tx: Sender<HostCommand>,
}

impl HostHandle {
    /// Request shutdown without blocking. The thread posts
    /// [`HostEvent::Stopped`] when it's actually gone.
    pub fn request_shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.cmd_tx.send_blocking(HostCommand::Shutdown);
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

/// Thread entrypoint — set up the wayland display + socket, then run the
/// calloop event loop until the stop flag flips.
pub(crate) fn run(
    cmd_rx: Receiver<HostCommand>,
    evt_tx: Sender<HostEvent>,
    stop: Arc<AtomicBool>,
) -> Result<(), Error> {
    let mut display =
        Display::<State>::new().map_err(|e| Error::EventLoopInit(format!("Display::new: {e}")))?;
    let dh = display.handle();

    // Bind a named socket under XDG_RUNTIME_DIR. We use `lmux-<pid>-<n>`
    // so the name is unique per cockpit instance AND per `start()` call
    // within the same process — the latter matters for tests that spin
    // up several hosts in parallel.
    static HOST_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = HOST_SEQ.fetch_add(1, Ordering::Relaxed);
    let socket_name = format!("lmux-{}-{}", std::process::id(), seq);
    let listening = ListeningSocket::bind(&socket_name).map_err(|e| {
        Error::SocketBind(
            socket_name.clone().into(),
            std::io::Error::other(e.to_string()),
        )
    })?;
    tracing::info!(socket = %socket_name, "lmux-wayland-host: socket bound");

    let mut event_loop: EventLoop<'_, State> = EventLoop::try_new()
        .map_err(|e| Error::EventLoopInit(format!("EventLoop::try_new: {e}")))?;

    // Register the listening socket: every incoming connection gets
    // inserted into the Display as a new client.
    let loop_handle = event_loop.handle();
    let mut dh_for_listen = dh.clone();
    loop_handle
        .insert_source(
            smithay::reexports::calloop::generic::Generic::new(
                listening,
                Interest::READ,
                Mode::Level,
            ),
            move |_, listening, _state: &mut State| {
                loop {
                    match listening.accept() {
                        Ok(Some(stream)) => {
                            let client_data = Arc::new(ClientCompositorState::default());
                            match dh_for_listen.insert_client(stream, client_data) {
                                Ok(_) => tracing::debug!("lmux-wayland-host: client connected"),
                                Err(err) => tracing::warn!(
                                    error = %err,
                                    "lmux-wayland-host: insert_client failed"
                                ),
                            }
                        }
                        Ok(None) => break,
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                "lmux-wayland-host: accept() failed"
                            );
                            break;
                        }
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| Error::EventLoopInit(format!("insert_source(listening): {e}")))?;

    let mut state = State::new(dh.clone(), evt_tx.clone());

    // Signal readiness once the loop is wired up. Clients that wait on
    // this event can now safely set WAYLAND_DISPLAY.
    let _ = evt_tx.send_blocking(HostEvent::Ready {
        display_name: socket_name.clone(),
    });

    // Main dispatch loop. Tear down on either the shared stop flag OR the
    // HostCommand::Shutdown variant. The 100 ms poll is a floor on
    // shutdown latency; protocol dispatch continues to fire at wire
    // speed because the wayland socket is a calloop Generic source.
    let mut last_heartbeat = std::time::Instant::now();
    // Stage tracking: an external watchdog thread reads these atomics every
    // second and screams if `stage` hasn't changed for too long. That's
    // the only way to identify a hang inside dispatch_clients / event_loop
    // dispatch, since the host thread itself can't print anything once
    // it's blocked inside smithay.
    const STAGE_IDLE: u8 = 0;
    const STAGE_DISPATCH_CLIENTS: u8 = 1;
    const STAGE_REAP: u8 = 2;
    const STAGE_FLUSH: u8 = 3;
    const STAGE_EVENT_LOOP: u8 = 4;
    const STAGE_DRAIN: u8 = 5;
    let stage = Arc::new(AtomicU8::new(STAGE_IDLE));
    let stage_started_ms = Arc::new(AtomicU64::new(0));
    let watchdog_stop = stop.clone();
    let watchdog_stage = stage.clone();
    let watchdog_started = stage_started_ms.clone();
    let host_start = std::time::Instant::now();
    std::thread::Builder::new()
        .name("lmux-wayland-host-watchdog".into())
        .spawn(move || {
            let names = [
                "idle",
                "dispatch_clients",
                "reap",
                "flush",
                "event_loop_dispatch",
                "drain",
            ];
            // Track which stage-entry we last warned for, plus a
            // re-arm timer so a sustained freeze keeps producing warns
            // (every ~2 s) instead of only firing once. Without this
            // the log can't tell us how long the host was actually
            // wedged — only that it crossed the 1 s threshold once.
            let mut last_warn_started: u64 = 0;
            let mut last_warn_now: u64 = 0;
            while !watchdog_stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(500));
                let s = watchdog_stage.load(Ordering::Relaxed) as usize;
                let started = watchdog_started.load(Ordering::Relaxed);
                let now = host_start.elapsed().as_millis() as u64;
                if started == 0 || now.saturating_sub(started) < 1000 {
                    continue;
                }
                let new_stage_entry = started != last_warn_started;
                let rearmed = now.saturating_sub(last_warn_now) >= 2000;
                if new_stage_entry || rearmed {
                    tracing::warn!(
                        stage = names.get(s).copied().unwrap_or("?"),
                        stuck_ms = now.saturating_sub(started),
                        "watchdog: host stage stuck",
                    );
                    last_warn_started = started;
                    last_warn_now = now;
                }
            }
        })
        .ok();

    let mark = |which: u8| {
        let now = host_start.elapsed().as_millis() as u64;
        stage_started_ms.store(now, Ordering::Relaxed);
        stage.store(which, Ordering::Relaxed);
    };

    // Per-stage duration logging: any stage that exceeds this floor
    // gets a `host: stage slow` warn so we can correlate cockpit
    // stalls with whatever was actually running. The floor is well
    // above normal (`event_loop.dispatch` has a 100 ms timeout, the
    // others normally complete in <1 ms).
    let slow_floor = Duration::from_millis(150);
    // Capture the host thread's gettid so a user observing a freeze
    // in the cockpit can attach `eu-stack -p <pid> -t <tid>` from
    // another shell. tracing only logs this once at startup.
    let host_tid = unsafe { libc::syscall(libc::SYS_gettid) } as i64;
    tracing::info!(
        host_tid,
        "lmux-wayland-host: thread tid (use `eu-stack -p $(pidof lmux) -t {host_tid}` to debug freezes)",
    );

    while !stop.load(Ordering::SeqCst) {
        if last_heartbeat.elapsed() >= Duration::from_secs(2) {
            tracing::debug!("host: loop heartbeat");
            last_heartbeat = std::time::Instant::now();
        }
        let stage_start = std::time::Instant::now();
        mark(STAGE_DISPATCH_CLIENTS);
        display
            .dispatch_clients(&mut state)
            .map_err(|e| Error::EventLoopInit(format!("dispatch_clients: {e}")))?;
        let dispatch_clients_dur = stage_start.elapsed();
        if dispatch_clients_dur >= slow_floor {
            tracing::warn!(
                stage = "dispatch_clients",
                dur_ms = dispatch_clients_dur.as_millis() as u64,
                "host: stage slow",
            );
        }

        let reap_start = std::time::Instant::now();
        mark(STAGE_REAP);
        // After each dispatch, reap toplevels whose client has
        // disconnected without cleanly destroying their xdg_toplevel —
        // otherwise the cockpit's pane lingers as a "white window".
        state.reap_dead_toplevels();
        // Same reasoning for popups: menus/dropdowns whose parent closed
        // or whose client crashed without a clean destroy would otherwise
        // stay as stale overlay widgets.
        state.reap_dead_popups();
        let reap_dur = reap_start.elapsed();
        if reap_dur >= slow_floor {
            tracing::warn!(
                stage = "reap",
                dur_ms = reap_dur.as_millis() as u64,
                "host: stage slow",
            );
        }

        let flush_start = std::time::Instant::now();
        mark(STAGE_FLUSH);
        display.flush_clients().ok();
        let flush_dur = flush_start.elapsed();
        if flush_dur >= slow_floor {
            tracing::warn!(
                stage = "flush_clients",
                dur_ms = flush_dur.as_millis() as u64,
                "host: stage slow",
            );
        }

        let evt_start = std::time::Instant::now();
        mark(STAGE_EVENT_LOOP);
        event_loop
            .dispatch(Some(Duration::from_millis(100)), &mut state)
            .map_err(|e| Error::EventLoopInit(format!("event_loop.dispatch: {e}")))?;
        let evt_dur = evt_start.elapsed();
        // event_loop.dispatch has a 100 ms timeout, so anything beyond
        // ~120 ms means a calloop callback ran long. This is the most
        // useful data point for the freeze hunt — log all violators.
        if evt_dur >= slow_floor {
            tracing::warn!(
                stage = "event_loop_dispatch",
                dur_ms = evt_dur.as_millis() as u64,
                "host: stage slow",
            );
        }
        mark(STAGE_DRAIN);

        // Drain any pending commands (non-blocking). Follow-up tasks
        // will add pointer/keyboard variants here.
        let queued = cmd_rx.len();
        if queued > 0 {
            tracing::debug!(queued, "host: draining cmd_rx");
        }
        while let Ok(cmd) = cmd_rx.try_recv() {
            let cmd_start = std::time::Instant::now();
            let cmd_label: &'static str = match &cmd {
                HostCommand::Shutdown => "shutdown",
                HostCommand::ResizeToplevel { .. } => "resize_toplevel",
                HostCommand::CloseToplevel { .. } => "close_toplevel",
                HostCommand::PointerButton { .. } => "pointer_button",
                HostCommand::KeyboardFocus { .. } => "keyboard_focus",
                HostCommand::PointerMotion { .. } => "pointer_motion",
                HostCommand::PointerLeave { .. } => "pointer_leave",
                HostCommand::PointerAxis { .. } => "pointer_axis",
                HostCommand::KeyInput { .. } => "key_input",
            };
            // Re-mark the drain stage timestamp per command so the
            // watchdog can distinguish a hung handler from a genuinely
            // long batch of commands.
            mark(STAGE_DRAIN);
            // Entry log for EVERY command so the next freeze tells us
            // exactly which handler hangs. Without this, silent variants
            // (PointerMotion etc.) leave a black hole in the log.
            tracing::trace!(cmd = cmd_label, "host: cmd entry");
            match cmd {
                HostCommand::Shutdown => {
                    stop.store(true, Ordering::SeqCst);
                    break;
                }
                HostCommand::ResizeToplevel { id, width, height } => {
                    tracing::debug!(?id, width, height, "host: ResizeToplevel");
                    state.resize_toplevel(id, width, height);
                }
                HostCommand::CloseToplevel { id } => {
                    tracing::debug!(?id, "host: CloseToplevel");
                    state.close_toplevel(id);
                }
                HostCommand::PointerButton {
                    id,
                    button,
                    pressed,
                } => {
                    tracing::debug!(?id, button, pressed, "host: PointerButton");
                    state.pointer_button(id, button, pressed);
                }
                HostCommand::KeyboardFocus { id } => {
                    tracing::debug!(?id, "host: KeyboardFocus");
                    state.keyboard_focus(id);
                }
                HostCommand::PointerMotion { id, x, y } => {
                    tracing::trace!(?id, x, y, "host: PointerMotion");
                    state.pointer_motion(id, x, y);
                }
                HostCommand::PointerLeave { id } => {
                    tracing::debug!(?id, "host: PointerLeave");
                    state.pointer_leave(id);
                }
                HostCommand::PointerAxis { id, dx, dy } => {
                    tracing::trace!(?id, dx, dy, "host: PointerAxis");
                    state.pointer_axis(id, dx, dy);
                }
                HostCommand::KeyInput {
                    id,
                    evdev_code,
                    pressed,
                } => {
                    tracing::debug!(?id, evdev_code, pressed, "host: KeyInput");
                    state.key_input(id, evdev_code, pressed);
                }
            }
            let cmd_dur = cmd_start.elapsed();
            if cmd_dur >= Duration::from_millis(100) {
                tracing::warn!(
                    cmd = cmd_label,
                    dur_ms = cmd_dur.as_millis() as u64,
                    "host: command handler slow",
                );
            }
        }
    }

    tracing::info!("lmux-wayland-host: event loop exiting cleanly");
    Ok(())
}
