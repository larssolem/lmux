//! Multi-session store — named sessions persisted as TOML under
//! `$XDG_STATE_HOME/lmux/sessions/`.
//!
//! Epic coverage: Epic 2 (FR1–FR6, FR61–FR63, NFR19).
//!
//! Each [`Session`] carries a name, timestamps, the pane-tree layout
//! (re-used from [`lmux_state::LayoutNode`] for wire compatibility with the
//! v0.1 snapshot format), per-pane cwds, and a list of anchor ids. The
//! [`SessionStore`] owns the on-disk layout; [`SessionIndex`] tracks names
//! + recency for the fuzzy switcher.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod migration;
pub mod store;

pub use store::{SessionStore, StoreError};

use std::path::PathBuf;

use lmux_state::{LayoutNode, PaneTitleSnapshot, SessionSnapshot, TerminalTabStackSnapshot};
use serde::{Deserialize, Serialize};

/// One named session persisted as `sessions/<name>.toml`. Layout reuses
/// the v0.1 snapshot node type so migration is lossless and ad-hoc
/// inspection is identical on-disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    /// User-visible name. Must match `^[A-Za-z0-9._-]+$` (validated at
    /// create/rename time; see [`store::valid_name`]).
    pub name: String,
    /// Creation timestamp (unix seconds).
    pub created_at_unix_seconds: u64,
    /// Last-opened timestamp (unix seconds). Drives switcher recency.
    pub last_opened_at_unix_seconds: u64,
    /// Pane tree — reused from `lmux-state` for format stability.
    pub layout: LayoutNode,
    /// Spawn cwd per leaf pane id. TOML requires string keys, so the
    /// on-disk form uses `"1" = "/tmp"`; in memory we keep the u32 keys.
    #[serde(default, with = "cwds_serde")]
    pub cwds: std::collections::BTreeMap<u32, String>,
    /// Anchor ids attached to this session (for v0.2 anchor work; empty
    /// in v0.1 migration).
    #[serde(default)]
    pub anchors: Vec<AnchorRef>,
    #[serde(
        default,
        with = "pane_titles_serde",
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub pane_titles: std::collections::BTreeMap<u32, PaneTitleSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminal_tabs: Vec<TerminalTabStackSnapshot>,
    #[serde(
        default,
        with = "pane_roots_serde",
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub pane_terminal_tab_roots: std::collections::BTreeMap<u32, u32>,
}

impl Session {
    /// Construct a fresh empty session with a single leaf pane.
    pub fn empty(name: impl Into<String>, now_unix_seconds: u64) -> Self {
        Self {
            name: name.into(),
            created_at_unix_seconds: now_unix_seconds,
            last_opened_at_unix_seconds: now_unix_seconds,
            layout: LayoutNode::Leaf { pane_id: 1 },
            cwds: std::collections::BTreeMap::new(),
            anchors: Vec::new(),
            pane_titles: std::collections::BTreeMap::new(),
            terminal_tabs: Vec::new(),
            pane_terminal_tab_roots: std::collections::BTreeMap::new(),
        }
    }

    /// Construct from a v0.1 `SessionSnapshot` carrying `name`. Used by
    /// [`migration::migrate_v01_last_session`].
    pub fn from_v01_snapshot(
        name: impl Into<String>,
        snap: SessionSnapshot,
        now_unix_seconds: u64,
    ) -> Self {
        Self {
            name: name.into(),
            created_at_unix_seconds: snap.created_at_unix_seconds,
            last_opened_at_unix_seconds: now_unix_seconds,
            layout: snap.layout,
            cwds: snap.cwds,
            anchors: Vec::new(),
            pane_titles: snap.pane_titles,
            terminal_tabs: snap.terminal_tabs,
            pane_terminal_tab_roots: snap.pane_terminal_tab_roots,
        }
    }
}

/// Sidecar reference to an anchor carried in-session. Full anchor state
/// (scrollback ring, process metadata) lives in `lmux-anchor`; this struct
/// is just the persistent part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnchorRef {
    pub pane_id: u32,
    pub argv: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub hide_on_session_close: bool,
}

/// Recency-sorted index of known sessions. Stored at `sessions/index.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionIndex {
    #[serde(default)]
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: String,
    pub last_opened_at_unix_seconds: u64,
}

impl SessionIndex {
    /// Bump (or insert) `name` to `now_unix_seconds`; resorts DESC.
    pub fn touch(&mut self, name: &str, now_unix_seconds: u64) {
        match self.entries.iter_mut().find(|e| e.name == name) {
            Some(e) => e.last_opened_at_unix_seconds = now_unix_seconds,
            None => self.entries.push(IndexEntry {
                name: name.into(),
                last_opened_at_unix_seconds: now_unix_seconds,
            }),
        }
        self.entries.sort_by(|a, b| {
            b.last_opened_at_unix_seconds
                .cmp(&a.last_opened_at_unix_seconds)
        });
    }

    /// Remove an entry by name. No-op if absent.
    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|e| e.name != name);
    }

    /// Rename `from` to `to` preserving timestamp. No-op if `from` absent.
    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.name == from) {
            e.name = to.into();
        }
    }
}

/// Resolve `$XDG_STATE_HOME/lmux/` (fallback `$HOME/.local/state/lmux/`).
pub fn state_home() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("lmux"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".local/state/lmux"))
}

/// Unix-seconds now() helper. Matches `lmux_state::now_unix_seconds` so
/// timestamps are comparable across crates.
pub fn now_unix_seconds() -> u64 {
    lmux_state::now_unix_seconds()
}

mod cwds_serde {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(map: &BTreeMap<u32, String>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let stringified: BTreeMap<String, &String> =
            map.iter().map(|(k, v)| (k.to_string(), v)).collect();
        stringified.serialize(ser)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<BTreeMap<u32, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stringified: BTreeMap<String, String> = BTreeMap::deserialize(de)?;
        stringified
            .into_iter()
            .map(|(k, v)| {
                k.parse::<u32>()
                    .map(|k| (k, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

mod pane_titles_serde {
    use std::collections::BTreeMap;

    use lmux_state::PaneTitleSnapshot;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(map: &BTreeMap<u32, PaneTitleSnapshot>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let stringified: BTreeMap<String, &PaneTitleSnapshot> =
            map.iter().map(|(k, v)| (k.to_string(), v)).collect();
        stringified.serialize(ser)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<BTreeMap<u32, PaneTitleSnapshot>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stringified: BTreeMap<String, PaneTitleSnapshot> = BTreeMap::deserialize(de)?;
        stringified
            .into_iter()
            .map(|(k, v)| {
                k.parse::<u32>()
                    .map(|k| (k, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

mod pane_roots_serde {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(map: &BTreeMap<u32, u32>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let stringified: BTreeMap<String, u32> =
            map.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        stringified.serialize(ser)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<BTreeMap<u32, u32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stringified: BTreeMap<String, u32> = BTreeMap::deserialize(de)?;
        stringified
            .into_iter()
            .map(|(k, v)| {
                k.parse::<u32>()
                    .map(|k| (k, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}
