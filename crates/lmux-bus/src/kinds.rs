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
    LmuxMcp,
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

/// One live pane as returned by `pane.list.result`. `anchor_id` is the owning
/// anchor UUID for panes that belong to an anchor workspace or terminal tab
/// stack; `cwd` mirrors the pane's last known working directory (best-effort,
/// may be absent on early-boot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSummary {
    pub pane_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// Agent identity carried by agent-aware requests. This is provenance and
/// prompt text, not a standalone authorization credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Provenance for a visible pane title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneTitleProvenance {
    Default,
    Agent,
    User,
}

/// Visible title plus provenance for pane/tab UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneTitle {
    pub title: String,
    pub provenance: PaneTitleProvenance,
    #[serde(default)]
    pub pinned: bool,
}

/// Where a new terminal pane should be inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanePlacement {
    Tab,
    SplitRight,
    SplitDown,
}

/// One transcript line captured from a PTY-backed terminal pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptLine {
    pub sequence: u64,
    pub unix_millis: u64,
    pub text: String,
}

/// Result payload for transcript tail/capture requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptRange {
    pub pane_id: Uuid,
    pub first_sequence: u64,
    pub last_sequence: u64,
    #[serde(default)]
    pub truncated: bool,
    pub lines: Vec<TranscriptLine>,
}

/// Scope vocabulary for cockpit-owned agent grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantScope {
    ReadOutput,
    SendInput,
    Rename,
    AttachWindow,
}

/// User decision for a pending agent grant request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum GrantDecision {
    AllowOnce,
    AllowUntil { expires_at_unix_seconds: u64 },
    Deny,
    Revoke,
}

/// Compact agent/pane status shown in anchor inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPaneStatus {
    pub pane_id: Uuid,
    pub agent: AgentIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// One anchor workspace as returned by `anchor.list.result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSummary {
    pub anchor_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<Uuid>,
    pub label: String,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_status: Vec<AgentPaneStatus>,
    #[serde(default)]
    pub pending_grants: u32,
    #[serde(default)]
    pub active_grants: u32,
}

/// Result of creating a terminal pane through an agent-aware path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneNewResult {
    pub pane_id: Uuid,
    pub anchor_id: Uuid,
    pub placement: PanePlacement,
}

