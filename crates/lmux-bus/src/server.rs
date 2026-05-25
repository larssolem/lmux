//! Cockpit-side bus server.
//!
//! * Owns `$XDG_RUNTIME_DIR/lmux/bus.sock` and the sidecar `bus.sock.pid`.
//! * On bind, reclaims a stale socket when the recorded PID is dead
//!   (ADR-0015 §Stale-socket recovery).
//! * Validates every incoming connection with `SO_PEERCRED` (UID must
//!   match the cockpit UID — NFR20, FR59 peer auth).
//! * Runs the envelope + kind parser for each frame, dispatches to a
//!   user-supplied [`Handler`], and writes responses back.
//!
//! The server is generic over the handler so unit tests can stand it up
//! against a fake.

use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::codec::{read_frame, write_frame};
use crate::envelope::{Envelope, PROTOCOL_VERSION};
use crate::error::{BusError, ErrorCode, ErrorPayload};
use crate::kinds::Kind;

/// Handler for cockpit-side request dispatch.
///
/// Handlers return a [`Kind`] which the server wraps back into an envelope
/// carrying the original request id. For errors the handler should return
/// `Err(BusError::...)`; the server converts it into a wire-format error
/// envelope.
#[async_trait::async_trait]
pub trait Handler: Send + Sync + 'static {
    /// Handle one request. The server has already validated `v` and kind.
    async fn handle(&self, req: Kind) -> Result<Kind, BusError>;

    /// Cockpit semver reported to clients on handshake.
    fn cockpit_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// Running server handle. Drop to unbind; the socket + pid file are removed
/// on clean shutdown via [`Server::shutdown`].
pub struct Server {
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<()>>,
    socket_path: PathBuf,
    pid_path: PathBuf,
}

impl Server {
    /// Bind, start accepting, and route each request through `handler`.
    /// Stale `bus.sock` files whose recorded pid is dead are reclaimed.
    pub async fn bind<H: Handler>(
        socket_path: PathBuf,
        pid_path: PathBuf,
        handler: Arc<H>,
    ) -> Result<Self, BusError> {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).map_err(BusError::Io)?;
        }

        reclaim_stale(&socket_path, &pid_path)?;

        let listener = UnixListener::bind(&socket_path).map_err(BusError::Io)?;
        set_mode_0600(&socket_path)?;
        write_pid_file(&pid_path)?;

        let cockpit_uid = current_uid();
        let (tx, rx) = oneshot::channel::<()>();
        let socket_for_cleanup = socket_path.clone();
        let pid_for_cleanup = pid_path.clone();

        let join = tokio::spawn(accept_loop(
            listener,
            handler,
            cockpit_uid,
            rx,
            socket_for_cleanup,
            pid_for_cleanup,
        ));

        info!(path = %socket_path.display(), "lmux-bus: listening");

        Ok(Self {
            shutdown: Some(tx),
            join: Some(join),
            socket_path,
            pid_path,
        })
    }

    /// Graceful shutdown: stop the accept loop, join, remove the socket
    /// + pid file. Safe to call multiple times.
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.pid_path);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Best-effort cleanup if the caller forgot to await shutdown.
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.pid_path);
    }
}

async fn accept_loop<H: Handler>(
    listener: UnixListener,
    handler: Arc<H>,
    cockpit_uid: u32,
    mut shutdown_rx: oneshot::Receiver<()>,
    _socket_path: PathBuf,
    _pid_path: PathBuf,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                debug!("lmux-bus: shutdown signalled");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let handler = Arc::clone(&handler);
                        tokio::spawn(async move {
                            if let Err(err) = serve_connection(stream, handler, cockpit_uid).await {
                                debug!(error = %err, "lmux-bus: connection terminated with error");
                            }
                        });
                    }
                    Err(err) => {
                        warn!(error = %err, "lmux-bus: accept failed");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
}

