//! ADR-0016 event-kind catalog.
//!
//! Every frame on the bus carries one of these payloads. The `Kind` enum is
//! serde-tagged by the envelope's `kind` field; unknown kinds deserialize into
//! [`Kind::Unknown`] so the server can respond with a structured
//! `error.unknown_kind` rather than a generic parse failure.
//!
//! Per ADR-0016 §Decision, the v0.2 surface is closed: `session.*`,
//! `anchor.*`, `satellite.*`, `compositor.*`, plus meta (`hello`, `hello_ack`,
//! `subscribe`, `unsubscribe`, `error`, `status.get`). Any new kind requires
//! an ADR amendment.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ErrorPayload;

/// Rect in screen coordinates, used by `satellite.geometry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Opaque compositor-specific window id (KWin window uuid in v0.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositorWindowId(pub String);

/// Known client roles announced during handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientRole {
    LmuxCli,
    KwinScript,
    Satellite,
    Plugin,
}

/// Compositor health state — payload for `compositor.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositorState {
    Online,
    Offline,
}

/// Anchor lifecycle state — payload for `anchor.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    Live,
    Paused,
    Hidden,
    Dead,
}

/// Summary of one session, as returned by `session.list.result`.
///
/// `created_at_unix_seconds` is the epoch timestamp the store wrote on first
/// serialize; `last_active_unix_seconds` is an opt-in "last time this
/// session was the active one" marker (absent for sessions that have never
/// been opened).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub name: String,
    pub created_at_unix_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_unix_seconds: Option<u64>,
}

/// Payload for `status.get.result` — the reply to `status.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub cockpit_version: String,
    pub pid: i32,
    pub session_count: u32,
    pub anchor_count: u32,
    pub compositor: CompositorState,
    /// Successful `satellite.open` spawns since cockpit start (Epic 11 S4).
    /// `#[serde(default)]` so pre-v0.2 clients keep parsing newer payloads.
    #[serde(default)]
    pub satellite_spawn_ok: u32,
    /// Failed `satellite.open` spawns since cockpit start (Epic 11 S4).
    #[serde(default)]
    pub satellite_spawn_fail: u32,
}

