//! Cockpit-side bus server glue.
//!
//! Spawns a dedicated tokio runtime on its own OS thread so the bus can
//! serve incoming `lmux-cli` connections without entangling itself with
//! the GTK main loop. Pure file-backed reads (`session.list`, `status.get`)
//! are answered directly on the tokio thread. Kinds that need cockpit
//! mutation (anchor.*, session.new, ...) are forwarded over
//! [`DeferredRequestSender`] to the GTK thread; that thread applies the
//! mutation on [`AppState`] and replies over the oneshot carried in the
//! request. Kinds the GTK side doesn't recognise return
//! `not_implemented`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use async_trait::async_trait;
use lmux_bus::{
    kinds::CompositorState,
    paths::{bus_pid_path, bus_socket_path},
    BusError, Kind, PaneSummary, Server, SessionSummary, StatusSnapshot,
};
use lmux_compositor::{CompositorControl, Health};
use tokio::sync::oneshot;

use crate::state::SharedAppState;

#[derive(Clone)]
pub struct SatelliteCounters {
    #[allow(dead_code)]
    pub spawn_ok: Arc<AtomicU32>,
    pub spawn_fail: Arc<AtomicU32>,
}

/// Envelope posted from the tokio bus thread to the GTK main thread for
/// kinds that need cockpit state. The oneshot sender carries the reply
/// back; the bus handler awaits it before answering the client.
pub type DeferredRequest = (Kind, oneshot::Sender<Result<Kind, BusError>>);
pub type DeferredRequestSender = async_channel::Sender<DeferredRequest>;
pub type DeferredRequestReceiver = async_channel::Receiver<DeferredRequest>;

/// Shared read-only state handed to the bus handler. Held by an
/// `Arc`, so the bus thread and the GTK thread can both peek without
/// locking.
#[derive(Clone)]
pub struct BusContext {
    pub store_root: PathBuf,
    pub cockpit_version: String,
    /// Dispatcher to the GTK main thread. `None` means write kinds
    /// return `not_implemented` — handy for tests that exercise the
    /// read paths in isolation.
    pub write_tx: Option<DeferredRequestSender>,
    /// Live anchor count, kept in sync by `AppState`'s anchors-changed
    /// hook. The bus thread reads it to answer `status.get` without
    /// needing to round-trip to GTK.
    pub anchor_count: Arc<AtomicU32>,
    /// Compositor probe. Hit on every `status.get` so the bus can report
    /// whether the window-manager half of the stack is reachable.
    pub compositor: Arc<dyn CompositorControl>,
    /// Successful `satellite.open` spawns since cockpit start (Epic 11 S4).
    pub satellite_spawn_ok: Arc<AtomicU32>,
    /// Failed `satellite.open` spawns since cockpit start (Epic 11 S4).
    pub satellite_spawn_fail: Arc<AtomicU32>,
}

pub struct LmuxBusHandler {
    ctx: BusContext,
}

#[async_trait]
impl lmux_bus::Handler for LmuxBusHandler {
    async fn handle(&self, req: Kind) -> Result<Kind, BusError> {
        match req {
            Kind::SessionList {} => self.handle_session_list().await,
            Kind::StatusGet {} => self.handle_status_get().await,
            other => self.dispatch_to_gtk(other).await,
        }
    }

    fn cockpit_version(&self) -> String {
        self.ctx.cockpit_version.clone()
    }
}

impl LmuxBusHandler {
    async fn dispatch_to_gtk(&self, req: Kind) -> Result<Kind, BusError> {
        let Some(tx) = &self.ctx.write_tx else {
            return Err(BusError::Domain(format!(
                "not_implemented: {req:?} (no GTK dispatcher installed)"
            )));
        };
        let (resp_tx, resp_rx) = oneshot::channel();
        tx.send((req, resp_tx))
            .await
            .map_err(|e| BusError::Domain(format!("dispatch send: {e}")))?;
        resp_rx
            .await
            .map_err(|e| BusError::Domain(format!("dispatch response dropped: {e}")))?
    }

