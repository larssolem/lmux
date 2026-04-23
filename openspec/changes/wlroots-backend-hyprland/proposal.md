## Why

v0.2 ships a single `CompositorControl` implementor (`KwinCompositor`) plus the `NoopCompositor` fallback. That locks docking — geometry-follow, detach, reattach — to KDE Plasma users. The lmux target audience (enthusiast Linux devs who compose their own tools) is heavily wlroots-leaning; shipping "satellite panes work on KDE only" undersells the product.

ADR-0006 picks **Hyprland first** as the wlroots backend because `hyprctl`'s Unix-socket JSON IPC is more ergonomic than i3 IPC (Sway's transport), and `wlr-foreign-toplevel-management-v1` gives us a stable cross-wlroots way to observe toplevel lifecycle without relying on compositor-specific extensions. Sway is an explicit v0.4 goal; this change does not deliver Sway parity.

## What Changes

- A new `HyprlandCompositor` implementor of `CompositorControl` in a new `crates/lmux-compositor-wlroots` crate, behind a `hyprland` feature flag in the top-level workspace.
- `hyprctl` dispatch calls over `$HYPRLAND_INSTANCE_SIGNATURE`'s Unix socket for `set_geometry`, `detach` (float), `attach` (tile/move), and window-rule injection.
- A `wlr-foreign-toplevel-management-v1` client (via `wayland-client`) for the windowAdded/Removed correlation that on KWin is handled by JS script — used to populate the existing `satellites.json` mapping.
- Backend selection at cockpit startup: if `HYPRLAND_INSTANCE_SIGNATURE` is set, prefer `HyprlandCompositor`; else if Plasma 6 KWin is detected, prefer `KwinCompositor`; else `NoopCompositor`. The existing order stays for KWin; Hyprland wins when both somehow appear (unlikely but well-defined).
- A `lmux compositor info` subcommand that prints the selected backend and its version probe.
- Documentation: README section on the backend matrix; a `BUILD.md` note on the `hyprland` feature flag; the `HyprlandCompositor` header pins a known-good Hyprland version.

Sway is explicitly out of scope for this change — it gets its own proposal if/when the Hyprland impl factors cleanly enough that i3 IPC is <1 day of additional work.

## Capabilities

### New Capabilities

(none — this extends `compositor-control` and `satellites`)

### Modified Capabilities

- `compositor-control`: adds `Requirement: Hyprland backend via hyprctl and wlr-foreign-toplevel` and modifies the existing `Requirement: Compositor abstraction trait` to list `HyprlandCompositor` as a third implementor. Adds a scenario to the runtime-selection requirement covering Hyprland.
- `satellites`: modifies `Requirement: KWin best-effort placement` into a compositor-agnostic `Requirement: Docking lifecycle` that delegates the actual mechanism to whatever `CompositorControl` impl is live; adds a `Requirement: Hyprland docking path` that specifies how correlation and geometry-follow work on Hyprland.

## Impact

- Code: new `crates/lmux-compositor-wlroots` crate; modifications to `crates/lmux-compositor/src/lib.rs` (backend-picker) and the `CompositorControl` glue in `crates/lmux`. Feature-gated so KDE-only builds can skip the `wayland-client` + `hyprland` dependencies.
- ADR: references ADR-0006; adds a short follow-up ADR (drafted as part of task #1) if `hyprctl` surface choices need to be pinned beyond what ADR-0006 covered.
- Test strategy: integration tests need a live Hyprland session, so they go under `crates/lmux-compositor-wlroots/tests/` gated behind `CI_HAS_HYPRLAND=1`; unit tests use a fake `hyprctl` transport.
- No breaking changes for KWin users; Plasma 6 path is unchanged.
