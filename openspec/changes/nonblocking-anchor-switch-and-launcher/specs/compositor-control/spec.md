### Requirement: Latest-wins grouped window operations

The compositor-control layer SHALL treat active-anchor window-group switches as latest-wins operations. A stale grouped operation MUST NOT raise, focus, or otherwise foreground windows for an anchor that is no longer active.

#### Scenario: Queued group switches coalesce

- **WHEN** multiple anchor-switch visibility commands are queued before the compositor bridge processes them
- **THEN** the bridge processes the newest command and drops older stale commands

#### Scenario: In-flight group switch becomes stale

- **WHEN** a grouped window operation is already running
- **AND** a newer active-anchor sequence is issued
- **THEN** the running operation stops before any further raise/focus work when possible
- **AND** it MUST NOT foreground windows for the stale anchor

#### Scenario: Group switch returns per-window results

- **WHEN** a backend applies a grouped hide/show operation
- **THEN** it returns success or failure per window
- **AND** a failure for one window does not abort the whole active-anchor switch

### Requirement: macOS native helper fast path

On macOS, compositor-control SHALL prefer stable native window identity for normal visibility and raise operations. AppleScript fallback MUST NOT run after a native stable-window-id operation has already succeeded.

#### Scenario: Stable window id succeeds

- **WHEN** the helper successfully restores or raises a satellite window by stable native window id
- **THEN** the operation is complete
- **AND** no process-wide `System Events` or `osascript` fallback is invoked for that window

#### Scenario: Native operation is unavailable

- **WHEN** stable native window control is unavailable or fails
- **THEN** the backend MAY use a degraded fallback with timeout and structured diagnostics
- **AND** the fallback failure is reported for that window without blocking future anchor switches
