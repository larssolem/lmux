## ADDED Requirements

### Requirement: Hyprland docking path

On Hyprland sessions the cockpit SHALL perform docking operations (initial-placement, geometry-follow, detach, reattach) via `hyprctl` dispatches against a window matched by address, with the satellite spawned under a pre-registered `windowrulev2 = float` rule so it starts floating and stays freely positioned.

#### Scenario: Spawn injects a per-satellite float rule

- **WHEN** `HyprlandCompositor::spawn_satellite(argv, cwd)` is called with `request_id` R
- **THEN** before the child process is forked, the cockpit dispatches `keyword windowrulev2 "float, class:^(lmux-sat-<R>)$"` to the Hyprland socket

#### Scenario: Initial placement uses movewindowpixel + resizewindowpixel

- **WHEN** a new satellite toplevel correlates (its mapping entry is written)
- **THEN** the cockpit issues `set_geometry` with the pane's current rect, which dispatches exactly one `movewindowpixel exact` and one `resizewindowpixel exact` targeting the Hyprland address

#### Scenario: Detach is a togglefloating no-op when already floating

- **WHEN** `HyprlandCompositor::detach(window_id)` is called on a window that the spawn rule already made floating
- **THEN** the implementor inspects current tiling state via `clients -j` and either dispatches `togglefloating` once (if tiled) or returns `Ok(())` without dispatch (if already floating)

#### Scenario: Geometry-follow reuses the 8 ms debounce

- **WHEN** a pane owning a Hyprland-backed satellite is resized
- **THEN** the same `SatelliteGeometryBridge` debounce from the KWin path emits at most one `set_geometry` per 8 ms window, and the Hyprland implementor translates that into one `movewindowpixel`+`resizewindowpixel` dispatch pair

#### Scenario: Dispatch failure surfaces a typed error

- **WHEN** `hyprctl` replies with an error to any of the docking dispatches
- **THEN** the implementor returns `CompositorError::DispatchRejected { cmd, reason }` and appends a one-line entry to `$XDG_RUNTIME_DIR/lmux/hypr-errors.log`; the cockpit surfaces a toast naming the dispatch command

## MODIFIED Requirements

### Requirement: KWin best-effort placement

On KWin the cockpit SHALL perform an *initial* best-effort placement of a freshly-spawned satellite and MUST then transition the satellite into the full docking lifecycle (geometry-follow, detach, reattach, close-policy) once the `satellites.json` mapping entry is written. Initial placement is the first call in the KWin docking lifecycle; it is not the only one. On non-KWin backends the initial-placement step is performed by the active `CompositorControl` implementor using its own native mechanism (see `Requirement: Hyprland docking path`), but the cross-backend lifecycle — mapping-write → `set_geometry` → geometry-follow → detach/reattach → close-policy — is identical.

#### Scenario: Initial placement runs as before on KWin

- **WHEN** `KwinCompositor::spawn_satellite` is called on a KWin session
- **THEN** the cockpit returns `(request_id, pid)` immediately and schedules an async task that writes a `lmux-place-<pid>` script, loads and runs it over D-Bus, then unloads it

#### Scenario: Initial placement transitions into docking lifecycle

- **WHEN** `satellites.json` gains a new mapping entry for the satellite (regardless of backend)
- **THEN** the cockpit moves the satellite from "best-effort placed" to "docked"; subsequent layout changes trigger `set_geometry` and the `Detach`/`Reattach` CLI + sidebar actions become available for this satellite

#### Scenario: Placement is best-effort, not gating

- **WHEN** the backend's placement mechanism fails or the window cannot be correlated within a short window
- **THEN** the satellite remains running as a floating window without any error surfaced to the user; the sidebar indicates "floating_fallback" with a toast per the correlation-timeout requirement

#### Scenario: Hyprland performs initial placement via spawn rule plus movewindowpixel

- **WHEN** `HyprlandCompositor::spawn_satellite` is called on a Hyprland session
- **THEN** the pre-spawn `windowrulev2 = float` injection (see `Requirement: Hyprland docking path`) puts the window in a movable state; the first mapping-write triggers the same `set_geometry` call the KWin path does, reaching the docked state via the compositor-agnostic lifecycle
