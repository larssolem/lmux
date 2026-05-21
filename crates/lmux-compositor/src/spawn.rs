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
            | "Chromium"
            | "google-chrome"
            | "google-chrome-stable"
            | "google-chrome-beta"
            | "google-chrome-unstable"
            | "Google Chrome"
            | "chrome"
            | "brave"
            | "brave-browser"
            | "Brave Browser"
            | "microsoft-edge"
            | "microsoft-edge-stable"
            | "Microsoft Edge"
            | "vivaldi"
            | "vivaldi-stable"
            | "Vivaldi"
            | "opera"
            | "Opera"
    )
}

fn is_vscode_family(cmd: &str) -> bool {
    let base = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd);
    matches!(base, "Code" | "Code - Insiders" | "VSCodium" | "codium")
}

/// Return true if `args` already contains `--flag[=…]` (long-form only).
fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter()
        .any(|a| a == flag || a.starts_with(&format!("{flag}=")))
}

fn has_jvm_property(args: &[String], key: &str) -> bool {
    let prefix = format!("-D{key}=");
    args.iter().any(|arg| arg.starts_with(&prefix))
}

/// Per-browser persistent lmux profile dir. Keeps bookmarks / cookies across
/// cockpit restarts while staying isolated from the user's host-side browser.
fn chromium_profile_dir(cmd: &str) -> Option<PathBuf> {
    chromium_profile_dir_named(cmd, None)
}

