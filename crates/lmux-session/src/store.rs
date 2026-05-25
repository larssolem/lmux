//! Session CRUD + on-disk layout.
//!
//! Layout (under [`crate::state_home`]):
//!
//! ```text
//! lmux/
//!   sessions/
//!     index.toml           -- SessionIndex
//!     <name>.toml          -- Session
//!     <name>.toml.bad.<ts> -- renamed-aside when parse fails
//! ```

use std::path::{Path, PathBuf};

use lmux_state::atomic_write;
use thiserror::Error;

use crate::{IndexEntry, Session, SessionIndex};

/// Top-level error for the session store.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("session name {name:?} rejected: {reason}")]
    InvalidName { name: String, reason: &'static str },

    #[error("session {name:?} already exists")]
    AlreadyExists { name: String },

    #[error("session {name:?} not found")]
    NotFound { name: String },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("atomic write: {0}")]
    AtomicWrite(#[from] atomic_write::Error),

    #[error("toml encode: {0}")]
    TomlEncode(#[from] toml::ser::Error),

    #[error("toml decode of {file}: {source}")]
    TomlDecode {
        file: String,
        #[source]
        source: toml::de::Error,
    },
}

/// File-system-backed session store rooted at `<state_home>/sessions/`.
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// Construct a store rooted at `root/sessions/`. Does not touch disk.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let mut r: PathBuf = root.into();
        r.push("sessions");
        Self { root: r }
    }

    /// The `sessions/` directory this store owns.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn session_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.toml"))
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.toml")
    }

    /// Create a new session with `name`. Fails if it already exists or if
    /// the name is invalid. FR1.
    pub fn create(&self, name: &str, now_unix_seconds: u64) -> Result<Session, StoreError> {
        validate_name(name)?;
        if self.session_path(name).exists() {
            return Err(StoreError::AlreadyExists { name: name.into() });
        }
        let session = Session::empty(name, now_unix_seconds);
        self.write_session(&session)?;
        self.bump_index(name, now_unix_seconds)?;
        Ok(session)
    }

    /// Rename a session. FR2.
    pub fn rename(&self, from: &str, to: &str) -> Result<(), StoreError> {
        validate_name(from)?;
        validate_name(to)?;
        let src = self.session_path(from);
        let dst = self.session_path(to);
        if !src.exists() {
            return Err(StoreError::NotFound { name: from.into() });
        }
        if dst.exists() {
            return Err(StoreError::AlreadyExists { name: to.into() });
        }
        let mut session = self.load(from)?;
        session.name = to.into();
        self.write_session(&session)?;
        // On disk: write the new file (done above) then remove the old.
        std::fs::remove_file(&src)?;

        let mut index = self.read_index()?;
        index.rename(from, to);
        self.write_index(&index)?;
        Ok(())
    }

    /// Delete a session. FR3. No-op if missing.
    pub fn delete(&self, name: &str) -> Result<(), StoreError> {
        validate_name(name)?;
        let p = self.session_path(name);
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
        let mut index = self.read_index()?;
        index.remove(name);
        self.write_index(&index)?;
        Ok(())
    }

    /// Load a session by name. Malformed files are renamed aside as
    /// `<name>.toml.bad.<ts>` and an empty session with the same name is
    /// returned; FR62 (corruption never blocks startup).
    pub fn load(&self, name: &str) -> Result<Session, StoreError> {
        validate_name(name)?;
        let path = self.session_path(name);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound { name: name.into() });
            }
            Err(err) => return Err(StoreError::Io(err)),
        };
        let text = String::from_utf8_lossy(&bytes);
        match toml::from_str::<Session>(&text) {
            Ok(s) => Ok(s),
            Err(err) => {
                tracing::warn!(session = name, error = %err, "session file malformed, renaming aside");
                rename_aside(&path);
                Ok(Session::empty(name, crate::now_unix_seconds()))
            }
        }
    }

    /// List sessions sorted by recency (most-recent first). FR4.
    pub fn list(&self) -> Result<Vec<IndexEntry>, StoreError> {
        Ok(self.read_index()?.entries)
    }

    /// Write a session back to disk and bump its index entry. Called on
    /// session-open and before a switcher swap (FR10).
    pub fn save(&self, session: &Session) -> Result<(), StoreError> {
        validate_name(&session.name)?;
        self.write_session(session)?;
        self.bump_index(&session.name, session.last_opened_at_unix_seconds)?;
        Ok(())
    }

    fn write_session(&self, session: &Session) -> Result<(), StoreError> {
        let text = toml::to_string_pretty(session)?;
        atomic_write::write_bytes(&self.session_path(&session.name), text.as_bytes())?;
        Ok(())
    }

    fn read_index(&self) -> Result<SessionIndex, StoreError> {
        let path = self.index_path();
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SessionIndex::default());
            }
            Err(err) => return Err(StoreError::Io(err)),
        };
        let text = String::from_utf8_lossy(&bytes);
        toml::from_str::<SessionIndex>(&text).map_err(|source| StoreError::TomlDecode {
            file: path.display().to_string(),
            source,
        })
    }

    fn write_index(&self, index: &SessionIndex) -> Result<(), StoreError> {
        let text = toml::to_string_pretty(index)?;
        atomic_write::write_bytes(&self.index_path(), text.as_bytes())?;
        Ok(())
    }

    fn bump_index(&self, name: &str, now_unix_seconds: u64) -> Result<(), StoreError> {
        let mut index = self.read_index()?;
        index.touch(name, now_unix_seconds);
        self.write_index(&index)
    }
}

