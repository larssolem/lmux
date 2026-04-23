//! KWin integration over D-Bus + the `lmux-dock` KWin script.
//!
//! # v0.2 status
//!
//! * `ensure_script_loaded` — WIRED live: verifies the script file exists,
//!   connects to the session bus, queries `org.kde.kwin.Scripting.isScriptLoaded`
//!   and — if the plugin is not loaded — calls `loadScript(path, plugin)`
//!   followed by `run()` on the returned `/Scripting/ScriptN` object.
//! * `health` — WIRED live: probes the scripting proxy; returns `Offline`
//!   when the connection or proxy fails, `ScriptMissing` when the plugin
//!   reports not-loaded (or the file is gone), `Online` otherwise.
//! * `spawn_satellite`, `set_geometry`, `detach`, `attach` — STUBBED:
//!   return `Ok(())` for v0.2. Wiring them to the script lives in the next
//!   iteration when the compositor-IPC spike's phase-2 wlroots work lands.

use async_trait::async_trait;
use uuid::Uuid;
use zbus::Connection;

use crate::{CompositorControl, CompositorError, Health, Rect, WindowId};

const KWIN_SERVICE: &str = "org.kde.KWin";
const KWIN_SCRIPTING_PATH: &str = "/Scripting";
const KWIN_SCRIPTING_IFACE: &str = "org.kde.kwin.Scripting";
const KWIN_SCRIPT_IFACE: &str = "org.kde.kwin.Script";
const LMUX_PLUGIN_NAME: &str = "lmux-dock";

/// KWin-backed `CompositorControl`. See module-level docs for the v0.2
/// implementation status table.
#[derive(Debug)]
pub struct KwinCompositor {
    /// Path to the lmux-dock script on disk. The v0.2 default is
    /// `share/lmux/kwin/lmux-dock.js` relative to the installed prefix.
    script_path: String,
}

impl KwinCompositor {
    /// Construct with a path to the lmux-dock script. The path is not
    /// touched until [`CompositorControl::ensure_script_loaded`] is called.
    pub fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
        }
    }

    /// Script path this compositor wrapper will try to load.
    pub fn script_path(&self) -> &str {
        &self.script_path
    }

    async fn scripting_proxy(&self) -> Result<(Connection, zbus::Proxy<'static>), CompositorError> {
        let conn = Connection::session()
            .await
            .map_err(|e| CompositorError::Offline(format!("session bus: {e}")))?;
        let proxy = zbus::Proxy::new(
            &conn,
            KWIN_SERVICE,
            KWIN_SCRIPTING_PATH,
            KWIN_SCRIPTING_IFACE,
        )
        .await
        .map_err(|e| CompositorError::Offline(format!("scripting proxy: {e}")))?;
        Ok((conn, proxy))
    }
}

