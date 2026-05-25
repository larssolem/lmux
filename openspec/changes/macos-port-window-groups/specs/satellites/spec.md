## ADDED Requirements

### Requirement: macOS managed-native satellite groups

On macOS the cockpit SHALL represent GUI satellites as native app windows grouped under the anchor that was active when the satellite was spawned or manually attached. Switching anchors MUST make the incoming anchor's GUI windows visible and the outgoing anchor's GUI windows hidden/minimized.

#### Scenario: Spawned macOS app is owned by active anchor

- **WHEN** the user opens a GUI app from the launcher or `lmux open` while anchor `A` is active on macOS
- **THEN** the cockpit stamps the request with `LMUX_SATELLITE_ID=<uuid>`
- **AND** correlated windows are registered as satellites owned by anchor `A`

#### Scenario: Switching anchors swaps native GUI window group

- **WHEN** the active anchor changes from `A` to `B`
- **THEN** native macOS windows owned by `A` are minimized or hidden according to the macOS visibility policy
- **AND** native macOS windows owned by `B` are restored, placed, and optionally focused
- **AND** the terminal pane for `B` becomes the active terminal context

### Requirement: macOS satellite correlation and fallback

The cockpit SHALL correlate macOS app windows to satellite requests using request id, process/app metadata, and bounded observation. If correlation is ambiguous or unavailable, the satellite MUST fall back to unmanaged floating mode without breaking the terminal workflow.

#### Scenario: Fresh process correlates by request id and process metadata

- **WHEN** the cockpit launches a macOS app process for request `R`
- **THEN** the helper observes windows created by that launch and reports a `SatelliteWindowId` for each matching window
- **AND** the cockpit binds those windows to the active anchor

#### Scenario: Single-instance app correlation is ambiguous

- **WHEN** macOS routes a launch request to an already-running app and the helper cannot unambiguously identify the new window within the correlation window
- **THEN** the cockpit marks the satellite request as `floating_fallback`
- **AND** the app remains open
- **AND** the user can attach the focused macOS window to the active anchor manually

### Requirement: Manual attach of focused macOS window

The cockpit SHALL provide a recovery path to attach the currently focused macOS window to the active anchor when automatic correlation fails or when the user wants to bring an already-open app into a workspace.

#### Scenario: Attach focused window to active anchor

- **WHEN** the user invokes "Attach focused window" from the sidebar or CLI while anchor `A` is active
- **THEN** the helper identifies the focused non-lmux app window
- **AND** the cockpit registers it as a satellite owned by `A`
- **AND** subsequent anchor switches manage its visibility with `A`

#### Scenario: No active anchor rejects attach

- **WHEN** the user invokes manual attach and no anchor is active
- **THEN** the cockpit rejects the operation with a clear diagnostic and does not modify satellite state

### Requirement: macOS placement emulates docking

The macOS backend SHALL place active satellite windows in a predictable satellite region adjacent to the cockpit and update placement when the cockpit moves, resizes, or switches anchors. This placement MUST stop for detached windows.

#### Scenario: Active group placed beside cockpit

- **WHEN** a macOS satellite window is restored for the active anchor
- **THEN** the helper places it in the configured satellite region adjacent to the cockpit's current screen frame

#### Scenario: Detached window is not moved

- **WHEN** the user detaches a macOS satellite window
- **THEN** the cockpit stops sending placement updates for that window
- **AND** anchor switching no longer minimizes/restores it until it is reattached

## MODIFIED Requirements

### Requirement: Per-anchor satellite visibility

The cockpit SHALL bind each satellite to the anchor that was active at spawn or attach time; switching the active anchor MUST hide/minimize satellites bound to the outgoing anchor and restore satellites bound to the incoming anchor. Backends MAY implement this through KWin scripts, Wayland/nested-host widgets, macOS native window control, or a floating fallback, but the user-visible anchor/group behavior MUST remain consistent.

#### Scenario: Switching anchor away hides outgoing satellites

- **WHEN** the active anchor transitions from `A` to `B`
- **THEN** the cockpit broadcasts a grouped visibility operation for satellites owned by `A` and `B`
- **AND** Linux KWin toggles minimization for matching windows
- **AND** macOS minimizes or hides native windows by stable `SatelliteWindowId`

#### Scenario: Backend cannot manage windows

- **WHEN** the selected backend cannot manage GUI windows because it is `NoopCompositor` or macOS Accessibility permission is missing
- **THEN** satellites remain open as floating unmanaged windows
- **AND** anchor switching never errors solely because GUI window management is unavailable
