## 1. Mapping file (`satellites.json`)

- [ ] 1.1 Define schema: `{ entries: [{ request_id, window_id, pid }] }`; freeze in `crates/lmux-compositor/src/kwin/mapping.rs`
- [ ] 1.2 Implement `SatelliteMap::{load, insert, remove, prune_dead}` with stage + rename under a `satellites.json.lock` guard
- [ ] 1.3 Unit tests: concurrent insert from two threads → both entries present; crash mid-rename → previous file intact; stale-lock reclaim after 500 ms
- [ ] 1.4 On cockpit startup, prune entries whose PID no longer exists

## 2. KWin script rewrite

- [ ] 2.1 Replace `lmux-dock.js` stub with the docking agent: install `windowAdded` / `windowRemoved` handlers that read+mutate `satellites.json`
- [ ] 2.2 Add one-shot script templates for `set_geometry`, `detach`, `attach` operations (written via `KwinCompositor::run_oneshot`)
- [ ] 2.3 Script-side probe: on `init`, verify the KWin APIs we use are present; if not, log a loud banner message the cockpit can surface
- [ ] 2.4 Add `try/catch` + `kwin-errors.log` emission on every mutation path
- [ ] 2.5 Verify against Plasma 6.x manually; document the tested version in the script header

## 3. `CompositorControl` surface

- [ ] 3.1 Implement `KwinCompositor::set_geometry(window_id, rect)`: load `SatelliteMap`, translate `window_id` (our UUID) to KWin id, issue one-shot `set_geometry` script, return typed result
- [ ] 3.2 Implement `KwinCompositor::detach(window_id)` and `attach(window_id, rect)` analogously
- [ ] 3.3 Add `error.satellite_gone` typed variant; return it on mapping miss
- [ ] 3.4 Unit tests with a fake scripting D-Bus endpoint: each call is idempotent under a same-rect repeat

## 4. Geometry-follow hook

- [ ] 4.1 Add `SatelliteGeometryBridge` on the `compositor_bridge` thread that accepts `(window_id, rect)` events
- [ ] 4.2 Debounce at 8 ms; coalesce identical-rect events
- [ ] 4.3 Hook the cockpit's layout tick to emit geometry events for any pane that owns a docked satellite
- [ ] 4.4 Integration test: programmatically resize a pane → observe a `set_geometry` D-Bus call within one frame

## 5. Detach / reattach actions

- [ ] 5.1 Wire `lmux satellite detach <pane-id-or-uuid>` and `lmux satellite reattach <pane-id-or-uuid>` into `lmux-cli` over the bus
- [ ] 5.2 Add `satellite.detach` and `satellite.reattach` bus kinds; round-trip tests
- [ ] 5.3 Sidebar context menu: `Detach` on satellite-owned pane rows; `Reattach` on satellite-owning panes whose satellite is currently floating
- [ ] 5.4 Toasts: success, `error.satellite_gone`, `error.no_satellite`

## 6. Close-pane policy

- [ ] 6.1 Add `[satellites].close_behavior` to the config schema with default `"detach"`; expose as enum `CloseBehavior::{Detach, Sigterm, Leave}`
- [ ] 6.2 Apply policy in `AppState::close_pane` for panes that own a docked satellite; each branch is its own testable path
- [ ] 6.3 `Leave` policy surfaces a toast with a "Reattach to focused pane" action
- [ ] 6.4 Integration test per policy: close the owning pane → assert the satellite state matches the policy

## 7. Error surfacing

- [ ] 7.1 Poll `$XDG_RUNTIME_DIR/lmux/kwin-errors.log` when a docking operation returns with no observable state change; surface captured errors as toasts
- [ ] 7.2 Prune the error log on cockpit shutdown
- [ ] 7.3 Cap log growth at 64 KiB; truncate oldest lines when exceeded

## 8. Observability

- [ ] 8.1 Spans: `satellite.docking_lifecycle` covering initial-placement → mapping-write → transition-to-docked
- [ ] 8.2 Spans: `satellite.detach`, `satellite.reattach`, `satellite.close_pane_policy`
- [ ] 8.3 Counter: `satellites.geometry_follow_calls` (lifetime) and `satellites.geometry_follow_debounced` (to confirm D3 is effective)
- [ ] 8.4 `lmux status` prints the geometry-follow counters when non-zero

## 9. Graceful degradation

- [ ] 9.1 If the KWin API probe (2.3) fails, `KwinCompositor` logs a warning, marks docking as unavailable, and falls through to initial-placement-only behavior
- [ ] 9.2 A sidebar banner explains "Full docking unavailable on this Plasma version" with a link to the versions we test against
- [ ] 9.3 `lmux open` on a degraded KWin still succeeds with a `floating_fallback` toast

## 10. Documentation

- [ ] 10.1 Update `share/lmux/kwin/lmux-dock.js` header with the schema it writes
- [ ] 10.2 Update `BUILD.md` with the new install path and permissions for the script
- [ ] 10.3 Add a short dogfooding note in the repo README on how to verify docking is working end-to-end
