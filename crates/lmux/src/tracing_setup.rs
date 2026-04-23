use std::backtrace::Backtrace;
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Holds the `tracing-appender` worker guard so the background writer
/// thread keeps flushing until the cockpit exits. Dropping the guard
/// flushes any buffered records.
pub struct LogGuard {
    _file: Option<WorkerGuard>,
}

pub fn init() -> LogGuard {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,lmux=info"));
    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(false);

    // Rolling file log at `$XDG_STATE_HOME/lmux/logs/lmux.log` — daily
    // rotation keeps the on-disk footprint bounded without pulling in a
    // size-based rotation dependency. NFR27 asks for 5 rolled copies;
    // tracing-appender rolls by date so we cap retention via the
    // built-in `max_log_files` knob.
    let file_layer = log_dir().and_then(|dir| {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("lmux: unable to create log dir {}: {err}", dir.display());
            return None;
        }
        let appender = rolling::Builder::new()
            .rotation(rolling::Rotation::DAILY)
            .filename_prefix("lmux")
            .filename_suffix("log")
            .max_log_files(5)
            .build(&dir)
            .ok()?;
        let (writer, guard) = tracing_appender::non_blocking(appender);
        let layer = fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(false);
        Some((layer, guard))
    });

    let (file_layer, file_guard) = match file_layer {
        Some((layer, guard)) => (Some(layer), Some(guard)),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    LogGuard { _file: file_guard }
}

/// `$XDG_STATE_HOME/lmux/logs/` with `$HOME/.local/state/lmux/logs/`
/// fallback. Returns `None` when neither env var is set — callers then
/// skip the file layer and log only to stderr.
fn log_dir() -> Option<PathBuf> {
    let base = lmux_session::state_home()?;
    Some(base.join("logs"))
}

/// Log panic info + backtrace before letting the default hook run (which
/// prints to stderr and terminates via the runtime). This satisfies NFR15
/// regardless of where the panic originates.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = Backtrace::force_capture();
        tracing::error!(panic = %info, backtrace = %bt, "panic");
        default(info);
    }));
}

pub fn log_startup_banner() {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        libghostty = lmux_libghostty::version(),
        desktop = %std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into()),
        runtime_dir = %std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "unset".into()),
        state_dir = %std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| "unset".into()),
        data_dir = %std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| "unset".into()),
        "lmux starting"
    );
}
