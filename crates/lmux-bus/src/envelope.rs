//! Versioned envelope per ADR-0015.
//!
//! Every frame on the bus carries `{"v": 2, "kind": "...", "id": "<uuid>", ...}`.
//! The envelope parses with a strict version check: any frame whose `v` field
//! is not [`PROTOCOL_VERSION`] is rejected before `kind` dispatch.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{BusError, ErrorCode};

/// Current bus protocol version. Bump on breaking envelope changes only.
pub const PROTOCOL_VERSION: u32 = 2;

/// Raw envelope view used for the version + id check before kind dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Envelope {
    /// Protocol version. Always `PROTOCOL_VERSION` for v0.2.
    pub v: u32,
    /// Event kind (e.g. `"session.list"`, `"satellite.open"`). See ADR-0016.
    pub kind: String,
    /// Client- or server-generated message id; responses copy this into
    /// `in_reply_to`.
    pub id: Uuid,
}

impl Envelope {
    /// Construct an envelope with a fresh v4 UUID for the given `kind`.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            kind: kind.into(),
            id: Uuid::new_v4(),
        }
    }

    /// Parse an envelope from a JSON body, rejecting unknown protocol versions.
    pub fn parse(bytes: &[u8]) -> Result<Self, BusError> {
        let env: Envelope = serde_json::from_slice(bytes).map_err(BusError::Json)?;
        if env.v != PROTOCOL_VERSION {
            return Err(BusError::VersionMismatch {
                code: ErrorCode::VersionMismatch,
                got: env.v,
                expected: PROTOCOL_VERSION,
            });
        }
        Ok(env)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let env = Envelope::new("hello");
        let bytes = serde_json::to_vec(&env).unwrap();
        let back = Envelope::parse(&bytes).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn rejects_wrong_version() {
        let body = br#"{"v":1,"kind":"hello","id":"00000000-0000-0000-0000-000000000000"}"#;
        let err = Envelope::parse(body).unwrap_err();
        assert!(matches!(err, BusError::VersionMismatch { got: 1, .. }));
    }

    #[test]
    fn rejects_missing_version() {
        let body = br#"{"kind":"hello","id":"00000000-0000-0000-0000-000000000000"}"#;
        assert!(matches!(Envelope::parse(body), Err(BusError::Json(_))));
    }

    #[test]
    fn rejects_non_uuid_id() {
        let body = br#"{"v":2,"kind":"hello","id":"not-a-uuid"}"#;
        assert!(matches!(Envelope::parse(body), Err(BusError::Json(_))));
    }
}
