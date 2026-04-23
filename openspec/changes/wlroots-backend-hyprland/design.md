## Context

`KwinCompositor` uses D-Bus + KWin's JS scripting service to correlate satellites (matching `WM_CLASS` and PID) and to mutate window state. Wlroots has no equivalent scripting surface. Instead, we have two complementary primitives:

- **`hyprctl` IPC** — a Unix-socket JSON protocol under `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` (dispatch) and `.socket2.sock` (event stream). Supports `dispatch movewindowpixel`, `dispatch resizewindowpixel`, `dispatch togglefloating`, `dispatch movetoworkspace`, and structured `clients -j` / `activewindow -j` queries. This is Hyprland-specific but stable and ergonomic.
- **`wlr-foreign-toplevel-management-v1`** — a Wayland protocol extension implemented by every wlroots-based compositor. Emits `toplevel` objects with `title`, `app_id`, `state`, `done`, `closed` events. No PID in the protocol, but the `app_id` matches what we already set for KWin correlation (`lmux-sat-<request_id>`).

These two split the work cleanly: `wlr-foreign-toplevel` handles correlation (the windowAdded/Removed equivalent), `hyprctl` handles mutation (the set_geometry / detach / attach equivalent).

## Goals / Non-Goals

**Goals**
- One new `HyprlandCompositor` impl of `CompositorControl` with feature parity for the docking lifecycle (initial-placement, geometry-follow, detach, reattach, close-pane policy).
- Reuse the existing `satellites.json` mapping and the existing bus kinds — the change should be invisible to code above `CompositorControl`.
- Clean crate separation so users on KDE-only distros can build without Wayland client dependencies.

**Non-goals**
- Sway parity. Out of scope; explicitly deferred to v0.4.
- GNOME / Mutter. No foreign-toplevel on Mutter; Noop-only forever.
- A cross-compositor dispatch abstraction. The trait already is the abstraction; we don't add a second one.
- Plugin-author customization of dispatch calls. Comes later with `plugin-sdk-public-bus`.

## Decisions

### D1 — One crate per backend family

Decision: new crate `crates/lmux-compositor-wlroots` houses `HyprlandCompositor` (and later `SwayCompositor`). Reasons:

- `wayland-client` + protocol codegen pull in a nontrivial build-time dependency tree that KDE-only users shouldn't pay for.
- The existing `crates/lmux-compositor` crate gets a `Backend` enum dispatcher that imports the wlroots crate behind `cfg(feature = "hyprland")`.
- A future `SwayCompositor` slots in next to `HyprlandCompositor` inside the same crate, sharing the `wlr-foreign-toplevel` client code.

Alternatives considered: one backend per crate (too much boilerplate for the foreign-toplevel client); everything in `lmux-compositor` (too heavy for KDE builds; dependency bloat).

### D2 — `hyprctl` over spawning the CLI

Decision: talk to Hyprland's dispatch socket directly as a Rust client using `tokio::net::UnixStream`. Do not spawn the `hyprctl` binary. Reasons:

- Shelling out per `set_geometry` call during pane-resize storms would dominate CPU. With an 8 ms debounce we'd still issue many calls per pane drag.
- The JSON wire format is stable and trivially serialized.
- Direct socket access lets us multiplex dispatch + event-stream on the same task (via `tokio::select!`).

The `hyprctl` binary remains useful for one-off debugging and appears in error messages as a diagnostic hint.

### D3 — Correlation via `app_id`, not PID

KWin script can read window PID via `window.pid`. The foreign-toplevel protocol does not expose PID. Decision: tighten the satellite launch to always set a unique `app_id` (`lmux-sat-<request_id>`) via the existing `WAYLAND_DISPLAY`-aware argv mutation, and correlate purely on `app_id`. The PID is still written into `satellites.json` via `/proc/<pid>/status` lookup on the toplevel's client (matching cockpit-side spawn bookkeeping), used only for stale-entry pruning.

### D4 — `detach` maps to `togglefloating`, `attach` maps to retile + move

Hyprland doesn't have a direct "dock to rect" operation; it has tiling windows and floating windows. Decision:

