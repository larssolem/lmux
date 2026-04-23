//! Wire-level test for Task #13: the launcher must redirect satellites
//! onto the nested Wayland socket.
//!
//! We can't pop a real GTK window in CI, but we can verify the env
//! overrides applied by [`lmux_compositor::spawn::spawn_tagged_with_env`]
//! by spawning `sh -c 'env >envdump'` and reading the resulting file.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use lmux_compositor::spawn::spawn_tagged_with_env;

fn wait_for_file(path: &std::path::Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(path) {
            // env may finish before the file is flushed; retry on empty
            if !content.is_empty() {
                return Some(content);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

#[test]
fn spawn_with_nested_display_pins_wayland_env() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_str().unwrap();

    let argv: Vec<String> = ["sh", "-c", "env > envdump; exit 0"]
        .into_iter()
        .map(String::from)
        .collect();

    // Sanity check: pre-set DISPLAY + GDK_BACKEND to something we know
    // the override must remove / replace. spawn() inherits parent env,
    // so this proves we're not just observing parent defaults.
    std::env::set_var("DISPLAY", ":99");
    std::env::set_var("GDK_BACKEND", "x11");

    let (_id, _pid) = spawn_tagged_with_env(&argv, Some(cwd), Some("lmux-test-socket")).unwrap();

    let out = wait_for_file(&tmp.path().join("envdump"), Duration::from_secs(5))
        .expect("child never produced envdump");

    // Positive assertions: nested display + wayland backends set.
    assert!(
        out.lines().any(|l| l == "WAYLAND_DISPLAY=lmux-test-socket"),
        "WAYLAND_DISPLAY not pinned:\n{out}"
    );
    assert!(
        out.lines().any(|l| l == "GDK_BACKEND=wayland"),
        "GDK_BACKEND should be forced to wayland:\n{out}"
    );
    assert!(
        out.lines().any(|l| l == "QT_QPA_PLATFORM=wayland"),
        "QT_QPA_PLATFORM should be forced to wayland:\n{out}"
    );

    // Negative assertion: DISPLAY must be stripped so a Wayland-capable
    // toolkit doesn't fall back to X11 just because the cockpit has one.
    assert!(
        !out.lines().any(|l| l.starts_with("DISPLAY=")),
        "DISPLAY should be removed when nested display is forced:\n{out}"
    );

    // The request-id env marker the bus/KWin script relies on must
    // always be present — regression guard for the existing v0.1
    // contract that #13's env work doesn't remove it.
    assert!(
        out.lines().any(|l| l.starts_with("LMUX_SATELLITE_ID=")),
        "LMUX_SATELLITE_ID must still be set:\n{out}"
    );
}

#[test]
fn spawn_without_nested_display_inherits_parent_env() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_str().unwrap();

    std::env::set_var("LMUX_TEST_MARKER", "inherited-ok");

    let argv: Vec<String> = ["sh", "-c", "env > envdump; exit 0"]
        .into_iter()
        .map(String::from)
        .collect();

    // None => no Wayland redirect. Parent env must pass through
    // verbatim (this is the legacy KWin-mode path, must not regress).
    let (_id, _pid) = spawn_tagged_with_env(&argv, Some(cwd), None).unwrap();

    let out = wait_for_file(&tmp.path().join("envdump"), Duration::from_secs(5))
        .expect("child never produced envdump");

    assert!(
        out.lines().any(|l| l == "LMUX_TEST_MARKER=inherited-ok"),
        "parent env should be inherited when no nested display is set:\n{out}"
    );

    std::env::remove_var("LMUX_TEST_MARKER");
}
