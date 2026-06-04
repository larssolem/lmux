//! v0.1 → v0.2 migration of `last-session.json` into the session store.
//!
//! If the user has a v0.1 `last-session.json` at `$XDG_DATA_HOME/lmux/` but
//! no v0.2 `sessions/default.toml`, we copy the snapshot into a session
//! named `default` and leave the JSON file alone. The JSON file remains the
//! authoritative "last active" pointer (FR63); this migration just exposes
//! its contents to the multi-session switcher.
//!
//! The migration is idempotent: re-running it once `default.toml` exists is
//! a no-op.

use std::path::Path;

use lmux_state::LoadOutcome;

use crate::{Session, SessionStore, StoreError};

/// Outcome of an attempted migration.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// Migration ran and created `sessions/default.toml`.
    Migrated,
    /// Nothing to do — either the source was missing or the destination
    /// already existed.
    NotNeeded,
    /// Source file existed but was unparseable; no session was created.
    SourceCorrupt { error: String },
}

/// Migrate a v0.1 `last-session.json` at `source_path` into `store` under
/// the name `default`.
pub fn migrate_v01_last_session(
    source_path: &Path,
    store: &SessionStore,
    now_unix_seconds: u64,
) -> Result<MigrationOutcome, StoreError> {
    // Idempotency gate: if default.toml already exists, skip. We don't
    // overwrite the user's subsequent edits.
    if store.load("default").is_ok() {
        return Ok(MigrationOutcome::NotNeeded);
    }
    match lmux_state::load(source_path) {
        LoadOutcome::Missing => Ok(MigrationOutcome::NotNeeded),
        LoadOutcome::Corrupt { error, .. } => Ok(MigrationOutcome::SourceCorrupt { error }),
        LoadOutcome::Ok(snap) => {
            let session = Session::from_v01_snapshot("default", snap, now_unix_seconds);
            store.save(&session)?;
            tracing::info!(
                source = %source_path.display(),
                "migrated v0.1 last-session.json to sessions/default.toml"
            );
            Ok(MigrationOutcome::Migrated)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::path::PathBuf;

    use lmux_state::{LayoutNode, SessionSnapshot, SplitDir, SCHEMA_VERSION};

    fn tempdir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lmux-migrate-test-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_snapshot() -> SessionSnapshot {
        let mut pane_titles = std::collections::BTreeMap::new();
        pane_titles.insert(
            1,
            lmux_state::PaneTitleSnapshot {
                title: "logs".into(),
                provenance: lmux_state::PaneTitleProvenanceSnapshot::User,
                pinned: true,
            },
        );
        SessionSnapshot {
            v: SCHEMA_VERSION,
            created_at_unix_seconds: 1000,
            anchor_pane_id: Some(1),
            anchor_pane_ids: vec![],
            layout: LayoutNode::Split {
                dir: SplitDir::Horizontal,
                a: Box::new(LayoutNode::Leaf { pane_id: 1 }),
                b: Box::new(LayoutNode::Leaf { pane_id: 2 }),
                ratio: 0.6,
            },
            cwds: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(1, "/home/lars".into());
                m
            },
            pane_titles,
            terminal_tabs: vec![lmux_state::TerminalTabStackSnapshot {
                anchor_pane_id: 1,
                tab_roots: vec![1, 2],
                active_tab: Some(2),
            }],
            pane_terminal_tab_roots: std::collections::BTreeMap::from([(1, 1), (2, 2)]),
        }
    }

    #[test]
    fn migrates_when_source_exists_and_default_absent() {
        let dir = tempdir();
        let store = SessionStore::new(&dir);
        let src = dir.join("last-session.json");
        lmux_state::save(&src, &sample_snapshot()).unwrap();

        let outcome = migrate_v01_last_session(&src, &store, 2000).unwrap();
        assert_eq!(outcome, MigrationOutcome::Migrated);

        let s = store.load("default").unwrap();
        assert_eq!(s.name, "default");
        assert_eq!(s.created_at_unix_seconds, 1000);
        assert_eq!(s.last_opened_at_unix_seconds, 2000);
        assert_eq!(s.cwds.get(&1).map(String::as_str), Some("/home/lars"));
        assert_eq!(
            s.pane_titles.get(&1).map(|title| title.title.as_str()),
            Some("logs")
        );
        assert_eq!(s.terminal_tabs[0].active_tab, Some(2));
        assert_eq!(s.pane_terminal_tab_roots.get(&2), Some(&2));
    }

    #[test]
    fn idempotent_when_default_exists() {
        let dir = tempdir();
        let store = SessionStore::new(&dir);
        store.create("default", 500).unwrap();
        let src = dir.join("last-session.json");
        lmux_state::save(&src, &sample_snapshot()).unwrap();

        let outcome = migrate_v01_last_session(&src, &store, 2000).unwrap();
        assert_eq!(outcome, MigrationOutcome::NotNeeded);

        // Unchanged timestamp.
        let s = store.load("default").unwrap();
        assert_eq!(s.last_opened_at_unix_seconds, 500);
    }

    #[test]
    fn missing_source_is_not_needed() {
        let dir = tempdir();
        let store = SessionStore::new(&dir);
        let outcome =
            migrate_v01_last_session(&dir.join("nonexistent.json"), &store, 2000).unwrap();
        assert_eq!(outcome, MigrationOutcome::NotNeeded);
    }

    #[test]
    fn corrupt_source_surfaces_error_and_skips() {
        let dir = tempdir();
        let store = SessionStore::new(&dir);
        let src = dir.join("last-session.json");
        std::fs::write(&src, b"not json").unwrap();
        let outcome = migrate_v01_last_session(&src, &store, 2000).unwrap();
        assert!(matches!(outcome, MigrationOutcome::SourceCorrupt { .. }));
        // No default.toml was created.
        assert!(store.load("default").is_err());
    }
}
