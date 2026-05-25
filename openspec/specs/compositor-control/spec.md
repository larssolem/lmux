# compositor-control

## Purpose

`CompositorControl` is the boundary between the GTK cockpit and platform window
control. It lists attachable native windows, validates explicit window attach,
shows/hides or raises managed windows, reports health, and keeps backend-specific
details out of `AppState`.

## Requirements

### Requirement: Backend-neutral trait

The cockpit SHALL depend on a backend-neutral `CompositorControl` trait.

#### Scenario: Implemented backends compile behind one trait

- **WHEN** Linux, macOS, X11, and test builds compile
- **THEN** callers use `CompositorControl` plus shared types such as
  `Health`, `CompositorError`, `WindowCandidate`, `SatelliteWindowId`,
  `WindowGroupSwitch`, `WindowOpResult`, and `WindowControlCapabilities`

#### Scenario: Runtime backend selection

- **WHEN** the cockpit starts on Linux X11
- **THEN** it uses `X11Compositor`
- **WHEN** it starts on Linux KDE/Wayland with a discoverable KWin script
- **THEN** it uses `KwinCompositor`
- **WHEN** no supported native window backend is available
- **THEN** it uses `NoopCompositor`

### Requirement: Window control capability flags

Each backend SHALL report whether it can list windows, attach windows, set
visibility, and raise windows.

#### Scenario: Sidebar attach button follows capabilities

- **WHEN** `window_control_capabilities().attach_window` is false
- **THEN** the sidebar attach button is disabled with explanatory tooltip text

#### Scenario: Native listing follows capabilities

- **WHEN** `satellite.list_windows` is requested and the backend cannot list
  windows
- **THEN** the bus returns a clear domain error

### Requirement: Explicit native window attach

Backends that support native attach SHALL validate a `WindowCandidate` and
return a stable `SatelliteWindowId` for the exact window.

#### Scenario: KWin attach validates current inventory

- **WHEN** a KWin window candidate is attached
- **THEN** the backend verifies that the backend window id is still present in
  the current inventory before returning a managed identity

#### Scenario: macOS attach uses per-window identity

- **WHEN** a macOS window is attached
- **THEN** the managed identity preserves stable CoreGraphics window id when
  available, otherwise the helper-provided window index fallback is retained

#### Scenario: X11 attach validates window id

- **WHEN** an X11 candidate is attached
- **THEN** the backend verifies the X11 window id before returning a managed
  identity

### Requirement: Anchor switch window group operation

The compositor bridge SHALL apply anchor-driven native window switching off the
GTK main thread.

#### Scenario: AppState emits group switch

- **WHEN** the active anchor changes or a native window is attached
- **THEN** `AppState` computes the windows to hide and show from
  `satellite_windows_by_anchor`
- **AND** queues one `ApplyWindowGroupSwitch` command with a monotonically
  increasing sequence number

#### Scenario: Bridge coalesces stale switches

- **WHEN** multiple anchor switches are queued quickly
- **THEN** the bridge drops older queued group-switch commands and applies the
  latest sequence

#### Scenario: Backend reports per-window results

- **WHEN** a group switch is applied
- **THEN** the backend returns one `WindowOpResult` per attempted window
- **AND** failures are logged without blocking the cockpit UI

### Requirement: User placement is preserved

lmux SHALL NOT manage monitor topology or move windows between displays as part
of anchor switching. The user and compositor own physical placement.

#### Scenario: Anchor switch does not set display geometry

- **WHEN** the user switches anchors
- **THEN** lmux shows/raises the incoming anchor's tracked windows and hides or
  minimizes outgoing tracked windows according to backend ability
- **AND** lmux does not choose a monitor, rewrite window coordinates, or
  re-home windows on hotplug

### Requirement: KWin backend

The KWin backend SHALL use KWin scripting for window inventory, previews,
visibility, and raising.

#### Scenario: KWin visibility targets exact backend id

- **WHEN** `set_window_visible` is called with a KWin `SatelliteWindowId`
- **THEN** the backend validates the backend window id and runs a one-shot KWin
  script matching that exact id

#### Scenario: KWin raise activates exact backend id

- **WHEN** `raise_window` is called for a KWin window
- **THEN** the backend unminimizes and activates the matching KWin window

### Requirement: macOS backend

The macOS backend SHALL use `lmux-macos-helper` for listing and per-window
visibility group operations when helper support is available.

#### Scenario: Group switch is batched

- **WHEN** `apply_window_group_switch_latest` is called on macOS
- **THEN** the backend sends one helper request containing hide and show lists
- **AND** skips the helper call if the operation is already stale

### Requirement: Noop backend degrades gracefully

`NoopCompositor` SHALL keep the terminal cockpit usable when native window
control is unavailable.

#### Scenario: Unsupported operations do not crash cockpit

- **WHEN** a native-window operation is unsupported
- **THEN** callers receive an `Unsupported`/domain error or a harmless no-op
  depending on the trait default
- **AND** terminal panes continue running
