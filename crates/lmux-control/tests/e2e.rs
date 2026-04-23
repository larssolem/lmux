//! End-to-end test for the control socket — real UnixListener, real
//! `send_request`, dispatcher runs on a worker thread and replies via
//! `async_channel`. Tests are serialised behind a Mutex because they all
//! mutate `XDG_RUNTIME_DIR`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lmux_control::{send_request, spawn_server, AppEvent, Request, Response, PROTOCOL_VERSION};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Point XDG_RUNTIME_DIR at a fresh tmp dir and return it. Caller must hold
/// the env lock — the variable is process-global.
fn setup_runtime_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tid = std::thread::current().id();
    let mut dir = std::env::temp_dir();
    dir.push(format!("lmux-control-e2e-{nanos}-{tid:?}"));
    std::fs::create_dir_all(&dir).expect("create tmp runtime dir");
    // SAFETY: tests hold `env_lock()` while mutating env.
    std::env::set_var("XDG_RUNTIME_DIR", &dir);
    dir
}

#[test]
fn mark_anchor_roundtrip_with_ok_response() {
    let _g = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _dir = setup_runtime_dir();

    let (ev_tx, ev_rx) = async_channel::unbounded::<AppEvent>();
    let _server = spawn_server(move |ev| {
        let _ = ev_tx.send_blocking(ev);
    })
    .expect("server bound");

    // Dispatcher — pretend to be the UI thread. Replies with Ok{pane_id:7}.
    let dispatcher = std::thread::spawn(move || {
        let ev = ev_rx.recv_blocking().expect("event");
        let AppEvent::MarkAnchor { source_pid, reply } = ev;
        assert_eq!(source_pid, 4242);
        reply
            .send_blocking(Response::Ok {
                v: PROTOCOL_VERSION,
                pane_id: Some(7),
            })
            .expect("send reply");
    });

    let resp = send_request(
        &Request::MarkAnchor {
            v: PROTOCOL_VERSION,
            source_pid: 4242,
        },
        Duration::from_secs(2),
    )
    .expect("send_request");

    match resp {
        Response::Ok { pane_id, .. } => assert_eq!(pane_id, Some(7)),
        other => panic!("expected Ok, got {other:?}"),
    }

    dispatcher.join().expect("dispatcher joined");
}

#[test]
fn mark_anchor_error_response_is_surfaced() {
    let _g = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _dir = setup_runtime_dir();

    let (ev_tx, ev_rx) = async_channel::unbounded::<AppEvent>();
    let _server = spawn_server(move |ev| {
        let _ = ev_tx.send_blocking(ev);
    })
    .expect("server bound");

    let dispatcher = std::thread::spawn(move || {
        let AppEvent::MarkAnchor { reply, .. } = ev_rx.recv_blocking().expect("event");
        reply
            .send_blocking(Response::Error {
                v: PROTOCOL_VERSION,
                message: "no pane owns pid".into(),
            })
            .expect("send reply");
    });

    let resp = send_request(
        &Request::MarkAnchor {
            v: PROTOCOL_VERSION,
            source_pid: 1,
        },
        Duration::from_secs(2),
    )
    .expect("send_request");

    match resp {
        Response::Error { message, .. } => assert!(message.contains("no pane")),
        other => panic!("expected Error, got {other:?}"),
    }
    dispatcher.join().expect("dispatcher joined");
}

#[test]
fn connect_to_missing_socket_errors_fast() {
    let _g = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    // Point XDG at a dir we create but don't bind any server in.
    let dir = setup_runtime_dir();
    // Explicitly ensure no socket file exists.
    let sock = dir.join("lmux").join("control.sock");
    let _ = std::fs::remove_file(&sock);

    let start = std::time::Instant::now();
    let err = send_request(
        &Request::MarkAnchor {
            v: PROTOCOL_VERSION,
            source_pid: 1,
        },
        Duration::from_millis(500),
    )
    .expect_err("should fail");
    let elapsed = start.elapsed();

    // Real failure (ECONNREFUSED / ENOENT), not a timeout. Should be very
    // fast — well under our 500 ms budget.
    assert!(
        elapsed < Duration::from_millis(500),
        "connect to missing socket took {elapsed:?}"
    );
    // Any error is acceptable as long as it surfaced.
    let _ = err;
}