    async fn handle_session_list(&self) -> Result<Kind, BusError> {
        let root = self.ctx.store_root.clone();
        // SessionStore uses std::fs; keep the tokio runtime responsive by
        // bouncing the read onto a blocking worker.
        let entries = tokio::task::spawn_blocking(move || -> Result<Vec<_>, BusError> {
            let store = lmux_session::SessionStore::new(&root);
            if !store.root().exists() {
                return Ok(Vec::new());
            }
            store
                .list()
                .map_err(|e| BusError::Domain(format!("session store: {e}")))
        })
        .await
        .map_err(|e| BusError::Domain(format!("join blocking worker: {e}")))??;

        let sessions = entries
            .into_iter()
            .map(|e| SessionSummary {
                name: e.name,
                // v0.2-alpha: the store only tracks last-opened; reuse it
                // for created_at so the CLI column stays identical to the
                // pre-bus output. Split in v0.3 once SessionIndex carries
                // both fields.
                created_at_unix_seconds: e.last_opened_at_unix_seconds,
                last_active_unix_seconds: Some(e.last_opened_at_unix_seconds),
            })
            .collect();
        Ok(Kind::SessionListResult { sessions })
    }

    async fn handle_status_get(&self) -> Result<Kind, BusError> {
        let pid = std::process::id() as i32;
        let root = self.ctx.store_root.clone();
        let session_count = tokio::task::spawn_blocking(move || -> u32 {
            let store = lmux_session::SessionStore::new(&root);
            if !store.root().exists() {
                return 0;
            }
            store.list().map(|v| v.len() as u32).unwrap_or(0)
        })
        .await
        .unwrap_or(0);
        let compositor = match self.ctx.compositor.health().await {
            Health::Online => CompositorState::Online,
            Health::ScriptMissing | Health::Offline { .. } => CompositorState::Offline,
        };
        Ok(Kind::StatusGetResult(StatusSnapshot {
            cockpit_version: self.ctx.cockpit_version.clone(),
            pid,
            session_count,
            anchor_count: self.ctx.anchor_count.load(Ordering::Relaxed),
            compositor,
            satellite_spawn_ok: self.ctx.satellite_spawn_ok.load(Ordering::Relaxed),
            satellite_spawn_fail: self.ctx.satellite_spawn_fail.load(Ordering::Relaxed),
        }))
    }
}

/// GTK-side dispatcher: consumes [`DeferredRequest`]s off the channel
/// and applies them to [`SessionStore`] / the cockpit state. First write
/// kind wired is `session.new`; the rest still return `not_implemented`
/// so the external surface is clear about where the cut is.
///
/// Invoked from `glib::MainContext::spawn_local` so every mutation runs
/// on the GTK main loop. Does NOT borrow `AppState` here — the caller
/// pre-resolves whatever it needs and passes it in via closures.
pub async fn run_dispatcher(
    rx: DeferredRequestReceiver,
    store_root: std::path::PathBuf,
    state: SharedAppState,
    satellite_counters: SatelliteCounters,
) {
    while let Ok((req, resp_tx)) = rx.recv().await {
        let result = handle_deferred(req, &store_root, &state, &satellite_counters);
        let _ = resp_tx.send(result);
    }
}