/// One live pane as returned by `pane.list.result`. `anchor_id` is set
/// when the pane is tagged as an anchor; `cwd` mirrors the pane's last
/// known working directory (best-effort, may be absent on early-boot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSummary {
    pub pane_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// Satellite docking state — payload for `satellite.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SatelliteState {
    Docked,
    Detached,
    FloatingFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacosWindowCandidate {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<i64>,
    pub window_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The v0.2 bus kind catalog. Tag is the envelope's `kind` field.
///
/// Note: this enum participates in tagged deserialization via
/// [`parse_payload`]; it intentionally does NOT carry the envelope's `v` /
/// `id` fields — those live in [`crate::envelope::Envelope`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Kind {
    // ---- meta ----
    #[serde(rename = "hello")]
    Hello { client: ClientRole, pid: i32 },
    #[serde(rename = "hello_ack")]
    HelloAck { cockpit_version: String },
    #[serde(rename = "subscribe")]
    Subscribe { patterns: Vec<String> },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { subscription_id: Uuid },
    #[serde(rename = "error")]
    Error(ErrorPayload),
    /// Generic success ack used for side-effect-only requests (anchor.tag,
    /// satellite.detach, etc.). `of` echoes the request `kind` so clients
    /// can route acks without tracking request ids. Optional: servers MAY
    /// omit it when the transport-level `id` round-trip suffices.
    #[serde(rename = "ok")]
    Ok {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<String>,
    },
    #[serde(rename = "status.get")]
    StatusGet {},
    #[serde(rename = "status.get.result")]
    StatusGetResult(StatusSnapshot),

    // ---- session.* ----
    #[serde(rename = "session.list")]
    SessionList {},
    #[serde(rename = "session.list.result")]
    SessionListResult { sessions: Vec<SessionSummary> },
    #[serde(rename = "session.new")]
    SessionNew { name: String },
    #[serde(rename = "session.rename")]
    SessionRename { from: String, to: String },
    #[serde(rename = "session.delete")]
    SessionDelete { name: String },
    #[serde(rename = "session.open")]
    SessionOpen { name: String },

    // ---- pane.* ----
    #[serde(rename = "pane.list")]
    PaneList {},
    #[serde(rename = "pane.list.result")]
    PaneListResult { panes: Vec<PaneSummary> },

    // ---- anchor.* ----
    #[serde(rename = "anchor.tag")]
    AnchorTag { pane_id: Uuid },
    #[serde(rename = "anchor.new")]
    AnchorNew {},
    #[serde(rename = "anchor.activate")]
    AnchorActivate { pane_id: Uuid },
    #[serde(rename = "anchor.untag")]
    AnchorUntag { pane_id: Uuid },
    #[serde(rename = "anchor.pause")]
    AnchorPause { pane_id: Uuid },
    #[serde(rename = "anchor.resume")]
    AnchorResume { pane_id: Uuid },
    #[serde(rename = "anchor.hide")]
    AnchorHide { pane_id: Uuid },
    #[serde(rename = "anchor.reattach")]
    AnchorReattach { pane_id: Uuid },
    #[serde(rename = "anchor.respawn")]
    AnchorRespawn { pane_id: Uuid },
    #[serde(rename = "anchor.status")]
    AnchorStatus { pane_id: Uuid, state: AnchorState },

    // ---- satellite.* ----
    #[serde(rename = "satellite.open")]
    SatelliteOpen {
        argv: Vec<String>,
        target_pane: Uuid,
        #[serde(default)]
        no_sandbox: bool,
    },
    #[serde(rename = "satellite.detach")]
    SatelliteDetach { pane_id: Uuid },
    #[serde(rename = "satellite.reattach")]
    SatelliteReattach { pane_id: Uuid },
    #[serde(rename = "satellite.attach_focused")]
    SatelliteAttachFocused {},
    #[serde(rename = "satellite.list_windows")]
    SatelliteListWindows {},
    #[serde(rename = "satellite.list_windows.result")]
    SatelliteListWindowsResult { windows: Vec<MacosWindowCandidate> },
    #[serde(rename = "satellite.attach_window")]
    SatelliteAttachWindow {
        pid: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_index: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    #[serde(rename = "satellite.map")]
    SatelliteMap {
        request_id: Uuid,
        window_id: CompositorWindowId,
    },
    #[serde(rename = "satellite.geometry")]
    SatelliteGeometry {
        window_id: CompositorWindowId,
        rect: Rect,
    },
    #[serde(rename = "satellite.status")]
    SatelliteStatus {
        pane_id: Uuid,
        state: SatelliteState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    // ---- compositor.* ----
    #[serde(rename = "compositor.status")]
    CompositorStatus {
        state: CompositorState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "compositor.reinject")]
    CompositorReinject {},
}

/// Parse `body` as the envelope-plus-kind shape. Returns the parsed [`Kind`]
/// alongside the envelope's `v` and `id` (already validated to version 2 by
/// the caller via [`crate::envelope::Envelope::parse`] before routing here is
/// recommended for consistent error surfaces).
///
/// Unknown kinds surface as `serde_json::Error` with a message containing the
/// unknown tag; the server should intercept this and reply with a structured
/// `error.unknown_kind` envelope.
pub fn parse_payload(body: &[u8]) -> Result<Kind, serde_json::Error> {
    // The wire frame includes {v, id, kind, ...payload}. Kind only reads
    // `kind` and payload fields; `v` / `id` are simply ignored by the
    // tagged-enum deserializer.
    serde_json::from_slice::<Kind>(body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Wrap a `Kind` with a `{v: 2, id: ...}` envelope and round-trip it.
    fn wrap_and_roundtrip(kind: Kind) -> Kind {
        let mut v = serde_json::to_value(&kind).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert("v".into(), serde_json::json!(2));
        obj.insert("id".into(), serde_json::json!(Uuid::new_v4().to_string()));
        let bytes = serde_json::to_vec(&v).unwrap();
        parse_payload(&bytes).unwrap()
    }

    #[test]
    fn hello_roundtrip() {
        let k = Kind::Hello {
            client: ClientRole::LmuxCli,
            pid: 1234,
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn session_list_roundtrip() {
        let k = Kind::SessionList {};
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn satellite_map_roundtrip() {
        let k = Kind::SatelliteMap {
            request_id: Uuid::new_v4(),
            window_id: CompositorWindowId("kwin-12345".into()),
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn satellite_geometry_roundtrip() {
        let k = Kind::SatelliteGeometry {
            window_id: CompositorWindowId("kwin-12345".into()),
            rect: Rect {
                x: 100,
                y: 200,
                w: 800,
                h: 600,
            },
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn compositor_status_roundtrip() {
        let k = Kind::CompositorStatus {
            state: CompositorState::Offline,
            reason: Some("script missing".into()),
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn anchor_status_roundtrip() {
        let k = Kind::AnchorStatus {
            pane_id: Uuid::new_v4(),
            state: AnchorState::Paused,
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn unknown_kind_rejected() {
        let bytes =
            br#"{"v":2,"id":"00000000-0000-0000-0000-000000000000","kind":"session.teleport"}"#;
        let err = parse_payload(bytes).unwrap_err();
        // serde reports the unknown variant.
        let msg = err.to_string();
        assert!(
            msg.contains("session.teleport") || msg.contains("unknown variant"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn anchor_state_snake_case_wire() {
        let k = Kind::AnchorStatus {
            pane_id: Uuid::nil(),
            state: AnchorState::Hidden,
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(s.contains("\"state\":\"hidden\""), "got: {s}");
    }

    #[test]
    fn session_list_result_roundtrip() {
        let k = Kind::SessionListResult {
            sessions: vec![
                SessionSummary {
                    name: "work".into(),
                    created_at_unix_seconds: 1_700_000_000,
                    last_active_unix_seconds: Some(1_700_000_900),
                },
                SessionSummary {
                    name: "home".into(),
                    created_at_unix_seconds: 1_700_000_100,
                    last_active_unix_seconds: None,
                },
            ],
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn pane_list_result_roundtrip() {
        let k = Kind::PaneListResult {
            panes: vec![
                PaneSummary {
                    pane_id: Uuid::new_v4(),
                    anchor_id: Some(Uuid::new_v4()),
                    cwd: Some("/home/u/work".into()),
                },
                PaneSummary {
                    pane_id: Uuid::new_v4(),
                    anchor_id: None,
                    cwd: None,
                },
            ],
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn pane_list_result_skips_optional_when_none() {
        let k = Kind::PaneListResult {
            panes: vec![PaneSummary {
                pane_id: Uuid::nil(),
                anchor_id: None,
                cwd: None,
            }],
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(!s.contains("anchor_id"), "got: {s}");
        assert!(!s.contains("cwd"), "got: {s}");
    }

    #[test]
    fn session_list_result_skips_last_active_when_none() {
        let k = Kind::SessionListResult {
            sessions: vec![SessionSummary {
                name: "home".into(),
                created_at_unix_seconds: 1_700_000_100,
                last_active_unix_seconds: None,
            }],
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(!s.contains("last_active_unix_seconds"), "got: {s}");
    }

    #[test]
    fn ok_ack_omits_empty_of_field() {
        let k = Kind::Ok { of: None };
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, r#"{"kind":"ok"}"#);
    }

    #[test]
    fn ok_ack_carries_of_when_present() {
        let k = Kind::Ok {
            of: Some("anchor.tag".into()),
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn status_get_result_roundtrip() {
        let k = Kind::StatusGetResult(StatusSnapshot {
            cockpit_version: "0.2.0".into(),
            pid: 4242,
            session_count: 2,
            anchor_count: 1,
            compositor: CompositorState::Online,
            satellite_spawn_ok: 5,
            satellite_spawn_fail: 1,
        });
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn satellite_state_floating_fallback_wire() {
        let k = Kind::SatelliteStatus {
            pane_id: Uuid::nil(),
            state: SatelliteState::FloatingFallback,
            reason: None,
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(s.contains("\"state\":\"floating_fallback\""), "got: {s}");
    }

    #[test]
    fn all_known_kinds_parse() {
        // Sanity: every branch at least round-trips one instance.
        let kinds = vec![
            Kind::Hello {
                client: ClientRole::KwinScript,
                pid: 1,
            },
            Kind::HelloAck {
                cockpit_version: "0.2.0".into(),
            },
            Kind::Subscribe {
                patterns: vec!["anchor.*".into()],
            },
            Kind::Unsubscribe {
                subscription_id: Uuid::nil(),
            },
            Kind::StatusGet {},
            Kind::SessionList {},
            Kind::SessionNew { name: "a".into() },
            Kind::SessionRename {
                from: "a".into(),
                to: "b".into(),
            },
            Kind::SessionDelete { name: "a".into() },
            Kind::SessionOpen { name: "a".into() },
            Kind::AnchorTag {
                pane_id: Uuid::nil(),
            },
            Kind::AnchorUntag {
                pane_id: Uuid::nil(),
            },
            Kind::AnchorPause {
                pane_id: Uuid::nil(),
            },
            Kind::AnchorResume {
                pane_id: Uuid::nil(),
            },
            Kind::AnchorHide {
                pane_id: Uuid::nil(),
            },
            Kind::AnchorReattach {
                pane_id: Uuid::nil(),
            },
            Kind::AnchorRespawn {
                pane_id: Uuid::nil(),
            },
            Kind::SatelliteOpen {
                argv: vec!["kate".into()],
                target_pane: Uuid::nil(),
                no_sandbox: false,
            },
            Kind::SatelliteDetach {
                pane_id: Uuid::nil(),
            },
            Kind::SatelliteReattach {
                pane_id: Uuid::nil(),
            },
            Kind::SatelliteAttachFocused {},
            Kind::SatelliteListWindows {},
            Kind::SatelliteListWindowsResult {
                windows: vec![MacosWindowCandidate {
                    pid: 42,
                    window_id: Some(1001),
                    window_index: 1,
                    bundle_id: Some("com.example.App".into()),
                    title: Some("Example".into()),
                }],
            },
            Kind::SatelliteAttachWindow {
                pid: 42,
                window_id: Some(1001),
                window_index: Some(1),
                bundle_id: Some("com.example.App".into()),
                title: Some("Example".into()),
            },
            Kind::CompositorReinject {},
        ];
        for k in kinds {
            assert_eq!(wrap_and_roundtrip(k.clone()), k);
        }
    }
}
