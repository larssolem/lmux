## 1. Crate scaffolding

- [ ] 1.1 Create `crates/lmux-compositor-wlroots/` with `Cargo.toml` pulling in `tokio`, `serde`, `serde_json`, `wayland-client`, `wayland-protocols-wlr`, and `tracing`
- [ ] 1.2 Add `hyprland` feature to top-level workspace `Cargo.toml`; default-enabled
- [ ] 1.3 Re-export `HyprlandCompositor` from `crates/lmux-compositor/src/lib.rs` behind `#[cfg(feature = "hyprland")]`
- [ ] 1.4 Backend picker in `crates/lmux-compositor/src/lib.rs`: detect Hyprland first (env `HYPRLAND_INSTANCE_SIGNATURE` + socket reachable), then KWin, then Noop

## 2. hyprctl socket client

- [ ] 2.1 `crates/lmux-compositor-wlroots/src/hyprctl/socket.rs`: async client over `$XDG_RUNTIME_DIR/hypr/$HIS/.socket.sock`
- [ ] 2.2 `dispatch(cmd: &str) -> Result<String>`: single-line command-response protocol
- [ ] 2.3 `query_json<T: DeserializeOwned>(cmd: &str) -> Result<T>`: for `clients -j`, `activewindow -j`, `version -j`
- [ ] 2.4 Error taxonomy: `HyprctlError::{SocketUnreachable, Timeout(Duration), DispatchRejected(String), ParseError(serde_json::Error)}`
- [ ] 2.5 Unit tests against a fake Unix socket that emits canned payloads

## 3. wlr-foreign-toplevel client

- [ ] 3.1 `crates/lmux-compositor-wlroots/src/foreign_toplevel.rs`: bind `zwlr_foreign_toplevel_manager_v1`
- [ ] 3.2 Emit `ToplevelEvent::{Added { handle, app_id }, TitleChanged, Closed}` onto an mpsc channel
- [ ] 3.3 Shared state task merges foreign-toplevel events + hyprctl event-stream (for optional geometry echo) via `tokio::select!`
- [ ] 3.4 Unit tests with a fake Wayland display that emits the protocol events in sequence

## 4. Correlation → satellites.json

- [ ] 4.1 On `ToplevelEvent::Added` whose `app_id` starts with `lmux-sat-`, extract `request_id` and write `{ request_id, window_id: <addr hex>, pid }` via the existing `SatelliteMap` (from `kwin-full-docking`)
- [ ] 4.2 Resolve the Hyprland address by matching `app_id` against the `hyprctl clients -j` output taken at the time of the Added event
- [ ] 4.3 Fallback: if the address isn't in `clients -j` yet, retry up to 200 ms before giving up
- [ ] 4.4 On `Closed`, prune the mapping entry
- [ ] 4.5 Integration test: spawn a dummy `lmux-sat-<uuid>` app under Hyprland → assert a mapping entry appears within 500 ms

## 5. CompositorControl impl

- [ ] 5.1 `HyprlandCompositor::ensure_script_loaded` returns `Ok(())` — no script to install; health-probe instead
- [ ] 5.2 `health()` probes `hyprctl version -j` and compares against `MIN_KNOWN_GOOD_VERSION`; returns `Online` / `Offline { reason }`
- [ ] 5.3 `spawn_satellite(argv, cwd)`: shell out the child with `WAYLAND_DISPLAY` unchanged, `app_id` env set; before spawn, inject `hyprctl keyword windowrulev2 "float, class:^(lmux-sat-<request_id>)$"`
- [ ] 5.4 `set_geometry(window_id, rect)`: dispatch `movewindowpixel exact <x> <y>, address:0x<addr>` + `resizewindowpixel exact <w> <h>, address:0x<addr>`; short-circuit on same-rect idempotence
- [ ] 5.5 `detach(window_id)`: dispatch `togglefloating address:0x<addr>` if currently tiled; no-op otherwise
- [ ] 5.6 `attach(window_id, rect)`: ensure floating (as D5 in design.md), then `set_geometry`
- [ ] 5.7 `error.satellite_gone` mapping on `DispatchRejected`
- [ ] 5.8 Unit tests with a fake socket: each operation round-trips to a single expected dispatch string

## 6. Backend selection plumbing

- [ ] 6.1 `detect_backend()` in `lmux-compositor` returns an enum `{Hyprland, Kwin, Noop}` with the reason for the choice
- [ ] 6.2 Cockpit startup logs the chosen backend + reason at INFO
- [ ] 6.3 `lmux compositor info` CLI subcommand prints the backend, version probe, and health
- [ ] 6.4 `lmux status` gains a `compositor.backend` field

## 7. Docking lifecycle parity

- [ ] 7.1 Geometry-follow debounce wiring (8 ms, as in `kwin-full-docking`) reused by the Hyprland path — no Hyprland-specific timing
- [ ] 7.2 Close-pane policy (`sigterm` / `detach` / `leave`) applies uniformly; `detach` path on Hyprland invokes the impl's `detach`
- [ ] 7.3 Integration test: resize a pane under Hyprland → observe a single `movewindowpixel`+`resizewindowpixel` pair within one frame
- [ ] 7.4 Integration test: `lmux satellite detach` on a Hyprland-backed satellite → `togglefloating` dispatched once

## 8. Observability

- [ ] 8.1 Spans: `satellite.hypr.dispatch { cmd }`, `satellite.hypr.toplevel_event { kind }`
- [ ] 8.2 Counter: `satellites.hypr.dispatch_calls` (lifetime) and `satellites.hypr.dispatch_failures`
- [ ] 8.3 Error-log file: `$XDG_RUNTIME_DIR/lmux/hypr-errors.log`, same shape and size cap (64 KiB) as `kwin-errors.log`
- [ ] 8.4 `lmux status` prints Hyprland counters when backend is Hyprland

## 9. Version probe and graceful degradation

- [ ] 9.1 `MIN_KNOWN_GOOD_VERSION` constant + README entry listing tested Hyprland versions
- [ ] 9.2 Below-minimum version logs a sidebar-banner warning but continues
- [ ] 9.3 If the initial `clients -j` parse fails, log a loud warning and fall through to `NoopCompositor`

## 10. Documentation

- [ ] 10.1 README: backend support matrix (KWin 6.x ✓, Hyprland 0.x ✓, Sway planned v0.4, GNOME out of scope)
- [ ] 10.2 BUILD.md: feature flags (`--features hyprland,kwin` default; `--no-default-features --features kwin` for KDE-only)
- [ ] 10.3 Update ADR-0006 status: mark v0.3 "shipping" once this change lands
- [ ] 10.4 Dogfooding note: how to verify Hyprland docking end-to-end (spawn a satellite, resize the pane, observe the window follows)
