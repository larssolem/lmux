//! lmux control socket (Unix domain, length-prefixed JSON).
//!
//! Shared between the lmux main binary (which hosts the server) and the
//! `lmux-cli` client. The server thread reads framed JSON requests, hands
//! them to the UI thread via an `async_channel::Sender<AppEvent>`, waits for
//! the matching `Response` on a per-request channel, and writes it back on
//! the same connection.

use std::io::{self};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME_BYTES: u32 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("XDG_RUNTIME_DIR not set")]
    NoRuntimeDir,
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(u32),
    #[error("server replied with error: {0}")]
    ServerError(String),
    #[error("timed out waiting for server reply")]
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Request {
    MarkAnchor { v: u32, source_pid: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Response {
    Ok { v: u32, pane_id: Option<u32> },
    Error { v: u32, message: String },
}

/// Events the server emits onto the UI thread. `reply` is a one-shot
/// channel — the UI handler must send exactly one `Response`.
#[derive(Debug)]
pub enum AppEvent {
    MarkAnchor {
        source_pid: u32,
        reply: async_channel::Sender<Response>,
    },
}

/// Resolve the control socket path — `$XDG_RUNTIME_DIR/lmux/control.sock`.
pub fn socket_path() -> Result<PathBuf, Error> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or(Error::NoRuntimeDir)?;
    let mut p = PathBuf::from(dir);
    p.push("lmux");
    p.push("control.sock");
    Ok(p)
}

/// Ensure `$XDG_RUNTIME_DIR/lmux/` exists with mode 0700 and unlink a stale
/// socket file if present. Returns the socket path.
fn prepare_socket_dir() -> Result<PathBuf, Error> {
    let path = socket_path()?;
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(parent)?;
    }
    if path.exists() {
        std::fs::remove_file(&path).ok();
    }
    Ok(path)
}

/// Spawn the server thread. `dispatch` is invoked on the server thread
/// (NOT the UI thread) — callers should pass an `async_channel::Sender`
/// and let the UI thread consume on the main context.
pub fn spawn_server<F>(dispatch: F) -> Result<ServerHandle, Error>
where
    F: Fn(AppEvent) + Send + Sync + 'static,
{
    let path = prepare_socket_dir()?;
    let listener = UnixListener::bind(&path)?;
    tracing::info!(?path, "control socket bound");
    let self_uid = unsafe { libc::getuid() };
    let handle = ServerHandle { path: path.clone() };
    thread::Builder::new()
        .name("lmux-control".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        if let Err(err) = handle_conn(stream, self_uid, &dispatch) {
                            tracing::warn!(error = %err, "control conn failed");
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "accept failed");
                    }
                }
            }
        })?;
    Ok(handle)
}

pub struct ServerHandle {
    path: PathBuf,
}

impl ServerHandle {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

fn handle_conn<F>(mut stream: UnixStream, self_uid: u32, dispatch: &F) -> Result<(), Error>
where
    F: Fn(AppEvent),
{
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let peer_uid = peer_uid(&stream)?;
    if peer_uid != self_uid {
        let _ = write_frame(
            &mut stream,
            &Response::Error {
                v: PROTOCOL_VERSION,
                message: format!("peer uid {peer_uid} rejected"),
            },
        );
        return Ok(());
    }
    let req: Request = match read_frame(&mut stream) {
        Ok(r) => r,
        Err(err) => {
            let _ = write_frame(
                &mut stream,
                &Response::Error {
                    v: PROTOCOL_VERSION,
                    message: err.to_string(),
                },
            );
            return Ok(());
        }
    };
    let response = match req {
        Request::MarkAnchor { v: _, source_pid } => {
            let (reply_tx, reply_rx) = async_channel::bounded::<Response>(1);
            dispatch(AppEvent::MarkAnchor {
                source_pid,
                reply: reply_tx,
            });
            reply_rx.recv_blocking().unwrap_or(Response::Error {
                v: PROTOCOL_VERSION,
                message: "ui thread closed before replying".into(),
            })
        }
    };
    write_frame(&mut stream, &response)?;
    Ok(())
}

fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    #[repr(C)]
    struct Ucred {
        pid: libc::pid_t,
        uid: libc::uid_t,
        gid: libc::gid_t,
    }
    let mut cred: Ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<Ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(cred.uid as u32)
}

fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T, Error> {
    read_frame_from(stream)
}

