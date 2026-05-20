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
brew bundle
```

The vendored Ghostty build expects Zig 0.15.2. `mise install` pins that version;
the build script also checks the Zig version before invoking Ghostty's Zig build.
GTK's Rust `*-sys` crates also require Homebrew/pkg-config metadata for GTK4
and Graphene; `brew bundle` installs both and `mise run doctor:macos` verifies
`gtk4` plus `graphene-gobject-1.0`.

## Toolchain via mise

This branch pins the build tool versions in `mise.toml`:

```sh
mise trust
mise install
mise run doctor:macos
mise run test:port
mise run build:app
mise run macos:smoke
mise run terminal
```

`mise install` installs the pinned Rust/Zig toolchains and runs `brew bundle` on macOS, including Graphene for `graphene-sys`. `mise run macos:smoke` puts the pinned Zig on `PATH`, so the Ghostty build should use Zig 0.15.2 without a manual `ZIG=...` override. Use `mise run doctor:macos` before the first build to catch missing Xcode/Homebrew/GTK/Graphene/pkg-config/Zig prerequisites. Use `mise run verify` on Linux for the formatting and Linux-testable port checks.

## Build Checks

Fast path:

```sh
mise run doctor:macos
mise run macos:smoke
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
mise run build:app
mise run terminal
```

## Manual Runtime Smoke

1. Start `lmux`.
2. Verify the initial terminal pane opens and accepts input.
3. Split panes and switch anchors with the normal prefix keybindings.
4. Open the launcher and confirm macOS-specific blockers are logged clearly.

## macOS Keyboard Shortcuts

The cockpit prefix remains `Ctrl+B` on macOS so the pane-management model
matches Linux and tmux:

- `Ctrl+B`, then `|` / `\` / `+` splits vertically.
- `Ctrl+B`, then `-` splits horizontally.
- `Ctrl+B`, then `o` / `]` cycles focus forward.
- `Ctrl+B`, then `p` / `[` cycles focus backward.
- `Ctrl+B`, then `s` opens the session switcher.
- `Ctrl+B`, then `l` opens the GUI launcher.
- `Ctrl+B`, then `x` closes the focused pane.
- `Ctrl+B`, then `q` shuts down lmux.

Terminal copy/paste accepts the existing Linux chords and macOS Command chords:

- `Ctrl+Shift+C` and `Ctrl+Shift+V`
- `Command+C` and `Command+V`

When a GUI satellite has focus, lmux intentionally lets key events pass through
so the native application keeps its own macOS shortcuts.

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
