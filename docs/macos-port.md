# macOS Port Test Notes

This branch contains the foundation for the macOS port:

- Linux-only Wayland host dependencies are target-gated.
- Bus/control sockets have macOS runtime-dir fallbacks.
- Peer credential checks have a macOS path.
- PTY cwd/trampoline behavior is portable enough for non-Linux builds.
- `lmux-compositor` has a stable `SatelliteWindowId` model and a macOS backend that delegates grouped window operations to `lmux-macos-helper`.
- `lmux-macos-helper` lists windows with CoreGraphics `CGWindowID` and uses Accessibility/System Events only for metadata and visibility control fallback.

## Current Scope

The branch is intended to run native terminal panes and controlled macOS GUI
satellites. GUI ownership is intentionally conservative: lmux controls only
windows the user explicitly attaches, keyed by CoreGraphics window id. Ambiguous
windows are left unmanaged instead of falling back to bundle-wide control.

## macOS Prerequisites

Install developer tools and dependencies:

```sh
xcode-select --install
brew bundle
```

The vendored Ghostty build expects Zig 0.15.2 (Ghostty has not yet migrated to
0.16; upstream issue ghostty-org/ghostty#12228). `mise install` pins that
version; the build script also checks the Zig version before invoking Ghostty's
Zig build.

On macOS 26+ there is an additional SDK problem: newer Apple SDKs can omit
`arm64-macos` slices that Zig 0.15.2 needs while linking its native build
runner, so the build can fail with `undefined symbol: _abort` and friends.
Keep a compatible Command Line Tools SDK such as `MacOSX15.4.sdk` installed;
the lmux build script detects it and uses a local `xcrun` shim for the Zig
process without modifying Apple's SDK files.
GTK's Rust `*-sys` crates also require Homebrew/pkg-config metadata for GTK4
and Graphene; `brew bundle` installs both and `mise run doctor:macos` verifies
`gtk4` plus `graphene-gobject-1.0`.

## Toolchain via mise

This branch pins the build tool versions in `mise.toml`:

```sh
mise trust
mise install
mise run deps:macos
mise run doctor:macos
mise run test:port
mise run build:app
mise run macos:smoke
mise run terminal
```

`mise install` installs the pinned Rust/Zig toolchains and attempts to run the macOS Homebrew dependency hook. If the toolchains are already installed, run `mise run deps:macos` explicitly; the build/start tasks also depend on that task. It runs `brew bundle` on macOS, including Graphene for `graphene-sys`. `mise run macos:smoke` puts the pinned Zig on `PATH`, so the Ghostty build should use Zig 0.15.2 without a manual `ZIG=...` override. Use `mise run doctor:macos` before the first build to catch missing Xcode/Homebrew/GTK/Graphene/pkg-config/Zig prerequisites. Use `mise run verify` on Linux for the formatting and Linux-testable port checks.

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
4. Open Safari or VS Code normally, then attach the exact window to the active
   anchor.
5. Open Chrome or IntelliJ normally, then attach the real work window rather
   than setup or project chooser windows.

## App Profiles

The old lmux launcher flow is disabled on macOS. Open apps normally and attach
the specific window lmux should manage. Any legacy lmux-managed app profiles are
stored under the platform state directory:

```text
$XDG_STATE_HOME/lmux/app-profiles/<app-key>/
```

On macOS, when `XDG_STATE_HOME` is not set, this currently resolves through the
same state fallback used by the rest of the Rust workspace:

```text
~/.local/state/lmux/app-profiles/<app-key>/
```

Resetting one managed app profile is safe but destructive for that lmux-owned
profile:

```sh
rm -rf ~/.local/state/lmux/app-profiles/"Google Chrome"
rm -rf ~/.local/state/lmux/app-profiles/idea
```

These directories are not the user's normal Chrome or JetBrains profiles.

The native helper exposes CoreGraphics window ids. lmux stores those ids as
the ownership key and carries the old window index only as a visibility
fallback for apps whose Accessibility tree does not expose `AXWindowNumber`.
Ambiguous native windows are left unmanaged rather than guessed by bundle id.

## macOS Window Ownership Model

Anchor switching asks the helper to apply grouped visibility operations only
for windows already registered as lmux-owned. Existing user windows from the
same bundle are not hidden or restored unless the user attached that exact
window.

To attach a window, focus it and press the sidebar link button or use the
`satellite.attach_focused` bus kind. The manual attach path records the exact
native window identity and does not infer ownership from bundle id alone.

## macOS Keyboard Shortcuts

The lmux prefix remains `Ctrl+B` on macOS so the pane-management model
matches Linux and tmux:

- `Ctrl+B`, then `|` / `\` / `+` splits vertically.
- `Ctrl+B`, then `-` splits horizontally.
- `Ctrl+B`, then `o` / `]` cycles focus forward.
- `Ctrl+B`, then `p` / `[` cycles focus backward.
- `Ctrl+B`, then `s` opens the session switcher.
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

Manual managed-window smoke:

1. Start lmux.
2. Create anchor A and anchor B.
3. Under anchor A, open Chrome normally, complete any setup window, then attach
   the normal browser window.
4. Switch to B and assert only A's Chrome window is minimized.
5. Under anchor B, open and attach another Chrome window. Switch between A and
   B and assert the two Chrome windows alternate independently.
6. Repeat the same flow with IntelliJ: project chooser first, then a real
   project window, attach the real project window, then attach a second IntelliJ
   window on the other anchor.
7. Close a tracked window and switch anchors; lmux should stop trying to
   restore the closed window.
