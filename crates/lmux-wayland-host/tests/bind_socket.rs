//! Smoke test: the host binds a wayland socket under `$XDG_RUNTIME_DIR`,
//! emits `HostEvent::Ready`, and tears down cleanly when the handle is
//! dropped. Doesn't connect a real wayland-client yet — protocol-level
//! assertions ride the xdg_shell task (Task #8) where handlers exist.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::time::Duration;

use lmux_wayland_host::{start, HostEvent};

#[test]
fn bind_socket_and_emit_ready() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Smithay reads XDG_RUNTIME_DIR when binding; point it at the tmp so
    // the test doesn't pollute the developer's real session.
    std::env::set_var("XDG_RUNTIME_DIR", tmp.path());

    let (handle, _cmd_tx, evt_rx) = start().expect("host start");

    // Wait up to 2s for the Ready event — the compositor thread only
    // sends it after the socket is listening.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut got_ready = false;
    while std::time::Instant::now() < deadline {
        if let Ok(evt) = evt_rx.try_recv() {
            if matches!(evt, HostEvent::Ready { .. }) {
                got_ready = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(got_ready, "host never emitted HostEvent::Ready");

    // Socket file should exist on disk under the tmp runtime dir.
    let pid = std::process::id();
    let expected = tmp.path().join(format!("lmux-{pid}"));
    assert!(
        expected.exists(),
        "socket file not found at {}",
        expected.display()
    );

    // Dropping the handle posts Shutdown and joins the thread.
    drop(handle);

    // After shutdown we should receive Stopped. Give it a moment to
    // unwind — the join already blocked, so this should be immediate.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let mut got_stopped = false;
    while std::time::Instant::now() < deadline {
        if let Ok(evt) = evt_rx.try_recv() {
            if matches!(evt, HostEvent::Stopped) {
                got_stopped = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(got_stopped, "host never emitted HostEvent::Stopped");
}