/// Validate a session name: non-empty, ≤64 chars, `[A-Za-z0-9._-]+`, no
/// leading dot (so `.toml` files aren't confused with `<name>.toml`).
pub fn valid_name(name: &str) -> bool {
    validate_name(name).is_ok()
}

fn validate_name(name: &str) -> Result<(), StoreError> {
    if name.is_empty() {
        return Err(StoreError::InvalidName {
            name: name.into(),
            reason: "empty",
        });
    }
    if name.len() > 64 {
        return Err(StoreError::InvalidName {
            name: name.into(),
            reason: "longer than 64 chars",
        });
    }
    if name.starts_with('.') {
        return Err(StoreError::InvalidName {
            name: name.into(),
            reason: "leading dot not allowed",
        });
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok {
        return Err(StoreError::InvalidName {
            name: name.into(),
            reason: "only [A-Za-z0-9._-] permitted",
        });
    }
    Ok(())
}

fn rename_aside(path: &Path) {
    let ts = crate::now_unix_seconds();
    let mut target = path.as_os_str().to_owned();
    target.push(format!(".bad.{ts}"));
    let target = PathBuf::from(target);
    if let Err(err) = std::fs::rename(path, &target) {
        tracing::warn!(error = %err, target = %target.display(), "failed to rename corrupt session aside");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tempdir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lmux-session-test-{nanos}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_lists_and_loads() {
        let store = SessionStore::new(tempdir());
        let s = store.create("alpha", 100).unwrap();
        assert_eq!(s.name, "alpha");
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "alpha");

        let back = store.load("alpha").unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn create_rejects_duplicates() {
        let store = SessionStore::new(tempdir());
        store.create("alpha", 100).unwrap();
        let err = store.create("alpha", 101).unwrap_err();
        assert!(matches!(err, StoreError::AlreadyExists { .. }));
    }

    #[test]
    fn rename_moves_file_and_index() {
        let store = SessionStore::new(tempdir());
        store.create("alpha", 100).unwrap();
        store.rename("alpha", "beta").unwrap();

        assert!(store.load("alpha").is_err());
        let s = store.load("beta").unwrap();
        assert_eq!(s.name, "beta");

        let names: Vec<_> = store.list().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["beta".to_string()]);
    }

    #[test]
    fn delete_removes_from_store_and_index() {
        let store = SessionStore::new(tempdir());
        store.create("alpha", 100).unwrap();
        store.delete("alpha").unwrap();
        assert!(store.load("alpha").is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn delete_is_idempotent() {
        let store = SessionStore::new(tempdir());
        store.delete("ghost").unwrap();
    }

    #[test]
    fn list_is_recency_sorted() {
        let store = SessionStore::new(tempdir());
        store.create("a", 100).unwrap();
        store.create("b", 200).unwrap();
        store.create("c", 150).unwrap();
        let names: Vec<_> = store.list().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn save_bumps_recency() {
        let store = SessionStore::new(tempdir());
        let mut a = store.create("a", 100).unwrap();
        let _b = store.create("b", 200).unwrap();
        a.last_opened_at_unix_seconds = 300;
        store.save(&a).unwrap();
        let names: Vec<_> = store.list().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn invalid_names_rejected() {
        let store = SessionStore::new(tempdir());
        for bad in [
            "",
            ".hidden",
            "has space",
            "a/b",
            "a\x00b",
            "a".repeat(65).as_str(),
        ] {
            let err = store.create(bad, 100).unwrap_err();
            assert!(
                matches!(err, StoreError::InvalidName { .. }),
                "name: {bad:?}"
            );
        }
    }

    #[test]
    fn unsafe_names_are_rejected_for_all_path_based_operations() {
        let store = SessionStore::new(tempdir());
        store.create("alpha", 100).unwrap();

        for bad in ["../escape", "nested/name", ".hidden", "has space"] {
            assert!(matches!(
                store.load(bad),
                Err(StoreError::InvalidName { .. })
            ));
            assert!(matches!(
                store.delete(bad),
                Err(StoreError::InvalidName { .. })
            ));
            assert!(matches!(
                store.rename(bad, "beta"),
                Err(StoreError::InvalidName { .. })
            ));
            assert!(matches!(
                store.rename("alpha", bad),
                Err(StoreError::InvalidName { .. })
            ));
        }
    }

    #[test]
    fn malformed_file_returns_empty_session_and_renames_aside() {
        let dir = tempdir();
        let store = SessionStore::new(&dir);
        store.create("alpha", 100).unwrap();
        // Corrupt the file.
        let p = store.root().join("alpha.toml");
        std::fs::write(&p, b"this is not valid toml [[[").unwrap();
        let loaded = store.load("alpha").unwrap();
        assert_eq!(loaded.name, "alpha");
        // Ring buffer is empty on a fresh empty session.
        assert!(loaded.anchors.is_empty());
        // `.bad.<ts>` sibling exists.
        let bad = std::fs::read_dir(store.root()).unwrap().any(|e| {
            let name = e.unwrap().file_name();
            name.to_string_lossy().starts_with("alpha.toml.bad.")
        });
        assert!(bad, "expected alpha.toml.bad.<ts>");
    }

    #[test]
    fn index_file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let store = SessionStore::new(tempdir());
        store.create("alpha", 100).unwrap();
        let meta = std::fs::metadata(store.root().join("index.toml")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
