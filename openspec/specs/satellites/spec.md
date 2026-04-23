# satellites

## Purpose

Bring GUI applications (JetBrains IDE, browser, Figma) into the cockpit as "satellites" bound to a pane. Two docking paths coexist: (1) on KWin, spawn a process with a unique `WM_CLASS`, correlate via bus events, and nudge it into place with a one-shot KWin script; (2) everywhere, a nested Wayland host (smithay) renders satellite toplevels into `SatelliteWidget`s inside the cockpit window. Sandboxed by default via bubblewrap with a safe escape hatch. A `.desktop`-aware GUI launcher exposes the feature to the user.

## Requirements

### Requirement: `lmux open` launches a docked satellite

The user SHALL spawn a satellite targeting the focused pane via `lmux open <app> [args...]`; the CLI MUST return immediately after a handshake without blocking on the docking result.

#### Scenario: CLI handshake returns a request id

- **WHEN** the user runs `lmux open kate` with a running cockpit and a focused pane
- **THEN** the CLI sends `satellite.open { argv: ["kate"], target_pane: <id> }` on the bus, receives a `request_id` in the ack, and exits `0` immediately

#### Scenario: Missing cockpit yields a clear error

- **WHEN** the user runs `lmux open kate` with no cockpit running
- **THEN** the CLI exits non-zero with a diagnostic naming the absent cockpit and suggesting `lmux` be started first

### Requirement: Satellites tagged by env id and sandboxed by default

The cockpit SHALL stamp `LMUX_SATELLITE_ID=<uuid>` on every satellite child's environment, attempt to spawn under a bubblewrap profile when available, and fall back to direct spawn with a warning toast when bubblewrap is missing.

#### Scenario: Environment carries the satellite id

- **WHEN** the cockpit spawns a satellite child
- **THEN** the child's environment includes `LMUX_SATELLITE_ID=<uuid>` matching the `request_id`

#### Scenario: Bubblewrap profile binds sensitive paths read-only-none

- **WHEN** `bwrap` is present on `PATH`
- **THEN** the cockpit prefixes argv with `bwrap` and the default profile (`~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/lmux/config.toml`, `$XDG_RUNTIME_DIR/lmux/` bound such that satellites cannot read them)

#### Scenario: `--no-sandbox` bypasses bwrap

- **WHEN** the user runs `lmux open --no-sandbox <app>`
- **THEN** the cockpit spawns the app directly without `bwrap`, for that invocation only

#### Scenario: Missing bwrap warns but does not fail

- **WHEN** `bwrap` is not on `PATH` and the user runs `lmux open <app>`
- **THEN** the cockpit spawns the app directly, shows a warning toast, and never errors out solely because of missing bwrap

### Requirement: Nested Wayland host for in-process satellites

The cockpit SHALL run a smithay-based Wayland compositor on a dedicated OS thread, binding `wl_compositor`, `wl_shm`, `wl_seat`, and `xdg_wm_base` on a socket under `$XDG_RUNTIME_DIR/lmux-<pid>`, and SHALL expose host events (`Ready`, `ToplevelCreated`, `ToplevelClosed`, `FrameReady`, `Stopped`) to the GTK main loop.

#### Scenario: Nested socket binds on startup

- **WHEN** the cockpit launches and nested-compositor support is available
- **THEN** the host thread starts, binds a Wayland socket under `$XDG_RUNTIME_DIR/lmux-<pid>`, and emits `HostEvent::Ready`; the cockpit records the socket name in `wayland_display_name`

#### Scenario: Host failure does not block cockpit boot

- **WHEN** the nested host fails to start for any reason
- **THEN** the cockpit logs a warning and continues running as a pure-terminal multiplexer; `wayland_display_name` remains `None`

#### Scenario: Clean shutdown of the host on drop

- **WHEN** the `HostHandle` is dropped or `HostCommand::Shutdown` is sent
- **THEN** the host thread unbinds the socket, emits `HostEvent::Stopped`, and joins within the cockpit's shutdown budget

### Requirement: Satellite panes rendered in-process

The cockpit SHALL represent each nested-host toplevel as a `SatelliteWidget` housed in a `Pane::Satellite` variant; the widget MUST paint `FrameReady` buffers as GDK textures and forward keyboard, pointer, and scroll events back as `HostCommand`s.

#### Scenario: New toplevel splits into a pane

- **WHEN** the nested host emits `ToplevelCreated`
- **THEN** the cockpit allocates a fresh `PaneId`, builds a `SatelliteWidget`, splices the pane into the focused workspace (default split direction, 0.5 ratio), and maintains `surface_to_pane`, `pane_uuids`, and `pane_workspace` entries for it

#### Scenario: Toplevel closed removes the pane

- **WHEN** the nested host emits `ToplevelClosed`
- **THEN** the satellite pane is marked closed and removed from the layout unless it is the last pane; sibling panes collapse into the vacated slot

#### Scenario: Input events forward back to the client

- **WHEN** the user presses a key, moves the pointer, scrolls, or clicks inside a `SatelliteWidget`
- **THEN** the event is translated to the matching `HostCommand::SendKey`/`SendPointer`/`SendScroll` and delivered to the nested client with XKB keycodes offset by `+8`

