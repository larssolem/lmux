# satellites

## Purpose

A satellite is an app/window associated with an anchor workspace. The current
product model is explicit attach first: the user opens native app windows where
they want them, then adds the exact window to the active workspace. On anchor
switch, lmux asks the compositor backend to hide/show or raise the attached
windows. lmux does not own monitor placement.

Linux also contains an experimental nested-Wayland satellite path and a legacy
spawn path, but explicit native window attach is the reliable current workflow.

## Requirements

### Requirement: Explicit native window listing

The cockpit SHALL list native windows when the active compositor backend
supports it.

#### Scenario: Sidebar add-window button opens picker

- **WHEN** the backend reports `attach_window = true`
- **THEN** the sidebar header shows an enabled add-window button
- **AND** clicking it opens a modal "Add window" picker

#### Scenario: Unsupported backend disables picker

- **WHEN** the backend cannot attach native windows
- **THEN** the add-window button is disabled with a tooltip explaining that
  native window attach is unavailable

#### Scenario: Picker ranks attached windows first

- **WHEN** the picker renders candidates
- **THEN** windows already attached to the active workspace rank first,
  windows attached to another workspace rank next, and unattached windows rank
  last

#### Scenario: Picker displays ownership state

- **WHEN** a listed window is already attached
- **THEN** the row says either `Active workspace` or `Other workspace: <label>`
- **AND** the action label is `Active`, `Move here`, or `Add`

### Requirement: Explicit native window attach

The cockpit SHALL attach a selected native window to the currently active
anchor.

#### Scenario: Attach requires active anchor

- **WHEN** the user attaches a native window and no active anchor exists
- **THEN** the operation fails with `no active anchor`

#### Scenario: Attach validates backend identity

- **WHEN** `satellite.attach_window` is handled
- **THEN** the compositor backend validates and converts the candidate into a
  `SatelliteWindowId`
- **AND** `AppState` registers that exact window under the active anchor

#### Scenario: Moving attached window between anchors

- **WHEN** a backend window id already belongs to another anchor and is attached
  to the active anchor
- **THEN** lmux removes the old owner record and registers the window under the
  active anchor

### Requirement: Anchor-owned native windows

Native windows SHALL be tracked per anchor and switched by anchor activation.

#### Scenario: Registering native window triggers visibility sync

- **WHEN** a window is registered under an anchor
- **THEN** lmux immediately queues a compositor group-switch based on the current
  active anchor

#### Scenario: Anchor switch shows active windows

- **WHEN** active anchor changes
- **THEN** windows owned by the new active anchor are sent in the show list
- **AND** windows owned by every other anchor are sent in the hide list

#### Scenario: Same-process app windows stay independent

- **WHEN** two windows from the same application process are attached to
  different anchors and have distinct backend window ids
- **THEN** lmux tracks and switches them independently

### Requirement: User-controlled placement

lmux SHALL respect the user's native window placement.

#### Scenario: Attach does not move the window

- **WHEN** a user attaches an already-open native window
- **THEN** lmux records ownership but does not choose a display or rewrite the
  window's geometry

#### Scenario: Anchor switch does not manage displays

- **WHEN** an attached window is shown for the active anchor
- **THEN** it stays wherever the user/compositor placed it

### Requirement: macOS explicit attach

On macOS, lmux SHALL use exact window identities instead of app-bundle-wide
ownership.

#### Scenario: Focused macOS attach requires stable identity

- **WHEN** `satellite.attach_focused` is used on macOS
- **THEN** lmux reads the focused helper window and requires a stable window id
  before registering it

#### Scenario: Selected macOS attach accepts window id or index

- **WHEN** `satellite.attach_window` targets macOS
- **THEN** lmux accepts a helper window with a stable window id or window index
- **AND** stores bundle id and title when available

#### Scenario: macOS launcher is disabled

- **WHEN** the launcher open function is called on macOS
- **THEN** it logs that launcher is disabled and expects users to attach
  already-open native windows

### Requirement: Linux native attach

On Linux, KWin and X11 backends SHALL provide native window listing/attach when
their required compositor/tools are available.

#### Scenario: KWin candidate carries backend id

- **WHEN** KWin lists windows
- **THEN** each attachable candidate carries a backend window id used for exact
  visibility and raise operations

#### Scenario: X11 candidate uses xprop/xdotool

- **WHEN** X11 support is available
- **THEN** listing uses X11 window properties and visibility/raise uses
  `xdotool`

### Requirement: Legacy `satellite.open`

The bus SHALL keep a legacy spawn kind, but it is not the managed attach
workflow.

#### Scenario: Non-macOS open launches without ownership

- **WHEN** `satellite.open` succeeds on a non-macOS build
- **THEN** it spawns the process and increments the success counter
- **AND** logs that the app was launched without ownership and must be attached
  separately to be managed

#### Scenario: macOS open is disabled

- **WHEN** `satellite.open` is called on macOS
- **THEN** it increments the failure counter and returns an error instructing
  the user to use focused/native attach

### Requirement: Nested Wayland host

On Linux, lmux SHALL be able to run a nested Wayland host for in-process
satellite panes when the host starts successfully.

#### Scenario: Host ready records display name

- **WHEN** the host emits `Ready { display_name }`
- **THEN** `AppState` records the display name for child environment use

#### Scenario: New toplevel creates satellite pane

- **WHEN** the host emits a toplevel-created event
- **THEN** lmux allocates a new satellite pane in the active workspace and maps
  the surface id to that pane

#### Scenario: Frame events update satellite widget

- **WHEN** the host emits frame, dmabuf, popup, title, app-id, or close events
- **THEN** lmux routes them to the owning `SatelliteWidget` or removes the pane
  on close
