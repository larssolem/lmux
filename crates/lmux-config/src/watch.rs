//! Config file watcher (Epic 10). Re-reads the TOML on disk when it
//! changes and fires the installed callback with the freshly-parsed
//! [`Config`].
//!
//! Debouncing matters here: most editors write by truncating + rewriting,
//! which emits several filesystem events in rapid succession. We collect
//! events with a short grace window before parsing so one user save never
//! triggers more than one reload.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;

use crate::{load, Config, LoadError};

/// Debounce window. Editors that truncate-then-rewrite (vim's default,
/// emacs backup-then-rename, VS Code atomic write) can spray 2–4 events
/// within ~10 ms. 150 ms is well above that without feeling laggy.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(150);

/// Owns the background thread + the [`notify`] watcher handle. Dropping
/// the handle stops the watcher and joins the thread.
pub struct WatchHandle {
    // `Option` so `drop` can take the watcher and drop it *before* joining
    // the worker thread. Dropping the watcher closes the channel used to
    // deliver events, which unblocks the worker's `rx.recv()` with `Err`.
    watcher: Option<RecommendedWatcher>,
    stop_tx: mpsc::Sender<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        // Close the notify → worker channel so the worker's blocking
        // `rx.recv()` returns immediately. Without this, the join below
        // would wait forever.
        drop(self.watcher.take());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Watch `path` for changes. On every debounced change `on_change` is
/// invoked with either the re-parsed [`Config`] or the parse/IO error.
/// Runs on its own thread; the callback is NOT invoked from the GTK main
/// loop, so callers on the GTK side must dispatch to `glib::idle_add_local`
/// (or similar) before mutating widgets.
///
/// The watcher monitors the parent directory rather than the file itself:
/// atomic-write editors delete-and-recreate the file, which invalidates
/// a file-level watch immediately. Directory-level watches survive that
/// pattern.
pub fn spawn<F>(path: impl AsRef<Path>, mut on_change: F) -> Result<WatchHandle, WatchError>
where
    F: FnMut(Result<Config, LoadError>) + Send + 'static,
{
    let path = path.as_ref().to_path_buf();
    let parent = path
        .parent()
        .ok_or_else(|| WatchError::NoParent(path.clone()))?
        .to_path_buf();

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(WatchError::Notify)?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .map_err(WatchError::Notify)?;

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let path_for_thread = path.clone();
    let join = thread::Builder::new()
        .name("lmux-config-watch".into())
        .spawn(move || {
            worker(rx, stop_rx, path_for_thread, &mut on_change);
        })
        .map_err(WatchError::Spawn)?;

    Ok(WatchHandle {
        watcher: Some(watcher),
        stop_tx,
        join: Some(join),
    })
}

fn worker<F>(
    rx: mpsc::Receiver<notify::Result<Event>>,
    stop_rx: mpsc::Receiver<()>,
    target: PathBuf,
    on_change: &mut F,
) where
    F: FnMut(Result<Config, LoadError>),
{
    loop {
        // Block for the next event (or stop). `recv` returns Err when the
        // sender is dropped; that happens when the watcher dies, so we
        // exit cleanly.
        let first = match rx.recv() {
            Ok(ev) => ev,
            Err(_) => return,
        };
        if stop_rx.try_recv().is_ok() {
            return;
        }
        if !event_touches(&first, &target) {
            // Keep waiting for a relevant event — no sense starting the
            // debounce window for an unrelated sibling file.
            continue;
        }
        // Drain any follow-up events that arrive within the debounce
        // window. Hitting the timeout means the editor is done writing.
        let deadline = std::time::Instant::now() + DEBOUNCE_WINDOW;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        if stop_rx.try_recv().is_ok() {
            return;
        }
        let result = load(&target);
        on_change(result);
    }
}

fn event_touches(event: &notify::Result<Event>, target: &Path) -> bool {
    let Ok(event) = event else {
        return false;
    };
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|p| p == target)
}

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("path has no parent directory: {0:?}")]
    NoParent(PathBuf),
    #[error("notify: {0}")]
    Notify(#[from] notify::Error),
    #[error("spawn watcher thread: {0}")]
    Spawn(std::io::Error),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    use super::*;

    fn tempdir() -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }

    #[test]
    fn writing_the_config_fires_a_single_reload() {
        let dir = tempdir();
        let path = dir.join("config.toml");
        std::fs::write(&path, b"[general]\nfont_size = 11\n").unwrap();

        let (tx, rx) = std_mpsc::channel();
        let handle = spawn(&path, move |res| {
            let _ = tx.send(res);
        })
        .unwrap();

        // A single logical "save" — write twice back-to-back to mimic
        // an editor's truncate + write sequence.
        std::fs::write(&path, b"[general]\nfont_size = 13\n").unwrap();
        std::fs::write(&path, b"[general]\nfont_size = 13\n").unwrap();

        let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let cfg = first.unwrap();
        assert_eq!(cfg.general.font_size, 13);

        // Any further events should be absorbed into the same debounce
        // window — `recv_timeout` after the debounce window + slack must
        // not produce a second reload unless someone writes again.
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());

        drop(handle);
    }

    #[test]
    fn unrelated_sibling_file_does_not_fire() {
        let dir = tempdir();
        let path = dir.join("config.toml");
        std::fs::write(&path, b"").unwrap();

        let (tx, rx) = std_mpsc::channel();
        let handle = spawn(&path, move |res| {
            let _ = tx.send(res);
        })
        .unwrap();

        std::fs::write(dir.join("unrelated.txt"), b"hi").unwrap();

        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
        drop(handle);
    }
}
