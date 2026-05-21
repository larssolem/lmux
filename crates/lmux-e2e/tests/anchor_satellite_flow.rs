//! Live E2E: start lmux, launch real windowed programs under anchors,
//! create more anchors, then switch between all anchors.
//!
//! This opens a real lmux window and spawns real child processes, so it is
//! gated behind `LMUX_E2E_LIVE=1`. On macOS it builds a tiny AppKit
//! executable at test time so the test observes real windows instead of
//! merely proving that a process was spawned.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use assert_cmd::assert::OutputAssertExt;
use lmux_e2e::{Env, RunningLmux};
use predicates::prelude::*;

#[test]
fn three_anchor_satellite_switch_flow() {
    if std::env::var("LMUX_E2E_LIVE").as_deref() != Ok("1") {
        eprintln!("skipping live lmux e2e; set LMUX_E2E_LIVE=1");
        return;
    }

    build_live_binaries();

    #[cfg(target_os = "macos")]
    run_macos_window_flow();

    #[cfg(not(target_os = "macos"))]
    panic!("LMUX_E2E_LIVE anchor satellite flow needs a platform-specific real-window harness");
}

#[cfg(target_os = "macos")]
fn run_macos_window_flow() {
    let env = Env::new();
    let test_app = compile_macos_test_app(env.root());
    let first_pid_file = env.root().join("first.pid");
    let second_pid_file = env.root().join("second.pid");
    let mut pids = PidGuard::default();

    let mut lmux = env.spawn_lmux();
    lmux.wait_until_ready(&env);
    let initial_anchors = anchors(&env);
    assert_eq!(initial_anchors.len(), 1, "anchors: {initial_anchors:?}");
    let first_anchor = initial_anchors[0].clone();

    env.cli("lmux-cli")
        .args(["satellite", "open"])
        .arg(&test_app)
        .arg("lmux-e2e-first")
        .arg(&first_pid_file)
        .assert()
        .success()
        .stdout(predicate::str::contains("satellite spawned"));
    let first_pid = wait_for_pid_file(&first_pid_file);
    pids.push(first_pid);
    wait_for_window_state(first_pid, "lmux-e2e-first", WindowState::Visible, &lmux);
    wait_for_log_contains(&lmux, "registered satellite under anchor");

    env.cli("lmux-cli")
        .args(["anchor", "new"])
        .assert()
        .success()
        .stdout(predicate::str::contains("anchor created"));
    let two_anchors = anchors(&env);
    assert_eq!(two_anchors.len(), 2, "anchors: {two_anchors:?}");
    let second_anchor = added_anchor(&initial_anchors, &two_anchors);

    env.cli("lmux-cli")
        .args(["satellite", "open"])
        .arg(&test_app)
        .arg("lmux-e2e-second")
        .arg(&second_pid_file)
        .assert()
        .success()
        .stdout(predicate::str::contains("satellite spawned"));
    let second_pid = wait_for_pid_file(&second_pid_file);
    pids.push(second_pid);
    wait_for_window_state(second_pid, "lmux-e2e-second", WindowState::Visible, &lmux);

    env.cli("lmux-cli")
        .args(["anchor", "new"])
        .assert()
        .success()
        .stdout(predicate::str::contains("anchor created"));
    let three_anchors = anchors(&env);
    assert_eq!(
        three_anchors.len(),
        3,
        "expected three anchors: {three_anchors:?}"
    );
    let third_anchor = added_anchor(&two_anchors, &three_anchors);
    let anchors = [first_anchor, second_anchor, third_anchor];

    for anchor in anchors.iter().cycle().take(6) {
        env.cli("lmux-cli")
            .args(["anchor", "activate", anchor])
            .assert()
            .success()
            .stdout(predicate::str::contains("activated:"));

        let active_index = anchors
            .iter()
            .position(|candidate| candidate == anchor)
            .expect("activated anchor exists");
        match active_index {
            0 => {
                wait_for_window_state(first_pid, "lmux-e2e-first", WindowState::Visible, &lmux);
            }
            1 => {
                wait_for_window_state(second_pid, "lmux-e2e-second", WindowState::Visible, &lmux);
            }
            2 => {}
            _ => unreachable!("test creates exactly three anchors"),
        }
    }

    env.cli("lmux-cli")
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("anchors: 3"))
        .stdout(predicate::str::contains("satellites: ok=2 fail=0"));
}

fn build_live_binaries() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut args = vec!["build", "-p", "lmux", "-p", "lmux-cli"];
    #[cfg(target_os = "macos")]
    args.extend(["-p", "lmux-macos-helper"]);
    let status = Command::new(cargo).args(args).status().unwrap();
    assert!(status.success(), "live e2e binary build failed: {status}");
}

