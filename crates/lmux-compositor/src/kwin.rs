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
//! * Native attach/list/control — WIRED through `lmux-dock.js` calling back
//!   into this process over D-Bus with exact KWin window identities.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use zbus::zvariant::{Fd, OwnedValue, Value};
use zbus::Connection;

use crate::{
    CompositorControl, CompositorError, Health, Rect, SatelliteWindowId, WindowAppIdentity,
    WindowCandidate, WindowCandidateBackend, WindowControlCapabilities, WindowId, WindowPreview,
    WindowPreviewData,
};

const KWIN_SERVICE: &str = "org.kde.KWin";
const KWIN_SCRIPTING_PATH: &str = "/Scripting";
const KWIN_SCRIPTING_IFACE: &str = "org.kde.kwin.Scripting";
const KWIN_SCRIPT_IFACE: &str = "org.kde.kwin.Script";
const KWIN_SCREENSHOT_PATH: &str = "/org/kde/KWin/ScreenShot2";
const KWIN_SCREENSHOT_IFACE: &str = "org.kde.KWin.ScreenShot2";
const LMUX_PLUGIN_NAME: &str = "lmux-dock";
const LMUX_BRIDGE_SERVICE: &str = "no.jpro.lmux.KWinBridge";
const LMUX_BRIDGE_PATH: &str = "/no/jpro/lmux/KWinBridge";

/// KWin-backed `CompositorControl`. See module-level docs for the v0.2
/// implementation status table.
#[derive(Debug)]
pub struct KwinCompositor {
    /// Path to the lmux-dock script on disk. The v0.2 default is
    /// `share/lmux/kwin/lmux-dock.js` relative to the installed prefix.
    script_path: String,
    inventory: Arc<Mutex<HashMap<String, WindowCandidate>>>,
    bridge_connection: Mutex<Option<Connection>>,
    bridge_start_lock: tokio::sync::Mutex<()>,
    bridge_token: String,
}

impl KwinCompositor {
    /// Construct with a path to the lmux-dock script. The path is not
    /// touched until [`CompositorControl::ensure_script_loaded`] is called.
    pub fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            inventory: Arc::new(Mutex::new(HashMap::new())),
            bridge_connection: Mutex::new(None),
            bridge_start_lock: tokio::sync::Mutex::new(()),
            bridge_token: Uuid::new_v4().to_string(),
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

    async fn ensure_bridge_started(&self) -> Result<(), CompositorError> {
        let _start_guard = self.bridge_start_lock.lock().await;
        if self
            .bridge_connection
            .lock()
            .map_err(|_| CompositorError::Domain("KWin bridge lock poisoned".into()))?
            .is_some()
        {
            return Ok(());
        }

        let iface = KwinBridgeInterface {
            inventory: self.inventory.clone(),
            token: self.bridge_token.clone(),
        };
        let conn = zbus::connection::Builder::session()
            .map_err(|e| CompositorError::Offline(format!("session bus: {e}")))?
            .name(LMUX_BRIDGE_SERVICE)
            .map_err(|e| CompositorError::Domain(format!("bridge name: {e}")))?
            .serve_at(LMUX_BRIDGE_PATH, iface)
            .map_err(|e| CompositorError::Domain(format!("bridge object: {e}")))?
            .build()
            .await
            .map_err(|e| CompositorError::Offline(format!("bridge connection: {e}")))?;

        let mut guard = self
            .bridge_connection
            .lock()
            .map_err(|_| CompositorError::Domain("KWin bridge lock poisoned".into()))?;
        *guard = Some(conn);
        Ok(())
    }

