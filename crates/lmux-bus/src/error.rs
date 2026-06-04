//! Bus error envelope per ADR-0015 §Error Envelope.
//!
//! Error codes are a closed enum; adding a new user-surfaced error requires
//! adding a variant here (NOT just a string), so wire-format breakage is caught
//! by the compiler.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Closed set of protocol-level error codes emitted on the bus.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Frame exceeded `MAX_FRAME_BYTES`.
    FrameTooLarge,
    /// Envelope carried a protocol version this cockpit does not speak.
    VersionMismatch,
    /// Envelope `kind` was not in the known set (see ADR-0016).
    UnknownKind,
    /// SO_PEERCRED reported a UID that does not match the cockpit UID.
    PeerDenied,
    /// Request payload failed schema validation.
    BadRequest,
    /// Handler rejected the operation due to domain-level state (e.g.
    /// session not found). Carries a human-readable message.
    Domain,
    /// Requester is not authorized to perform the operation.
    Unauthorized,
    /// User denied or revoked the grant needed for the operation.
    GrantDenied,
    /// Requested pane has no transcript surface.
    TranscriptUnavailable,
    /// Requested transcript sequence has fallen out of the ringbuffer.
    StaleSequence,
    /// Pane title is user-pinned and cannot be overwritten automatically.
    UserPinnedTitle,
    /// Catch-all for internal I/O errors surfaced to the client.
    Io,
}

/// Top-level bus error type. Used internally and mapped to wire-format
/// `error` envelopes at the connection boundary.
#[derive(Debug, Error)]
pub enum BusError {
    #[error("frame too large: {len} bytes > {max} limit")]
    FrameTooLarge {
        code: ErrorCode,
        len: usize,
        max: usize,
    },

    #[error("protocol version mismatch: got v{got}, expected v{expected}")]
    VersionMismatch {
        code: ErrorCode,
        got: u32,
        expected: u32,
    },

    #[error("unknown kind: {kind_received}")]
    UnknownKind {
        code: ErrorCode,
        kind_received: String,
    },

    #[error("peer denied (SO_PEERCRED UID mismatch)")]
    PeerDenied { code: ErrorCode },

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("domain error: {0}")]
    Domain(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("grant denied: {0}")]
    GrantDenied(String),

    #[error("transcript unavailable: {0}")]
    TranscriptUnavailable(String),

    #[error("stale transcript sequence: {0}")]
    StaleSequence(String),

    #[error("user-pinned title: {0}")]
    UserPinnedTitle(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[source] std::io::Error),
}

impl BusError {
    /// The wire-format `code` field for this error.
    pub fn code(&self) -> ErrorCode {
        match self {
            BusError::FrameTooLarge { code, .. }
            | BusError::VersionMismatch { code, .. }
            | BusError::UnknownKind { code, .. }
            | BusError::PeerDenied { code } => *code,
            BusError::BadRequest(_) => ErrorCode::BadRequest,
            BusError::Domain(_) => ErrorCode::Domain,
            BusError::Unauthorized(_) => ErrorCode::Unauthorized,
            BusError::GrantDenied(_) => ErrorCode::GrantDenied,
            BusError::TranscriptUnavailable(_) => ErrorCode::TranscriptUnavailable,
            BusError::StaleSequence(_) => ErrorCode::StaleSequence,
            BusError::UserPinnedTitle(_) => ErrorCode::UserPinnedTitle,
            BusError::Json(_) => ErrorCode::BadRequest,
            BusError::Io(_) => ErrorCode::Io,
        }
    }
}

/// Wire-format error envelope body — serialized into the `kind: "error"` frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kind_received: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub in_reply_to: Option<uuid::Uuid>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn error_code_serde_round_trip() {
        for code in [
            ErrorCode::FrameTooLarge,
            ErrorCode::VersionMismatch,
            ErrorCode::UnknownKind,
            ErrorCode::PeerDenied,
            ErrorCode::BadRequest,
            ErrorCode::Domain,
            ErrorCode::Unauthorized,
            ErrorCode::GrantDenied,
            ErrorCode::TranscriptUnavailable,
            ErrorCode::StaleSequence,
            ErrorCode::UserPinnedTitle,
            ErrorCode::Io,
        ] {
            let s = serde_json::to_string(&code).unwrap();
            let back: ErrorCode = serde_json::from_str(&s).unwrap();
            assert_eq!(code, back);
        }
    }

    #[test]
    fn error_code_uses_snake_case() {
        let s = serde_json::to_string(&ErrorCode::FrameTooLarge).unwrap();
        assert_eq!(s, "\"frame_too_large\"");
    }

    #[test]
    fn error_payload_omits_optional_fields() {
        let p = ErrorPayload {
            code: ErrorCode::UnknownKind,
            message: "kaboom".into(),
            kind_received: None,
            in_reply_to: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"code":"unknown_kind","message":"kaboom"}"#);
    }
}
