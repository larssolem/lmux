//! Shared satellite-process spawner.
//!
//! The compositor impls ([`KwinCompositor`](crate::KwinCompositor),
//! [`NoopCompositor`](crate::NoopCompositor)) both fork a real child process
//! for `spawn_satellite`. Only the post-spawn docking (window geometry,
//! detach/attach) differs — that stays in the impl. The spawned child
//! inherits `LMUX_SATELLITE_ID=<request_id>` so the KWin script (or a
//! future wayland helper) can correlate the new window back to the
//! cockpit's spawn request.

use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use uuid::Uuid;

use crate::CompositorError;

/// Resolve `$XDG_STATE_HOME/lmux/sat/<uuid>.stderr`, creating parents.
fn open_stderr_log(request_id: Uuid) -> Option<File> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    let dir = base.join("lmux").join("sat");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{request_id}.stderr"));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

/// Env var the spawned satellite inherits, carrying the request-id the
/// cockpit will echo back via `satellite.map`.
pub const SATELLITE_ID_ENV: &str = "LMUX_SATELLITE_ID";

/// Return true if `cmd` looks like a Chromium-family browser binary.
fn is_chromium_family(cmd: &str) -> bool {
    let base = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd);
    matches!(
        base,
        "chromium"
            | "chromium-browser"
            | "google-chrome"
            | "google-chrome-stable"
            | "google-chrome-beta"
            | "google-chrome-unstable"
            | "chrome"
            | "brave"
            | "brave-browser"
            | "microsoft-edge"
            | "microsoft-edge-stable"
            | "vivaldi"
            | "vivaldi-stable"
            | "opera"
    )
}

/// Return true if `args` already contains `--flag[=…]` (long-form only).
fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter()
        .any(|a| a == flag || a.starts_with(&format!("{flag}=")))
}

/// Per-browser persistent lmux profile dir. Keeps bookmarks / cookies across
/// cockpit restarts while staying isolated from the user's host-side browser.
fn chromium_profile_dir(cmd: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    let file = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("chromium");
    let dir = base.join("lmux").join("browser-profiles").join(file);
    std::fs::create_dir_all(&dir).ok()?;
    clear_stale_chromium_singletons(&dir);
    Some(dir)
}

/// Chromium writes `SingletonLock`, `SingletonCookie`, `SingletonSocket`
/// symlinks into the user-data-dir to dispatch new launches to a running
/// instance. If the target PID is dead but the symlink remains, a fresh
/// launch can race on the half-broken state and exit silently (the user
/// sees: "Chrome never comes up"). Chrome's own stale-lock handling
/// sometimes misses this when the PID has been reused by the OS or when
/// lmux shut the browser down abruptly. Preflight the symlinks ourselves
/// so every new lmux-spawned Chromium starts from a clean slate.
fn clear_stale_chromium_singletons(dir: &std::path::Path) {
    for name in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let path = dir.join(name);
        // read_link so we inspect the symlink *target*, not its referent
        // (the referent may not exist, in which case path.metadata() errors).
        let Ok(target) = std::fs::read_link(&path) else {
            continue;
        };
        let alive = match name {
            // SingletonLock target format: "<hostname>-<pid>"
            "SingletonLock" => target
                .to_str()
                .and_then(|s| s.rsplit('-').next())
                .and_then(|s| s.parse::<u32>().ok())
                .map(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
                .unwrap_or(false),
            // Cookie/Socket point at /tmp/com.google.Chrome.XXXXXX/… —
            // if the /tmp dir is gone, the singleton is stale.
            _ => target.parent().map(|p| p.exists()).unwrap_or(false),
        };
        if !alive {
            let _ = std::fs::remove_file(&path);
            tracing::info!(?path, ?target, "cleared stale chromium singleton");
        }
    }
}

