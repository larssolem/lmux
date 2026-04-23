//! E2E: `lmux-cli satellite open` (Epic 9 + launcher UX).
//!
//! The CLI is the same wire surface the GUI launcher uses (both emit
//! `Kind::SatelliteOpen`). Driving the CLI as a black box exercises:
//!   * argv parsing (`--target`, positional program + args)
//!   * UUID validation
//!   * bus-connect error reporting when no cockpit is running
//!
//! The GUI launcher popover itself is headless-untestable (requires GTK
//! display); its pure logic is covered by unit tests in `launcher.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::assert::OutputAssertExt;
use lmux_e2e::Env;
use predicates::prelude::*;

#[test]
fn open_rejects_invalid_target_uuid() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["satellite", "open", "--target", "not-a-uuid", "firefox"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid target UUID"));
}

#[test]
fn open_requires_argv() {
    let env = Env::new();
    // clap's `required = true` on the positional argv should reject
    // invocations with no program.
    env.cli("lmux-cli")
        .args(["satellite", "open"])
        .assert()
        .failure();
}

#[test]
fn open_surfaces_bus_error_when_no_cockpit() {
    let env = Env::new();
    // No cockpit is running in the sandbox, so the bus connect must fail
    // and the CLI must exit non-zero with a readable error.
    env.cli("lmux-cli")
        .args(["satellite", "open", "true"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lmux-cli:"));
}

#[test]
fn open_forwards_nil_target_by_default() {
    // Smoke test: `--target` has a default of the nil UUID, so omitting it
    // must not produce a parse error. We can't assert the on-wire message
    // without a cockpit, but we can observe that the CLI gets past UUID
    // parsing and into the bus-connect path.
    let env = Env::new();
    let out = env
        .cli("lmux-cli")
        .args(["satellite", "open", "firefox", "--new-window"])
        .output()
        .unwrap_or_else(|e| panic!("spawn cli: {e}"));
    assert!(
        !out.status.success(),
        "expected non-zero exit without cockpit"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("invalid target UUID"),
        "nil UUID default should pass validation — got:\n{stderr}"
    );
}

#[test]
fn help_lists_open_subcommand() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["satellite", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("open"));
}