#[async_trait]
impl CompositorControl for KwinCompositor {
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError> {
        match tokio::fs::metadata(&self.script_path).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(CompositorError::ScriptMissing);
            }
            Err(err) => return Err(CompositorError::Io(err)),
        }

        let (conn, scripting) = self.scripting_proxy().await?;

        let already: bool = scripting
            .call("isScriptLoaded", &(LMUX_PLUGIN_NAME,))
            .await
            .map_err(|e| CompositorError::Domain(format!("isScriptLoaded: {e}")))?;
        if already {
            return Ok(());
        }

        let script_id: i32 = scripting
            .call("loadScript", &(self.script_path.as_str(), LMUX_PLUGIN_NAME))
            .await
            .map_err(|e| CompositorError::Domain(format!("loadScript: {e}")))?;

        let script_obj_path = format!("{KWIN_SCRIPTING_PATH}/Script{script_id}");
        let script_proxy = zbus::Proxy::new(
            &conn,
            KWIN_SERVICE,
            script_obj_path.as_str(),
            KWIN_SCRIPT_IFACE,
        )
        .await
        .map_err(|e| CompositorError::Domain(format!("script proxy: {e}")))?;

        script_proxy
            .call::<_, _, ()>("run", &())
            .await
            .map_err(|e| CompositorError::Domain(format!("run: {e}")))?;

        Ok(())
    }

    async fn health(&self) -> Health {
        if tokio::fs::metadata(&self.script_path).await.is_err() {
            return Health::ScriptMissing;
        }
        let (_conn, scripting) = match self.scripting_proxy().await {
            Ok(pair) => pair,
            Err(CompositorError::Offline(reason)) => return Health::Offline { reason },
            Err(e) => {
                return Health::Offline {
                    reason: e.to_string(),
                }
            }
        };
        match scripting
            .call::<_, _, bool>("isScriptLoaded", &(LMUX_PLUGIN_NAME,))
            .await
        {
            Ok(true) => Health::Online,
            Ok(false) => Health::ScriptMissing,
            Err(e) => Health::Offline {
                reason: format!("isScriptLoaded: {e}"),
            },
        }
    }

    async fn spawn_satellite(
        &self,
        argv: &[String],
        cwd: Option<&str>,
    ) -> Result<Uuid, CompositorError> {
        // v0.2: fork the child process directly and stamp it with
        // LMUX_SATELLITE_ID, then schedule a best-effort placement snippet
        // that asks KWin to move the resulting window into a satellite
        // slot (right half of the active screen). Full docking with the
        // main lmux-dock.js script arrives in v0.3.
        let (request_id, pid) = crate::spawn::spawn_tagged_with_pid(argv, cwd)?;
        tracing::info!(pid, %request_id, "kwin: satellite spawned, scheduling placement");
        tokio::spawn(async move {
            match place_window_by_pid(pid).await {
                Ok(()) => tracing::info!(pid, "kwin: placement snippet dispatched"),
                Err(e) => tracing::warn!(pid, error = %e, "kwin: placement snippet failed"),
            }
        });
        Ok(request_id)
    }

    async fn set_geometry(&self, _window: &WindowId, _rect: Rect) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn detach(&self, _window: &WindowId) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn attach(&self, _window: &WindowId) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn set_window_visible_by_pid(
        &self,
        pid: u32,
        visible: bool,
    ) -> Result<(), CompositorError> {
        set_window_visibility_by_pid(pid, visible).await
    }
}

/// Best-effort placement: wait briefly for the spawned window to appear,
/// then inject a one-shot KWin script that finds the client with matching
/// PID and snaps it into the right half of its active screen's placement
/// area. No-op if KWin scripting is unreachable — the window simply stays
/// where the compositor's default placement put it.
async fn place_window_by_pid(pid: u32) -> Result<(), CompositorError> {
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let snippet = format!(
        r#""use strict";
var targetPid = {pid};
var windows = typeof workspace.windowList === "function"
    ? workspace.windowList()
    : (typeof workspace.clientList === "function" ? workspace.clientList() : []);
for (var i = 0; i < windows.length; i++) {{
    var w = windows[i];
    if (!w) {{ continue; }}
    var wpid = typeof w.pid === "number" ? w.pid : -1;
    if (wpid !== targetPid) {{ continue; }}
    var screen = typeof w.screen !== "undefined" ? w.screen : 0;
    var desktop = typeof w.desktop !== "undefined" ? w.desktop : workspace.currentDesktop;
    var g;
    try {{
        g = workspace.clientArea(KWin.PlacementArea, screen, desktop);
    }} catch (e) {{
        g = workspace.clientArea(0, screen, desktop);
    }}
    var half = Math.floor(g.width / 2);
    var rect = Qt.rect(g.x + half, g.y, half, g.height);
    w.frameGeometry = rect;
    print("lmux-place: pid=" + targetPid + " -> " + rect.x + "," + rect.y + " " + rect.width + "x" + rect.height);
    break;
}}
"#
    );
    run_oneshot_script(&snippet, &format!("lmux-place-{pid}")).await
}