fn chromium_profile_dir_named(cmd: &str, suffix: Option<&str>) -> Option<PathBuf> {
    let base = state_base_dir();
    let mut file = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("chromium")
        .to_owned();
    if let Some(suffix) = suffix {
        file.push('-');
        file.push_str(suffix);
    }
    let mut dir = base.join("lmux").join("app-profiles").join(&file);
    if std::fs::create_dir_all(&dir).is_err() {
        let fallback = std::env::temp_dir()
            .join("lmux-state")
            .join("app-profiles")
            .join(&file);
        std::fs::create_dir_all(&fallback).ok()?;
        dir = fallback;
    }
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
/// at its own isolated config/system/log/plugin dirs.
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

fn clear_stale_jetbrains_profile_locks(props_path: &std::path::Path) {
    let Some(profile_dir) = props_path.parent() else {
        return;
    };
    let pid_path = profile_dir.join("system").join(".pid");
    let stale = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map(|pid| !process_is_alive(pid))
        .unwrap_or(true);
    if !stale {
        return;
    }

    for path in [
        pid_path,
        profile_dir.join("system").join(".port"),
        profile_dir.join("config").join(".lock"),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!(?path, "cleared stale JetBrains profile lock"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::debug!(?path, error = %err, "JetBrains profile lock cleanup failed")
            }
        }
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

/// Persistent per-IDE isolated config dir. Writes an `idea.properties`
/// file the first time it's needed, then returns that file's path — the
/// caller sets `IDEA_PROPERTIES=<path>` on the child so the JVM routes
/// config/system/log/plugins into lmux state. On macOS callers pass a
/// request suffix so every launch gets its own single-instance lock root.
/// Other platforms keep the persistent per-IDE fallback, same convention
/// as `chromium_profile_dir`.
fn jetbrains_properties_file(cmd: &str) -> Option<PathBuf> {
    jetbrains_properties_file_named(cmd, None)
}

fn jetbrains_properties_file_named(cmd: &str, suffix: Option<&str>) -> Option<PathBuf> {
    let base = state_base_dir();
    let mut file = std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("jetbrains")
        .to_owned();
    file = file.strip_suffix(".sh").unwrap_or(&file).to_owned();
    if let Some(suffix) = suffix {
        file.push('-');
        file.push_str(suffix);
    }
    let mut dir = base.join("lmux").join("app-profiles").join(&file);
    if std::fs::create_dir_all(dir.join("config")).is_err() {
        let fallback = std::env::temp_dir()
            .join("lmux-state")
            .join("app-profiles")
            .join(&file);
        dir = fallback;
        std::fs::create_dir_all(dir.join("config")).ok()?;
    }
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
    clear_stale_jetbrains_profile_locks(&props_path);
    Some(props_path)
}

fn isolated_jetbrains_properties_file(cmd: &str, request_id: Uuid) -> Option<PathBuf> {
    macos_jetbrains_properties_file(cmd, request_id).or_else(|| jetbrains_properties_file(cmd))
}

fn apply_jetbrains_isolation_args(
    cmd: &str,
    request_id: Uuid,
    args: &mut Vec<String>,
) -> Option<PathBuf> {
    let props = isolated_jetbrains_properties_file(cmd, request_id)?;
    if !has_jvm_property(args, "idea.properties.file") {
        args.insert(0, format!("-Didea.properties.file={}", props.display()));
    }
    Some(props)
}

fn state_base_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| std::env::temp_dir().join("lmux-state"))
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
    #[cfg(target_os = "macos")]
    if is_macos_open_command(cmd) {
        ensure_macos_open_env(&mut owned_args, SATELLITE_ID_ENV, &request_id.to_string());
    }
    let chromium = should_isolate_chromium(wayland_display, cmd);
    if chromium {
        if !has_flag(&owned_args, "--user-data-dir") {
            let profile_dir =
                macos_chromium_profile_dir(cmd, request_id).or_else(|| chromium_profile_dir(cmd));
            if let Some(dir) = profile_dir {
                owned_args.insert(0, format!("--user-data-dir={}", dir.display()));
            }
        }
        #[cfg(target_os = "macos")]
        if !has_flag(&owned_args, "--new-window") {
            owned_args.insert(0, "--new-window".into());
        }
        if wayland_display.is_some() {
            if !has_flag(&owned_args, "--ozone-platform") {
                owned_args.insert(0, "--ozone-platform=wayland".into());
            }
        }
        // --enable-logging=stderr surfaces Chrome's startup chatter to our
        // per-satellite stderr file instead of /var/log; without this Chrome
        // silently exits and we have no signal.
        if !has_flag(&owned_args, "--enable-logging") {
            owned_args.insert(0, "--enable-logging=stderr".into());
            owned_args.insert(1, "--v=1".into());
        }
    }
    #[cfg(target_os = "macos")]
    if is_vscode_family(cmd) && !has_flag(&owned_args, "--new-window") {
        owned_args.insert(0, "--new-window".into());
    }
    // JetBrains IDEs single-instance-dispatch via `idea.lock` under the
    // IDE config root. If a launch reuses the user's normal root, it can
    // forward to an already-running IntelliJ and exits without creating a
    // new lmux-owned window. Use a request-scoped `idea.properties` on
    // macOS so each launch gets a fresh config/system/log/plugin root.
    let jetbrains_props = if should_isolate_jetbrains(wayland_display, cmd) {
        let props = apply_jetbrains_isolation_args(cmd, request_id, &mut owned_args);
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

#[cfg(target_os = "macos")]
fn macos_jetbrains_properties_file(cmd: &str, request_id: Uuid) -> Option<PathBuf> {
    jetbrains_properties_file_named(cmd, Some(&request_id.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn macos_jetbrains_properties_file(_cmd: &str, _request_id: Uuid) -> Option<PathBuf> {
    None
}

fn should_isolate_chromium(wayland_display: Option<&str>, cmd: &str) -> bool {
    is_chromium_family(cmd) && (wayland_display.is_some() || macos_target())
}

fn should_isolate_jetbrains(wayland_display: Option<&str>, cmd: &str) -> bool {
    is_jetbrains_family(cmd) && (wayland_display.is_some() || macos_target())
}

#[cfg(target_os = "macos")]
fn macos_target() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
fn macos_target() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn macos_chromium_profile_dir(cmd: &str, request_id: Uuid) -> Option<PathBuf> {
    chromium_profile_dir_named(cmd, Some(&request_id.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn macos_chromium_profile_dir(_cmd: &str, _request_id: Uuid) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn is_macos_open_command(cmd: &str) -> bool {
    std::path::Path::new(cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|name| name == "open")
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn ensure_macos_open_env(args: &mut Vec<String>, name: &str, value: &str) {
    let assignment = format!("{name}={value}");
    let already_set = args
        .windows(2)
        .any(|pair| pair[0] == "--env" && pair[1].starts_with(name));
    if !already_set {
        args.insert(0, assignment);
        args.insert(0, "--env".into());
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_open_args_get_explicit_satellite_env() {
        let mut args = vec!["-n".into(), "/Applications/Finder.app".into()];
        ensure_macos_open_env(&mut args, SATELLITE_ID_ENV, "abc");
        assert_eq!(
            args,
            vec![
                "--env".to_string(),
                "LMUX_SATELLITE_ID=abc".to_string(),
                "-n".to_string(),
                "/Applications/Finder.app".to_string()
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_recognizes_app_bundle_chromium_names() {
        assert!(is_chromium_family(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        ));
        assert!(is_chromium_family(
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_recognizes_vscode_app_binary_name() {
        assert!(is_vscode_family(
            "/Applications/Visual Studio Code.app/Contents/MacOS/Code"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_chromium_uses_request_scoped_lmux_profile_dir() {
        let first = macos_chromium_profile_dir("Google Chrome", Uuid::from_u128(0xfeed)).unwrap();
        let second = macos_chromium_profile_dir("Google Chrome", Uuid::from_u128(0xbeef)).unwrap();

        assert!(first.to_string_lossy().contains("Google Chrome"));
        assert_ne!(first, second);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_jetbrains_uses_request_scoped_lmux_properties_dir() {
        let first = macos_jetbrains_properties_file("idea", Uuid::from_u128(0x1234)).unwrap();
        let second = macos_jetbrains_properties_file("idea", Uuid::from_u128(0x5678)).unwrap();

        assert!(first.to_string_lossy().contains("app-profiles"));
        assert_ne!(first, second);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_jetbrains_launch_adds_explicit_properties_argument() {
        let request_id = Uuid::from_u128(0x9012);
        let mut args = vec!["/tmp/project".to_string()];

        let props = apply_jetbrains_isolation_args("idea", request_id, &mut args).unwrap();
        let expected = format!("-Didea.properties.file={}", props.display());

        assert_eq!(args.first().map(String::as_str), Some(expected.as_str()));
        assert_eq!(args.get(1).map(String::as_str), Some("/tmp/project"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_jetbrains_launch_keeps_existing_properties_argument() {
        let mut args = vec![
            "-Didea.properties.file=/tmp/custom.properties".to_string(),
            "/tmp/project".to_string(),
        ];

        let _props =
            apply_jetbrains_isolation_args("idea", Uuid::from_u128(0x9013), &mut args).unwrap();

        assert_eq!(
            args,
            vec![
                "-Didea.properties.file=/tmp/custom.properties".to_string(),
                "/tmp/project".to_string()
            ]
        );
    }

    #[test]
    fn clears_dead_jetbrains_profile_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path();
        std::fs::create_dir_all(profile.join("config")).unwrap();
        std::fs::create_dir_all(profile.join("system")).unwrap();
        let props = profile.join("idea.properties");
        std::fs::write(&props, "").unwrap();
        std::fs::write(profile.join("system").join(".pid"), "999999").unwrap();
        std::fs::write(profile.join("system").join(".port"), "").unwrap();
        std::fs::write(profile.join("config").join(".lock"), "").unwrap();

        clear_stale_jetbrains_profile_locks(&props);

        assert!(!profile.join("system").join(".pid").exists());
        assert!(!profile.join("system").join(".port").exists());
        assert!(!profile.join("config").join(".lock").exists());
    }
}
