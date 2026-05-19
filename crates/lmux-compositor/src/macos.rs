//! macOS window-control backend scaffold.
//!
//! The real macOS implementation will delegate to a small native helper
//! using AppKit/Accessibility. This Rust-side backend is intentionally
//! protocol-shaped and Linux-testable: it records the same grouped window
//! operations the helper must receive, while still using the shared
//! satellite spawner so macOS users can dogfood floating app launches early.

use async_trait::async_trait;
use tokio::sync::Mutex;
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

    pub async fn recorded_commands(&self) -> Vec<MacHelperCommand> {
        self.recorded.lock().await.clone()
    }
}

impl Default for MacWindowCompositor {
    fn default() -> Self {
        Self::degraded()
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
        Ok(())
    }

    async fn apply_window_group_switch(
        &self,
        switch: WindowGroupSwitch,
    ) -> Result<Vec<WindowOpResult>, CompositorError> {
        self.recorded
            .lock()
            .await
            .push(MacHelperCommand::ApplyGroup {
                hide: switch.hide.clone(),
                show: switch.show.clone(),
                focus_policy: switch.focus_policy,
            });
        let mut out = Vec::with_capacity(switch.hide.len() + switch.show.len());
        for window in switch.hide.into_iter().chain(switch.show.into_iter()) {
            out.push(WindowOpResult {
                window,
                ok: true,
                error: None,
            });
        }
        Ok(out)
    }
}

pub fn macos_window_for_pid(pid: u32) -> SatelliteWindowId {
    SatelliteWindowId::for_pid(WindowBackend::Macos, pid)
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
}