async fn serve_connection<H: Handler>(
    mut stream: UnixStream,
    handler: Arc<H>,
    cockpit_uid: u32,
) -> Result<(), BusError> {
    // NFR20 / FR59: check peer UID.
    let peer_uid = so_peercred_uid(&stream)?;
    if peer_uid != cockpit_uid {
        let payload = ErrorPayload {
            code: ErrorCode::PeerDenied,
            message: format!("peer uid {peer_uid} != cockpit uid {cockpit_uid}"),
            kind_received: None,
            in_reply_to: None,
        };
        let _ = write_error(&mut stream, Uuid::nil(), payload).await;
        return Ok(());
    }

    let mut handshake_complete = false;
    loop {
        let frame = match read_frame(&mut stream).await {
            Ok(f) => f,
            Err(BusError::Io(err)) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => return Err(err),
        };

        let envelope = match Envelope::parse(&frame) {
            Ok(env) => env,
            Err(err) => {
                let payload = ErrorPayload {
                    code: err.code(),
                    message: err.to_string(),
                    kind_received: None,
                    in_reply_to: None,
                };
                write_error(&mut stream, Uuid::nil(), payload).await?;
                continue;
            }
        };

        if !handshake_complete && envelope.kind != "hello" {
            let payload = ErrorPayload {
                code: ErrorCode::BadRequest,
                message: "hello handshake required before other requests".into(),
                kind_received: Some(envelope.kind.clone()),
                in_reply_to: Some(envelope.id),
            };
            write_error(&mut stream, envelope.id, payload).await?;
            continue;
        }

        // Try to parse the full kind; unknown-kind surfaces as serde error,
        // which we map back to `error.unknown_kind` carrying the received
        // tag so clients can introspect.
        let kind = match serde_json::from_slice::<Kind>(&frame) {
            Ok(k) => k,
            Err(err) => {
                let payload = ErrorPayload {
                    code: ErrorCode::UnknownKind,
                    message: err.to_string(),
                    kind_received: Some(envelope.kind.clone()),
                    in_reply_to: Some(envelope.id),
                };
                write_error(&mut stream, envelope.id, payload).await?;
                continue;
            }
        };

        let response_kind = match &kind {
            Kind::Hello { client, pid } => {
                debug!(?client, pid, "lmux-bus: hello");
                handshake_complete = true;
                Kind::HelloAck {
                    cockpit_version: handler.cockpit_version(),
                }
            }
            _ => match handler.handle(kind).await {
                Ok(resp) => resp,
                Err(err) => {
                    let payload = ErrorPayload {
                        code: err.code(),
                        message: err.to_string(),
                        kind_received: None,
                        in_reply_to: Some(envelope.id),
                    };
                    write_error(&mut stream, envelope.id, payload).await?;
                    continue;
                }
            },
        };

        write_response(&mut stream, envelope.id, &response_kind).await?;
    }
}

async fn write_response(
    stream: &mut UnixStream,
    in_reply_to: Uuid,
    kind: &Kind,
) -> Result<(), BusError> {
    let mut value = serde_json::to_value(kind)?;
    let obj = value.as_object_mut().ok_or_else(|| {
        BusError::Domain("response kind did not serialize as a JSON object".into())
    })?;
    obj.insert("v".into(), json!(PROTOCOL_VERSION));
    obj.insert("id".into(), json!(in_reply_to));
    let bytes = serde_json::to_vec(&value)?;
    write_frame(stream, &bytes).await
}

async fn write_error(
    stream: &mut UnixStream,
    in_reply_to: Uuid,
    payload: ErrorPayload,
) -> Result<(), BusError> {
    let kind = Kind::Error(payload);
    write_response(stream, in_reply_to, &kind).await
}

fn reclaim_stale(socket_path: &Path, pid_path: &Path) -> Result<(), BusError> {
    if !socket_path.exists() {
        // No socket to reclaim.
        return Ok(());
    }
    match std::fs::read_to_string(pid_path) {
        Ok(s) => {
            let pid: i32 = s.trim().parse().unwrap_or(0);
            if pid > 0 && pid_is_alive(pid) {
                return Err(BusError::Domain(format!(
                    "another cockpit appears to be running (pid {pid})"
                )));
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(BusError::Io(err)),
    }
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(pid_path);
    info!("lmux-bus: reclaimed stale bus.sock");
    Ok(())
}

fn pid_is_alive(pid: i32) -> bool {
    // SAFETY: `kill` with signal 0 is defined and only tests for process
    // existence; it does not modify any process.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn write_pid_file(pid_path: &Path) -> Result<(), BusError> {
    // SAFETY: getpid is always safe to call.
    let pid = unsafe { libc::getpid() };
    std::fs::write(pid_path, pid.to_string()).map_err(BusError::Io)
}

fn set_mode_0600(path: &Path) -> Result<(), BusError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(BusError::Io)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(BusError::Io)
}

fn current_uid() -> u32 {
    // SAFETY: getuid is always safe.
    unsafe { libc::getuid() }
}

fn so_peercred_uid(stream: &UnixStream) -> Result<u32, BusError> {
    #[cfg(target_os = "macos")]
    {
        let mut euid: libc::uid_t = 0;
        let mut egid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut euid, &mut egid) };
        if rc != 0 {
            return Err(BusError::Io(io::Error::last_os_error()));
        }
        return Ok(euid as u32);
    }
    #[cfg(target_os = "linux")]
    {
        let fd = stream.as_raw_fd();
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len: libc::socklen_t = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: getsockopt on a connected Unix socket for SO_PEERCRED is
        // well-defined; `cred` + `len` point to correctly-sized storage.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(BusError::Io(io::Error::last_os_error()));
        }
        Ok(cred.uid as u32)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = stream;
        Ok(current_uid())
    }
}

/// Helper type: a [`Handler`] that returns `error.unknown_kind` for every
/// request. Useful as a default in the test harness; real cockpits wire up
/// a handler backed by their domain layer.
#[derive(Default)]
pub struct RejectAllHandler;

#[async_trait::async_trait]
impl Handler for RejectAllHandler {
    async fn handle(&self, req: Kind) -> Result<Kind, BusError> {
        Err(BusError::Domain(format!("no handler for {req:?}")))
    }
}