/// Pending grant request surfaced to the cockpit UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRequest {
    pub grant_id: Uuid,
    pub requester: AgentIdentity,
    pub scope: GrantScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_anchor: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_anchor: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_pane: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Satellite docking state — payload for `satellite.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SatelliteState {
    Docked,
    Detached,
    FloatingFallback,
}

/// Window-manager backend namespace for native windows that can be explicitly
/// attached to an anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowCandidateBackend {
    Macos,
    Kwin,
    X11,
    Hyprland,
    Sway,
    Noop,
    Unsupported,
}

/// Optional app identity attached to a native window candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WindowAppIdentity {
    BundleId(String),
    DesktopEntry(String),
    WmClass(String),
    AppId(String),
    Other(String),
}

/// One native GUI window that can be attached to an anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowCandidate {
    pub backend: WindowCandidateBackend,
    pub backend_window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_identity: Option<WindowAppIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
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
    #[serde(rename = "pane.new")]
    PaneNew {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_anchor: Option<Uuid>,
        placement: PanePlacement,
        #[serde(default)]
        activate: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default)]
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        purpose: Option<String>,
    },
    #[serde(rename = "pane.new.result")]
    PaneNewResult(PaneNewResult),
    #[serde(rename = "pane.tail")]
    PaneTail {
        pane_id: Uuid,
        lines: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
    },
    #[serde(rename = "pane.capture")]
    PaneCapture {
        pane_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since_sequence: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_lines: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
    },
    #[serde(rename = "pane.transcript.result")]
    PaneTranscriptResult(TranscriptRange),
    #[serde(rename = "pane.send_input")]
    PaneSendInput {
        pane_id: Uuid,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
    },
    #[serde(rename = "pane.rename")]
    PaneRename {
        pane_id: Uuid,
        title: String,
        #[serde(default)]
        pin: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
    },

    // ---- anchor.* ----
    #[serde(rename = "anchor.list")]
    AnchorList {},
    #[serde(rename = "anchor.list.result")]
    AnchorListResult { anchors: Vec<AnchorSummary> },
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

    // ---- grant.* ----
    #[serde(rename = "grant.request")]
    GrantRequest(GrantRequest),
    #[serde(rename = "grant.request.result")]
    GrantRequestResult {
        grant_id: Uuid,
        #[serde(default)]
        pending: bool,
    },
    #[serde(rename = "grant.decide")]
    GrantDecide {
        grant_id: Uuid,
        decision: GrantDecision,
    },

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
    SatelliteListWindowsResult { windows: Vec<WindowCandidate> },
    #[serde(rename = "satellite.attach_window")]
    SatelliteAttachWindow {
        backend: WindowCandidateBackend,
        backend_window_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_identity: Option<WindowAppIdentity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
    },
    #[serde(rename = "satellite.launch_attach")]
    SatelliteLaunchAttach {
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentIdentity>,
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

    fn candidate(
        backend: WindowCandidateBackend,
        backend_window_id: &str,
        pid: Option<u32>,
        app_identity: Option<WindowAppIdentity>,
        title: &str,
    ) -> WindowCandidate {
        WindowCandidate {
            backend,
            backend_window_id: backend_window_id.into(),
            pid,
            app_identity,
            title: Some(title.into()),
            workspace: None,
            output: None,
        }
    }

    fn agent() -> AgentIdentity {
        AgentIdentity {
            id: "codex".into(),
            name: Some("Codex".into()),
        }
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
    fn mcp_client_role_wire_name() {
        let k = Kind::Hello {
            client: ClientRole::LmuxMcp,
            pid: 1234,
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(s.contains("\"client\":\"lmux-mcp\""), "got: {s}");
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
    fn agent_payloads_roundtrip() {
        let pane_id = Uuid::new_v4();
        let anchor_id = Uuid::new_v4();
        let grant_id = Uuid::new_v4();
        let kinds = vec![
            Kind::AnchorList {},
            Kind::AnchorListResult {
                anchors: vec![AnchorSummary {
                    anchor_id,
                    pane_id: Some(pane_id),
                    label: "backend".into(),
                    active: true,
                    agent_status: vec![AgentPaneStatus {
                        pane_id,
                        agent: agent(),
                        title: Some("tests".into()),
                        purpose: Some("run tests".into()),
                    }],
                    pending_grants: 1,
                    active_grants: 2,
                }],
            },
            Kind::PaneNew {
                target_anchor: Some(anchor_id),
                placement: PanePlacement::Tab,
                activate: true,
                title: Some("tests".into()),
                argv: vec!["cargo".into(), "test".into()],
                agent: Some(agent()),
                purpose: Some("run tests".into()),
            },
            Kind::PaneNewResult(PaneNewResult {
                pane_id,
                anchor_id,
                placement: PanePlacement::Tab,
            }),
            Kind::PaneTail {
                pane_id,
                lines: 120,
                agent: None,
            },
            Kind::PaneCapture {
                pane_id,
                since_sequence: Some(42),
                max_lines: Some(80),
                agent: None,
            },
            Kind::PaneTranscriptResult(TranscriptRange {
                pane_id,
                first_sequence: 40,
                last_sequence: 42,
                truncated: false,
                lines: vec![TranscriptLine {
                    sequence: 42,
                    unix_millis: 1_700_000_000_000,
                    text: "ok".into(),
                }],
            }),
            Kind::PaneSendInput {
                pane_id,
                text: "q".into(),
                agent: None,
            },
            Kind::PaneRename {
                pane_id,
                title: "unit tests".into(),
                pin: true,
                agent: Some(agent()),
            },
            Kind::GrantRequest(GrantRequest {
                grant_id,
                requester: agent(),
                scope: GrantScope::ReadOutput,
                source_anchor: Some(anchor_id),
                target_anchor: Some(Uuid::new_v4()),
                target_pane: Some(pane_id),
                target_window: None,
                reason: Some("need frontend logs".into()),
            }),
            Kind::GrantRequestResult {
                grant_id,
                pending: true,
            },
            Kind::GrantDecide {
                grant_id,
                decision: GrantDecision::AllowUntil {
                    expires_at_unix_seconds: 1_700_000_600,
                },
            },
        ];

        for kind in kinds {
            assert_eq!(wrap_and_roundtrip(kind.clone()), kind);
        }
    }

    #[test]
    fn agent_payloads_omit_optional_empty_fields() {
        let pane_id = Uuid::nil();
        let k = Kind::PaneNew {
            target_anchor: None,
            placement: PanePlacement::SplitRight,
            activate: false,
            title: None,
            argv: Vec::new(),
            agent: None,
            purpose: None,
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(!s.contains("target_anchor"), "got: {s}");
        assert!(!s.contains("title"), "got: {s}");
        assert!(!s.contains("agent"), "got: {s}");
        assert!(!s.contains("purpose"), "got: {s}");

        let k = Kind::PaneRename {
            pane_id,
            title: "logs".into(),
            pin: false,
            agent: None,
        };
        let s = serde_json::to_string(&k).unwrap();
        assert!(!s.contains("agent"), "got: {s}");
    }

    #[test]
    fn title_provenance_and_grant_scope_wire_names() {
        let title = PaneTitle {
            title: "server".into(),
            provenance: PaneTitleProvenance::Agent,
            pinned: false,
        };
        let s = serde_json::to_string(&title).unwrap();
        assert!(s.contains("\"provenance\":\"agent\""), "got: {s}");

        let s = serde_json::to_string(&GrantScope::ReadOutput).unwrap();
        assert_eq!(s, "\"read-output\"");
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
    fn satellite_list_windows_platform_neutral_roundtrip() {
        let k = Kind::SatelliteListWindowsResult {
            windows: vec![
                candidate(
                    WindowCandidateBackend::Macos,
                    "macos-window-id:1001:index:1",
                    Some(42),
                    Some(WindowAppIdentity::BundleId("com.example.App".into())),
                    "macOS App",
                ),
                candidate(
                    WindowCandidateBackend::Kwin,
                    "kwin:9ec5",
                    Some(77),
                    Some(WindowAppIdentity::DesktopEntry("org.kde.kate".into())),
                    "Kate",
                ),
                candidate(
                    WindowCandidateBackend::X11,
                    "x11:0x03a00007",
                    Some(88),
                    Some(WindowAppIdentity::WmClass("firefox".into())),
                    "Firefox",
                ),
                WindowCandidate {
                    backend: WindowCandidateBackend::Unsupported,
                    backend_window_id: "unsupported:none".into(),
                    pid: None,
                    app_identity: None,
                    title: Some("Unsupported compositor".into()),
                    workspace: None,
                    output: None,
                },
            ],
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn satellite_attach_window_platform_neutral_roundtrip() {
        for (backend, backend_window_id, app_identity) in [
            (
                WindowCandidateBackend::Macos,
                "macos-window-id:1001:index:1",
                WindowAppIdentity::BundleId("com.example.App".into()),
            ),
            (
                WindowCandidateBackend::Kwin,
                "kwin:9ec5",
                WindowAppIdentity::DesktopEntry("org.kde.kate".into()),
            ),
            (
                WindowCandidateBackend::X11,
                "x11:0x03a00007",
                WindowAppIdentity::WmClass("firefox".into()),
            ),
        ] {
            let k = Kind::SatelliteAttachWindow {
                backend,
                backend_window_id: backend_window_id.into(),
                pid: Some(42),
                app_identity: Some(app_identity),
                title: Some("Window".into()),
                workspace: Some("1".into()),
                output: Some("HDMI-A-1".into()),
                agent: None,
            };
            assert_eq!(wrap_and_roundtrip(k.clone()), k);
        }
    }

    #[test]
    fn satellite_launch_attach_roundtrip() {
        let k = Kind::SatelliteLaunchAttach {
            argv: vec!["kate".into(), "--new-window".into()],
            title_hint: Some("notes".into()),
            app_hint: Some("org.kde.kate".into()),
            timeout_ms: Some(1500),
            agent: Some(agent()),
        };
        assert_eq!(wrap_and_roundtrip(k.clone()), k);
    }

    #[test]
    fn all_known_kinds_parse() {
        // Sanity: every branch at least round-trips one instance.
        let kinds = vec![
            Kind::Hello {
                client: ClientRole::KwinScript,
                pid: 1,
            },
            Kind::Hello {
                client: ClientRole::LmuxMcp,
                pid: 2,
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
            Kind::AnchorList {},
            Kind::AnchorListResult {
                anchors: Vec::new(),
            },
            Kind::PaneNew {
                target_anchor: None,
                placement: PanePlacement::Tab,
                activate: false,
                title: None,
                argv: Vec::new(),
                agent: None,
                purpose: None,
            },
            Kind::PaneNewResult(PaneNewResult {
                pane_id: Uuid::nil(),
                anchor_id: Uuid::nil(),
                placement: PanePlacement::Tab,
            }),
            Kind::PaneTail {
                pane_id: Uuid::nil(),
                lines: 20,
                agent: None,
            },
            Kind::PaneCapture {
                pane_id: Uuid::nil(),
                since_sequence: None,
                max_lines: None,
                agent: None,
            },
            Kind::PaneTranscriptResult(TranscriptRange {
                pane_id: Uuid::nil(),
                first_sequence: 0,
                last_sequence: 0,
                truncated: false,
                lines: Vec::new(),
            }),
            Kind::PaneSendInput {
                pane_id: Uuid::nil(),
                text: "q".into(),
                agent: None,
            },
            Kind::PaneRename {
                pane_id: Uuid::nil(),
                title: "logs".into(),
                pin: false,
                agent: None,
            },
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
            Kind::GrantRequest(GrantRequest {
                grant_id: Uuid::nil(),
                requester: agent(),
                scope: GrantScope::SendInput,
                source_anchor: None,
                target_anchor: None,
                target_pane: Some(Uuid::nil()),
                target_window: None,
                reason: None,
            }),
            Kind::GrantRequestResult {
                grant_id: Uuid::nil(),
                pending: true,
            },
            Kind::GrantDecide {
                grant_id: Uuid::nil(),
                decision: GrantDecision::Deny,
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
                windows: vec![candidate(
                    WindowCandidateBackend::Macos,
                    "macos-window-id:1001:index:1",
                    Some(42),
                    Some(WindowAppIdentity::BundleId("com.example.App".into())),
                    "Example",
                )],
            },
            Kind::SatelliteAttachWindow {
                backend: WindowCandidateBackend::Macos,
                backend_window_id: "macos-window-id:1001:index:1".into(),
                pid: Some(42),
                app_identity: Some(WindowAppIdentity::BundleId("com.example.App".into())),
                title: Some("Example".into()),
                workspace: None,
                output: None,
                agent: None,
            },
            Kind::SatelliteLaunchAttach {
                argv: vec!["kate".into()],
                title_hint: Some("Example".into()),
                app_hint: None,
                timeout_ms: Some(1000),
                agent: None,
            },
            Kind::CompositorReinject {},
        ];
        for k in kinds {
            assert_eq!(wrap_and_roundtrip(k.clone()), k);
        }
    }
}