/// Return true if `cmd` looks like a JetBrains IDE launcher. JetBrains
/// wrappers perform single-instance dispatch via an `idea.lock` file in
/// `~/.config/JetBrains/<product>/` — when a launch finds one, it tells
/// the old JVM to "open project" and exits silently. If the old JVM was
/// connected to a now-dead wayland socket, nothing appears on screen.
/// The cockpit works around this by pointing each JetBrains satellite
/// at its own isolated config/system/log/plugin dirs via `IDEA_PROPERTIES`.
fn is_jetbrains_family(cmd: &str) -> bool {
    let base = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd);
    let base = base.strip_suffix(".sh").unwrap_or(base);
    matches!(
        base,
        "idea"
            | "intellij-idea-ultimate"
            | "intellij-idea-community"
            | "pycharm"
            | "pycharm-professional"
            | "pycharm-community"
            | "webstorm"
            | "phpstorm"
            | "goland"
            | "rider"
            | "clion"
            | "rubymine"
            | "datagrip"
            | "rustrover"
            | "studio"
            | "android-studio"
    )
}

/// Walk `/proc/*/environ` for JetBrains JVMs orphaned by earlier cockpit
/// sessions and SIGTERM them. Without this, a zombie JVM from a prior run
/// (connected to a dead wayland socket) intercepts every subsequent launch
/// via JetBrains' single-instance dispatch — the new process detects the
/// existing lock, forwards "open project" to the dead JVM, and exits
/// silently. The user sees a white pane that never fills in.
///
/// We kill a process when it has `LMUX_SATELLITE_ID` set (so we only touch
/// lmux-spawned processes) AND either:
///   * its `IDEA_PROPERTIES` matches the isolation file we're about to
///     hand to the new spawn (same-config-dir collision), or
///   * its `WAYLAND_DISPLAY` points at a runtime socket that no longer
///     exists (orphaned from a closed cockpit — wayland can't recover,
///     only input-forwarding dispatch can, which is exactly the bug).
///
/// The process must also be a JetBrains binary (exe path contains
/// `jetbrains`, `intellij`, `jbr`, or `idea`). This last check keeps us
/// from accidentally killing an unrelated lmux satellite (e.g. Chrome)
/// whose wayland socket happens to have rotated.
fn kill_stale_jetbrains_jvms(props_path: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    let my_pid = std::process::id();
    let wanted_props = format!("IDEA_PROPERTIES={}", props_path.display());
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    // Wayland socket bound by *this* lmux session's nested host (see
    // `lmux-wayland-host::host::run`). Children of the current session
    // carry this in WAYLAND_DISPLAY — we must never SIGTERM them, since
    // they're live siblings, not orphans.
    let my_wayland = format!("lmux-{my_pid}");
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == my_pid {
            continue;
        }
        let env_path = entry.path().join("environ");
        let Ok(bytes) = std::fs::read(&env_path) else {
            continue;
        };
        let mut has_props = false;
        let mut has_satellite_id = false;
        let mut wayland_display: Option<String> = None;
        for var in bytes.split(|b| *b == 0) {
            let Ok(s) = std::str::from_utf8(var) else {
                continue;
            };
            if s == wanted_props {
                has_props = true;
            } else if s.starts_with("LMUX_SATELLITE_ID=") {
                has_satellite_id = true;
            } else if let Some(rest) = s.strip_prefix("WAYLAND_DISPLAY=") {
                wayland_display = Some(rest.to_owned());
            }
        }
        if !has_satellite_id {
            continue;
        }
        // Live sibling from THIS lmux session — never kill, even if
        // IDEA_PROPERTIES collides. JetBrains' own single-instance
        // dispatch will handle the duplicate launch (the new process
        // forwards "open project" to this one and exits silently).
        if wayland_display.as_deref() == Some(my_wayland.as_str()) {
            continue;
        }
        let socket_dead = match (&runtime_dir, &wayland_display) {
            (Some(rd), Some(display)) => !rd.join(display).exists(),
            _ => false,
        };
        if !(has_props || socket_dead) {
            continue;
        }
        // Confirm it's a JetBrains process before signalling. `exe`
        // points at the launcher or the bundled JVM binary.
        let exe_link = std::fs::read_link(entry.path().join("exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_jetbrains_proc = ["jetbrains", "intellij", "jbr", "idea"]
            .iter()
            .any(|needle| exe_link.to_lowercase().contains(needle));
        if !is_jetbrains_proc {
            continue;
        }
        tracing::info!(
            pid,
            exe = %exe_link,
            wayland = ?wayland_display,
            reason = if has_props { "same IDEA_PROPERTIES" } else { "dead wayland socket" },
            "killing stale JetBrains JVM from prior cockpit session"
        );
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// Persistent per-IDE isolated config dir. Writes an `idea.properties`
/// file the first time it's needed, then returns that file's path — the
/// caller sets `IDEA_PROPERTIES=<path>` on the child so the JVM routes
/// config/system/log/plugins into lmux state. Subsequent launches of
/// the same IDE share this dir so projects and settings persist across
/// cockpit restarts, same convention as `chromium_profile_dir`.
fn jetbrains_properties_file(cmd: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    let file = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("jetbrains");
    let file = file.strip_suffix(".sh").unwrap_or(file);
    let dir = base.join("lmux").join("jetbrains").join(file);
    std::fs::create_dir_all(dir.join("config")).ok()?;
    std::fs::create_dir_all(dir.join("system")).ok()?;
    std::fs::create_dir_all(dir.join("log")).ok()?;
    std::fs::create_dir_all(dir.join("plugins")).ok()?;
    let props_path = dir.join("idea.properties");
    if !props_path.exists() {
        let contents = format!(
            "idea.config.path={dir}/config\n\
             idea.system.path={dir}/system\n\
             idea.log.path={dir}/log\n\
             idea.plugins.path={dir}/plugins\n",
            dir = dir.display()
        );
        std::fs::write(&props_path, contents).ok()?;
    }
    Some(props_path)
}

/// Fork+exec `argv` with `LMUX_SATELLITE_ID=<request_id>` set. Returns the
/// request id the caller should correlate with `satellite.map`. The child
/// is fully detached — its stdio is redirected to `/dev/null` so the
/// cockpit isn't on the hook for draining output.
pub fn spawn_tagged(argv: &[String], cwd: Option<&str>) -> Result<Uuid, CompositorError> {
    spawn_tagged_with_pid(argv, cwd).map(|(id, _pid)| id)
}

/// Like [`spawn_tagged`] but also returns the child PID so the caller can
/// correlate the new window with its process (e.g., for KWin placement
/// by PID while the full `satellite.map` round-trip is still v0.3).
pub fn spawn_tagged_with_pid(
    argv: &[String],
    cwd: Option<&str>,
) -> Result<(Uuid, u32), CompositorError> {
    spawn_tagged_with_env(argv, cwd, None)
}

/// Generalized spawner: like [`spawn_tagged_with_pid`] but the caller can
/// pin `WAYLAND_DISPLAY` to a specific socket. Used by the cockpit to
/// point satellites at the nested compositor socket (ADR-0018) so the
/// child's toplevel lands inside lmux rather than on the host compositor.
///
/// `wayland_display = None` leaves the child's `WAYLAND_DISPLAY`
/// inherited from the parent (the v0.1 KWin-side behaviour).
pub fn spawn_tagged_with_env(
    argv: &[String],
    cwd: Option<&str>,
    wayland_display: Option<&str>,
) -> Result<(Uuid, u32), CompositorError> {
    let (cmd, rest) = argv
        .split_first()
        .ok_or_else(|| CompositorError::Domain("satellite argv is empty".into()))?;
    let request_id = Uuid::new_v4();
    // Chromium-family browsers dispatch to an already-running instance via
    // DBus when they find one, so the new window pops up on the host
    // compositor instead of ours. Inject a cockpit-owned `--user-data-dir`
    // + `--ozone-platform=wayland` so we always get a fresh, wayland-backed
    // process we can actually embed. Only applies under the nested socket.
    let mut owned_args: Vec<String> = rest.to_vec();
    let chromium = wayland_display.is_some() && is_chromium_family(cmd);
    if chromium {
        if !has_flag(&owned_args, "--user-data-dir") {
            if let Some(dir) = chromium_profile_dir(cmd) {
                owned_args.insert(0, format!("--user-data-dir={}", dir.display()));
            }
        }
        if !has_flag(&owned_args, "--ozone-platform") {
            owned_args.insert(0, "--ozone-platform=wayland".into());
        }
        // --enable-logging=stderr surfaces Chrome's startup chatter to our
        // per-satellite stderr file instead of /var/log; without this Chrome
        // silently exits and we have no signal.
        if !has_flag(&owned_args, "--enable-logging") {
            owned_args.insert(0, "--enable-logging=stderr".into());
            owned_args.insert(1, "--v=1".into());
        }
    }
    // JetBrains IDEs single-instance-dispatch via `idea.lock` under
    // `~/.config/JetBrains/…`. If a stale JVM from a previous cockpit
    // run still holds the lock (connected to a now-dead wayland socket),
    // new launches silently forward to it and nothing shows up in our
    // nested compositor. Override `IDEA_PROPERTIES` so each cockpit gets
    // its own config/system/log/plugin root — the lock lives there too,
    // and stale JVMs from prior runs are invisible.
    let jetbrains_props = if wayland_display.is_some() && is_jetbrains_family(cmd) {
        let props = jetbrains_properties_file(cmd);
        if let Some(p) = props.as_deref() {
            kill_stale_jetbrains_jvms(p);
        }
        props
    } else {
        None
    };
    let mut builder = Command::new(cmd);
    builder
        .args(&owned_args)
        .env(SATELLITE_ID_ENV, request_id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    // Redirect stderr to a per-satellite log under $XDG_STATE_HOME/lmux/sat/
    // so we can diagnose why Chromium/IntelliJ/etc. fail to start under the
    // nested compositor. Falls back to /dev/null if the log dir can't be
    // created — never a spawn-blocking error.
    if let Some(log) = open_stderr_log(request_id) {
        builder.stderr(log);
    } else {
        builder.stderr(Stdio::null());
    }
    if let Some(dir) = cwd {
        builder.current_dir(dir);
    }
    if let Some(display) = wayland_display {
        // Force satellites onto the nested socket. Also strip GDK/Qt
        // backend overrides that might have been set to "x11" for the
        // cockpit process itself — we want the child to speak Wayland.
        builder
            .env("WAYLAND_DISPLAY", display)
            .env("GDK_BACKEND", "wayland")
            .env("QT_QPA_PLATFORM", "wayland")
            .env("XDG_SESSION_TYPE", "wayland")
            .env("MOZ_ENABLE_WAYLAND", "1")
            .env_remove("DISPLAY");
        // Diagnostic: dump every wayland-protocol op the child performs to
        // stderr so we can see where Chrome / IntelliJ silently fail to
        // attach. Remove once the cockpit is stable on Chromium.
        if chromium {
            builder.env("WAYLAND_DEBUG", "client");
        }
    }
    if let Some(props) = jetbrains_props {
        builder.env("IDEA_PROPERTIES", props);
    }
    match builder.spawn() {
        Ok(child) => {
            let pid = child.id();
            let cmd_for_log = cmd.to_string();
            // Reap the child on a dedicated thread and log its exit so we
            // can diagnose silent satellite crashes (Chrome's "Connection
            // refused" + immediate exit, JetBrains single-instance handoff,
            // etc). Without this, children become zombies and we never see
            // why they vanished.
            std::thread::Builder::new()
                .name(format!("sat-reap-{pid}"))
                .spawn(move || {
                    let mut child = child;
                    match child.wait() {
                        Ok(status) => tracing::info!(
                            pid,
                            cmd = %cmd_for_log,
                            request_id = %request_id,
                            exit_code = ?status.code(),
                            success = status.success(),
                            "satellite process exited"
                        ),
                        Err(err) => tracing::warn!(
                            pid,
                            cmd = %cmd_for_log,
                            error = %err,
                            "satellite reap failed"
                        ),
                    }
                })
                .ok();
            Ok((request_id, pid))
        }
        Err(err) => Err(CompositorError::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn spawn_true_returns_uuid() {
        let id = spawn_tagged(&["true".into()], None).unwrap();
        assert_ne!(id, Uuid::nil());
    }

    #[test]
    fn empty_argv_is_rejected() {
        match spawn_tagged(&[], None) {
            Err(CompositorError::Domain(_)) => {}
            other => panic!("expected Domain error, got {other:?}"),
        }
    }
}
