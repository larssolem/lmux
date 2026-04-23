//! `org.freedesktop.Notifications` client.
//!
//! Runs on its own background thread with a small tokio runtime so the GTK
//! main thread never blocks on D-Bus. The `Notifier` handle exposes an async
//! `notify()` that returns the notification id (for later `replaces_id`
//! reuse), and registers an `on_click` callback that fires when the user
//! clicks the notification.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use futures_util::stream::StreamExt;
use zbus::{proxy, zvariant};

pub type ClickCallback = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("zbus: {0}")]
    Zbus(#[from] zbus::Error),
    #[error("notifier thread vanished")]
    Dropped,
}

#[derive(Debug)]
enum Cmd {
    Notify {
        replaces_id: u32,
        title: String,
        body: String,
        reply: async_channel::Sender<Result<u32, Error>>,
    },
}

pub struct Notifier {
    tx: async_channel::Sender<Cmd>,
}

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
}

impl Notifier {
    /// Spawn the notifier. `on_click` fires (on the notifier thread) when the
    /// user clicks a notification; the callback should dispatch onto the UI
    /// thread itself.
    pub fn spawn(on_click: ClickCallback) -> Self {
        let (tx, rx) = async_channel::unbounded::<Cmd>();
        thread::Builder::new()
            .name("lmux-notify".into())
            .spawn(move || run_notifier(rx, on_click))
            .ok();
        Self { tx }
    }

    /// Send a notification. Returns the daemon-assigned id on success; log
    /// and swallow on failure (Story 6.2: D-Bus errors are non-fatal).
    pub async fn notify(
        &self,
        replaces_id: u32,
        title: String,
        body: String,
    ) -> Result<u32, Error> {
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        self.tx
            .send(Cmd::Notify {
                replaces_id,
                title,
                body,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Dropped)?;
        reply_rx.recv().await.map_err(|_| Error::Dropped)?
    }
}

fn run_notifier(rx: async_channel::Receiver<Cmd>, on_click: ClickCallback) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, "notifier tokio runtime failed; disabling notifications");
            return;
        }
    };
    rt.block_on(async move {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "no session bus; notifications disabled");
                return;
            }
        };
        let proxy = match NotificationsProxy::new(&conn).await {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = %err, "Notifications proxy failed; disabling");
                return;
            }
        };
        let active: Arc<Mutex<HashMap<u32, ()>>> = Arc::new(Mutex::new(HashMap::new()));
        // Listen for ActionInvoked to fire the click callback. Only fires
        // for ids we created ourselves (tracked in `active`).
        match proxy.receive_action_invoked().await {
            Ok(mut stream) => {
                let cb = on_click.clone();
                let active_sig = active.clone();
                tokio::spawn(async move {
                    while let Some(sig) = stream.next().await {
                        if let Ok(args) = sig.args() {
                            let id = args.id();
                            let known = active_sig
                                .lock()
                                .map(|m| m.contains_key(id))
                                .unwrap_or(false);
                            if known {
                                cb();
                            }
                        }
                    }
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, "ActionInvoked signal subscribe failed");
            }
        }

        while let Ok(cmd) = rx.recv().await {
            match cmd {
                Cmd::Notify {
                    replaces_id,
                    title,
                    body,
                    reply,
                } => {
                    let mut hints: HashMap<&str, zvariant::Value<'_>> = HashMap::new();
                    hints.insert("desktop-entry", zvariant::Value::from("lmux"));
                    let actions: &[&str] = &["default", "Focus lmux"];
                    let result = proxy
                        .notify("lmux", replaces_id, "", &title, &body, actions, hints, -1)
                        .await;
                    match result {
                        Ok(id) => {
                            if let Ok(mut m) = active.lock() {
                                m.insert(id, ());
                            }
                            let _ = reply.send(Ok(id)).await;
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "notification delivery failed");
                            let _ = reply.send(Err(Error::Zbus(err))).await;
                        }
                    }
                }
            }
        }
    });
}