/// Generic read-frame used by `read_frame` and by the codec unit tests.
fn read_frame_from<R: io::Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> Result<T, Error> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(Error::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), Error> {
    write_frame_to(stream, value)
}

fn write_frame_to<W: io::Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), Error> {
    let body = serde_json::to_vec(value)?;
    let len = u32::try_from(body.len()).map_err(|_| Error::FrameTooLarge(u32::MAX))?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Connect to a running lmux server, send `request`, and wait for the
/// response. `connect_timeout` guards against an unresponsive or busy main
/// thread (Story 5.2, FR38).
pub fn send_request(request: &Request, connect_timeout: Duration) -> Result<Response, Error> {
    let path = socket_path()?;
    let stream = connect_with_timeout(&path, connect_timeout)?;
    send_on_stream(stream, request, connect_timeout)
}

fn connect_with_timeout(path: &Path, timeout: Duration) -> Result<UnixStream, Error> {
    // std::os::unix::net::UnixStream doesn't expose a connect-with-timeout
    // constructor. We emulate one by performing the connect on a worker
    // thread and waiting with `recv_timeout`.
    use std::sync::mpsc::{channel, RecvTimeoutError};
    let (tx, rx) = channel::<io::Result<UnixStream>>();
    let path_owned = path.to_path_buf();
    thread::spawn(move || {
        let res = UnixStream::connect(&path_owned);
        let _ = tx.send(res);
    });
    let stream = match rx.recv_timeout(timeout) {
        Ok(res) => res?,
        Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout),
        // Worker thread dropped without sending — treat as timeout.
        Err(RecvTimeoutError::Disconnected) => return Err(Error::Timeout),
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    Ok(stream)
}

fn send_on_stream(
    mut stream: UnixStream,
    request: &Request,
    timeout: Duration,
) -> Result<Response, Error> {
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write_frame(&mut stream, request)?;
    read_frame(&mut stream)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn request_roundtrip_through_frame_codec() {
        let req = Request::MarkAnchor {
            v: PROTOCOL_VERSION,
            source_pid: 1234,
        };
        let mut buf: Vec<u8> = Vec::new();
        write_frame_to(&mut buf, &req).expect("write");
        let mut cur = Cursor::new(buf);
        let back: Request = read_frame_from(&mut cur).expect("read");
        match back {
            Request::MarkAnchor { v, source_pid } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(source_pid, 1234);
            }
        }
    }

    #[test]
    fn response_error_roundtrip() {
        let resp = Response::Error {
            v: PROTOCOL_VERSION,
            message: "hello".into(),
        };
        let mut buf: Vec<u8> = Vec::new();
        write_frame_to(&mut buf, &resp).expect("write");
        let mut cur = Cursor::new(buf);
        let back: Response = read_frame_from(&mut cur).expect("read");
        match back {
            Response::Error { message, .. } => assert_eq!(message, "hello"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn oversize_length_prefix_is_rejected() {
        // Craft a frame whose len prefix says 128 MiB. read_frame must
        // reject it before trying to allocate.
        let huge: u32 = 128 * 1024 * 1024;
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&huge.to_be_bytes());
        // No body — we shouldn't get that far.
        let mut cur = Cursor::new(buf);
        let err = read_frame_from::<_, Request>(&mut cur).expect_err("should reject");
        match err {
            Error::FrameTooLarge(n) => assert_eq!(n, huge),
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn truncated_body_surfaces_as_io_error() {
        // Len prefix promises 16 bytes, body is 4. `read_exact` fails.
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&16u32.to_be_bytes());
        buf.extend_from_slice(b"{\"v\"");
        let mut cur = Cursor::new(buf);
        let err = read_frame_from::<_, Request>(&mut cur).expect_err("should fail");
        assert!(matches!(err, Error::Io(_)), "got {err:?}");
    }

    #[test]
    fn malformed_json_surfaces_as_json_error() {
        let body = b"{not json}";
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
        buf.extend_from_slice(body);
        let mut cur = Cursor::new(buf);
        let err = read_frame_from::<_, Request>(&mut cur).expect_err("should fail");
        assert!(matches!(err, Error::Json(_)), "got {err:?}");
    }

    #[test]
    fn socket_path_uses_xdg_runtime_dir_when_set() {
        let lock = std::sync::Mutex::new(());
        let _g = lock.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp/xdg-test-dir");
        let got = socket_path().expect("ok");
        assert_eq!(
            got,
            std::path::PathBuf::from("/tmp/xdg-test-dir/lmux/control.sock")
        );
        match prev {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }
}
