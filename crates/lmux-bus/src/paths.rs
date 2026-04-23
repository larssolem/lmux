//! Socket path resolution.

use std::path::PathBuf;

use crate::error::BusError;

/// Resolve `$XDG_RUNTIME_DIR/lmux/bus.sock`. Errors if `$XDG_RUNTIME_DIR`
/// is unset (server cannot safely pick a default — running-user state must
/// live under the per-session runtime dir).
pub fn bus_socket_path() -> Result<PathBuf, BusError> {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(s) if !s.is_empty() => Ok(PathBuf::from(s).join("lmux").join("bus.sock")),
        _ => Err(BusError::Domain(
            "$XDG_RUNTIME_DIR is not set; cannot resolve bus.sock path".into(),
        )),
    }
}

/// Companion pid-file path, `bus.sock.pid`, used for stale-socket recovery.
pub fn bus_pid_path() -> Result<PathBuf, BusError> {
    let mut p = bus_socket_path()?;
    let mut os = p.into_os_string();
    os.push(".pid");
    p = PathBuf::from(os);
    Ok(p)
}