fn handle_deferred(
    req: Kind,
    store_root: &std::path::Path,
    state: &SharedAppState,
    satellite_counters: &SatelliteCounters,
) -> Result<Kind, BusError> {
    match req {
        Kind::AnchorPause { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st.pane_for_anchor_id(pane_id).ok_or_else(|| {
                BusError::Domain(format!("anchor.pause: unknown anchor {pane_id}"))
            })?;
            st.pause_anchor(pid).map_err(BusError::Domain)?;
            Ok(Kind::Ok {
                of: Some("anchor.pause".into()),
            })
        }
        Kind::AnchorResume { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st.pane_for_anchor_id(pane_id).ok_or_else(|| {
                BusError::Domain(format!("anchor.resume: unknown anchor {pane_id}"))
            })?;
            st.resume_anchor(pid).map_err(BusError::Domain)?;
            Ok(Kind::Ok {
                of: Some("anchor.resume".into()),
            })
        }
        Kind::AnchorHide { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st.pane_for_anchor_id(pane_id).ok_or_else(|| {
                BusError::Domain(format!("anchor.hide: unknown anchor {pane_id}"))
            })?;
            st.hide_anchor(pid).map_err(BusError::Domain)?;
            Ok(Kind::Ok {
                of: Some("anchor.hide".into()),
            })
        }
        Kind::AnchorReattach { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st.pane_for_anchor_id(pane_id).ok_or_else(|| {
                BusError::Domain(format!("anchor.reattach: unknown anchor {pane_id}"))
            })?;
            st.reattach_anchor(pid).map_err(BusError::Domain)?;
            Ok(Kind::Ok {
                of: Some("anchor.reattach".into()),
            })
        }
        Kind::SessionNew { name } => {
            let store = lmux_session::SessionStore::new(store_root);
            // SessionStore::create wraps std::fs; safe to call from the
            // GTK thread because it's fast (single-file write). If the
            // store root doesn't exist yet we let SessionStore own the
            // mkdir via its usual write path.
            match store.create(&name, lmux_session::now_unix_seconds()) {
                Ok(_) => Ok(Kind::Ok {
                    of: Some("session.new".into()),
                }),
                Err(err) => Err(BusError::Domain(format!("session.new: {err}"))),
            }
        }
        Kind::SessionRename { from, to } => {
            let store = lmux_session::SessionStore::new(store_root);
            match store.rename(&from, &to) {
                Ok(()) => Ok(Kind::Ok {
                    of: Some("session.rename".into()),
                }),
                Err(err) => Err(BusError::Domain(format!("session.rename: {err}"))),
            }
        }
        Kind::SessionDelete { name } => {
            let store = lmux_session::SessionStore::new(store_root);
            match store.delete(&name) {
                Ok(()) => Ok(Kind::Ok {
                    of: Some("session.delete".into()),
                }),
                Err(err) => Err(BusError::Domain(format!("session.delete: {err}"))),
            }
        }
        Kind::SessionOpen { name } => {
            let mut st = state.borrow_mut();
            match st.switch_session(name.clone(), store_root) {
                Ok(()) => Ok(Kind::Ok {
                    of: Some("session.open".into()),
                }),
                Err(err) => Err(BusError::Domain(format!("session.open: {err}"))),
            }
        }
        Kind::AnchorUntag { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st.pane_for_anchor_id(pane_id).ok_or_else(|| {
                BusError::Domain(format!("anchor.untag: unknown anchor {pane_id}"))
            })?;
            st.remove_anchor(pid);
            Ok(Kind::Ok {
                of: Some("anchor.untag".into()),
            })
        }
        Kind::PaneList {} => {
            let st = state.borrow();
            let panes = st
                .pane_listing()
                .into_iter()
                .map(|(pane_id, anchor_id, cwd)| PaneSummary {
                    pane_id,
                    anchor_id,
                    cwd: cwd.map(|p| p.display().to_string()),
                })
                .collect();
            Ok(Kind::PaneListResult { panes })
        }
        Kind::AnchorTag { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st
                .pane_for_uuid(pane_id)
                .ok_or_else(|| BusError::Domain(format!("anchor.tag: unknown pane {pane_id}")))?;
            if st.is_anchor(pid) {
                return Err(BusError::Domain(format!(
                    "anchor.tag: pane {pid} is already an anchor"
                )));
            }
            st.add_anchor(pid);
            Ok(Kind::Ok {
                of: Some("anchor.tag".into()),
            })
        }
        Kind::AnchorNew {} => {
            let mut st = state.borrow_mut();
            st.create_new_anchor();
            Ok(Kind::Ok {
                of: Some("anchor.new".into()),
            })
        }
        Kind::AnchorActivate { pane_id } => {
            let mut st = state.borrow_mut();
            let pid = st.pane_for_anchor_id(pane_id).ok_or_else(|| {
                BusError::Domain(format!("anchor.activate: unknown anchor {pane_id}"))
            })?;
            st.set_active_anchor(Some(pid));
            Ok(Kind::Ok {
                of: Some("anchor.activate".into()),
            })
        }
        Kind::SatelliteOpen {
            argv,
            target_pane: _,
            no_sandbox: _,
        } => {
            #[cfg(target_os = "macos")]
            {
                let _ = argv;
                satellite_counters
                    .spawn_fail
                    .fetch_add(1, Ordering::Relaxed);
                return Err(BusError::Domain(
                    "satellite.open is disabled on macOS; focus a native window and use satellite.attach_focused".into(),
                ));
            }
            #[cfg(not(target_os = "macos"))]
            {
                if argv.is_empty() {
                    satellite_counters
                        .spawn_fail
                        .fetch_add(1, Ordering::Relaxed);
                    return Err(BusError::Domain("satellite.open: argv is empty".into()));
                }
                let anchor_at_launch = state.borrow().active_anchor();
                let nested_display = state.borrow().wayland_display_name().map(str::to_string);
                match lmux_compositor::spawn::spawn_tagged_with_env(
                    &argv,
                    None,
                    nested_display.as_deref(),
                ) {
                    Ok((request_id, pid)) => {
                        satellite_counters.spawn_ok.fetch_add(1, Ordering::Relaxed);
                        if let Some(anchor) = anchor_at_launch {
                            let mut st = state.borrow_mut();
                            st.register_satellite_spawn(anchor, request_id, pid, None);
                        } else {
                            tracing::warn!(pid, "satellite.open: no active anchor; unmanaged");
                        }
                        Ok(Kind::Ok {
                            of: Some("satellite.open".into()),
                        })
                    }
                    Err(err) => {
                        satellite_counters
                            .spawn_fail
                            .fetch_add(1, Ordering::Relaxed);
                        Err(BusError::Domain(format!("satellite.open: {err}")))
                    }
                }
            }
        }
        Kind::SatelliteAttachFocused {} => {
            #[cfg(target_os = "macos")]
            {
                let mut st = state.borrow_mut();
                st.attach_focused_macos_window_to_active_anchor()
                    .map_err(|err| BusError::Domain(format!("satellite.attach_focused: {err}")))?;
                Ok(Kind::Ok {
                    of: Some("satellite.attach_focused".into()),
                })
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err(BusError::Domain(
                    "satellite.attach_focused: macOS only".into(),
                ))
            }
        }
        Kind::SatelliteListWindows {} => {
            #[cfg(target_os = "macos")]
            {
                let windows = lmux_macos_helper::list_windows(None, None)
                    .map_err(|err| BusError::Domain(format!("satellite.list_windows: {err}")))?
                    .into_iter()
                    .map(|window| lmux_bus::kinds::MacosWindowCandidate {
                        pid: window.pid,
                        window_id: window.window_id,
                        window_index: window.window_index,
                        bundle_id: window.bundle_id,
                        title: window.title,
                    })
                    .collect();
                Ok(Kind::SatelliteListWindowsResult { windows })
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err(BusError::Domain(
                    "satellite.list_windows: macOS only".into(),
                ))
            }
        }
        Kind::SatelliteAttachWindow {
            pid,
            window_id,
            window_index,
            bundle_id,
            title,
        } => {
            #[cfg(target_os = "macos")]
            {
                let mut st = state.borrow_mut();
                st.attach_macos_window_to_active_anchor(lmux_macos_helper::WindowInfo {
                    window_id,
                    pid,
                    bundle_id,
                    window_index: window_index.unwrap_or(1),
                    title,
                })
                .map_err(|err| BusError::Domain(format!("satellite.attach_window: {err}")))?;
                Ok(Kind::Ok {
                    of: Some("satellite.attach_window".into()),
                })
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (pid, window_id, window_index, bundle_id, title);
                Err(BusError::Domain(
                    "satellite.attach_window: macOS only".into(),
                ))
            }
        }
        other => Err(BusError::Domain(format!(
            "not_implemented: {other:?} (dispatcher)"
        ))),
    }
}

