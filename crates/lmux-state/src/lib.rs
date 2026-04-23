//! XDG-anchored session state persistence (FR27-FR31, NFR7, NFR9).
//!
//! The snapshot format is a tagged JSON document — `{ "v": 1, ... }`.
//! Missing files return `LoadOutcome::Missing`; unparseable files are
//! renamed aside as `.bad.<unix-seconds>` and return `LoadOutcome::Corrupt`
//! so the caller can fall back to a fresh session.

pub mod atomic_write;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Stable schema version. Bump when the snapshot shape changes in an
/// incompatible way. Unknown `v` is treated as corrupt (NFR9).
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub v: u32,
    pub created_at_unix_seconds: u64,
    /// Legacy single-anchor slot (v0.1). Retained for read compatibility;
    /// v0.2 writers also populate [`SessionSnapshot::anchor_pane_ids`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_pane_id: Option<u32>,
    /// Multi-anchor list (v0.2+). When empty on read, readers must fall
    /// back to [`SessionSnapshot::anchor_pane_id`] so v0.1 snapshots still
    /// round-trip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchor_pane_ids: Vec<u32>,
    pub layout: LayoutNode,
    /// Spawn CWD per leaf pane id. Absent entries fall back to `$HOME` on
    /// restore (Story 8.3 — recorded cwd missing → warn + $HOME).
    pub cwds: std::collections::BTreeMap<u32, String>,
}