fn anchors(env: &Env) -> Vec<String> {
    let output = env.cli("lmux-cli").args(["pane", "list"]).output().unwrap();
    assert!(output.status.success(), "pane list failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| line.split_once("anchor ").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

fn added_anchor(before: &[String], after: &[String]) -> String {
    after
        .iter()
        .find(|anchor| !before.contains(anchor))
        .unwrap_or_else(|| panic!("no new anchor found; before={before:?} after={after:?}"))
        .clone()
}

#[cfg(target_os = "macos")]
fn compile_macos_test_app(root: &Path) -> String {
    let source = root.join("lmux-e2e-window.m");
    let binary = root.join("lmux-e2e-window");
    std::fs::write(
        &source,
        r#"
#import <Cocoa/Cocoa.h>
#import <unistd.h>

static NSString *gTitle;

@interface AppDelegate : NSObject <NSApplicationDelegate>
@property(strong) NSWindow *window;
@end

@implementation AppDelegate
- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    NSRect frame = NSMakeRect(160, 160, 480, 240);
    self.window = [[NSWindow alloc] initWithContentRect:frame
        styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable)
        backing:NSBackingStoreBuffered
        defer:NO];
    [self.window setTitle:gTitle];
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
}
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    (void)sender;
    return YES;
}
@end

int main(int argc, const char * argv[]) {
    @autoreleasepool {
        if (argc < 3) {
            return 64;
        }
        gTitle = [[NSString alloc] initWithUTF8String:argv[1]];
        NSString *pidPath = [[NSString alloc] initWithUTF8String:argv[2]];
        NSString *pidText = [NSString stringWithFormat:@"%d\n", getpid()];
        [pidText writeToFile:pidPath atomically:YES encoding:NSUTF8StringEncoding error:NULL];

        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        AppDelegate *delegate = [AppDelegate new];
        [app setDelegate:delegate];
        [app run];
    }
    return 0;
}
"#,
    )
    .unwrap();

    let output = Command::new("xcrun")
        .args(["clang", "-fobjc-arc", "-framework", "Cocoa"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compile macOS E2E test app failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    binary.display().to_string()
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowState {
    Missing,
    Visible,
    Hidden,
    Minimized,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct PidGuard(Vec<u32>);

#[cfg(target_os = "macos")]
impl PidGuard {
    fn push(&mut self, pid: u32) {
        self.0.push(pid);
    }
}

#[cfg(target_os = "macos")]
impl Drop for PidGuard {
    fn drop(&mut self) {
        for pid in &self.0 {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();
        }
    }
}

#[cfg(target_os = "macos")]
fn wait_for_pid_file(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                return pid;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("pid file was not written: {}", path.display());
}

#[cfg(target_os = "macos")]
fn wait_for_log_contains(lmux: &RunningLmux, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let log = lmux.log_text();
        if log.contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("lmux log never contained `{needle}`\n{}", lmux.log_text());
}

#[cfg(target_os = "macos")]
fn wait_for_window_state(pid: u32, title: &str, expected: WindowState, lmux: &RunningLmux) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = WindowState::Missing;
    while Instant::now() < deadline {
        last = macos_window_state(pid, title);
        if last == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "window `{title}` pid={pid} was {last:?}, expected {expected:?}\nlmux log:\n{}",
        lmux.log_text()
    );
}

fn macos_window_state(pid: u32, title: &str) -> WindowState {
    let script = format!(
        r#"tell application "System Events"
    set matches to application processes whose unix id is {pid}
    if (count of matches) is 0 then return "missing"
    set p to item 1 of matches
    set processVisible to true
    try
        set processVisible to visible of p
    end try
    set fallbackState to "missing"
    repeat with w in windows of p
        set windowTitle to ""
        try
            set windowTitle to name of w
        end try
        set windowState to "visible"
        if processVisible is false then
            set windowState to "hidden"
        else
            try
                if (value of attribute "AXMinimized" of w) is true then set windowState to "minimized"
            end try
            try
                if (miniaturized of w) is true then set windowState to "minimized"
            end try
        end if
        if fallbackState is "missing" then set fallbackState to windowState
        if windowTitle is {title} then
            return windowState
        end if
    end repeat
    return fallbackState
end tell"#,
        title = apple_script_string(title)
    );
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "window-state osascript failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    match String::from_utf8_lossy(&output.stdout).trim() {
        "visible" => WindowState::Visible,
        "hidden" => WindowState::Hidden,
        "minimized" => WindowState::Minimized,
        _ => WindowState::Missing,
    }
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
