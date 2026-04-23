//! A compositor impl that does nothing but return plausible answers.
//!
//! Used:
//! * In tests for anything that needs a `CompositorControl` without a
//!   real KWin.
//! * At runtime when the user is not on KDE (NFR14) — satellites fall back
//!   to free-floating windows and the cockpit records "no compositor
//!   integration" rather than erroring out.

use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{CompositorControl, CompositorError, Health, Rect, WindowId};

#[derive(Debug, Default)]
pub struct NoopCompositor {
    /// Records the latest geometry for each window id, for test
    /// introspection. Callers can snapshot via [`NoopCompositor::last_rect`].
    last_geometry: Mutex<std::collections::HashMap<String, Rect>>,
}

impl NoopCompositor {
    pub async fn last_rect(&self, window: &WindowId) -> Option<Rect> {
        self.last_geometry.lock().await.get(&window.0).copied()
    }
}

#[async_trait]
impl CompositorControl for NoopCompositor {
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn health(&self) -> Health {
        Health::Online
    }

    async fn spawn_satellite(
        &self,
        argv: &[String],
        cwd: Option<&str>,
    ) -> Result<Uuid, CompositorError> {
        // NoopCompositor still spawns the process — the "noop" half is only
        // the docking; NFR14 says "no compositor → satellites open as free-
        // floating windows", not "no compositor → satellites don't open".
        crate::spawn::spawn_tagged(argv, cwd)
    }

    async fn set_geometry(&self, window: &WindowId, rect: Rect) -> Result<(), CompositorError> {
        self.last_geometry
            .lock()
            .await
            .insert(window.0.clone(), rect);
        Ok(())
    }

    async fn detach(&self, _window: &WindowId) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn attach(&self, _window: &WindowId) -> Result<(), CompositorError> {
        Ok(())
    }
}
