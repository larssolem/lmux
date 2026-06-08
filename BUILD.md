# Building lmux

## Prerequisites

- **Rust stable ≥ 1.93** — `rustup toolchain install stable`
- **Zig 0.15.2** — required to build vendored `libghostty-vt` (fetched from source as a pinned Zig package during the first build). On macOS 26+, keep a compatible Command Line Tools SDK such as `MacOSX15.4.sdk` installed; the build script will select it automatically when Xcode's default `MacOSX.sdk` points at a newer SDK that Zig 0.15.2 cannot link against.
- **GTK 4 ≥ 4.12** with development headers — on Arch/Manjaro: `pacman -S gtk4 pango cairo`; on Debian/Ubuntu: `apt install libgtk-4-dev libpango1.0-dev libcairo2-dev`
- **KDE Plasma Wayland session** for native KWin attach/preview support. Other Wayland compositors run the terminal cockpit but native attach is disabled until backend support lands. X11 has a best-effort attach backend when `xprop` and `xdotool` are installed.
- **pkg-config** — for locating GTK/Pango/Cairo
- **C compiler** (`gcc` or `clang`) — for `bindgen`

## Build

From the repository root:

```sh
cargo build --release
```

This:

1. Runs `zig build` under `crates/lmux-libghostty/vendor-ghostty/` to produce `libghostty-vt.a` (statically linked into the final binary)
2. Runs `bindgen` to generate FFI bindings from the vendored headers
3. Compiles the Rust workspace

The resulting binary is `target/release/lmux`. Confirm static linkage:

```sh
ldd target/release/lmux | grep -i ghostty   # should print nothing
```

## Development

### mise toolchain

The repo includes `mise.toml` for pinned development tool versions and common
test targets:

For Linux development:

```sh
mise trust
mise install
mise run verify
mise run build:app
mise run install:local
mise run terminal
```

macOS-specific helper tasks are available as `mise run deps:macos` and
`mise run doctor:macos`.

`mise run install:local` installs the `lmux` and `lmux-cli` binaries into
Cargo's local bin directory and registers an OS launcher entry:
`~/Applications/lmux.app` on macOS, or
`~/.local/share/applications/no.jpro.lmux.desktop` on Linux. On Linux it also
installs the KWin script to `~/.local/share/lmux/kwin/lmux-dock.js`.

### macOS port

macOS work-in-progress notes live in [docs/macos-port.md](docs/macos-port.md).
The first supported path is a native build on Apple hardware; Docker is not
used for macOS GUI E2E because the tests need the macOS window server and
Accessibility permissions.

GUI ownership is per window, not per bundle. Open GUI apps normally, then use
the sidebar link button or `lmux-cli satellite attach-window` to attach the
specific window lmux should manage.

On macOS, `lmux-macos-helper` lists windows with CoreGraphics ids. If a window
is missed, focus it and use the bus kind `satellite.attach_focused`.

### Pre-commit hook

The repo ships a pre-commit hook under `.githooks/`. Enable it once:

```sh
git config core.hooksPath .githooks
```

The hook runs:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- An `.rs` scanner that fails on `.unwrap()` / `.expect()` outside `#[cfg(test)]` blocks (NFR11)

### Running

```sh
RUST_LOG=lmux=debug ./target/release/lmux
```

Log output goes to stderr.

### Tests

```sh
mise exec -- cargo test --workspace
```

## Manual smoke tests

Until the full E2E harness lands (see `docs/history/e2e-test-strategy.md`),
these checks cover the current local build at a high level:

- **Terminal cockpit** — launch lmux; the first terminal renders a shell prompt;
  typing produces output; resizing the window updates the terminal grid.
- **Pane lifecycle** — split right and down; close a non-last pane; verify the
  last pane is not closed by the close-pane command.
- **Workspace anchor** — start with one auto-created anchor; create/cycle anchors
  with `Ctrl+B a`; verify the sidebar follows the active workspace.
- **Native attach on supported desktops** — open a normal app window; attach it
  from the sidebar picker; switch anchors and verify the attached window follows
  the active anchor by visibility/fronting, without lmux moving it between
  monitors.
- **Process cleanup** — `kill -9 $(pgrep -x lmux)` from another terminal;
  `pgrep -P 1 -a | grep $SHELL` should be empty within 1 s.

### Bell to toast latency

Verifies p50 ≤ 250 ms and p95 ≤ 500 ms from BEL byte arrival to freedesktop notification delivery.

1. Launch lmux with latency spans enabled — the `bell_to_toast` span wraps the UI-handler → `zbus::Notify()` call:

   ```sh
   RUST_LOG=lmux=info,lmux_notify=info ./target/release/lmux 2> /tmp/lmux-bell.log
   ```

2. In the focused pane, fire 30 bells at 3 s intervals:

   ```sh
   for i in $(seq 1 30); do printf '\a'; sleep 3; done
   ```

3. Each `\a` should produce exactly one toast (the 500 ms per-pane debounce in `BellScanner` coalesces bursts but leaves solitary bells alone).

4. Extract `bell_to_toast` span durations and compute percentiles:

   ```sh
   grep 'close time.busy' /tmp/lmux-bell.log \
     | grep bell_to_toast \
     | sed -E 's/.*busy=([0-9.]+).*/\1/' \
     | sort -n \
     | awk 'BEGIN {c=0} {a[c++]=$1} END {
         printf "n=%d  p50=%.2fms  p95=%.2fms\n",
           c, a[int(c*0.50)], a[int(c*0.95)]
       }'
   ```

5. Acceptance — p50 ≤ 250 ms, p95 ≤ 500 ms on the author's box. If p95 exceeds the budget, the likely suspects are (a) notification daemon latency (try `dunstctl`/KDE's daemon), (b) UI-channel backpressure (check `pty_to_paint` span durations in the same log), or (c) zbus `Notify()` round-trip.

## Troubleshooting

- **`zig: command not found`** — install Zig 0.15.2; set `ZIG=/path/to/zig` if not on `$PATH`
- **macOS 26+: `undefined symbol: _abort` / `_dispatch_*` during `zig build`** — Apple's newer SDKs can omit `arm64-macos` slices that Zig 0.15.2 needs while linking its native build runner. Install Command Line Tools that include a compatible SDK such as `MacOSX15.4.sdk`; `crates/lmux-libghostty/build.rs` detects that SDK and uses a local `xcrun` shim for the Zig process without modifying Apple's SDK files.
- **`Package gtk4 not found`** — install GTK4 dev headers (see Prereqs)
- **First build is slow** — Zig downloads + builds libghostty-vt from source; subsequent builds reuse the Zig cache