- `detach(window_id)` → `dispatch togglefloating address:0x<addr>` with the window currently tiled, then enable free placement.
- `attach(window_id, rect)` → if floating, `togglefloating` back to tiling; then `dispatch movewindowpixel exact <rect.x> <rect.y>, address:0x<addr>` followed by `dispatch resizewindowpixel exact <rect.w> <rect.h>, address:0x<addr>`.
- `set_geometry(window_id, rect)` on a tiled window → the same move+resize pair.

The tile path is subject to Hyprland's layout algorithm, so we also install a window rule `windowrulev2 = float, class:^(lmux-sat-<request_id>)$` at initial spawn so the satellite starts floating; `attach` then moves + resizes without fighting the tiling layout. This matches the KWin pattern (free-placement windows with explicit geometry) and avoids a "first docked, then untiled, then re-docked" dance.

### D5 — Window rule injection via `keyword windowrulev2`

Decision: the cockpit calls `hyprctl keyword windowrulev2 "float, class:^(lmux-sat-<request_id>)$"` at satellite spawn time, then relies on the rule to apply when the toplevel appears. The rule is process-local to the Hyprland session and reset on compositor restart, which is exactly the lifetime we need.

Alternatives considered: persistent config patching. Rejected — we'd have to own the user's `hyprland.conf`, which is invasive and politically fraught.

### D6 — Backend selection order

```
if env HYPRLAND_INSTANCE_SIGNATURE set and hyprctl socket reachable:
    HyprlandCompositor
elif Plasma 6 KWin reachable via D-Bus:
    KwinCompositor
else:
    NoopCompositor
```

Hyprland gets priority because a user running Hyprland has made an intentional compositor choice; if they also have KDE installed for theming reasons, we should still pick Hyprland.

### D7 — Version probe

Hyprland publishes breaking dispatcher changes sometimes between 0.x minor versions. Decision: on backend init, call `hyprctl version -j`, parse the `version` field, and compare against a `MIN_KNOWN_GOOD_VERSION` constant. If older, log a warning banner but continue — don't refuse to run. The README tracks the currently-tested range.

## Risks / Trade-offs

- *Risk: Hyprland breaks a dispatcher we rely on between minor versions.* → Mitigation: the version probe (D7) + a fallback to NoopCompositor if the initial `clients -j` query fails parse.
- *Risk: `wlr-foreign-toplevel` does not expose PID.* → Mitigation: cockpit-side PID bookkeeping from the spawn call, correlated on `app_id` (D3).
- *Risk: tiled-vs-floating state transitions during `attach` cause a flash.* → Mitigation: always spawn floating (D5), so `attach` never has to untile mid-resize.
- *Risk: the wayland-client pulls in extra runtime deps and slows CI.* → Mitigation: feature-gate (D1) and skip integration tests on boxes without `CI_HAS_HYPRLAND=1`.
- *Risk: Hyprland users run without a compositor-injected keyboard shortcut daemon, and our prefix dispatcher behaves differently.* → Out of scope here; that's the existing terminal-core spec's concern and doesn't change with this backend.

## Migration Plan

- No runtime migration. Existing KWin sessions continue to pick `KwinCompositor`. Hyprland users who were on `NoopCompositor` at v0.2 automatically get the wlroots backend on upgrade.
- Config: no new top-level keys. The backend is detected from the environment.
- `satellites.json` format is unchanged; the Hyprland path writes the same schema (request_id, window_id, pid), where `window_id` is the Hyprland address-as-hex-string.
- Feature flag default: `hyprland` is ON in the default workspace build. Packagers who want KDE-only builds can pass `--no-default-features --features kwin`.

## Open Questions

- Does Hyprland's event-stream socket give us a reliable "window geometry changed by user drag" signal that should feed back into the cockpit, or is the one-way geometry-follow enough? Leaning: one-way is enough for docked satellites since the user shouldn't be dragging them manually; revisit if user feedback disagrees.
- Do we need a `HyprlandCompositor`-specific `error.compositor_rule_rejected` variant when `windowrulev2` is rejected by the parser? Probably yes; add to `CompositorError` in tasks.md.
- Should Sway share the `wlr-foreign-toplevel` client code via a sibling module in `lmux-compositor-wlroots`, or deserve its own crate for its (slower) i3-IPC layer? Defer to the Sway change proposal.
