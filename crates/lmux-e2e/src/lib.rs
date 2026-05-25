//! End-to-end test harness for lmux binaries.
//!
//! E2E tests in this crate spawn the actual installed-workspace binaries
//! (`lmux-cli`, eventually `lmux`) via `assert_cmd` and drive them as
//! black boxes. Each test gets its own tempdir and overrides every
//! relevant XDG variable so nothing leaks into `$HOME`.
//!
//! The strategy this crate implements is described in
//! `docs/history/e2e-test-strategy.md`.

#![forbid(unsafe_op_in_unsafe_fn)]
// This crate is test-only infrastructure; panics on setup failure are
// the right behaviour so NFR11's production unwrap/expect ban does not
// apply here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::CommandCargoExt;
use tempfile::TempDir;

/// Per-test environment. Owns a `TempDir` that cleans up on drop, and
/// exposes the XDG directories the cockpit + CLI read from.
pub struct Env {
    _tmp: TempDir,
    root: PathBuf,
}

impl Env {
    /// Create a fresh sandbox. Panics if `tempfile` cannot allocate a
    /// directory — that's a hard test-infrastructure failure, not a
    /// product failure, so it's acceptable to surface as a panic here.
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("tempfile::tempdir: {e}"));
        let root = tmp.path().to_path_buf();
        for sub in ["state", "runtime", "config", "data", "home"] {
            std::fs::create_dir_all(root.join(sub))
                .unwrap_or_else(|e| panic!("create_dir_all {sub}: {e}"));
        }
        Self { _tmp: tmp, root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_home(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn config_home(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn data_home(&self) -> PathBuf {
        self.root.join("data")
    }

    /// Invoke the named workspace binary with every XDG var pointing
    /// into this sandbox and `$HOME` redirected as a fallback.
    pub fn cli(&self, bin: &str) -> Command {
        let mut cmd = Command::cargo_bin(bin).unwrap_or_else(|e| panic!("cargo_bin {bin}: {e}"));
        self.apply_env(&mut cmd);
        cmd
    }

    /// Start a real `lmux` cockpit process in this sandbox.
    pub fn spawn_lmux(&self) -> RunningLmux {
        let mut cmd = Command::cargo_bin("lmux").unwrap_or_else(|e| panic!("cargo_bin lmux: {e}"));
        self.apply_env(&mut cmd);
        let log_path = self.root.join("lmux.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap_or_else(|e| panic!("open lmux log {}: {e}", log_path.display()));
        let log_err = log
            .try_clone()
            .unwrap_or_else(|e| panic!("clone lmux log {}: {e}", log_path.display()));
        cmd.stdout(Stdio::from(log)).stderr(Stdio::from(log_err));
        let child = cmd.spawn().unwrap_or_else(|e| panic!("spawn lmux: {e}"));
        RunningLmux { child, log_path }
    }

    fn apply_env(&self, cmd: &mut Command) {
        cmd.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.root.join("home"))
            .env("XDG_STATE_HOME", self.state_home())
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env("XDG_CONFIG_HOME", self.config_home())
            .env("XDG_DATA_HOME", self.data_home());
        for key in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "SSH_AUTH_SOCK",
            "LANG",
            "LC_ALL",
            "RUST_LOG",
            "RUST_BACKTRACE",
        ] {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
    }

    /// Pre-seed a named session in `$XDG_STATE_HOME/lmux/sessions/`.
    /// Returns the backing `SessionStore` so the caller can read it back.
    pub fn seed_session(&self, name: &str, now_unix_seconds: u64) -> lmux_session::SessionStore {
        let store = lmux_session::SessionStore::new(self.state_home().join("lmux"));
        store
            .create(name, now_unix_seconds)
            .unwrap_or_else(|e| panic!("seed session {name}: {e}"));
        store
    }
}

pub struct RunningLmux {
    child: Child,
    log_path: PathBuf,
}

impl RunningLmux {
    pub fn wait_until_ready(&mut self, env: &Env) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .unwrap_or_else(|e| panic!("lmux try_wait: {e}"))
            {
                panic!("lmux exited before ready: {status}");
            }
            let output = env
                .cli("lmux-cli")
                .arg("status")
                .output()
                .unwrap_or_else(|e| panic!("lmux-cli status: {e}"));
            if output.status.success() {
                return;
            }
            assert!(Instant::now() < deadline, "lmux did not become ready");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }
}

impl Drop for RunningLmux {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}