/// Show or hide a window by PID. Minimize=hide, unminimize=show. KWin's
/// `minimized` property is writable, so we flip it via a one-shot script.
async fn set_window_visibility_by_pid(pid: u32, visible: bool) -> Result<(), CompositorError> {
    let want_minimized = if visible { "false" } else { "true" };
    let snippet = format!(
        r#""use strict";
var targetPid = {pid};
var wantMin = {want_minimized};
var windows = typeof workspace.windowList === "function"
    ? workspace.windowList()
    : (typeof workspace.clientList === "function" ? workspace.clientList() : []);
for (var i = 0; i < windows.length; i++) {{
    var w = windows[i];
    if (!w) {{ continue; }}
    var wpid = typeof w.pid === "number" ? w.pid : -1;
    if (wpid !== targetPid) {{ continue; }}
    try {{ w.minimized = wantMin; }} catch (e) {{ print("lmux-vis: set minimized failed: " + e); }}
    print("lmux-vis: pid=" + targetPid + " minimized=" + wantMin);
    break;
}}
"#
    );
    run_oneshot_script(&snippet, &format!("lmux-vis-{pid}")).await
}

/// Write `snippet` to a tempfile, `loadScript` + `run` it via KWin's
/// scripting D-Bus, then `unloadScript`. Returns Ok if the dispatch
/// round-trip succeeded; the script's own effect is fire-and-forget.
async fn run_oneshot_script(snippet: &str, plugin_name: &str) -> Result<(), CompositorError> {
    let tmp = tempfile::Builder::new()
        .prefix(plugin_name)
        .suffix(".js")
        .tempfile()
        .map_err(CompositorError::Io)?;
    std::fs::write(tmp.path(), snippet.as_bytes()).map_err(CompositorError::Io)?;

    let conn = Connection::session()
        .await
        .map_err(|e| CompositorError::Offline(format!("session bus: {e}")))?;
    let scripting = zbus::Proxy::new(
        &conn,
        KWIN_SERVICE,
        KWIN_SCRIPTING_PATH,
        KWIN_SCRIPTING_IFACE,
    )
    .await
    .map_err(|e| CompositorError::Offline(format!("scripting proxy: {e}")))?;

    let path_str = tmp.path().to_string_lossy().to_string();
    let script_id: i32 = scripting
        .call("loadScript", &(path_str.as_str(), plugin_name))
        .await
        .map_err(|e| CompositorError::Domain(format!("loadScript: {e}")))?;

    let script_obj_path = format!("{KWIN_SCRIPTING_PATH}/Script{script_id}");
    let script_proxy = zbus::Proxy::new(
        &conn,
        KWIN_SERVICE,
        script_obj_path.as_str(),
        KWIN_SCRIPT_IFACE,
    )
    .await
    .map_err(|e| CompositorError::Domain(format!("script proxy: {e}")))?;

    if let Err(e) = script_proxy.call::<_, _, ()>("run", &()).await {
        tracing::debug!(plugin = %plugin_name, error = %e, "kwin: run failed");
    }
    let _ = scripting
        .call::<_, _, ()>("unloadScript", &(plugin_name,))
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn missing_script_reports_script_missing() {
        let c = KwinCompositor::new("/nonexistent/path/lmux-dock.js");
        assert_eq!(c.health().await, Health::ScriptMissing);
        match c.ensure_script_loaded().await {
            Err(CompositorError::ScriptMissing) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// When the script file exists but no KWin session bus is reachable,
    /// `ensure_script_loaded` must surface `Offline` rather than succeed
    /// silently. Runs only when DBUS_SESSION_BUS_ADDRESS is unset (CI boxes
    /// + headless shells); a developer laptop with a live session bus will
    /// skip this assertion.
    #[tokio::test]
    async fn no_session_bus_reports_offline() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
            // A session bus is present; we can't assert Offline reliably.
            // The live-session path is exercised manually on a KDE box.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lmux-dock.js");
        std::fs::write(&p, b"// stub").unwrap();
        let c = KwinCompositor::new(p.to_string_lossy().to_string());
        match c.ensure_script_loaded().await {
            Err(CompositorError::Offline(_)) => {}
            other => panic!("expected Offline, got {other:?}"),
        }
        match c.health().await {
            Health::Offline { .. } => {}
            other => panic!("expected Offline, got {other:?}"),
        }
    }
}