impl SessionSnapshot {
    /// Canonical anchor list — prefers the multi-anchor field, falling back
    /// to the legacy singleton when the multi field is empty.
    pub fn anchors(&self) -> Vec<u32> {
        if !self.anchor_pane_ids.is_empty() {
            return self.anchor_pane_ids.clone();
        }
        self.anchor_pane_id.into_iter().collect()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LayoutNode {
    Leaf {
        pane_id: u32,
    },
    Split {
        dir: SplitDir,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
        ratio: f64,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

#[derive(Debug)]
pub enum LoadOutcome {
    Missing,
    Ok(SessionSnapshot),
    Corrupt {
        error: String,
        renamed_to: Option<PathBuf>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("resolve path: XDG data dir unavailable")]
    NoDataDir,
    #[error("atomic write: {0}")]
    Write(#[from] atomic_write::Error),
}

/// Resolve `$XDG_DATA_HOME/lmux/last-session.json`, falling back to
/// `$HOME/.local/share/lmux/last-session.json`. Returns `None` if neither
/// environment variable is set.
pub fn session_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("lmux").join("last-session.json"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".local/share/lmux")
            .join("last-session.json"),
    )
}

pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn save(path: &Path, snap: &SessionSnapshot) -> Result<(), SaveError> {
    atomic_write::write_json(path, snap)?;
    Ok(())
}

/// Read and parse the snapshot. Missing file → `Missing`. Parse or schema
/// mismatch → `Corrupt` and the file is renamed to `.bad.<unix-seconds>`
/// (NFR9, FR29).
pub fn load(path: &Path) -> LoadOutcome {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return LoadOutcome::Missing,
        Err(err) => {
            return LoadOutcome::Corrupt {
                error: format!("read failed: {err}"),
                renamed_to: None,
            };
        }
    };
    match serde_json::from_slice::<SessionSnapshot>(&bytes) {
        Ok(snap) if snap.v == SCHEMA_VERSION => LoadOutcome::Ok(snap),
        Ok(snap) => {
            let err = format!("unknown schema version: {}", snap.v);
            let renamed_to = rename_aside(path);
            LoadOutcome::Corrupt {
                error: err,
                renamed_to,
            }
        }
        Err(err) => {
            let renamed_to = rename_aside(path);
            LoadOutcome::Corrupt {
                error: err.to_string(),
                renamed_to,
            }
        }
    }
}

fn rename_aside(path: &Path) -> Option<PathBuf> {
    let ts = now_unix_seconds();
    let mut target = path.as_os_str().to_owned();
    target.push(format!(".bad.{ts}"));
    let target = PathBuf::from(target);
    match std::fs::rename(path, &target) {
        Ok(()) => Some(target),
        Err(err) => {
            tracing::warn!(error = %err, "failed to rename corrupt snapshot aside");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn sample() -> SessionSnapshot {
        SessionSnapshot {
            v: SCHEMA_VERSION,
            created_at_unix_seconds: 42,
            anchor_pane_id: Some(2),
            anchor_pane_ids: vec![],
            layout: LayoutNode::Split {
                dir: SplitDir::Vertical,
                a: Box::new(LayoutNode::Leaf { pane_id: 1 }),
                b: Box::new(LayoutNode::Leaf { pane_id: 2 }),
                ratio: 0.5,
            },
            cwds: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(1, "/tmp".to_string());
                m.insert(2, "/home".to_string());
                m
            },
        }
    }

    #[test]
    fn roundtrip_through_disk() {
        let dir = tempdir();
        let path = dir.join("session.json");
        save(&path, &sample()).unwrap_or_else(|e| panic!("save: {e}"));
        match load(&path) {
            LoadOutcome::Ok(s) => {
                assert_eq!(s.anchor_pane_id, Some(2));
                assert_eq!(s.cwds.get(&1).map(String::as_str), Some("/tmp"));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_is_missing() {
        let dir = tempdir();
        let path = dir.join("does-not-exist.json");
        matches!(load(&path), LoadOutcome::Missing);
    }

    #[test]
    fn corrupt_file_renamed_aside() {
        let dir = tempdir();
        let path = dir.join("session.json");
        std::fs::write(&path, b"{not json").unwrap_or_else(|e| panic!("write: {e}"));
        match load(&path) {
            LoadOutcome::Corrupt { renamed_to, .. } => {
                let Some(renamed) = renamed_to else {
                    panic!("renamed target missing");
                };
                assert!(renamed.exists(), "renamed file should exist");
                assert!(!path.exists(), "original should have been moved");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn legacy_singleton_read_via_anchors_helper() {
        let snap = SessionSnapshot {
            v: SCHEMA_VERSION,
            created_at_unix_seconds: 0,
            anchor_pane_id: Some(7),
            anchor_pane_ids: vec![],
            layout: LayoutNode::Leaf { pane_id: 7 },
            cwds: std::collections::BTreeMap::new(),
        };
        assert_eq!(snap.anchors(), vec![7]);
    }

    #[test]
    fn multi_anchor_preferred_over_legacy() {
        let snap = SessionSnapshot {
            v: SCHEMA_VERSION,
            created_at_unix_seconds: 0,
            anchor_pane_id: Some(1),
            anchor_pane_ids: vec![3, 4],
            layout: LayoutNode::Leaf { pane_id: 3 },
            cwds: std::collections::BTreeMap::new(),
        };
        assert_eq!(snap.anchors(), vec![3, 4]);
    }

    #[test]
    fn v1_legacy_json_parses_into_anchors_helper() {
        // A real on-disk v=1 snapshot from before multi-anchor existed.
        let legacy = r#"{
            "v": 1,
            "created_at_unix_seconds": 1000,
            "anchor_pane_id": 5,
            "layout": {"kind": "leaf", "pane_id": 5},
            "cwds": {}
        }"#;
        let snap: SessionSnapshot = serde_json::from_str(legacy).unwrap();
        assert_eq!(snap.anchors(), vec![5]);
        assert!(snap.anchor_pane_ids.is_empty());
    }

    fn tempdir() -> PathBuf {
        let mut base = std::env::temp_dir();
        base.push(format!("lmux-state-test-{}", now_unix_seconds_nanos()));
        std::fs::create_dir_all(&base).unwrap_or_else(|e| panic!("mkdir: {e}"));
        base
    }

    fn now_unix_seconds_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
