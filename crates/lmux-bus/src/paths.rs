//! Socket path resolution.

use std::path::PathBuf;

use crate::error::BusError;

/// Resolve the lmux bus socket path.
///
/// Linux follows `$XDG_RUNTIME_DIR/lmux/bus.sock`. macOS commonly has no
/// `XDG_RUNTIME_DIR`, so fall back to a user-private directory under
/// `$TMPDIR` (or `/tmp`) while keeping the same `lmux/bus.sock` suffix.
pub fn bus_socket_path() -> Result<PathBuf, BusError> {
    if let Ok(s) = std::env::var("XDG_RUNTIME_DIR") {
        if !s.is_empty() {
            return Ok(PathBuf::from(s).join("lmux").join("bus.sock"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        return Ok(base
            .join(format!("lmux-{}", unsafe { libc::getuid() }))
            .join("bus.sock"));
    }
    #[cfg(not(target_os = "macos"))]
    Err(BusError::Domain(
        "$XDG_RUNTIME_DIR is not set; cannot resolve bus.sock path".into(),
    ))
}

/// Companion pid-file path, `bus.sock.pid`, used for stale-socket recovery.
pub fn bus_pid_path() -> Result<PathBuf, BusError> {
    let mut p = bus_socket_path()?;
    let mut os = p.into_os_string();
    os.push(".pid");
    p = PathBuf::from(os);
    Ok(p)
}
