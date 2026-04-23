//! lmux bus — Unix-socket IPC transport between the cockpit and its clients
//! (CLI, KWin script bridge, v0.3 plugins).
//!
//! Protocol is specified in ADR-0015 (framing + envelope + peer auth) and
//! ADR-0016 (kind set). Epic coverage: see epics.md Epic 3.
//!
//! # Wire format
//!
//! Each frame is `u32-be length || JSON body`. The JSON body always matches
//! the [`Envelope`] shape: `{"v": 2, "kind": "...", "id": "<uuid>"}` plus
//! kind-specific payload. Maximum frame size is [`MAX_FRAME_BYTES`].

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod client;
pub mod codec;
pub mod envelope;
pub mod error;
pub mod kinds;
pub mod paths;
pub mod server;

pub use client::Client;
pub use codec::{read_frame, write_frame, MAX_FRAME_BYTES};
pub use envelope::{Envelope, PROTOCOL_VERSION};
pub use error::{BusError, ErrorCode, ErrorPayload};
pub use kinds::{ClientRole, Kind, PaneSummary, SessionSummary, StatusSnapshot};
pub use paths::{bus_pid_path, bus_socket_path};
pub use server::{Handler, RejectAllHandler, Server};
