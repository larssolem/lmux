# Building lmux

## Prerequisites

- **Rust stable ≥ 1.93** — `rustup toolchain install stable`
- **Zig ≥ 0.15.2** — required to build vendored `libghostty-vt` (fetched from source as a pinned Zig package during the first build)
- **GTK 4 ≥ 4.12** with development headers — on Arch/Manjaro: `pacman -S gtk4 pango cairo`; on Debian/Ubuntu: `apt install libgtk-4-dev libpango1.0-dev libcairo2-dev`
- **Wayland session** (X11 is not exercised; GTK4 still starts but some features are Wayland-first)
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
3. Compiles the Rust workspace (8 crates)

The resulting binary is `target/release/lmux`. Confirm static linkage:

```sh
ldd target/release/lmux | grep -i ghostty   # should print nothing
```

## Development

### mise toolchain

The repo includes `mise.toml` for pinned development tool versions and common
test targets:

```sh
mise trust
mise install
mise run verify
mise run terminal
```

### macOS port branch

macOS work-in-progress notes live in [docs/macos-port.md](docs/macos-port.md).
The first supported path is a native build on Apple hardware; Docker is not
used for macOS GUI E2E because the tests need the macOS window server and
Accessibility permissions.

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
cargo test --workspace
```

## Manual smoke tests

Until the full E2E harness lands (see `docs/history/e2e-test-strategy.md`), these manual checks verify the v0.1 story acceptance criteria:

- **Story 1.4** — launch lmux; shell prompt renders; typing produces output; resize window; cols/rows track the window
- **Story 7.3 (PDEATHSIG)** — `kill -9 $(pgrep -x lmux)` from another terminal; `pgrep -P 1 -a | grep $SHELL` should be empty within 1 s

### Bell → toast latency (Story 6.3, NFR3)

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

- **`zig: command not found`** — install Zig 0.15.2+; set `ZIG=/path/to/zig` if not on `$PATH`
- **`Package gtk4 not found`** — install GTK4 dev headers (see Prereqs)
- **First build is slow** — Zig downloads + builds libghostty-vt from source; subsequent builds reuse the Zig cache
