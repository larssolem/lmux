## Context

The cockpit already launches satellites and nudges them into a half-screen placement on spawn. Full docking — geometry follow, detach, reattach, close-pane policy — needs a bidirectional channel between the Rust cockpit and KWin's JS scripting environment. This change designs that channel, lands the `satellites.json` mapping as the correlation source of truth, and wires the four lifecycle operations through `KwinCompositor`.

## Goals / Non-Goals

**Goals**
- A persistent, race-tolerant mapping of `request_id ↔ window_id ↔ pid` usable from both sides.
- Rust-side primitives `set_geometry`, `detach`, `attach` that are idempotent and typed.
- Layout-tick driven geometry follow so moving a pane is visually indistinguishable from moving its satellite.
- User-visible `Detach` / `Reattach` actions.
- A sensible default close-pane behavior and a config key for opt-in alternatives.

**Non-goals**
- wlroots/Hyprland parity; that is the next ADR and a separate change.
- Replaying pane state (scrollback, cwd) into satellites. Satellites remain opaque GUI apps.
- Recovering docking for satellites that existed before this change shipped. Any satellite in `floating_fallback` remains floating until the user reattaches.

## Decisions

### D1 — Bidirectional channel = filesystem mapping + D-Bus scripting calls

Decision: use `$XDG_RUNTIME_DIR/lmux/satellites.json` as the shared source of truth (KWin script writes, Rust reads), and use one-shot KWin scripts loaded+run+unloaded per operation for Rust → KWin calls (the same pattern `spawn_satellite` already uses). Keeps the resident script read-only; all mutations flow via one-shot scripts that the KWin scripting service accepts and isolates.

Alternatives considered:
- *Resident script with `callDBus` back to the cockpit.* KWin's `callDBus` is asynchronous and does not let the cockpit synchronously wait for a response. Would require an async-ack pattern and more state in the script.
- *X11-style `_NET_WM_*` hints on the satellite window.* Doesn't exist on Wayland toplevels generically.
- *Port all logic into a Rust-side KWin client library.* No such stable API for KWin's scripting surface.

### D2 — `satellites.json` is whole-file atomic stage + rename

Concurrent `windowAdded` callbacks in KWin script can interleave. Decision: the script reads the current file, splices its own update, and stage+rename. A file-local `satellites.json.lock` (mode 0600, same directory, `O_CREAT | O_EXCL`) is held across the read-modify-write; held for at most 50 ms; `stale-lock` reclaim after 500 ms. The file is small (one line per active satellite) so whole-file writes are acceptable.

### D3 — Geometry follow is debounced on the Rust side, not the script side

Pane resize animations can fire many rect updates per second. Decision: the cockpit debounces geometry-follow events at 8 ms (half a frame) and coalesces identical-rect events; only the final rect of a burst hits `set_geometry`. Keeps the D-Bus traffic bounded and the script behavior dumb/idempotent.

### D4 — `CompositorWindowId` is the request_id, not the KWin internal id

Callers of `KwinCompositor::{set_geometry, detach, attach}` should not know about KWin's internal ids. Decision: `CompositorWindowId` remains the opaque type alias over `Uuid` (matching `request_id`); the KWin impl translates via `satellites.json` at call time. If translation fails, return `error.satellite_gone`.

### D5 — Close-pane policy defaults to `detach`

Picking `sigterm` would lose IDE state (unsaved files, breakpoints). Picking `leave` orphans the window visually. `detach` preserves everything the user cares about and is the least surprising default. Decision: `[satellites].close_behavior = "detach"` by default, with the two alternatives `sigterm` and `leave` documented inline.

### D6 — KWin script error handling: swallow and log

A failed `frameGeometry` assignment in the KWin script cannot propagate back to the cockpit synchronously. Decision: the script wraps each mutation in `try/catch`, writes a one-line error to `$XDG_RUNTIME_DIR/lmux/kwin-errors.log` (not rotated — the file is pruned on cockpit shutdown), and continues. The cockpit polls this file when a toast-worthy operation returns with no observable state change and surfaces the captured error.

## Risks / Trade-offs

- *Risk: KWin scripting API changes between Plasma minor versions break the mutation surface.* → Mitigation: version-probe on script load (check for the methods we use); degrade to "best-effort placement only" with a banner if probes fail. Pin a known-good Plasma version in the release notes.
- *Risk: `satellites.json` file grows unbounded across crashes.* → Mitigation: the cockpit prunes stale entries on startup (D-Bus-check each `window_id` exists; remove missing) and on every `windowRemoved` event.
- *Risk: geometry-follow storms when a user drag-resizes the whole cockpit window.* → Mitigation: 8 ms debounce (D3) + same-rect idempotence in the script.
- *Risk: detach + close-pane race — user detaches while a pane-close is in flight.* → Mitigation: close-pane policy is evaluated exactly once, at the moment the pane's layout-removal commits; any concurrent detach wins if it happened first, otherwise the policy runs. Both paths end in the satellite being in a consistent state.

## Migration Plan

- Ship the new `lmux-dock.js` alongside the existing one under a versioned path; `ensure_script_loaded` prefers the new version and unloads the old one if both are registered.
- Config migration: if `[satellites]` exists without `close_behavior`, infer `detach` and write a comment on next config reload.
- Runtime data: no migration. Previously-floating satellites remain floating.

## Open Questions

- Should a detached satellite retain a weak `last_pane` pointer so Reattach defaults to it instead of "currently-focused"? Probably yes — this is less surprising. Resolve in tasks.md decomposition.
- Do we need a KWin script unload + reload on config-driven changes to the docking policy? Leaning no (policy lives on the cockpit side), but confirm once the minimal script is running.
- Is a per-satellite `close_behavior` override via the sidebar worth shipping now, or should we defer? Defer; dogfooding will tell.