    async fn refresh_inventory(&self) -> Result<(), CompositorError> {
        self.ensure_bridge_started().await?;
        let snippet = kwin_snapshot_script(&self.bridge_token)?;
        run_oneshot_script(&snippet, "lmux-snapshot").await?;
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct KwinBridgeInterface {
    inventory: Arc<Mutex<HashMap<String, WindowCandidate>>>,
    token: String,
}

#[zbus::interface(name = "no.jpro.lmux.KWinBridge")]
impl KwinBridgeInterface {
    fn replace_windows(&self, token: &str, json: &str) -> zbus::fdo::Result<()> {
        self.validate_token(token)?;
        let windows = parse_kwin_window_snapshot(json).map_err(zbus_failed)?;
        let mut guard = self
            .inventory
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("KWin inventory lock poisoned".into()))?;
        guard.clear();
        for window in windows {
            guard.insert(window.backend_window_id.clone(), window);
        }
        Ok(())
    }

    fn upsert_window(&self, token: &str, json: &str) -> zbus::fdo::Result<()> {
        self.validate_token(token)?;
        let mut windows = parse_kwin_window_snapshot(json).map_err(zbus_failed)?;
        let Some(window) = windows.pop() else {
            return Ok(());
        };
        self.inventory
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("KWin inventory lock poisoned".into()))?
            .insert(window.backend_window_id.clone(), window);
        Ok(())
    }

    fn remove_window(&self, token: &str, backend_window_id: &str) -> zbus::fdo::Result<()> {
        self.validate_token(token)?;
        self.inventory
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("KWin inventory lock poisoned".into()))?
            .remove(backend_window_id);
        Ok(())
    }

    fn ping(&self) -> &str {
        "ok"
    }
}

impl KwinBridgeInterface {
    fn validate_token(&self, token: &str) -> zbus::fdo::Result<()> {
        if token == self.token {
            Ok(())
        } else {
            Err(zbus::fdo::Error::AccessDenied(
                "invalid lmux KWin bridge token".into(),
            ))
        }
    }
}

fn zbus_failed(err: CompositorError) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(err.to_string())
}

