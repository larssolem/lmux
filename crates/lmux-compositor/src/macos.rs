//! macOS window-control backend scaffold.
//!
//! The real macOS implementation will delegate to a small native helper
//! using AppKit/Accessibility. This Rust-side backend is intentionally
//! protocol-shaped and Linux-testable: it records the same grouped window
//! operations the helper must receive, while still using the shared
//! satellite spawner so macOS users can dogfood floating app launches early.

use async_trait::async_trait;
#[cfg(not(test))]
use lmux_macos_helper::{HelperRequest, HelperResponse};
use lmux_macos_helper::{OperationResult, WindowRef};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
#[cfg(not(test))]
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::{
    CompositorControl, CompositorError, FocusPolicy, Health, Rect, SatelliteWindowId,
    WindowBackend, WindowGroupSwitch, WindowId, WindowOpResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacHelperCommand {
    SetVisible {
        window: SatelliteWindowId,
        visible: bool,
    },
    ApplyGroup {
        hide: Vec<SatelliteWindowId>,
        show: Vec<SatelliteWindowId>,
        focus_policy: FocusPolicy,
    },
}

#[derive(Debug)]
pub struct MacWindowCompositor {
    accessibility: PermissionState,
    recorded: Mutex<Vec<MacHelperCommand>>,
}

impl MacWindowCompositor {
    pub fn new(accessibility: PermissionState) -> Self {
        Self {
            accessibility,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub fn degraded() -> Self {
        Self::new(PermissionState::NotDetermined)
    }

    pub fn detect_or_prompt() -> Self {
        Self::new(accessibility_permission_state(true))
    }

    pub async fn recorded_commands(&self) -> Vec<MacHelperCommand> {
        self.recorded.lock().await.clone()
    }
}

impl Default for MacWindowCompositor {
    fn default() -> Self {
        Self::detect_or_prompt()
    }
}

#[async_trait]
impl CompositorControl for MacWindowCompositor {
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn health(&self) -> Health {
        match self.accessibility {
            PermissionState::Granted => Health::Online,
            PermissionState::Denied => Health::Offline {
                reason: "accessibility-permission-denied".into(),
            },
            PermissionState::NotDetermined => Health::Offline {
                reason: "accessibility-permission-not-determined".into(),
            },
        }
    }

    async fn spawn_satellite(
        &self,
        argv: &[String],
        cwd: Option<&str>,
    ) -> Result<Uuid, CompositorError> {
        crate::spawn::spawn_tagged(argv, cwd)
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

    async fn set_window_visible(
        &self,
        window: &SatelliteWindowId,
        visible: bool,
    ) -> Result<(), CompositorError> {
        self.recorded
            .lock()
            .await
            .push(MacHelperCommand::SetVisible {
                window: window.clone(),
                visible,
            });
        set_app_visible(window, visible).await
    }

    async fn apply_window_group_switch(
        &self,
        switch: WindowGroupSwitch,
    ) -> Result<Vec<WindowOpResult>, CompositorError> {
        self.apply_window_group_switch_inner(switch, None).await
    }

    async fn apply_window_group_switch_latest(
        &self,
        switch: WindowGroupSwitch,
        sequence: u64,
        latest_sequence: Arc<AtomicU64>,
    ) -> Result<Vec<WindowOpResult>, CompositorError> {
        self.apply_window_group_switch_inner(switch, Some((sequence, latest_sequence)))
            .await
    }
}

impl MacWindowCompositor {
    async fn apply_window_group_switch_inner(
        &self,
        switch: WindowGroupSwitch,
        latest: Option<(u64, Arc<AtomicU64>)>,
    ) -> Result<Vec<WindowOpResult>, CompositorError> {
        let started = Instant::now();
        if is_stale(&latest) {
            tracing::debug!(
                operation = "macos.window_group_switch",
                duration_ms = elapsed_ms(started),
                windows = switch.show.len(),
                stale = true,
                "macOS window group switch skipped before helper call"
            );
            return Ok(Vec::new());
        }
        self.recorded
            .lock()
            .await
            .push(MacHelperCommand::ApplyGroup {
                hide: switch.hide.clone(),
                show: switch.show.clone(),
                focus_policy: switch.focus_policy,
            });

        // Only lmux-owned windows reach this point. Hiding the inactive
        // anchor's tracked windows is required to prevent single-instance
        // apps such as IntelliJ from appearing to move between anchors.
        let hide = switch.hide;
        let show = switch.show;
        match apply_app_visibility_group(&hide, &show).await {
            Ok(results) if results.iter().all(|result| result.ok) => {
                tracing::debug!(
                    operation = "macos.window_group_switch",
                    duration_ms = elapsed_ms(started),
                    windows = results.len(),
                    failures = 0usize,
                    "macOS window group switch applied"
                );
                return Ok(results);
            }
            Ok(results) => {
                let failures = results.iter().filter(|result| !result.ok).count();
                for result in results.iter().filter(|result| !result.ok) {
                    tracing::warn!(
                        window = ?result.window,
                        error = ?result.error,
                        "macOS helper group window operation failed"
                    );
                }
                tracing::warn!(
                    operation = "macos.window_group_switch",
                    duration_ms = elapsed_ms(started),
                    windows = results.len(),
                    failures,
                    "macOS window group switch partially failed"
                );
                return Ok(results);
            }
            Err(err) => {
                tracing::warn!(error = %err, "macOS helper group visibility failed");
                return Err(err);
            }
        }
    }
}

fn is_stale(latest: &Option<(u64, Arc<AtomicU64>)>) -> bool {
    latest
        .as_ref()
        .is_some_and(|(sequence, latest)| latest.load(Ordering::Relaxed) > *sequence)
}

async fn set_app_visible(window: &SatelliteWindowId, visible: bool) -> Result<(), CompositorError> {
    #[cfg(test)]
    {
        let _ = (window, visible);
        return Ok(());
    }
    #[cfg(not(test))]
    {
        match run_helper_request_timed(HelperRequest::SetVisible {
            window: helper_window_ref(window),
            visible,
        })
        .await
        {
            Ok(HelperResponse::Ok) => Ok(()),
            Ok(HelperResponse::Error { message }) => Err(CompositorError::Domain(format!(
                "macOS helper visibility failed: {message}"
            ))),
            Ok(response) => Err(CompositorError::Domain(format!(
                "macOS helper returned unexpected visibility response: {response:?}"
            ))),
            Err(err) => Err(err),
        }
    }
}

async fn apply_app_visibility_group(
    hide: &[SatelliteWindowId],
    show: &[SatelliteWindowId],
) -> Result<Vec<WindowOpResult>, CompositorError> {
    #[cfg(test)]
    {
        let mut out = Vec::with_capacity(hide.len() + show.len());
        out.extend(hide.iter().cloned().map(|window| WindowOpResult {
            window,
            ok: true,
            error: None,
        }));
        out.extend(show.iter().cloned().map(|window| WindowOpResult {
            window,
            ok: true,
            error: None,
        }));
        return Ok(out);
    }
    #[cfg(not(test))]
    {
        let request = HelperRequest::ApplyGroup {
            hide: hide.iter().map(helper_window_ref).collect(),
            show: show.iter().map(helper_window_ref).collect(),
        };
        match run_helper_request_timed(request).await? {
            HelperResponse::Applied { results } => {
                let mut out = Vec::with_capacity(results.len());
                for result in results {
                    out.push(window_op_result(result, hide, show));
                }
                Ok(out)
            }
            HelperResponse::Error { message } => Err(CompositorError::Domain(format!(
                "macOS helper group failed: {message}"
            ))),
            response => Err(CompositorError::Domain(format!(
                "macOS helper returned unexpected group response: {response:?}"
            ))),
        }
    }
}

fn helper_window_ref(window: &SatelliteWindowId) -> WindowRef {
    WindowRef {
        pid: window.pid,
        bundle_id: window.bundle_id.clone(),
        window_index: macos_window_index(&window.backend_window_id),
        window_id: macos_window_id(&window.backend_window_id),
        title: window.title.clone(),
    }
}

fn macos_window_index(backend_window_id: &str) -> Option<u32> {
    if let Some(value) = backend_window_id.strip_prefix("macos-window-index:") {
        return value.parse().ok();
    }
    if let Some(value) = backend_window_id.strip_prefix("macos-window-pid:") {
        return value
            .split_once(":index:")
            .and_then(|(_, index)| index.parse().ok());
    }
    backend_window_id
        .strip_prefix("macos-window-id:")
        .and_then(|value| value.split_once(":index:"))
        .and_then(|(_, index)| index.parse().ok())
}

fn macos_window_id(backend_window_id: &str) -> Option<i64> {
    backend_window_id
        .strip_prefix("macos-window-id:")
        .map(|value| {
            value
                .split_once(":index:")
                .map_or(value, |(window_id, _)| window_id)
        })
        .and_then(|value| value.parse().ok())
}

fn macos_backend_window_id(window: &WindowRef) -> String {
    match (window.window_id, window.window_index) {
        (Some(window_id), _) => format!("macos-window-id:{window_id}"),
        (None, Some(window_index)) => format!("macos-window-index:{window_index}"),
        (None, None) => "macos-window:unknown".into(),
    }
}

fn window_op_result(
    helper_result: OperationResult,
    hide: &[SatelliteWindowId],
    show: &[SatelliteWindowId],
) -> WindowOpResult {
    let window = hide
        .iter()
        .chain(show)
        .find(|candidate| {
            candidate.pid == helper_result.window.pid
                && candidate.bundle_id == helper_result.window.bundle_id
                && macos_window_index(&candidate.backend_window_id)
                    == helper_result.window.window_index
                && macos_window_id(&candidate.backend_window_id) == helper_result.window.window_id
        })
        .cloned()
        .unwrap_or_else(|| SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: None,
            pid: helper_result.window.pid,
            backend_window_id: macos_backend_window_id(&helper_result.window),
            bundle_id: helper_result.window.bundle_id.clone(),
            title: helper_result.window.title.clone(),
        });
    WindowOpResult {
        window,
        ok: helper_result.ok,
        error: helper_result.error,
    }
}

#[cfg(not(test))]
async fn run_helper_request_timed(
    request: HelperRequest,
) -> Result<HelperResponse, CompositorError> {
    let task = tokio::task::spawn_blocking(move || lmux_macos_helper::handle_request(request));
    match timeout(Duration::from_millis(1200), task).await {
        Ok(Err(err)) => Err(CompositorError::Domain(format!(
            "lmux-macos-helper task failed: {err}"
        ))),
        Ok(Ok(result)) => Ok(result),
        Err(_) => Err(CompositorError::Domain(
            "lmux-macos-helper timed out".into(),
        )),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

pub fn macos_window_for_pid(pid: u32) -> SatelliteWindowId {
    SatelliteWindowId::for_pid(WindowBackend::Macos, pid)
}

#[cfg(target_os = "macos")]
type Boolean = u8;
#[cfg(target_os = "macos")]
type CFIndex = libc::c_long;
#[cfg(target_os = "macos")]
type CFAllocatorRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFDictionaryRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFTypeRef = *const libc::c_void;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const libc::c_void,
        values: *const *const libc::c_void,
        num_values: CFIndex,
        key_callbacks: *const libc::c_void,
        value_callbacks: *const libc::c_void,
    ) -> CFDictionaryRef;
    fn CFRelease(cf: CFTypeRef);
}

#[cfg(target_os = "macos")]
fn accessibility_permission_state(prompt: bool) -> PermissionState {
    let trusted = unsafe {
        if prompt {
            let keys = [kAXTrustedCheckOptionPrompt as *const libc::c_void];
            let values = [kCFBooleanTrue as *const libc::c_void];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                std::ptr::null(),
                std::ptr::null(),
            );
            if options.is_null() {
                false
            } else {
                let trusted = AXIsProcessTrustedWithOptions(options) != 0;
                CFRelease(options as CFTypeRef);
                trusted
            }
        } else {
            AXIsProcessTrustedWithOptions(std::ptr::null()) != 0
        }
    };
    if trusted {
        PermissionState::Granted
    } else if prompt {
        PermissionState::NotDetermined
    } else {
        PermissionState::Denied
    }
}

#[cfg(not(target_os = "macos"))]
fn accessibility_permission_state(_prompt: bool) -> PermissionState {
    PermissionState::NotDetermined
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn missing_accessibility_reports_degraded_health() {
        let c = MacWindowCompositor::degraded();
        assert_eq!(
            c.health().await,
            Health::Offline {
                reason: "accessibility-permission-not-determined".into()
            }
        );
    }

    #[tokio::test]
    async fn group_switch_records_single_helper_command() {
        let c = MacWindowCompositor::new(PermissionState::Granted);
        let hide = macos_window_for_pid(10);
        let show = macos_window_for_pid(20);
        let result = c
            .apply_window_group_switch(WindowGroupSwitch {
                hide: vec![hide.clone()],
                show: vec![show.clone()],
                focus_policy: FocusPolicy::Terminal,
            })
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        let commands = c.recorded_commands().await;
        assert_eq!(
            commands,
            vec![MacHelperCommand::ApplyGroup {
                hide: vec![hide],
                show: vec![show],
                focus_policy: FocusPolicy::Terminal,
            }]
        );
    }

    #[tokio::test]
    async fn stale_group_switch_skips_helper_command() {
        let c = MacWindowCompositor::new(PermissionState::Granted);
        let latest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2));
        let result = c
            .apply_window_group_switch_latest(
                WindowGroupSwitch {
                    hide: Vec::new(),
                    show: vec![macos_window_for_pid(20)],
                    focus_policy: FocusPolicy::Terminal,
                },
                1,
                latest,
            )
            .await
            .unwrap();

        assert!(result.is_empty());
        assert!(c.recorded_commands().await.is_empty());
    }

    #[test]
    fn helper_window_ref_preserves_pid_bundle_and_window_index() {
        let window = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: None,
            pid: Some(42),
            backend_window_id: "macos-window-index:7".into(),
            bundle_id: Some("com.example.App".into()),
            title: Some("Example".into()),
        };

        assert_eq!(
            helper_window_ref(&window),
            WindowRef {
                pid: Some(42),
                bundle_id: Some("com.example.App".into()),
                window_index: Some(7),
                window_id: None,
                title: Some("Example".into()),
            }
        );
    }

    #[test]
    fn helper_window_ref_preserves_stable_window_id() {
        let window = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: None,
            pid: Some(42),
            backend_window_id: "macos-window-id:1001:index:7".into(),
            bundle_id: Some("com.example.App".into()),
            title: Some("Example".into()),
        };

        assert_eq!(
            helper_window_ref(&window),
            WindowRef {
                pid: Some(42),
                bundle_id: Some("com.example.App".into()),
                window_index: Some(7),
                window_id: Some(1001),
                title: Some("Example".into()),
            }
        );
    }

    #[test]
    fn helper_result_prefers_stable_window_id_for_unknown_windows() {
        let result = window_op_result(
            OperationResult {
                window: WindowRef {
                    pid: Some(42),
                    bundle_id: Some("com.example.App".into()),
                    window_index: Some(7),
                    window_id: Some(1001),
                    title: Some("Example".into()),
                },
                ok: true,
                error: None,
            },
            &[],
            &[],
        );

        assert_eq!(result.window.backend_window_id, "macos-window-id:1001");
    }
}
