//! GTK → compositor command bridge.
//!
//! AppState runs on the GTK main thread and produces synchronous commands
//! (e.g. "minimize satellite PID 1234 because its owning anchor was just
//! switched away"). [`CompositorControl`] methods are async and talk to
//! KWin over zbus. To keep GTK non-blocking we route commands through a
//! `async_channel::Sender`, owned by AppState, drained on a dedicated
//! tokio current-thread runtime running on its own OS thread.

use std::sync::Arc;
use std::thread;

use lmux_compositor::CompositorControl;

/// Commands the bridge accepts. Extend with geometry/attach when full
/// docking (v0.3) lands.
#[derive(Debug, Clone)]
pub enum CompositorCommand {
    /// Show or hide the satellite window whose PID equals `pid`. GTK side
    /// fires this on every anchor switch, once per known satellite.
    SetSatelliteVisible { pid: u32, visible: bool },
}

pub type CompositorSender = async_channel::Sender<CompositorCommand>;
pub type CompositorReceiver = async_channel::Receiver<CompositorCommand>;

/// Spawn the bridge thread and return the sender AppState should use.
/// The thread terminates when every sender is dropped (channel close).
pub fn spawn(compositor: Arc<dyn CompositorControl>) -> CompositorSender {
    let (tx, rx) = async_channel::unbounded::<CompositorCommand>();
    match thread::Builder::new()
        .name("lmux-compositor-bridge".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    tracing::warn!(error = %err, "compositor-bridge: runtime failed");
                    return;
                }
            };
            rt.block_on(async move { drain(rx, compositor).await });
        }) {
        Ok(_) => {}
        Err(err) => tracing::warn!(error = %err, "compositor-bridge: thread spawn failed"),
    }
    tx
}

async fn drain(rx: CompositorReceiver, compositor: Arc<dyn CompositorControl>) {
    while let Ok(cmd) = rx.recv().await {
        match cmd {
            CompositorCommand::SetSatelliteVisible { pid, visible } => {
                match compositor.set_window_visible_by_pid(pid, visible).await {
                    Ok(()) => tracing::debug!(pid, visible, "compositor-bridge: visibility ok"),
                    Err(err) => {
                        tracing::warn!(pid, visible, error = %err, "compositor-bridge: visibility failed");
                    }
                }
            }
        }
    }
    tracing::debug!("compositor-bridge: drained and exiting");
}