#[async_trait]
impl CompositorControl for KwinCompositor {
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError> {
        match std::fs::metadata(&self.script_path) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(CompositorError::ScriptMissing);
            }
            Err(err) => return Err(CompositorError::Io(err)),
        }
        self.ensure_bridge_started().await?;

        let (conn, scripting) = self.scripting_proxy().await?;

        let already: bool = scripting
            .call("isScriptLoaded", &(LMUX_PLUGIN_NAME,))
            .await
            .map_err(|e| CompositorError::Offline(format!("isScriptLoaded: {e}")))?;
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
        if std::fs::metadata(&self.script_path).is_err() {
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

    fn window_control_capabilities(&self) -> WindowControlCapabilities {
        WindowControlCapabilities {
            list_windows: true,
            attach_window: true,
            set_visible: true,
            raise_window: true,
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

    async fn list_windows(&self) -> Result<Vec<WindowCandidate>, CompositorError> {
        self.ensure_script_loaded().await?;
        self.refresh_inventory().await?;
        let mut windows: Vec<_> = self
            .inventory
            .lock()
            .map_err(|_| CompositorError::Domain("KWin inventory lock poisoned".into()))?
            .values()
            .cloned()
            .collect();
        windows.sort_by(|a, b| {
            a.app_identity
                .as_ref()
                .and_then(app_identity_value)
                .cmp(&b.app_identity.as_ref().and_then(app_identity_value))
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.backend_window_id.cmp(&b.backend_window_id))
        });
        Ok(windows)
    }

    async fn attach_window(
        &self,
        candidate: &WindowCandidate,
    ) -> Result<SatelliteWindowId, CompositorError> {
        if candidate.backend != WindowCandidateBackend::Kwin {
            return Err(CompositorError::Domain(format!(
                "KWin backend cannot attach {:?} windows",
                candidate.backend
            )));
        }
        validate_kwin_backend_window_id(&candidate.backend_window_id)?;
        self.refresh_inventory().await?;
        let current = self
            .inventory
            .lock()
            .map_err(|_| CompositorError::Domain("KWin inventory lock poisoned".into()))?
            .get(&candidate.backend_window_id)
            .cloned()
            .ok_or_else(|| {
                CompositorError::Domain(format!(
                    "KWin window is not attachable or no longer exists: {}",
                    candidate.backend_window_id
                ))
            })?;
        Ok(SatelliteWindowId::for_attached(&current))
    }

    async fn window_preview(
        &self,
        candidate: &WindowCandidate,
        _max_width: u32,
        _max_height: u32,
    ) -> Result<Option<WindowPreview>, CompositorError> {
        if candidate.backend != WindowCandidateBackend::Kwin {
            return Ok(None);
        }
        validate_kwin_backend_window_id(&candidate.backend_window_id)?;
        capture_kwin_window_preview(&candidate.backend_window_id).await
    }

    async fn set_window_visible_by_pid(
        &self,
        pid: u32,
        visible: bool,
    ) -> Result<(), CompositorError> {
        set_window_visibility_by_pid(pid, visible).await
    }

    async fn set_window_visible(
        &self,
        window: &SatelliteWindowId,
        visible: bool,
    ) -> Result<(), CompositorError> {
        validate_kwin_backend_window_id(&window.backend_window_id)?;
        set_window_visibility_by_backend_id(&window.backend_window_id, visible).await
    }

    async fn raise_window(&self, window: &SatelliteWindowId) -> Result<(), CompositorError> {
        validate_kwin_backend_window_id(&window.backend_window_id)?;
        raise_window_by_backend_id(&window.backend_window_id).await
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
var matched = false;
var windows = typeof workspace.windowList === "function"
    ? workspace.windowList()
    : (typeof workspace.clientList === "function" ? workspace.clientList() : []);
for (var i = 0; i < windows.length; i++) {{
    var w = windows[i];
    if (!w) {{ continue; }}
    var wpid = typeof w.pid === "number" ? w.pid : -1;
    if (wpid !== targetPid) {{ continue; }}
    matched = true;
    try {{ w.minimized = wantMin; }} catch (e) {{ print("lmux-vis: set minimized failed: " + e); }}
    print("lmux-vis: pid=" + targetPid + " minimized=" + wantMin);
    break;
}}
if (!matched) {{ throw new Error("lmux-vis: no window matched pid=" + targetPid); }}
"#
    );
    run_oneshot_script(&snippet, &format!("lmux-vis-{pid}")).await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KwinScriptWindow {
    backend_window_id: String,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    resource_class: Option<String>,
    #[serde(default)]
    resource_name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    normal_window: Option<bool>,
    #[serde(default)]
    skip_taskbar: Option<bool>,
    #[serde(default)]
    skip_switcher: Option<bool>,
    #[serde(default)]
    special_window: Option<bool>,
    #[serde(default)]
    desktop_window: Option<bool>,
    #[serde(default)]
    dock: Option<bool>,
}

fn parse_kwin_window_snapshot(json: &str) -> Result<Vec<WindowCandidate>, CompositorError> {
    let windows: Vec<KwinScriptWindow> = serde_json::from_str(json)
        .map_err(|err| CompositorError::Domain(format!("KWin window JSON: {err}")))?;
    let mut out = Vec::with_capacity(windows.len());
    for window in windows {
        if should_include_kwin_window(&window) {
            out.push(kwin_candidate_from_script_window(window)?);
        }
    }
    Ok(out)
}

fn should_include_kwin_window(window: &KwinScriptWindow) -> bool {
    if window.pid == Some(std::process::id()) {
        return false;
    }
    if window.normal_window == Some(false)
        || window.skip_taskbar == Some(true)
        || window.skip_switcher == Some(true)
        || window.special_window == Some(true)
        || window.desktop_window == Some(true)
        || window.dock == Some(true)
    {
        return false;
    }

    let app = window
        .resource_class
        .as_deref()
        .or(window.resource_name.as_deref())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(
        app.as_str(),
        "lmux" | "no.jpro.lmux" | "plasmashell" | "krunner" | "ksmserver"
    ) {
        return false;
    }

    if window.backend_window_id.trim().is_empty() {
        return false;
    }

    window
        .title
        .as_deref()
        .is_some_and(|title| !title.trim().is_empty())
        || !app.is_empty()
        || window.pid.is_some()
}

fn kwin_candidate_from_script_window(
    window: KwinScriptWindow,
) -> Result<WindowCandidate, CompositorError> {
    validate_kwin_backend_window_id(&window.backend_window_id)?;
    Ok(WindowCandidate {
        backend: WindowCandidateBackend::Kwin,
        backend_window_id: window.backend_window_id,
        pid: window.pid,
        app_identity: window
            .resource_class
            .or(window.resource_name)
            .filter(|value| !value.trim().is_empty())
            .map(WindowAppIdentity::WmClass),
        title: window.title.filter(|value| !value.trim().is_empty()),
        workspace: window.workspace.filter(|value| !value.trim().is_empty()),
        output: window.output.filter(|value| !value.trim().is_empty()),
    })
}

fn validate_kwin_backend_window_id(value: &str) -> Result<(), CompositorError> {
    let Some(id) = value.strip_prefix("kwin:") else {
        return Err(CompositorError::Domain(format!(
            "invalid KWin backend window id: {value}"
        )));
    };
    if id.trim().is_empty() || id.contains(char::is_whitespace) {
        return Err(CompositorError::Domain(format!(
            "invalid KWin backend window id: {value}"
        )));
    }
    Ok(())
}

async fn capture_kwin_window_preview(
    backend_window_id: &str,
) -> Result<Option<WindowPreview>, CompositorError> {
    let handle = backend_window_id
        .strip_prefix("kwin:")
        .ok_or_else(|| {
            CompositorError::Domain(format!(
                "invalid KWin backend window id: {backend_window_id}"
            ))
        })?
        .to_string();
    let (read_fd, write_fd) = pipe_pair()?;
    let read_task = tokio::task::spawn_blocking(move || read_fd_to_vec(read_fd));

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(700),
        capture_kwin_window_preview_inner(&handle, &write_fd),
    )
    .await;
    drop(write_fd);

    let response = match result {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            let _ = read_task.await;
            if is_kwin_screenshot_denied(&err) {
                tracing::debug!(error = %err, "kwin preview denied");
                return Ok(None);
            }
            return Err(err);
        }
        Err(_) => {
            let _ = read_task.await;
            tracing::debug!(backend_window_id, "kwin preview timed out");
            return Ok(None);
        }
    };

    let bytes = read_task
        .await
        .map_err(|err| CompositorError::Domain(format!("KWin preview reader: {err}")))?
        .map_err(CompositorError::Io)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if let Some(preview) = preview_from_kwin_response(&response, bytes.clone()) {
        return Ok(Some(preview));
    }
    Ok(Some(WindowPreview {
        data: WindowPreviewData::EncodedImage(bytes),
    }))
}

