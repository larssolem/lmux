//! Atomic JSON writer — tempfile + fsync + rename. The single blessed way
//! to persist state under `$XDG_DATA_HOME/lmux/`. NFR7 is enforced by
//! construction: every `last-session.json` write goes through this helper.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("path has no parent directory: {0}")]
    NoParent(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Serialise `value` as pretty JSON to `path` atomically.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_bytes(path, &bytes)
}

/// Atomically write `bytes` to `path`. Writes to a unique temp file in the
/// same directory, `sync_all()`s, then `rename()`s into place. On POSIX,
/// `rename` is atomic within a filesystem. On success the file is 0600 (NFR19).
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path
        .parent()
        .ok_or_else(|| Error::NoParent(path.display().to_string()))?;
    fs::create_dir_all(parent)?;

    let mut tmp_path = unique_tmp_path(path);

    {
        let mut f = loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)
            {
                Ok(file) => break file,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    tmp_path = unique_tmp_path(path);
                }
                Err(err) => return Err(Error::Io(err)),
            }
        };
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match fs::rename(&tmp_path, path) {
        Ok(()) => {
            fs::File::open(parent)?.sync_all()?;
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp_path);
            Err(Error::Io(err))
        }
    }
}

fn unique_tmp_path(path: &Path) -> std::path::PathBuf {
    let pid = std::process::id();
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp.{pid}.{n}"));
    std::path::PathBuf::from(tmp)
}
