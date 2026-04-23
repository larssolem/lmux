//! Anchor lifecycle: tag/untag, pause (SIGSTOP) / hide / resume, scrollback
//! ring, crash capture, respawn, auto-detection pattern matching.
//!
//! See `openspec/specs/anchors/spec.md` for the living capability contract
//! and `openspec/changes/anchor-respawn-and-destructive-hide/` for the
//! in-flight respawn + destructive-hide work.
//!
//! v0.2-alpha surface:
//! * [`AnchorState`], [`Anchor`] — data types carried in the session file
//!   (mirrored onto the bus as `anchor.status` via [`lmux_bus::kinds`]).
//! * [`AnchorRegistry`] — in-memory map of live anchors with
//!   tag/untag/pause/hide/resume transitions (no OS effects yet).
//! * [`autodetect`] — pattern matcher shared with `lmux-config`; given a
//!   pane command line + environment, returns the first rule that matches
//!   so the cockpit can auto-tag.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod autodetect;
pub mod registry;

pub use autodetect::{match_rule, AutodetectMatch};
pub use registry::{AnchorRegistry, RegistryError};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle state of an anchor. Wire representation (snake_case) matches
/// [`lmux_bus::kinds::AnchorState`] exactly so state transitions can be
/// mirrored onto the bus without a separate mapping layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    /// Running and visible in a pane.
    Live,
    /// SIGSTOP'd — process exists but is frozen.
    Paused,
    /// Hidden from the pane tree but backing process still running.
    Hidden,
    /// Process exited (crashed or completed). Scrollback retained until
    /// the anchor is explicitly untagged or respawned.
    Dead,
}

/// Anchor metadata. The scrollback ring and OS-level handle live elsewhere
/// (`lmux-pty` owns the PTY; a future `AnchorBackingProcess` struct will
/// own the pid + ring); this struct is the pure-data part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub id: Uuid,
    /// Leaf pane this anchor is currently attached to, if any. `None` when
    /// `state == Hidden`.
    pub pane_id: Option<Uuid>,
    /// argv used to spawn the process. Kept so `anchor.respawn` can
    /// replay it.
    pub argv: Vec<String>,
    /// Working directory at spawn time.
    pub cwd: String,
    /// Current lifecycle state.
    pub state: AnchorState,
    /// If true, the anchor is hidden (not destroyed) when its session
    /// closes; on reopen we reattach. Matches FR40 / FR41.
    #[serde(default)]
    pub hide_on_session_close: bool,
    /// Name of the autodetect rule that tagged this anchor, if any.
    /// `None` for manual (Ctrl+B a) tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autodetect_rule: Option<String>,
    /// User-supplied display name shown in the sidebar. `None` means
    /// render a derived label (argv[0] + truncated cwd).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Sidebar group the anchor is filed under. `None` renders under an
    /// implicit "Ungrouped" bucket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Manual sort key within a group. Lower values sort first; ties break
    /// on display name. `None` → treat as 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<i64>,
}

impl Anchor {
    pub fn new_manual(pane_id: Uuid, argv: Vec<String>, cwd: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            pane_id: Some(pane_id),
            argv,
            cwd,
            state: AnchorState::Live,
            hide_on_session_close: false,
            autodetect_rule: None,
            name: None,
            group: None,
            sort_key: None,
        }
    }

    pub fn new_auto(
        pane_id: Uuid,
        argv: Vec<String>,
        cwd: String,
        rule_name: impl Into<String>,
        hide_on_session_close: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            pane_id: Some(pane_id),
            argv,
            cwd,
            state: AnchorState::Live,
            hide_on_session_close,
            autodetect_rule: Some(rule_name.into()),
            name: None,
            group: None,
            sort_key: None,
        }
    }

    /// Label shown in the sidebar: user-supplied `name` if set, otherwise
    /// `argv[0]` (the command basename, unqualified).
    pub fn display_label(&self) -> &str {
        if let Some(n) = self.name.as_deref() {
            return n;
        }
        self.argv.first().map(String::as_str).unwrap_or("(anchor)")
    }
}