async fn capture_kwin_window_preview_inner(
    handle: &str,
    write_fd: &OwnedFd,
) -> Result<HashMap<String, OwnedValue>, CompositorError> {
    let conn = Connection::session()
        .await
        .map_err(|e| CompositorError::Offline(format!("session bus: {e}")))?;
    let screenshot = zbus::Proxy::new(
        &conn,
        KWIN_SERVICE,
        KWIN_SCREENSHOT_PATH,
        KWIN_SCREENSHOT_IFACE,
    )
    .await
    .map_err(|e| CompositorError::Offline(format!("screenshot proxy: {e}")))?;
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    screenshot
        .call(
            "CaptureWindow",
            &(handle, options, Fd::from(write_fd.as_fd())),
        )
        .await
        .map_err(|err| CompositorError::Domain(format!("CaptureWindow: {err}")))
}

fn pipe_pair() -> Result<(OwnedFd, OwnedFd), CompositorError> {
    let mut fds = [0; 2];
    // SAFETY: `libc::pipe` initializes both file descriptors on success.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(CompositorError::Io(std::io::Error::last_os_error()));
    }
    // SAFETY: both descriptors are owned by this process after a successful
    // `pipe` call and are converted exactly once.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: see above.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read_fd, write_fd))
}

fn read_fd_to_vec(fd: OwnedFd) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn preview_from_kwin_response(
    response: &HashMap<String, OwnedValue>,
    bytes: Vec<u8>,
) -> Option<WindowPreview> {
    let width = response_u32(response, "width")?;
    let height = response_u32(response, "height")?;
    let bytes_per_row = response_u32(response, "stride")
        .or_else(|| response_u32(response, "bytesPerLine"))
        .or_else(|| response_u32(response, "bytes_per_line"))? as usize;
    if width == 0 || height == 0 || bytes_per_row == 0 {
        return None;
    }
    let min_len = bytes_per_row.checked_mul(height as usize)?;
    if bytes.len() < min_len {
        return None;
    }
    Some(WindowPreview {
        data: WindowPreviewData::Bgra {
            width,
            height,
            bytes_per_row,
            data: bytes,
        },
    })
}

fn response_u32(response: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    response
        .get(key)
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| {
            response
                .get(key)
                .and_then(|value| i32::try_from(value).ok())
                .and_then(|value| u32::try_from(value).ok())
        })
}

