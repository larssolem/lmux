## Context

Display awareness spans three capability boundaries: the compositor knows the hardware layout, sessions persist user intent about "this pane lives on my right monitor", satellites need to know where to dock. v0.2 side-stepped all three: GTK placed the cockpit somewhere, KWin best-effort placed satellites wherever, and session files stored no display info. That works for the "laptop-only" user and breaks otherwise.

The v0.3 model makes displays a typed first-class entity that every layer can reason about.

## Goals / Non-Goals

**Goals**
- A stable `DisplayId` that survives unplug/replug of the same physical monitor.
- A `Display` value carrying rect, scale, primary flag, connector name, and the stable id.
- Compositor backend implementations that publish display state + hotplug events.
- Session persistence and restore that remembers per-pane / per-satellite display placement.
- A hotplug migration policy that minimizes surprise when a display disappears.
- CLI + sidebar surface for manual display management.

**Non-goals**
- Display-configuration tooling (setting resolution, scale, orientation). That's the compositor's job.
- Per-display scrollback caches. Current cache is per-pane already.
- Custom pane-tree-per-display layouts (workspaces per display). ADR notwithstanding, this bleeds into workspace-management territory out of scope here.
- Headless / synthetic displays for CI. A display enumeration with zero entries is valid and the code must handle it without crashing.

## Decisions

### D1 — DisplayId: `edid-hash:connector-name`

Decision: the stable id is `<sha256(EDID)-first-8-hex>:<connector>` (e.g. `ab12cd34:HDMI-A-1`). The EDID prefix identifies the specific physical monitor (survives connector swap if the user's monitor has a stable EDID); the connector name disambiguates when the same model is plugged twice; falling back to `unknown:<connector>` if EDID read fails (live VMs, virtual displays).

Alternatives considered: raw connector name (breaks when the user replugs to a different port); index (breaks on any reorder); KWin/Hyprland internal ids (not portable across backends).

### D2 — Enumeration per backend

- **KWin**: call `Workspace.screens` over D-Bus scripting; for each screen, read `model`, `manufacturer`, `serial`, `geometry`, `scale`, and `primary` via the scripting surface. EDID isn't directly exposed but manufacturer+model+serial is a stable-enough proxy.
- **Hyprland**: `hyprctl monitors -j` returns EDID-derived identifiers; parse into the same `Display` struct.
- **Noop**: a single synthetic display at `0,0 1920x1080 scale 1.0 primary=true`, so code paths stay honest in CI.

### D3 — Hotplug event kind on the bus

`compositor.displays { displays: [Display], changed: DisplayChange }` where `DisplayChange ∈ { Added(id), Removed(id), Reconfigured(id), InitialEnumeration }`. Published on cockpit start (initial enumeration) and on every backend-reported change. Clients subscribe via the existing bus subscription mechanism.

### D4 — Session schema: additive, opt-in

New fields on `PaneRecord` and `SatelliteRecord`: `display_id: Option<String>` and `rect_on_display: Option<Rect>` (rect in display-local coords, 0..w × 0..h). Absent = v0.2 semantics (cockpit places the pane wherever). Rationale: migration safety; zero-config upgrade.

### D5 — Hotplug migration policy

Three explicit rules, each testable in isolation:

1. **Display removed**: every pane/satellite with a matching `display_id` migrates to the current primary. `rect_on_display` is clamped to the primary's rect. A toast summarises how many panes moved.
2. **Display added**: no pane is proactively moved. The user is in control.
3. **Display reconfigured** (resolution/scale change): panes stay on the display; `rect_on_display` is clamped to the new rect. Satellites with docked geometry get `set_geometry` called with the new rect to avoid being partially off-screen.

The policy is documented in the sidebar on hotplug (one-line toast) so the user knows what happened.

### D6 — `lmux open --display`

The flag accepts either a `display_id` (exact match) or a display short-name from `lmux display list` (the first 8 hex + connector). If the flag resolves to a display not currently present, the cockpit emits `error.display_not_found` and refuses the spawn — no "best-effort place anyway". Rationale: this flag expresses user intent; silently ignoring it causes confusion.

### D7 — Default display for unflagged `lmux open`

Current behaviour: cockpit-local display. Kept as-is. Users who want display-specific placement add the flag. This preserves single-display ergonomics.

### D8 — Fractional scale

`scale` is stored as `f32`. Pane layouts ignore it (GTK handles UI scaling already). Satellite geometry ignores it on KWin (KWin applies scale to frame geometry). Satellite geometry on Hyprland is passed in device pixels; the `rect_on_display` stays logical, translated at dispatch time. Tested cases: 1.0, 1.25, 1.5, 2.0.

### D9 — Initial enumeration has a timeout

If the compositor doesn't respond to the enumeration call within 500 ms at cockpit start, the cockpit proceeds with a single synthetic display (same as Noop) and logs a warning. Preserves startup latency budgets; real enumeration replaces the synthetic on the next hotplug event.

## Risks / Trade-offs

- *Risk: EDID reads are slow or blocking.* → Mitigation: enumeration is async; the 500 ms timeout above gates startup.
- *Risk: Users with many displays (6+ in some setups) overwhelm the per-pane UI.* → Mitigation: sidebar "Move to display..." is a scrollable submenu; the display picker groups primary + others.
- *Risk: The EDID-hash prefix collides for two different monitors.* → Mitigation: 32-bit prefix has 4B cardinality; in practice users own <10 monitors, so the collision probability is negligible. Collisions fall back to full sha256 hex after the first collision is detected.
- *Risk: Migration policy surprises a user who expected "sticky until I move it".* → Mitigation: the one-line toast + a config `[display].on_removed = "migrate_to_primary" | "hide"` for power users.

## Migration Plan

- Session files without `display_id` continue to work; first-save after upgrade writes the current display id for each pane.
- No config migration required unless the user wants to override the hotplug policy.
- CLI scripts that used `lmux open` keep working; `--display` is strictly additive.

## Open Questions

- Should sessions remember the full display layout (connector positions) or just per-pane `display_id`? Leaning latter — the compositor owns layout, we just remember intent.
- How should the sidebar display the display id for humans? A short-name (`HDMI-A-1`) is friendlier than the hex hash; store both, surface the connector name.
- Should `lmux open --display` queue the spawn if the display is expected soon (e.g., user just plugged it)? No, keep it synchronous; the user can retry.
