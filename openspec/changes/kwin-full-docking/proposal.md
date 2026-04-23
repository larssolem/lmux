## Why

Epic 9 shipped the foundation for KWin-docked satellites: `lmux open`, sandboxed spawn, `LMUX_SATELLITE_ID` env tagging, a `.desktop` launcher, best-effort initial placement, and per-anchor visibility toggling. What remains is the docking *contract* the PRD promises:

- **Geometry follow** (FR32): when a pane moves or resizes, its docked satellite follows within one frame.
- **Explicit detach** (FR33): the user can release a satellite to floating and the pane's "satellite-owned" indicator clears.
- **Explicit reattach** (FR34): a previously-detached satellite can be re-docked to the current pane rect.
- **Close-pane behavior** (FR35): when the owning pane closes, the satellite obeys the configured policy (SIGTERM it, detach to floating, or leave running).

All four require bidirectional communication between the Rust cockpit and the KWin script. The current lmux-dock.js is a stub that only logs events; it needs to write a persistent mapping `request_id → kwin_window_id → pid` and expose KWin-side primitives for set-geometry / detach / reattach that the cockpit can drive via D-Bus.

Without this change, the cockpit cannot honor the "JetBrains docked to pane 4" user story from Journey 1.

## What Changes

- **`lmux-dock.js` promoted from logger → docking agent.** On `windowAdded`, match `WM_CLASS`/`caption`/`pid`, and write the triple into `$XDG_RUNTIME_DIR/lmux/satellites.json` atomically. On `windowRemoved`, delete the entry.
- **KWin-side RPC surface.** The script exposes three callable entry points (via one-shot scripts or a resident script with `callDBus`): `lmux.setGeometry(window_id, rect)`, `lmux.detach(window_id)`, `lmux.attach(window_id, rect)`. Implementations flip `w.frameGeometry`, `w.noBorder`/`w.fullScreen`, and placement flags.
- **`KwinCompositor::{set_geometry, detach, attach}` live.** The Rust side reads `satellites.json` to translate a `CompositorWindowId` to a live KWin window, issues the matching script call via D-Bus, and returns typed errors on failure.
- **Geometry-follow loop.** When the cockpit's layout tick changes a pane rect and the pane owns a satellite, the cockpit emits `satellite.geometry { window_id, rect }` on its internal event channel, which translates to a `set_geometry` D-Bus call. Idempotent: repeated same-rect calls are no-ops in the script.
- **Close-pane policy.** New `[satellites].close_behavior = "detach" | "sigterm" | "leave"` config key (default `detach`). The cockpit applies the policy when a pane that owns a satellite is closed.
- **Sidebar actions.** `Detach` and `Reattach` actions on satellite-owned pane rows; surface the result as toasts.

## Capabilities

### New Capabilities

(none — this extends the existing `satellites` capability)

### Modified Capabilities

- `satellites`: adds `Requirement: Geometry follow`, `Requirement: Explicit detach`, `Requirement: Explicit reattach`, and `Requirement: Close-pane policy for owning satellites`; modifies `Requirement: KWin best-effort placement` to make initial placement the first call in a two-phase docking lifecycle.

## Impact

- Code: `share/lmux/kwin/lmux-dock.js` (major rewrite), `crates/lmux-compositor/src/kwin.rs` (set_geometry / detach / attach implementations), `crates/lmux-compositor/src/bridge.rs` (new event: geometry-follow dispatch), `crates/lmux/src/state.rs` (pane-resize → satellite.geometry hook, close-pane policy application), `crates/lmux-config` (new `close_behavior` field).
- Runtime artifact: `$XDG_RUNTIME_DIR/lmux/satellites.json`. Atomic stage+rename; per-line file lock for concurrent writes from multiple script callbacks.
- Depends on a KWin 6.x `callDBus` / scripting surface that we have probed but not exercised end-to-end. Design notes will cover the fallback (one-shot scripts per call) if the resident-script path proves flaky.
