//! PTY spawn + IO plumbing on top of portable-pty.

use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("openpty failed: {0}")]
    Open(String),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("clone_reader failed: {0}")]
    CloneReader(String),
    #[error("take_writer failed: {0}")]
    TakeWriter(String),
    #[error("resize failed: {0}")]
    Resize(String),
}

pub struct Pane {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Pane {
    pub fn writer(&mut self) -> &mut (dyn Write + Send) {
        &mut *self.writer
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_w_px: u16,
        cell_h_px: u16,
    ) -> Result<(), Error> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: cols.saturating_mul(cell_w_px),
                pixel_height: rows.saturating_mul(cell_h_px),
            })
            .map_err(|e| Error::Resize(e.to_string()))?;
        Ok(())
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        self.child.try_wait()
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    /// Send SIGTERM to the child. Returns `Ok(())` even if the child has
    /// already exited (ESRCH is swallowed). Epic 3 / Epic 7 orchestrate the
    /// 500 ms grace period on top of this, then fall back to `kill()`.
    pub fn terminate(&self) -> std::io::Result<()> {
        let Some(pid) = self.child.process_id() else {
            return Ok(());
        };
        let pid = pid as libc::pid_t;
        // SAFETY: libc::kill is always safe to call; signal = SIGTERM is the
        // standard cooperative termination signal.
        let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(err)
    }

    /// Read the child's current working directory from `/proc/<pid>/cwd`.
    /// Returns `None` if the child is gone or the link can't be read. Used to
    /// propagate CWD into sibling panes when splitting.
    pub fn cwd(&self) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "linux")]
        {
            let pid = self.child.process_id()?;
            std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }
}

pub struct SpawnOpts<'a> {
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    pub cwd: Option<&'a Path>,
}

/// Environment variable used to pass an alternate trampoline executable
/// path (the one that sets `PR_SET_PDEATHSIG`). Normally unused — we read
/// `std::env::current_exe()`. Exposed as an override for tests.
pub const TRAMPOLINE_ENV: &str = "LMUX_TRAMPOLINE";

fn trampoline_path() -> Option<std::path::PathBuf> {
    if let Some(v) = std::env::var_os(TRAMPOLINE_ENV) {
        if v.is_empty() {
            return None;
        }
        return Some(std::path::PathBuf::from(v));
    }
    let exe = std::env::current_exe().ok()?;
    // In dev loops (`cargo run` after `cargo build`) the parent's exe
    // may have been replaced on disk. Linux reports the path as
    // "/path/to/bin (deleted)" via /proc/self/exe, which naturally
    // doesn't exist. Fall back to a trampoline-less spawn so the `+`
    // button still creates panes after a rebuild.
    if exe.exists() {
        Some(exe)
    } else {
        None
    }
}

pub fn detect_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Open a new PTY, spawn `shell` in it, and return the pane handle plus a
/// blocking reader for the master side. The reader is a separate `Read` trait
/// object so the caller can move it into a dedicated thread.
pub fn spawn(opts: SpawnOpts<'_>) -> Result<(Pane, Box<dyn Read + Send>), Error> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: opts.rows,
            cols: opts.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| Error::Open(e.to_string()))?;

    // Wrap the shell in a trampoline that sets PR_SET_PDEATHSIG(SIGTERM)
    // before execing the real shell. On Linux this guarantees every PTY
    // child receives SIGTERM within one scheduler tick of the lmux parent
    // dying unexpectedly (Story 7.3 / FR34 / NFR8). When the trampoline
    // can't be located (e.g., unit tests with no current_exe) we fall back
    // to a direct spawn — still correct, just without the PDEATHSIG safety
    // net.
    let mut cmd = if let Some(trampoline) = trampoline_path() {
        let mut c = CommandBuilder::new(&trampoline);
        c.arg("--exec-pty");
        c.arg(&opts.shell);
        c
    } else {
        CommandBuilder::new(&opts.shell)
    };
    cmd.env("TERM", "xterm-256color");
    if let Some(cwd) = opts.cwd {
        cmd.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| Error::Spawn(e.to_string()))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| Error::CloneReader(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| Error::TakeWriter(e.to_string()))?;

    Ok((
        Pane {
            master: pair.master,
            writer,
            child,
        },
        reader,
    ))
}