/// Start the bus server on a background thread + tokio runtime. Returns a
/// handle to the OS thread so the caller can decide whether to join on
/// shutdown; for v0.2 we fire-and-forget — clean unbind happens via the
/// `Server` `Drop` impl when the process exits.
pub fn start(ctx: BusContext) -> Option<thread::JoinHandle<()>> {
    let socket_path = match bus_socket_path() {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(error = %err, "lmux-bus: disabled (no socket path)");
            return None;
        }
    };
    let pid_path = match bus_pid_path() {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(error = %err, "lmux-bus: disabled (no pid path)");
            return None;
        }
    };
    let handler = Arc::new(LmuxBusHandler { ctx });

    let join = thread::Builder::new()
        .name("lmux-bus".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    tracing::warn!(error = %err, "lmux-bus: tokio runtime failed");
                    return;
                }
            };
            rt.block_on(async move {
                match Server::bind(socket_path.clone(), pid_path.clone(), handler).await {
                    Ok(mut server) => {
                        tracing::info!(path = %socket_path.display(), "lmux-bus: up");
                        // Park the task forever; Server cleanup runs on drop.
                        std::future::pending::<()>().await;
                        // unreachable, but keeps `server` alive
                        server.shutdown().await;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "lmux-bus: bind failed");
                    }
                }
            });
        })
        .ok()?;
    Some(join)
}
