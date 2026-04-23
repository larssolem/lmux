## ADDED Requirements

### Requirement: Hyprland backend via hyprctl and wlr-foreign-toplevel

The cockpit SHALL provide a `HyprlandCompositor` implementor of `CompositorControl` that correlates satellites via `wlr-foreign-toplevel-management-v1` and mutates window state via `hyprctl`'s Unix-socket dispatch protocol. The implementor MUST be feature-gated (`hyprland`) so it can be omitted from KDE-only builds.

#### Scenario: Hyprland backend chosen when the session is Hyprland

- **WHEN** the cockpit starts with `HYPRLAND_INSTANCE_SIGNATURE` set and `$XDG_RUNTIME_DIR/hypr/$HIS/.socket.sock` reachable
- **THEN** it instantiates `HyprlandCompositor` and the choice is logged at INFO with reason `"hyprland-detected"`

#### Scenario: Dispatch to hyprctl is a direct socket call, not a shell-out

- **WHEN** `HyprlandCompositor::set_geometry(window_id, rect)` is called
- **THEN** the implementor sends the `dispatch movewindowpixel exact <x> <y>, address:0x<addr>` command directly over the Unix socket without invoking the `hyprctl` binary
- **AND** it sends the matching `resizewindowpixel exact <w> <h>, address:0x<addr>` dispatch in the same call

#### Scenario: Correlation via app_id, not PID

- **WHEN** a `zwlr_foreign_toplevel_handle_v1` emits a `Done` event whose `app_id` starts with `lmux-sat-`
- **THEN** the cockpit extracts the `request_id` suffix, resolves the Hyprland window address via `hyprctl clients -j`, and writes `{ request_id, window_id, pid }` to `satellites.json`

#### Scenario: Version probe warns but does not refuse to run

- **WHEN** `HyprlandCompositor::health()` observes a Hyprland version older than `MIN_KNOWN_GOOD_VERSION`
- **THEN** it logs a sidebar-banner warning identifying the detected version and the tested range, but returns `Online` and continues servicing dispatch calls

#### Scenario: Feature-flagged out for KDE-only builds

- **WHEN** the workspace is built with `--no-default-features --features kwin`
- **THEN** `crates/lmux-compositor-wlroots` is not compiled, `HyprlandCompositor` is not exported, and the backend picker never selects it

## MODIFIED Requirements

### Requirement: Compositor abstraction trait

The cockpit SHALL expose a `CompositorControl` trait whose method surface is compositor-agnostic (no raw D-Bus types, no KWin enums, no `hyprctl` strings in the trait signatures) and SHALL provide at minimum three implementors: `NoopCompositor`, `KwinCompositor`, and `HyprlandCompositor` (the latter behind the `hyprland` feature flag).

#### Scenario: Trait compiles against all three implementors

- **WHEN** `NoopCompositor`, `KwinCompositor`, and `HyprlandCompositor` are built
- **THEN** each implements `CompositorControl` without leaking backend-specific types into the trait surface; callers depend only on `CompositorControl` and the supporting types `CompositorError`, `CompositorHealth`, `SpawnRequest`, `SpawnHandle`, `CompositorWindowId`, and `Rect`

#### Scenario: Cockpit chooses the implementor at runtime

- **WHEN** the cockpit starts
- **THEN** it applies the backend-selection order: Hyprland (if `HYPRLAND_INSTANCE_SIGNATURE` set and socket reachable), else KWin (if Plasma 6 scripting service reachable), else `NoopCompositor`
- **AND** the chosen backend and the reason for the choice are logged at INFO
