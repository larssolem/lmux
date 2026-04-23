//! End-to-end exercise of the bus server + client. Stands up a fake handler
//! that answers `session.list` with a canned `hello_ack`-shaped response
//! (we reuse `Kind::CompositorStatus` as a signal kind for "it got there"
//! since v0.2 does not yet define a `session.list.ok` response kind in the
//! frozen catalog — response shapes are left to the concrete handler).

use std::sync::Arc;

use async_trait::async_trait;
use lmux_bus::{BusError, Client, ClientRole, Handler, Kind, Server};
use tempfile::tempdir;

struct EchoHandler;

#[async_trait]
impl Handler for EchoHandler {
    async fn handle(&self, req: Kind) -> Result<Kind, BusError> {
        match req {
            Kind::SessionList {} => Ok(Kind::CompositorStatus {
                state: lmux_bus::kinds::CompositorState::Online,
                reason: Some("session_list_stub".into()),
            }),
            Kind::StatusGet {} => Ok(Kind::CompositorStatus {
                state: lmux_bus::kinds::CompositorState::Online,
                reason: None,
            }),
            other => Err(BusError::Domain(format!("unhandled: {other:?}"))),
        }
    }
}

#[tokio::test]
async fn hello_and_session_list_roundtrip() {
    let dir = tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let sock = dir.path().join("bus.sock");
    let pid = dir.path().join("bus.sock.pid");

    let mut server = Server::bind(sock.clone(), pid, Arc::new(EchoHandler))
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));

    let mut client = Client::connect(&sock, ClientRole::LmuxCli)
        .await
        .unwrap_or_else(|e| panic!("connect: {e}"));
    assert!(!client.cockpit_version().is_empty());

    let resp = client
        .request(Kind::SessionList {})
        .await
        .unwrap_or_else(|e| panic!("request: {e}"));
    match resp {
        Kind::CompositorStatus { reason, .. } => {
            assert_eq!(reason.as_deref(), Some("session_list_stub"));
        }
        other => panic!("unexpected: {other:?}"),
    }

    server.shutdown().await;
}

#[tokio::test]
async fn unknown_kind_yields_structured_error() {
    let dir = tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let sock = dir.path().join("bus.sock");
    let pid = dir.path().join("bus.sock.pid");

    let mut server = Server::bind(sock.clone(), pid, Arc::new(EchoHandler))
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));

    // Manually craft a frame with an unknown kind to bypass the client's
    // typed API.
    use tokio::net::UnixStream;
    let mut stream = UnixStream::connect(&sock)
        .await
        .unwrap_or_else(|e| panic!("connect: {e}"));

    let body = br#"{"v":2,"id":"00000000-0000-0000-0000-000000000001","kind":"session.teleport"}"#;
    lmux_bus::write_frame(&mut stream, body)
        .await
        .unwrap_or_else(|e| panic!("write: {e}"));
    let frame = lmux_bus::read_frame(&mut stream)
        .await
        .unwrap_or_else(|e| panic!("read: {e}"));
    let resp: serde_json::Value =
        serde_json::from_slice(&frame).unwrap_or_else(|e| panic!("parse: {e}"));
    assert_eq!(resp["kind"], "error");
    assert_eq!(resp["code"], "unknown_kind");
    assert_eq!(resp["kind_received"], "session.teleport");

    server.shutdown().await;
}

#[tokio::test]
async fn stale_socket_is_reclaimed() {
    let dir = tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let sock = dir.path().join("bus.sock");
    let pid = dir.path().join("bus.sock.pid");

    // Simulate stale leftovers from a dead cockpit: socket file present,
    // pid file points to pid 1 (almost certainly init; but we write a
    // definitely-dead sentinel 0 so reclaim unconditionally succeeds).
    std::fs::write(&sock, b"").unwrap_or_else(|e| panic!("touch: {e}"));
    std::fs::write(&pid, b"0").unwrap_or_else(|e| panic!("touch pid: {e}"));

    let mut server = Server::bind(sock.clone(), pid.clone(), Arc::new(EchoHandler))
        .await
        .unwrap_or_else(|e| panic!("bind after stale: {e}"));

    let mut client = Client::connect(&sock, ClientRole::LmuxCli)
        .await
        .unwrap_or_else(|e| panic!("connect: {e}"));
    let _ = client
        .request(Kind::StatusGet {})
        .await
        .unwrap_or_else(|e| panic!("status: {e}"));
    server.shutdown().await;
}
