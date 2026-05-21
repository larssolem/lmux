//! `CompositorControl` trait + `KwinCompositor` + `NoopCompositor` impls,
//! health probe, re-inject. See ADR-0017 and epics.md Epic 8.
//!
//! The trait is intentionally minimal and async: v0.2 implementations talk
//! to KWin over D-Bus (see [`kwin`]), but tests and non-KDE fallbacks use
//! [`NoopCompositor`] which performs no real work but reports plausible
//! state so the rest of the stack keeps running (NFR14: "no compositor →
//! satellites open as free-floating windows").

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod kwin;
pub mod macos;
pub mod noop;
pub mod spawn;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

pub use kwin::KwinCompositor;
pub use macos::MacWindowCompositor;
pub use noop::NoopCompositor;

/// Rect in compositor screen coordinates. Mirrors [`lmux_bus::kinds::Rect`]
/// intentionally — the trait level does not depend on the bus types so that
/// a compositor impl can be unit-tested without the bus compiled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Opaque compositor-specific window id (KWin window uuid on KDE).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowId(pub String);

/// Backend namespace for a managed satellite window identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowBackend {
    Kwin,
    Hyprland,
    Macos,
    Noop,
    Unknown(String),
}

/// Stable cross-backend identity for one GUI satellite window.
///
/// PID-only matching is enough for the current KWin visibility path, but
/// macOS needs per-window identity because single-instance apps can own
/// windows for multiple anchors in the same process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SatelliteWindowId {
    pub backend: WindowBackend,
    pub request_id: Option<Uuid>,
    pub pid: Option<u32>,
    pub backend_window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl SatelliteWindowId {
    pub fn for_pid(backend: WindowBackend, pid: u32) -> Self {
        Self {
            backend,
            request_id: None,
            pid: Some(pid),
            backend_window_id: format!("pid:{pid}"),
            bundle_id: None,
            title: None,
        }
    }

    pub fn for_spawn(backend: WindowBackend, request_id: Uuid, pid: u32) -> Self {
        Self {
            backend,
            request_id: Some(request_id),
            pid: Some(pid),
            backend_window_id: format!("pid:{pid}"),
            bundle_id: None,
            title: None,
        }
    }

    pub fn for_spawn_with_bundle(
        backend: WindowBackend,
        request_id: Uuid,
        pid: u32,
        bundle_id: Option<String>,
    ) -> Self {
        Self {
            backend,
            request_id: Some(request_id),
            pid: Some(pid),
            backend_window_id: format!("pid:{pid}"),
            bundle_id,
            title: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusPolicy {
    Terminal,
    LastSatellite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGroupSwitch {
    pub hide: Vec<SatelliteWindowId>,
    pub show: Vec<SatelliteWindowId>,
    pub focus_policy: FocusPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowOpResult {
    pub window: SatelliteWindowId,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Health status reported by [`CompositorControl::health`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Script loaded, D-Bus reachable, rules accepting geometry.
    Online,
    /// D-Bus reachable but the lmux-dock KWin script is missing or unloaded.
    ScriptMissing,
    /// KWin / compositor D-Bus endpoint is unreachable.
    Offline { reason: String },
}

/// Error surface for compositor operations. Callers map `ScriptMissing`
/// onto a user-visible "re-inject" toast (FR14).
#[derive(Debug, Error)]
pub enum CompositorError {
    #[error("compositor script not loaded")]
    ScriptMissing,
    #[error("compositor offline: {0}")]
    Offline(String),
    #[error("compositor domain error: {0}")]
    Domain(String),
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
}

/// Cockpit-facing trait for controlling the window manager's satellite
/// docking. Every method is async and `&self` so the cockpit can stash
/// a single impl behind `Arc<dyn CompositorControl>` and share it across
/// workers.
#[async_trait]
pub trait CompositorControl: Send + Sync {
    /// Ensure the lmux-dock compositor script is loaded and addressable.
    /// Idempotent: repeated calls should be cheap and not reload the script.
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError>;

    /// Probe compositor health. MUST NOT have side effects.
    async fn health(&self) -> Health;

    /// Spawn `argv` with an lmux tag, and return the request id the
    /// compositor will echo back via `satellite.map` when the new window
    /// appears. Returning `Ok` does NOT mean the window is visible yet —
    /// the cockpit correlates via `request_id`.
    async fn spawn_satellite(
        &self,
        argv: &[String],
        cwd: Option<&str>,
    ) -> Result<Uuid, CompositorError>;

    /// Set geometry for a tagged window.
    async fn set_geometry(&self, window: &WindowId, rect: Rect) -> Result<(), CompositorError>;

    /// Detach a tagged satellite — the compositor stops treating it as
    /// part of the cockpit layout.
    async fn detach(&self, window: &WindowId) -> Result<(), CompositorError>;

    /// Re-attach a previously detached satellite.
    async fn attach(&self, window: &WindowId) -> Result<(), CompositorError>;

    /// Show or hide the compositor window whose PID matches `pid`. Used by
    /// the cockpit to tie satellite lifetimes to the active anchor: on
    /// anchor switch, windows owned by the incoming anchor are shown and
    /// windows owned by every other anchor are hidden. A no-op for
    /// backends that can't address windows (the trait supplies a default
    /// `Ok(())`).
    async fn set_window_visible_by_pid(
        &self,
        _pid: u32,
        _visible: bool,
    ) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Show or hide a specific managed satellite window. New backends should
    /// implement this identity-based method; the default preserves the
    /// existing PID path for KWin and Noop.
    async fn set_window_visible(
        &self,
        window: &SatelliteWindowId,
        visible: bool,
    ) -> Result<(), CompositorError> {
        if let Some(pid) = window.pid {
            self.set_window_visible_by_pid(pid, visible).await
        } else {
            Ok(())
        }
    }

    /// Apply an anchor switch as one group operation. Backends with native
    /// batching (macOS helper) can override; the default runs each window
    /// operation independently and returns per-window results.
    async fn apply_window_group_switch(
        &self,
        switch: WindowGroupSwitch,
    ) -> Result<Vec<WindowOpResult>, CompositorError> {
        let mut out = Vec::with_capacity(switch.hide.len() + switch.show.len());
        for window in switch.hide {
            let result = self.set_window_visible(&window, false).await;
            out.push(WindowOpResult {
                window,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
            });
        }
        for window in switch.show {
            let result = self.set_window_visible(&window, true).await;
            out.push(WindowOpResult {
                window,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
            });
        }
        Ok(out)
    }

    /// Apply a grouped switch with latest-wins cancellation context. The
    /// default implementation checks the shared sequence before each window
    /// operation so stale work stops as soon as possible.
    async fn apply_window_group_switch_latest(
        &self,
        switch: WindowGroupSwitch,
        sequence: u64,
        latest_sequence: Arc<AtomicU64>,
    ) -> Result<Vec<WindowOpResult>, CompositorError> {
        let mut out = Vec::with_capacity(switch.hide.len() + switch.show.len());
        for window in switch.hide {
            if latest_sequence.load(Ordering::Relaxed) > sequence {
                break;
            }
            let result = self.set_window_visible(&window, false).await;
            out.push(WindowOpResult {
                window,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
            });
        }
        for window in switch.show {
            if latest_sequence.load(Ordering::Relaxed) > sequence {
                break;
            }
            let result = self.set_window_visible(&window, true).await;
            out.push(WindowOpResult {
                window,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::sync::atomic::AtomicU64;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct RecordingCompositor {
        visible: Mutex<Vec<(String, bool)>>,
    }

    #[async_trait]
    impl CompositorControl for RecordingCompositor {
        async fn ensure_script_loaded(&self) -> Result<(), CompositorError> {
            Ok(())
        }

        async fn health(&self) -> Health {
            Health::Online
        }

        async fn spawn_satellite(
            &self,
            _argv: &[String],
            _cwd: Option<&str>,
        ) -> Result<Uuid, CompositorError> {
            Ok(Uuid::from_u128(1))
        }

        async fn set_geometry(
            &self,
            _window: &WindowId,
            _rect: Rect,
        ) -> Result<(), CompositorError> {
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
            self.visible
                .lock()
                .await
                .push((window.backend_window_id.clone(), visible));
            Ok(())
        }
    }

    #[tokio::test]
    async fn noop_compositor_health_reports_online() {
        let c = NoopCompositor::default();
        assert_eq!(c.health().await, Health::Online);
    }

    #[tokio::test]
    async fn noop_compositor_spawn_returns_request_id() {
        let c = NoopCompositor::default();
        // `true` is part of coreutils and exits immediately — avoids
        // depending on a GUI app being installed in CI.
        let id = c
            .spawn_satellite(&["true".into()], Some("/tmp"))
            .await
            .unwrap();
        // Non-nil: trait guarantees callers can correlate.
        assert_ne!(id, Uuid::nil());
    }

    #[test]
    fn satellite_window_id_for_spawn_keeps_request_id_and_pid() {
        let request_id = Uuid::from_u128(42);
        let window = SatelliteWindowId::for_spawn(WindowBackend::Macos, request_id, 123);

        assert_eq!(window.backend, WindowBackend::Macos);
        assert_eq!(window.request_id, Some(request_id));
        assert_eq!(window.pid, Some(123));
        assert_eq!(window.backend_window_id, "pid:123");
    }

    #[test]
    fn satellite_window_id_for_spawn_can_keep_bundle_id() {
        let request_id = Uuid::from_u128(43);
        let window = SatelliteWindowId::for_spawn_with_bundle(
            WindowBackend::Macos,
            request_id,
            123,
            Some("com.microsoft.VSCode".into()),
        );

        assert_eq!(window.request_id, Some(request_id));
        assert_eq!(window.bundle_id.as_deref(), Some("com.microsoft.VSCode"));
    }

    #[tokio::test]
    async fn noop_compositor_geometry_is_idempotent() {
        let c = NoopCompositor::default();
        let w = WindowId("fake".into());
        for _ in 0..3 {
            c.set_geometry(
                &w,
                Rect {
                    x: 0,
                    y: 0,
                    w: 800,
                    h: 600,
                },
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn latest_group_switch_skips_stale_sequence() {
        let c = RecordingCompositor::default();
        let latest = Arc::new(AtomicU64::new(2));
        let result = c
            .apply_window_group_switch_latest(
                WindowGroupSwitch {
                    hide: vec![SatelliteWindowId::for_pid(WindowBackend::Macos, 1)],
                    show: vec![SatelliteWindowId::for_pid(WindowBackend::Macos, 2)],
                    focus_policy: FocusPolicy::Terminal,
                },
                1,
                latest,
            )
            .await
            .unwrap();

        assert!(result.is_empty());
        assert!(c.visible.lock().await.is_empty());
    }
}
