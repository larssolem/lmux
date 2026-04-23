//! E2E: `lmux-cli session list` (Epic 2 + Epic 3).
//!
//! Drives the real `lmux-cli` binary against an isolated XDG sandbox and
//! asserts the output shape. The cockpit is not running, so this
//! exercises the store-fallback path the CLI advertises in its docstring.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::assert::OutputAssertExt;
use lmux_e2e::Env;
use predicates::prelude::*;

#[test]
fn lists_nothing_when_no_sessions_seeded() {
    let env = Env::new();
    env.cli("lmux-cli")
        .arg("session")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("(no sessions)"));
}

#[test]
fn lists_seeded_session_by_name() {
    let env = Env::new();
    env.seed_session("alpha", 1000);
    env.seed_session("beta", 2000);

    env.cli("lmux-cli")
        .arg("session")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha"))
        .stdout(predicate::str::contains("beta"));
}

#[test]
fn most_recently_opened_sorts_first() {
    let env = Env::new();
    // Earlier timestamp first; the store sorts DESC by last-opened.
    env.seed_session("older", 1000);
    env.seed_session("newer", 9999);

    let output = env
        .cli("lmux-cli")
        .arg("session")
        .arg("list")
        .output()
        .unwrap_or_else(|e| panic!("spawn cli: {e}"));
    assert!(output.status.success(), "non-zero exit: {:?}", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let newer_idx = stdout.find("newer").expect("newer missing");
    let older_idx = stdout.find("older").expect("older missing");
    assert!(
        newer_idx < older_idx,
        "recency order broken — got:\n{stdout}"
    );
}
