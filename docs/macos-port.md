# macOS Port Test Notes

This branch contains the Linux-testable foundation for the macOS port:

- Linux-only Wayland host dependencies are target-gated.
- Bus/control sockets have macOS runtime-dir fallbacks.
- Peer credential checks have a macOS path.
- PTY cwd/trampoline behavior is portable enough for non-Linux builds.
- `lmux-compositor` has a stable `SatelliteWindowId` model and a macOS backend scaffold that records the grouped window operations the native helper must perform.

## Current Scope

The branch is intended to compile far enough on macOS to expose the next native blockers. Terminal panes should be the first runtime target. GUI satellites on macOS currently use the Rust-side `MacWindowCompositor` scaffold; a real AppKit/Accessibility helper still needs to replace the recorded-command stub before managed native windows can pass full E2E.

## macOS Prerequisites

Install developer tools and dependencies:

```sh
xcode-select --install
brew install rust zig gtk4 pkg-config
```

The vendored Ghostty build currently expects Zig 0.15.2. If Homebrew has moved ahead, install/pin Zig 0.15.2 or set `ZIG=/path/to/zig-0.15.2` before building.

## Toolchain via mise

This branch pins the build tool versions in `mise.toml`:

```sh
mise trust
mise install
mise run test:port
mise run macos:smoke
mise run terminal
```

`mise run macos:smoke` puts the pinned Zig on `PATH`, so the Ghostty build should use Zig 0.15.2 without a manual `ZIG=...` override. Use `mise run verify` on Linux for the formatting and Linux-testable port checks.

## Build Checks

Fast path:

```sh
scripts/macos-smoke.sh
```

Start with the crates that do not require the GTK app binary:

```sh
cargo test -p lmux-compositor
cargo test -p lmux-bus -p lmux-control -p lmux-pty
cargo check -p lmux-wayland-host
```

Then try the app:

```sh
cargo check -p lmux
mise run terminal
```

## Manual Runtime Smoke

1. Start `lmux`.
2. Verify the initial terminal pane opens and accepts input.
3. Split panes and switch anchors with the normal prefix keybindings.
4. Open the launcher and confirm macOS-specific blockers are logged clearly.

## Future macOS E2E Lane

Full GUI-window E2E needs a real macOS desktop session, not Xcode Simulator:

- dedicated Mac runner or macOS VM via Apple Virtualization Framework;
- test user with Accessibility permission granted to the lmux helper;
- fake-helper tests in normal CI for protocol and grouping logic;
- real-helper E2E for window minimize/restore/focus/placement.

Expected managed-window E2E flow once the helper exists:

1. Start cockpit.
2. Create anchor A and anchor B.
3. Open TextEdit/Finder as a satellite under A.
4. Switch to B and assert A's native app window is minimized/hidden.
5. Switch back to A and assert the native app window is restored and placed.
