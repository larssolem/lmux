//! GTK → compositor command bridge.
//!
//! AppState runs on the GTK main thread and produces synchronous commands
//! (e.g. "minimize satellite PID 1234 because its owning anchor was just
//! switched away"). [`CompositorControl`] methods are async and talk to
//! KWin over zbus. To keep GTK non-blocking we route commands through a
//! `async_channel::Sender`, owned by AppState, drained on a dedicated
//! tokio current-thread runtime running on its own OS thread.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use lmux_compositor::{CompositorControl, FocusPolicy, SatelliteWindowId, WindowGroupSwitch};

/// Commands the bridge accepts. Extend with geometry/attach when full
/// docking (v0.3) lands.
#[derive(Debug, Clone)]
pub enum CompositorCommand {
    /// Apply a full anchor switch as a grouped operation. macOS uses this
    /// path to keep native windows visually consistent; Linux backends get
    /// a default per-window implementation.
    ApplyWindowGroupSwitch {
        sequence: u64,
        hide: Vec<SatelliteWindowId>,
        show: Vec<SatelliteWindowId>,
        focus_policy: FocusPolicy,
    },
}

#[derive(Clone)]
pub struct CompositorSender {
    tx: async_channel::Sender<CompositorCommand>,
    latest_sequence: Arc<AtomicU64>,
}

impl CompositorSender {
    pub fn try_send(
        &self,
        command: CompositorCommand,
    ) -> Result<(), async_channel::TrySendError<CompositorCommand>> {
        let CompositorCommand::ApplyWindowGroupSwitch { sequence, .. } = &command;
        self.latest_sequence.store(*sequence, Ordering::Relaxed);
        self.tx.try_send(command)
    }
}

pub type CompositorReceiver = async_channel::Receiver<CompositorCommand>;

/// Spawn the bridge thread and return the sender AppState should use.
/// The thread terminates when every sender is dropped (channel close).
pub fn spawn(compositor: Arc<dyn CompositorControl>) -> CompositorSender {
    let (tx, rx) = async_channel::unbounded::<CompositorCommand>();
    let latest_sequence = Arc::new(AtomicU64::new(0));
    let latest_for_drain = latest_sequence.clone();
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
            rt.block_on(async move { drain(rx, compositor, latest_for_drain).await });
        }) {
        Ok(_) => {}
        Err(err) => tracing::warn!(error = %err, "compositor-bridge: thread spawn failed"),
    }
    CompositorSender {
        tx,
        latest_sequence,
    }
}

async fn drain(
    rx: CompositorReceiver,
    compositor: Arc<dyn CompositorControl>,
    latest_sequence_shared: Arc<AtomicU64>,
) {
    let mut latest_sequence = 0;
    while let Ok(mut cmd) = rx.recv().await {
        let mut dropped = 0u32;
        while let Ok(next) = rx.try_recv() {
            match (&cmd, &next) {
                (
                    CompositorCommand::ApplyWindowGroupSwitch { sequence, .. },
                    CompositorCommand::ApplyWindowGroupSwitch {
                        sequence: next_sequence,
                        ..
                    },
                ) if next_sequence >= sequence => {
                    cmd = next;
                    dropped = dropped.saturating_add(1);
                }
                _ => {}
            }
        }
        if dropped > 0 {
            tracing::debug!(dropped, "compositor-bridge: coalesced queued commands");
        }
        match cmd {
            CompositorCommand::ApplyWindowGroupSwitch {
                sequence,
                hide,
                show,
                focus_policy,
            } => {
                if sequence < latest_sequence {
                    tracing::debug!(
                        sequence,
                        latest_sequence,
                        "compositor-bridge: dropping stale group switch"
                    );
                    continue;
                }
                latest_sequence = sequence;
                let started = Instant::now();
                let hide_count = hide.len();
                let show_count = show.len();
                match compositor
                    .apply_window_group_switch_latest(
                        WindowGroupSwitch {
                            hide,
                            show,
                            focus_policy,
                        },
                        sequence,
                        latest_sequence_shared.clone(),
                    )
                    .await
                {
                    Ok(results) => {
                        let failures = results.iter().filter(|r| !r.ok).count();
                        tracing::debug!(
                            operation = "compositor.group_switch",
                            duration_ms = elapsed_ms(started),
                            sequence,
                            hide = hide_count,
                            show = show_count,
                            windows = results.len(),
                            failures,
                            "compositor-bridge: group switch applied"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "compositor-bridge: group switch failed");
                    }
                }
            }
        }
    }
    tracing::debug!("compositor-bridge: drained and exiting");
}

fn elapsed_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use async_trait::async_trait;
    use lmux_compositor::{
        CompositorError, FocusPolicy, Health, Rect, SatelliteWindowId, WindowBackend, WindowId,
        WindowOpResult,
    };
    use tokio::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct RecordingCompositor {
        applied: Mutex<Vec<u64>>,
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

        async fn apply_window_group_switch_latest(
            &self,
            _switch: WindowGroupSwitch,
            sequence: u64,
            _latest_sequence: Arc<AtomicU64>,
        ) -> Result<Vec<WindowOpResult>, CompositorError> {
            self.applied.lock().await.push(sequence);
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn drain_coalesces_rapid_switches_to_latest_sequence() {
        let (tx, rx) = async_channel::unbounded();
        let latest = Arc::new(AtomicU64::new(0));
        let compositor = Arc::new(RecordingCompositor::default());

        for sequence in [1, 2, 3] {
            latest.store(sequence, Ordering::Relaxed);
            tx.send(CompositorCommand::ApplyWindowGroupSwitch {
                sequence,
                hide: Vec::new(),
                show: vec![SatelliteWindowId::for_pid(
                    WindowBackend::Macos,
                    sequence as u32,
                )],
                focus_policy: FocusPolicy::Terminal,
            })
            .await
            .unwrap();
        }
        drop(tx);

        drain(rx, compositor.clone(), latest).await;

        assert_eq!(compositor.applied.lock().await.as_slice(), &[3]);
    }
}
