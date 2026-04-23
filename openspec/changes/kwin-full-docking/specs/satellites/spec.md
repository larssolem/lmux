## ADDED Requirements

### Requirement: Satellite-to-window mapping file

The cockpit's KWin script SHALL maintain a JSON mapping file at `$XDG_RUNTIME_DIR/lmux/satellites.json` that associates each correlated satellite's `request_id` with its KWin `window_id` and `pid`; the file MUST be written atomically and pruned when the window closes.

#### Scenario: windowAdded writes a mapping entry

- **WHEN** the KWin script observes a `windowAdded` event whose `WM_CLASS` matches `lmux-sat-<request_id>` and whose PID is known to the cockpit
- **THEN** the script writes an entry `{ request_id, window_id, pid }` into `satellites.json` via stage + rename, preserving existing entries

#### Scenario: windowRemoved prunes the mapping

- **WHEN** the KWin script observes a `windowRemoved` event for a window in the mapping file
- **THEN** the entry is removed from `satellites.json` atomically

#### Scenario: Stale entries are ignored on startup

- **WHEN** the cockpit reads `satellites.json` on startup and finds entries whose PIDs no longer exist
- **THEN** those entries are discarded and the file is rewritten without them

### Requirement: Geometry follow

The cockpit SHALL update a docked satellite's screen geometry to match the owning pane's screen rect within one frame of any layout change; repeated same-rect calls MUST be no-ops.

#### Scenario: Pane resize propagates to the satellite

- **WHEN** a pane that owns a satellite is resized or moved by the layout engine
- **THEN** within one frame (~16 ms) the cockpit emits `satellite.geometry { window_id, rect }` and the KWin script calls `window.frameGeometry = rect` on the matching window

#### Scenario: Idempotent set_geometry

- **WHEN** the cockpit emits `satellite.geometry` with a rect equal to the satellite's current rect
- **THEN** the KWin script returns early without calling `frameGeometry =`; no visible redraw is triggered

#### Scenario: Off-screen rect is clamped, not dropped

- **WHEN** the cockpit emits a rect that lies partially outside the window's screen
- **THEN** the KWin script clamps the rect to the screen's placement area; it does not refuse the call

### Requirement: Explicit detach

The user SHALL be able to detach a docked satellite from its owning pane; the satellite MUST become a normal floating toplevel and the pane's satellite-ownership indicator MUST clear.

#### Scenario: CLI detach

- **WHEN** the user runs `lmux satellite detach <pane-id>` (or `<uuid>`) against a pane that owns a docked satellite
- **THEN** the cockpit calls `KwinCompositor::detach(window_id)`, which re-enables normal WM placement on the window (borders, free geometry), and clears the pane's "satellite-owned" state

#### Scenario: Sidebar detach action

- **WHEN** the user selects the "Detach" action in the sidebar context menu on a satellite-owned pane row
- **THEN** the same code path runs as the CLI invocation; a toast confirms detachment

#### Scenario: Detach on already-detached satellite is a no-op

- **WHEN** the user issues detach against a pane whose satellite is already floating
- **THEN** the operation returns `Ok(())` without side effects; no toast is surfaced

### Requirement: Explicit reattach

The user SHALL be able to reattach a detached satellite to its original pane (or to the currently-focused pane if the original is gone); the KWin script MUST re-dock the window to the current pane rect.

#### Scenario: Reattach to original pane

- **WHEN** the user runs `lmux satellite reattach <pane-id>` against a pane whose satellite was previously detached and still running
- **THEN** the cockpit calls `KwinCompositor::attach(window_id, current_rect)`, which disables free placement on the window and snaps it to the pane's screen rect

#### Scenario: Reattach after owning pane closed

- **WHEN** the satellite's original owning pane has been closed but the satellite is still alive
- **THEN** the sidebar offers "Reattach to focused pane"; the action reattaches the satellite to the currently-focused pane and transfers ownership

#### Scenario: Reattach after satellite died is a typed error

- **WHEN** the user triggers reattach against a satellite whose window no longer exists
- **THEN** the cockpit returns `error.satellite_gone` and the pane's satellite-ownership state is cleared if it was stale

### Requirement: Close-pane policy for owning satellites

The cockpit SHALL apply a configurable close-pane policy for panes that own a docked satellite: `sigterm` (send SIGTERM to the satellite process), `detach` (release to floating, default), or `leave` (do nothing).

#### Scenario: Default policy detaches to floating

- **WHEN** the user closes a pane that owns a docked satellite and `[satellites].close_behavior` is unset or `"detach"`
- **THEN** the satellite remains running as a floating toplevel; the pane is removed from the layout

#### Scenario: sigterm policy terminates the satellite

- **WHEN** the user closes a pane that owns a docked satellite and `[satellites].close_behavior = "sigterm"`
- **THEN** the cockpit sends `SIGTERM` to the satellite's PID; the pane is removed from the layout after the PTY shutdown contract resolves

#### Scenario: leave policy keeps satellite docked to no pane

- **WHEN** the user closes a pane that owns a docked satellite and `[satellites].close_behavior = "leave"`
- **THEN** the satellite's window is unmodified and remains docked to the now-vacant screen rect; the sidebar warns that a satellite is orphaned and offers a "Reattach to focused pane" toast

## MODIFIED Requirements

### Requirement: KWin best-effort placement

On KWin the cockpit SHALL perform an *initial* best-effort placement of a freshly-spawned satellite and MUST then transition the satellite into the full docking lifecycle (geometry-follow, detach, reattach, close-policy) once the `satellites.json` mapping entry is written. Initial placement is the first call in the docking lifecycle; it is not the only one.

#### Scenario: Initial placement runs as before

- **WHEN** `KwinCompositor::spawn_satellite` is called on a KWin session
- **THEN** the cockpit returns `(request_id, pid)` immediately and schedules an async task that writes a `lmux-place-<pid>` script, loads and runs it over D-Bus, then unloads it

#### Scenario: Initial placement transitions into docking lifecycle

- **WHEN** `satellites.json` gains a new mapping entry for the satellite
- **THEN** the cockpit moves the satellite from "best-effort placed" to "docked"; subsequent layout changes trigger `set_geometry` and the `Detach`/`Reattach` CLI + sidebar actions become available for this satellite

#### Scenario: Placement is best-effort, not gating

- **WHEN** the KWin placement script fails or the window cannot be found by PID within a short window
- **THEN** the satellite remains running as a floating window without any error surfaced to the user; the sidebar indicates "floating_fallback" with a toast per the correlation-timeout requirement
