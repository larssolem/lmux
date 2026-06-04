//! End-to-end PTY lifecycle: spawn a real /bin/sh, write a command,
//! confirm the output reaches the reader, resize the PTY, send SIGTERM,
//! and verify the child reaps within our budget.
//!
//! Tests use `LMUX_TRAMPOLINE` pointing at `/bin/sh` so we bypass the
//! lmux binary trampoline (we're not running inside a cargo-built `lmux`
//! binary). This exercises the non-trampoline spawn path end-to-end.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lmux_pty::{spawn, SpawnOpts, TRAMPOLINE_ENV};

fn with_no_trampoline<R>(f: impl FnOnce() -> R) -> R {
    // Empty TRAMPOLINE_ENV disables the lmux binary trampoline. The
    // trampoline only adds Linux PDEATHSIG; absence here is fine for the
    // lifecycle tests below and keeps them portable to non-Linux targets.
    let prev = std::env::var_os(TRAMPOLINE_ENV);
    std::env::set_var(TRAMPOLINE_ENV, "");
    let out = f();
    match prev {
        Some(v) => std::env::set_var(TRAMPOLINE_ENV, v),
        None => std::env::remove_var(TRAMPOLINE_ENV),
    }
    out
}

fn with_env_var<R>(key: &str, value: &str, f: impl FnOnce() -> R) -> R {
    let prev = std::env::var_os(key);
    std::env::set_var(key, value);
    let out = f();
    match prev {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    out
}

#[test]
fn spawn_echo_roundtrip() {
    with_no_trampoline(|| {
        let (mut pane, reader) = spawn(SpawnOpts {
            shell: "/bin/sh".into(),
            cols: 80,
            rows: 24,
            cwd: None,
        })
        .expect("spawn");

        // Collect reader output into a channel so we can wait with a
        // timeout rather than blocking read().
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut reader = reader;
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        pane.writer()
            .write_all(b"echo hello-pty-test\n")
            .expect("write echo");

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut acc = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    acc.extend_from_slice(&chunk);
                    if acc.windows(15).any(|w| w == b"hello-pty-test") {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let text = String::from_utf8_lossy(&acc);
        assert!(
            text.contains("hello-pty-test"),
            "echo did not round-trip; saw: {text:?}"
        );

        pane.kill().ok();
        let _ = pane.try_wait();
    });
}

#[test]
fn spawn_advertises_color_capabilities() {
    with_no_trampoline(|| {
        with_env_var("NO_COLOR", "1", || {
            let (mut pane, reader) = spawn(SpawnOpts {
                shell: "/bin/sh".into(),
                cols: 80,
                rows: 24,
                cwd: None,
            })
            .expect("spawn");

            let (tx, rx) = mpsc::channel::<Vec<u8>>();
            let mut reader = reader;
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            });

            pane.writer()
                .write_all(
                    b"printf '%s|%s|%s|%s\\n' \"$TERM\" \"$COLORTERM\" \"$TERM_PROGRAM\" \"$NO_COLOR\"\n",
                )
                .expect("write env check");

            let deadline = Instant::now() + Duration::from_secs(3);
            let mut acc = Vec::new();
            while Instant::now() < deadline {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(chunk) => {
                        acc.extend_from_slice(&chunk);
                        if acc
                            .windows(30)
                            .any(|w| w == b"xterm-256color|truecolor|lmux|")
                        {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }

            let text = String::from_utf8_lossy(&acc);
            assert!(
                text.contains("xterm-256color|truecolor|lmux|"),
                "color env was not advertised or NO_COLOR leaked; saw: {text:?}"
            );

            pane.kill().ok();
            let _ = pane.try_wait();
        });
    });
}

#[test]
fn resize_does_not_fail_on_a_live_pane() {
    with_no_trampoline(|| {
        let (mut pane, _reader) = spawn(SpawnOpts {
            shell: "/bin/sh".into(),
            cols: 80,
            rows: 24,
            cwd: None,
        })
        .expect("spawn");

        pane.resize(100, 30, 8, 16).expect("resize");
        pane.resize(40, 10, 8, 16).expect("shrink");

        pane.kill().ok();
    });
}

#[test]
fn terminate_reaps_child_within_budget() {
    with_no_trampoline(|| {
        let (mut pane, _reader) = spawn(SpawnOpts {
            shell: "/bin/sh".into(),
            cols: 80,
            rows: 24,
            cwd: None,
        })
        .expect("spawn");

        assert!(pane.child_pid().is_some(), "child should be alive");

        let start = Instant::now();
        pane.terminate().expect("terminate (SIGTERM)");

        // Poll try_wait until child exits. /bin/sh should exit on SIGTERM
        // well inside the 500 ms grace window that Epic 7 enforces.
        let deadline = start + Duration::from_millis(1500);
        let mut exited = None;
        while Instant::now() < deadline {
            match pane.try_wait() {
                Ok(Some(status)) => {
                    exited = Some(status);
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(e) => panic!("try_wait error: {e}"),
            }
        }
        let status = exited.expect("child did not exit within 1.5 s of SIGTERM");
        assert!(
            start.elapsed() < Duration::from_millis(1500),
            "reap took too long: {:?}",
            start.elapsed()
        );
        // portable-pty 0.9 reports signal-killed children with code=1 and a
        // non-empty signal field. We rely on the signal field to distinguish
        // "SIGTERM reaped this pane" from "shell exited with status 1".
        assert_eq!(
            status.exit_code(),
            1,
            "portable-pty contract: signal-killed child synthesizes code=1"
        );
        assert!(
            status.signal().is_some(),
            "expected a populated signal name, got {status:?}"
        );
    });
}

#[test]
fn cwd_reflects_initial_directory() {
    with_no_trampoline(|| {
        let cwd = std::env::temp_dir();
        let (mut pane, _reader) = spawn(SpawnOpts {
            shell: "/bin/sh".into(),
            cols: 80,
            rows: 24,
            cwd: Some(&cwd),
        })
        .expect("spawn");

        #[cfg(not(target_os = "linux"))]
        {
            assert!(
                pane.cwd().is_none(),
                "cwd lookup is only implemented through /proc on Linux"
            );
            pane.kill().ok();
            return;
        }

        // /proc/<pid>/cwd requires the child to have finished setting its
        // cwd; give the kernel a beat.
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut got = None;
        while Instant::now() < deadline {
            if let Some(p) = pane.cwd() {
                got = Some(p);
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let got = got.expect("cwd resolves");
        // On macOS /tmp is a symlink; we don't run there in CI but be
        // defensive and canonicalize both.
        let got_canon = std::fs::canonicalize(&got).unwrap_or(got);
        let want_canon = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        assert_eq!(got_canon, want_canon);

        pane.kill().ok();
    });
}
