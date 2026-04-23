# E2E Test Strategy — lmux v0.1

**Status:** proposed
**Author:** recorded during Story 1.2 implementation
**Date:** 2026-04-21

## TL;DR

Yes, lmux can have meaningful E2E tests. The natural seams are the **control socket** (ADR-0008), **layout state file**, and **structured tracing output**. Headless GTK4 runs under **Xvfb** locally and on CI. Full value unlocks at Epic 5 (CLI companion) because the control socket is the richest driver; before that, E2E is limited to lifecycle smoke tests.

## Layered test pyramid

| Layer | Scope | Lives in | Drivers |
|---|---|---|---|
| Unit | pure logic inside one crate | `src/ #[cfg(test)] mod tests` | standard `cargo test` |
| Integration | public API of one crate | `crates/<name>/tests/*.rs` | crate public API |
| E2E | `lmux` binary as a black box | `crates/lmux-e2e/tests/*.rs` | control socket, tracing logs, FS observation |

The pyramid skews wide at unit, narrow at E2E. Target for v0.1: one smoke E2E per epic, not per story.

## The seams

### 1. Control socket (primary driver — available from Epic 5)

Per ADR-0008 + PRD, `lmux-cli` speaks length-prefixed JSON over a Unix socket at `$XDG_RUNTIME_DIR/lmux/control.sock`. This is the natural E2E driver:

- Spawn `lmux` with a known `XDG_RUNTIME_DIR`
- Connect a test client to the socket
- Send commands (split, send-keys, query-pane, list-panes, …)
- Assert responses
- Tear down with a clean-exit command

**Why this is the best seam:** it's the same surface scripts/IDE integrations will use, so E2E tests double as integration coverage for the CLI contract.

### 2. Layout state file (for persistence tests — Epic 8)

`$XDG_STATE_HOME/lmux/layout.json` is atomically rewritten. E2E can:

- Pre-seed a layout file, launch lmux, observe that panes restore
- Run operations, exit, read the file, assert persisted state matches

### 3. Structured tracing output (for lifecycle + observability)

All lifecycle events emit `tracing::info!` with structured fields (NFR14). E2E can capture `stderr`, parse JSON lines (once we enable `tracing-subscriber` JSON output in a test profile), and assert a sequence like:

```
startup-complete → pane-open{id=0} → pane-exit{id=0, code=0} → shutdown-complete
```

This is the only seam available **before** Epic 5 — it's what Story 1.4's smoke test will use.

### 4. Notification bus (for Epic 6)

`zbus` speaks to `org.freedesktop.Notifications`. For E2E we don't need a real notification daemon; we can stand up a stub D-Bus service in-process that records received `Notify` calls. Requires a private session bus (`dbus-daemon --session --address=unix:path=…`) spun up by the test harness.

### 5. PTY grid inspection (optional)

libghostty exposes the VT grid state. If we wrap read-access in `lmux-libghostty`, E2E can assert what's on screen after sending input — more surgical than parsing rendered pixels. Defer until a test actually needs it.

## Headless display

GTK4 needs *some* display server. Options:

| Option | Pros | Cons |
|---|---|---|
| **Xvfb** | rock-solid, 5-line CI setup, `apt install xvfb` | X11-only (we also want Wayland coverage) |
| **weston --headless** | native Wayland, matches prod | slower startup, more moving parts |
| Mutter / GNOME nested | closer to real desktop | heavy |
| GDK offscreen backend | in-process, no subprocess | incomplete for GTK4; pointer/keyboard injection is awkward |

**Plan:** Xvfb for the CI default. Add a Wayland lane via `weston --headless` once Epic 6 (notifications) or Epic 5 (clipboard → Wayland-specific) actually needs it.

Local dev: same `Xvfb :99 -screen 0 1280x800x24 &` + `DISPLAY=:99 cargo test -p lmux-e2e`. The harness will start Xvfb on-demand if `DISPLAY` is unset.

## Harness crate layout

```
crates/lmux-e2e/
├── Cargo.toml              # [[test]] targets, dev-dependencies
├── src/lib.rs              # Harness struct, helpers, drop-on-scope cleanup
│   ├── display.rs          # Xvfb management
│   ├── bus.rs              # private D-Bus session for notification tests
│   ├── socket.rs           # control-socket client
│   └── env.rs              # tempdir for XDG_{STATE,RUNTIME,CONFIG,DATA}_HOME
└── tests/
    ├── smoke.rs            # Story 1.4 — lifecycle: start → pane-open → clean exit
    ├── splits.rs           # Epic 3
    ├── anchors.rs          # Epic 4
    ├── cli_contract.rs     # Epic 5 — the contract the CLI speaks over the socket
    ├── notifications.rs    # Epic 6
    ├── shutdown.rs         # Epic 7 — SIGTERM propagation, PDEATHSIG
    └── persistence.rs      # Epic 8
```

Key crate deps:
- `assert_cmd` — spawn the release binary with the right env
- `predicates` — stderr/exit assertions
- `nix` — send signals (SIGTERM, SIGKILL)
- `tokio` — async control-socket client
- `tempfile` — XDG isolation
- `serde_json` — control-socket frames
- `zbus` — stub notification service

One design rule: **each test gets its own tempdir for every XDG path and a fresh unix socket.** No shared state, no serial-only-please.

## Timing — when each layer lands

| Epic | Can add E2E for | Notes |
|---|---|---|
| 1.4 (first PTY pane) | Lifecycle smoke | stderr-log-only; no socket yet |
| 5 (CLI companion) | Full control-socket suite | biggest unlock; backfill splits/anchors coverage |
| 6 (bell → notify) | Notification assertions | private D-Bus stub |
| 7 (clean shutdown) | SIGTERM propagation, PDEATHSIG | no new infra |
| 8 (persistence) | Layout round-trip | inspect state file |

Pre-Epic-5 E2E (logs-only) has real value anyway: it catches segfaults, linker drift, GTK4 breakage, and race conditions at startup/teardown.

## CI implications

- GitHub Actions Ubuntu runner already has everything except `xvfb`, `dbus-daemon`, and maybe `libgtk-4-dev`/`libpango1.0-dev`. Installable with one `apt-get` line.
- Cache the `zig build` output for `lmux-libghostty` across CI runs — otherwise every PR does a full ghostty build. Cache key: git SHA of the ghostty dep + Zig version.
- Split `cargo test` into two jobs: `unit+integration` (fast) and `e2e` (slow, can block release only).

## Out of scope for v0.1

- Visual/pixel-diff regression (too noisy, low value for a terminal)
- Multi-monitor scenarios (v0.2)
- Compositor bring-front (v0.2 — spike is in `spikes/compositor-ipc/`)
- Performance SLO gating in CI (NFR1/NFR2 measurements are manual spot-checks per plan)

## Next action

Story 1.4 (first PTY pane end-to-end) is the trigger to scaffold `crates/lmux-e2e/` with one smoke test. We do **not** create the crate earlier — a harness with no tests is just dead code.