fn is_kwin_screenshot_denied(err: &CompositorError) -> bool {
    let message = err.to_string();
    message.contains("org.freedesktop.DBus.Error.AccessDenied")
        || message.contains("not authorized")
        || message.contains("permission")
}

fn app_identity_value(identity: &WindowAppIdentity) -> Option<String> {
    match identity {
        WindowAppIdentity::BundleId(value)
        | WindowAppIdentity::DesktopEntry(value)
        | WindowAppIdentity::WmClass(value)
        | WindowAppIdentity::AppId(value)
        | WindowAppIdentity::Other(value) => Some(value.clone()),
    }
}

async fn set_window_visibility_by_backend_id(
    backend_window_id: &str,
    visible: bool,
) -> Result<(), CompositorError> {
    let snippet = kwin_visibility_script(backend_window_id, visible)?;
    run_oneshot_script(&snippet, &format!("lmux-vis-exact-{}", std::process::id())).await
}

fn kwin_visibility_script(
    backend_window_id: &str,
    visible: bool,
) -> Result<String, CompositorError> {
    let target = serde_json::to_string(backend_window_id)
        .map_err(|err| CompositorError::Domain(format!("KWin target encode: {err}")))?;
    let want_minimized = if visible { "false" } else { "true" };
    let maybe_raise = if visible {
        "try { workspace.activeWindow = w; } catch (e) { try { workspace.activeClient = w; } catch (_) {} }"
    } else {
        ""
    };
    Ok(format!(
        r#""use strict";
{identity_js}
var targetId = {target};
var wantMin = {want_minimized};
var matched = false;
var windows = lmuxWindowList();
for (var i = 0; i < windows.length; i++) {{
    var w = windows[i];
    if (!w || lmuxBackendWindowId(w) !== targetId) {{ continue; }}
    matched = true;
    try {{ w.minimized = wantMin; }} catch (e) {{ print("lmux-vis: exact set minimized failed: " + e); }}
    {maybe_raise}
    print("lmux-vis: backendWindowId=" + targetId + " minimized=" + wantMin);
    break;
}}
if (!matched) {{ throw new Error("lmux-vis: no window matched backendWindowId=" + targetId); }}
"#,
        identity_js = kwin_exact_identity_js(),
    ))
}

async fn raise_window_by_backend_id(backend_window_id: &str) -> Result<(), CompositorError> {
    let snippet = kwin_raise_script(backend_window_id)?;
    run_oneshot_script(&snippet, &format!("lmux-raise-{}", std::process::id())).await
}

fn kwin_raise_script(backend_window_id: &str) -> Result<String, CompositorError> {
    let target = serde_json::to_string(backend_window_id)
        .map_err(|err| CompositorError::Domain(format!("KWin target encode: {err}")))?;
    Ok(format!(
        r#""use strict";
{identity_js}
var targetId = {target};
var matched = false;
var windows = lmuxWindowList();
for (var i = 0; i < windows.length; i++) {{
    var w = windows[i];
    if (!w || lmuxBackendWindowId(w) !== targetId) {{ continue; }}
    matched = true;
    try {{ w.minimized = false; }} catch (e) {{ print("lmux-raise: unminimize failed: " + e); }}
    try {{ workspace.activeWindow = w; }} catch (e1) {{
        try {{ workspace.activeClient = w; }} catch (e2) {{ print("lmux-raise: activate failed: " + e2); }}
    }}
    print("lmux-raise: backendWindowId=" + targetId);
    break;
}}
if (!matched) {{ throw new Error("lmux-raise: no window matched backendWindowId=" + targetId); }}
"#,
        identity_js = kwin_exact_identity_js(),
    ))
}

fn kwin_snapshot_script(token: &str) -> Result<String, CompositorError> {
    let token = serde_json::to_string(token)
        .map_err(|err| CompositorError::Domain(format!("KWin bridge token encode: {err}")))?;
    Ok(format!(
        r#""use strict";
{identity_js}
{bridge_js}
lmuxBridgeReplace(lmuxCollectWindowsJson());
"#,
        identity_js = kwin_identity_js(),
        bridge_js = kwin_bridge_js(&token),
    ))
}