### Requirement: Launcher UI spawns satellites

The cockpit SHALL provide a `.desktop`-driven spotlight launcher, bound by default to `prefix + l` (and reachable from a sidebar header button), that scans `$XDG_DATA_HOME/applications` and `$XDG_DATA_DIRS`, substring-filters on Name and Comment, and spawns the chosen entry via the same spawn path as `lmux open`.

#### Scenario: Launcher opens a modal window

- **WHEN** the user presses `prefix + l`
- **THEN** a modal window appears centered on the cockpit with a filter input focused; typing filters the entry list on Name + Comment

#### Scenario: Spawn on Enter

- **WHEN** the user presses `Enter` on a launcher entry
- **THEN** the Exec line is parsed (stripping `%u`, `%f`, `%F`, `%i`, `%c`, `%k`), and the resulting argv is spawned via the tagged spawn entry point

#### Scenario: Modal window, not popover

- **WHEN** the launcher is opened inside a Wayland KWin session
- **THEN** it is implemented as a `gtk::Window`, not a `Popover`, to avoid KWin-Wayland popover placement issues

### Requirement: Wayland-aware env override for nested satellites

When a nested Wayland host is active the cockpit SHALL force `WAYLAND_DISPLAY=<nested socket>`, `GDK_BACKEND=wayland`, and `QT_QPA_PLATFORM=wayland` on the spawned child, and SHALL strip `DISPLAY` so a toolkit cannot fall back to X11.

#### Scenario: Nested env forces Wayland

- **WHEN** the cockpit spawns a satellite and `wayland_display_name` is `Some(...)`
- **THEN** the child's environment contains the three forced variables, `DISPLAY` is absent, and `LMUX_SATELLITE_ID` is still set

#### Scenario: No host, parent env passes through

- **WHEN** the cockpit spawns a satellite and no nested host is available
- **THEN** the child inherits the parent environment unchanged except for the `LMUX_SATELLITE_ID` addition

### Requirement: KWin best-effort placement

On KWin the cockpit SHALL additionally schedule a one-shot KWin script that finds the window whose PID matches the freshly-spawned satellite and snaps it into the right half of its screen's placement area.

#### Scenario: One-shot placement script runs

- **WHEN** `KwinCompositor::spawn_satellite` is called on a KWin session
- **THEN** the cockpit returns `(request_id, pid)` immediately and schedules an async task that writes a `lmux-place-<pid>` script, loads and runs it over D-Bus, then unloads it

#### Scenario: Placement is best-effort, not gating

- **WHEN** the KWin placement script fails or the window cannot be found
- **THEN** the satellite remains running as a floating window without any error surfaced to the user

### Requirement: Per-anchor satellite visibility

The cockpit SHALL bind each satellite to the anchor that was active at spawn time; switching the active anchor MUST minimize satellites bound to the outgoing anchor and restore satellites bound to the incoming anchor.

#### Scenario: Switching anchor away minimizes satellites

- **WHEN** the active anchor transitions from `A` to `B`
- **THEN** the cockpit broadcasts `SetSatelliteVisible { pid, visible: false }` for satellites owned by `A` and `SetSatelliteVisible { pid, visible: true }` for satellites owned by `B` via the compositor bridge

#### Scenario: KWin toggles minimization on command

- **WHEN** `KwinCompositor::set_window_visible_by_pid(pid, visible)` is invoked
- **THEN** a one-shot KWin script toggles `w.minimized` for the matching window; failure is logged but does not propagate

### Requirement: Correlation timeout and floating fallback

When KWin script correlation of a spawned satellite does not complete within 500 ms the cockpit SHALL mark the satellite as `floating_fallback` and MUST retry correlation on subsequent `configure` events for up to 2 seconds total before giving up.

#### Scenario: Correlation timeout surfaces a toast

- **WHEN** a satellite does not produce a `satellite.map` within 500 ms of `lmux open`
- **THEN** the cockpit emits `satellite.status { state: "floating_fallback", reason: "correlation_timeout" }` on the bus and the sidebar shows a toast "Could not dock <app> — opened as floating window"

#### Scenario: Retry window extends for slow mappers

- **WHEN** the KWin script reports a `configure` event during the 2-second retry window
- **THEN** the cockpit attempts correlation again; a successful match transitions the satellite out of `floating_fallback`

#### Scenario: Non-KWin environments skip correlation

- **WHEN** `NoopCompositor` is the active implementor
- **THEN** `satellite.open` marks the satellite as `floating_fallback` without attempting correlation and without surfacing an error

### Requirement: Spawn metrics for dogfooding

The cockpit SHALL increment in-memory counters `satellite_spawn_ok` and `satellite_spawn_fail` on each resolved `satellite.open` dispatch and expose them via `status.get.result` and the `lmux status` CLI.

#### Scenario: Counters surface via status.get

- **WHEN** a client sends `status.get`
- **THEN** the response includes both counters and their current values

#### Scenario: `lmux status` prints the ratio

- **WHEN** the user runs `lmux status`
- **THEN** the printed output includes a line of the form `satellites: ok=N fail=M`
