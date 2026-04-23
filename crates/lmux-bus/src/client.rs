//! Cockpit-side bus client.
//!
//! Connects to `$XDG_RUNTIME_DIR/lmux/bus.sock`, performs the `hello`
//! handshake, and offers a synchronous `request(kind) -> Result<Kind, BusError>`
//! helper. One in-flight request at a time (NFR19: the v0.2 bus is strictly
//! request/response with at most one pending request per connection; push
//! notifications via `subscribe` are not yet implemented).

use std::path::{Path, PathBuf};

use serde_json::json;
use tokio::net::UnixStream;
use uuid::Uuid;

use crate::codec::{read_frame, write_frame};
use crate::envelope::{Envelope, PROTOCOL_VERSION};
use crate::error::{BusError, ErrorCode};
use crate::kinds::{ClientRole, Kind};

/// Connected bus client. Drop to close the socket.
pub struct Client {
    stream: UnixStream,
    cockpit_version: String,
}

impl Client {
    /// Connect to `path`, send `hello`, and return the client once the
    /// cockpit has replied with `hello_ack`.
    pub async fn connect(path: &Path, role: ClientRole) -> Result<Self, BusError> {
        let stream = UnixStream::connect(path).await.map_err(BusError::Io)?;
        let mut this = Self {
            stream,
            cockpit_version: String::new(),
        };
        let pid = current_pid();
        let ack = this.request(Kind::Hello { client: role, pid }).await?;
        match ack {
            Kind::HelloAck { cockpit_version } => {
                this.cockpit_version = cockpit_version;
                Ok(this)
            }
            other => Err(BusError::Domain(format!(
                "expected hello_ack, got {other:?}"
            ))),
        }
    }

    /// Convenience: resolve the default bus socket path and connect.
    pub async fn connect_default(role: ClientRole) -> Result<Self, BusError> {
        let path = crate::paths::bus_socket_path()?;
        Self::connect(&path, role).await
    }

    /// Send `kind` and await the response. Errors reported by the cockpit
    /// surface as [`BusError::Domain`] (or the more specific variants where
    /// the wire error code maps cleanly).
    pub async fn request(&mut self, kind: Kind) -> Result<Kind, BusError> {
        let id = Uuid::new_v4();
        let bytes = encode_request(&kind, id)?;
        write_frame(&mut self.stream, &bytes).await?;

        let frame = read_frame(&mut self.stream).await?;
        let env = Envelope::parse(&frame)?;
        if env.id != id {
            return Err(BusError::Domain(format!(
                "response id {} != request id {}",
                env.id, id
            )));
        }
        let response: Kind = serde_json::from_slice(&frame)?;
        if let Kind::Error(ref payload) = response {
            return Err(error_payload_to_bus_error(payload));
        }
        Ok(response)
    }

    /// Cockpit semver reported during handshake.
    pub fn cockpit_version(&self) -> &str {
        &self.cockpit_version
    }
}

fn encode_request(kind: &Kind, id: Uuid) -> Result<Vec<u8>, BusError> {
    let mut value = serde_json::to_value(kind)?;
    let obj = value.as_object_mut().ok_or_else(|| {
        BusError::Domain("request kind did not serialize as a JSON object".into())
    })?;
    obj.insert("v".into(), json!(PROTOCOL_VERSION));
    obj.insert("id".into(), json!(id));
    Ok(serde_json::to_vec(&value)?)
}

fn error_payload_to_bus_error(p: &crate::error::ErrorPayload) -> BusError {
    match p.code {
        ErrorCode::UnknownKind => BusError::UnknownKind {
            code: ErrorCode::UnknownKind,
            kind_received: p
                .kind_received
                .clone()
                .unwrap_or_else(|| "<unknown>".into()),
        },
        ErrorCode::PeerDenied => BusError::PeerDenied {
            code: ErrorCode::PeerDenied,
        },
        ErrorCode::BadRequest => BusError::BadRequest(p.message.clone()),
        _ => BusError::Domain(p.message.clone()),
    }
}

fn current_pid() -> i32 {
    // SAFETY: getpid is always safe.
    unsafe { libc::getpid() }
}

/// Default bus socket path (re-export of [`crate::paths::bus_socket_path`]).
pub fn default_socket() -> Result<PathBuf, BusError> {
    crate::paths::bus_socket_path()
}