fn kwin_identity_js() -> &'static str {
    r#"function lmuxString(value) {
    if (value === undefined || value === null) { return ""; }
    return value.toString();
}
function lmuxNumber(value) {
    return typeof value === "number" && isFinite(value) ? value : null;
}
function lmuxWindowList() {
    return typeof workspace.windowList === "function"
        ? workspace.windowList()
        : (typeof workspace.clientList === "function" ? workspace.clientList() : []);
}
function lmuxBackendWindowId(w) {
    var raw = "";
    if (w && w.internalId !== undefined && w.internalId !== null) {
        raw = lmuxString(w.internalId);
    } else if (w && w.windowId !== undefined && w.windowId !== null) {
        raw = lmuxString(w.windowId);
    } else if (w && w.uuid !== undefined && w.uuid !== null) {
        raw = lmuxString(w.uuid);
    }
    return raw.length > 0 ? "kwin:" + raw : "";
}
function lmuxWindowRecord(w) {
    var id = lmuxBackendWindowId(w);
    if (id.length === 0) { return null; }
    var pid = lmuxNumber(w.pid);
    var desktop = w.desktop !== undefined ? lmuxString(w.desktop) : "";
    if (w.desktops !== undefined && w.desktops !== null && w.desktops.length > 0) {
        desktop = lmuxString(w.desktops[0]);
    }
    var output = "";
    if (w.output !== undefined && w.output !== null) {
        output = lmuxString(w.output.name !== undefined ? w.output.name : w.output);
    } else if (w.screen !== undefined && w.screen !== null) {
        output = lmuxString(w.screen);
    }
    return {
        backendWindowId: id,
        pid: pid,
        resourceClass: lmuxString(w.resourceClass),
        resourceName: lmuxString(w.resourceName),
        title: lmuxString(w.caption),
        workspace: desktop,
        output: output,
        normalWindow: typeof w.normalWindow === "boolean" ? w.normalWindow : null,
        skipTaskbar: typeof w.skipTaskbar === "boolean" ? w.skipTaskbar : null,
        skipSwitcher: typeof w.skipSwitcher === "boolean" ? w.skipSwitcher : null,
        specialWindow: typeof w.specialWindow === "boolean" ? w.specialWindow : null,
        desktopWindow: typeof w.desktopWindow === "boolean" ? w.desktopWindow : null,
        dock: typeof w.dock === "boolean" ? w.dock : null
    };
}
function lmuxCollectWindows() {
    var result = [];
    var windows = lmuxWindowList();
    for (var i = 0; i < windows.length; i++) {
        var record = lmuxWindowRecord(windows[i]);
        if (record !== null) { result.push(record); }
    }
    return result;
}
function lmuxCollectWindowsJson() {
    return JSON.stringify(lmuxCollectWindows());
}"#
}

fn kwin_exact_identity_js() -> &'static str {
    r#"function lmuxString(value) {
    if (value === undefined || value === null) { return ""; }
    return value.toString();
}
function lmuxWindowList() {
    return typeof workspace.windowList === "function"
        ? workspace.windowList()
        : (typeof workspace.clientList === "function" ? workspace.clientList() : []);
}
function lmuxBackendWindowId(w) {
    var raw = "";
    if (w && w.internalId !== undefined && w.internalId !== null) {
        raw = lmuxString(w.internalId);
    } else if (w && w.windowId !== undefined && w.windowId !== null) {
        raw = lmuxString(w.windowId);
    } else if (w && w.uuid !== undefined && w.uuid !== null) {
        raw = lmuxString(w.uuid);
    }
    return raw.length > 0 ? "kwin:" + raw : "";
}"#
}

