## ADDED Requirements

### Requirement: macOS native window backend

On macOS the cockpit SHALL provide a `MacWindowCompositor` implementor of `CompositorControl` that controls native app windows through the macOS helper and preserves the same anchor-owned satellite group semantics as Linux backends.

#### Scenario: macOS backend selected when helper and permissions are usable

- **WHEN** the cockpit starts on macOS, the helper handshake succeeds, and Accessibility permission is granted
- **THEN** it instantiates `MacWindowCompositor`
- **AND** the backend choice is logged at INFO with reason `"macos-helper-ready"`

#### Scenario: macOS backend degrades when permission is missing

- **WHEN** the cockpit starts on macOS and the helper reports missing Accessibility permission
- **THEN** the selected backend reports degraded health with reason `"accessibility-permission-missing"`
- **AND** `spawn_satellite` still opens the app but returns a managed state of `floating_fallback`

### Requirement: Stable satellite window identity

The compositor abstraction SHALL support satellite visibility, focus, detach, attach, and placement using an opaque stable satellite window identity rather than PID-only matching.

#### Scenario: Backend returns window identity after correlation

- **WHEN** a spawned satellite window is correlated by any backend
- **THEN** the backend records a `SatelliteWindowId` containing the request id, backend name, backend-specific window id, and any known PID/title/application metadata
- **AND** subsequent visibility and placement commands address that `SatelliteWindowId`

#### Scenario: PID-only command remains a compatibility path

- **WHEN** existing Linux code calls `set_window_visible_by_pid(pid, visible)`
- **THEN** KWin behavior remains unchanged
- **AND** new code paths prefer `set_window_visible(window_id, visible)` when a correlated `SatelliteWindowId` exists

### Requirement: Grouped active-anchor visibility command

The compositor abstraction SHALL support applying an active-anchor switch as a grouped visibility/focus operation so a backend can hide outgoing windows and restore incoming windows consistently.

#### Scenario: Anchor switch emits grouped operation

- **WHEN** the active anchor transitions from `A` to `B`
- **THEN** the cockpit sends the backend one operation containing all satellite windows to hide for `A` and all satellite windows to show for `B`
- **AND** the backend returns per-window results without aborting the entire switch when one window fails

#### Scenario: macOS helper applies group operation

- **WHEN** `MacWindowCompositor` receives a grouped active-anchor operation
- **THEN** it asks the helper to minimize outgoing windows, restore incoming windows, place them in the active satellite region, and apply the configured focus policy