fn kwin_bridge_js(token_json: &str) -> String {
    format!(
        r#"var lmuxBridgeToken = {token_json};
function lmuxBridgeCall(method, payload) {{
    try {{
        callDBus("no.jpro.lmux.KWinBridge",
                 "/no/jpro/lmux/KWinBridge",
                 "no.jpro.lmux.KWinBridge",
                 method,
                 lmuxBridgeToken,
                 payload);
    }} catch (e) {{
        print("lmux-dock: bridge call " + method + " failed: " + e);
    }}
}}
function lmuxBridgeReplace(json) {{ lmuxBridgeCall("ReplaceWindows", json); }}
function lmuxBridgeUpsert(json) {{ lmuxBridgeCall("UpsertWindow", json); }}
function lmuxBridgeRemove(id) {{ lmuxBridgeCall("RemoveWindow", id); }}"#,
    )
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

    let run_result = script_proxy.call::<_, _, ()>("run", &()).await;
    let _ = scripting
        .call::<_, _, ()>("unloadScript", &(plugin_name,))
        .await;
    run_result.map_err(|e| CompositorError::Domain(format!("run: {e}")))?;
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

    #[test]
    fn parses_kwin_snapshot_into_exact_candidates() {
        let json = r#"[{
            "backendWindowId": "kwin:{6d5c}",
            "pid": 4242,
            "resourceClass": "firefox",
            "resourceName": "Navigator",
            "title": "Docs",
            "workspace": "2",
            "output": "eDP-1",
            "normalWindow": true
        }]"#;

        let windows = parse_kwin_window_snapshot(json).unwrap();

        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(window.backend, WindowCandidateBackend::Kwin);
        assert_eq!(window.backend_window_id, "kwin:{6d5c}");
        assert_eq!(window.pid, Some(4242));
        assert_eq!(
            window.app_identity,
            Some(WindowAppIdentity::WmClass("firefox".into()))
        );
        assert_eq!(window.title.as_deref(), Some("Docs"));
        assert_eq!(window.workspace.as_deref(), Some("2"));
        assert_eq!(window.output.as_deref(), Some("eDP-1"));
    }

    #[test]
    fn rejects_kwin_snapshot_without_exact_identity() {
        let json = r#"[{"backendWindowId":"pid:4242","pid":4242}]"#;

        assert!(parse_kwin_window_snapshot(json).is_err());
    }

    #[test]
    fn filters_kwin_snapshot_to_user_windows() {
        let json = format!(
            r#"[{{
                "backendWindowId": "kwin:{{slack}}",
                "pid": 4242,
                "resourceClass": "Slack",
                "title": "Slack",
                "normalWindow": true
            }}, {{
                "backendWindowId": "kwin:{{panel}}",
                "pid": 12,
                "resourceClass": "plasmashell",
                "title": "",
                "dock": true,
                "normalWindow": false
            }}, {{
                "backendWindowId": "kwin:{{own}}",
                "pid": {},
                "resourceClass": "lmux",
                "title": "Attach window",
                "normalWindow": true
            }}, {{
                "backendWindowId": "kwin:{{utility}}",
                "pid": 43,
                "resourceClass": "tool",
                "title": "Utility",
                "skipTaskbar": true,
                "normalWindow": true
            }}]"#,
            std::process::id()
        );

        let windows = parse_kwin_window_snapshot(&json).unwrap();

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].backend_window_id, "kwin:{slack}");
    }

    #[test]
    fn exact_visibility_script_uses_backend_window_id_not_pid_or_class() {
        let snippet = kwin_visibility_script("kwin:{6d5c}", false).unwrap();

        assert!(snippet.contains("lmuxBackendWindowId"));
        assert!(snippet.contains(r#"var targetId = "kwin:{6d5c}";"#));
        assert!(snippet.contains("lmuxBackendWindowId(w) !== targetId"));
        assert!(snippet.contains("var wantMin = true;"));
        assert!(snippet.contains("throw new Error"));
        assert!(!snippet.contains("targetPid"));
        assert!(!snippet.contains("resourceClass"));
        assert!(!snippet.contains("resourceName"));
    }

    #[test]
    fn snapshot_script_sends_bridge_token_with_inventory() {
        let snippet = kwin_snapshot_script("secret-token").unwrap();

        assert!(snippet.contains(r#"var lmuxBridgeToken = "secret-token";"#));
        assert!(snippet.contains("lmuxBridgeReplace(lmuxCollectWindowsJson());"));
        assert!(snippet.contains("method,"));
        assert!(snippet.contains("lmuxBridgeToken,"));
        assert!(snippet.contains("payload);"));
    }

    #[test]
    fn raise_script_fails_when_backend_window_id_does_not_match() {
        let snippet = kwin_raise_script("kwin:{6d5c}").unwrap();

        assert!(snippet.contains(r#"var targetId = "kwin:{6d5c}";"#));
        assert!(snippet.contains("var matched = false;"));
        assert!(snippet.contains("throw new Error"));
    }
}
