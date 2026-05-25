## ADDED Requirements

### Requirement: Window Management Capability Reporting

The compositor control layer SHALL report which window-management capabilities are available for the current backend.

#### Scenario: Attach-capable backend

- **WHEN** lmux starts in a KWin session with a working lmux KWin bridge
- **THEN** the compositor backend reports support for listing windows, attaching a selected window, and applying visibility to an attached window

#### Scenario: Unsupported Wayland backend

- **WHEN** lmux starts under a Wayland compositor without a supported window-control backend
- **THEN** the compositor backend reports attach as unsupported
- **AND** the cockpit continues to provide terminal/session features

### Requirement: Platform-Neutral Window Candidates

The compositor control layer SHALL expose attachable windows as platform-neutral candidates with backend, backend window id, title, pid when available, app identity when available, and optional workspace/output metadata.

#### Scenario: KWin candidate

- **WHEN** the KWin backend lists windows
- **THEN** each candidate includes `backend = "kwin"` and a backend window id stable enough for later visibility operations

#### Scenario: X11 candidate

- **WHEN** the X11/EWMH backend lists windows
- **THEN** each candidate includes `backend = "x11"`, the X11 window id, title when available, pid when available, and WM_CLASS when available

### Requirement: Exact Identity Window Operations

Compositor backends SHALL apply attach visibility/focus operations using the stored backend window identity from an attached satellite record.

#### Scenario: Exact window found

- **WHEN** the backend can resolve an attached satellite's backend window id
- **THEN** it applies the requested visibility or raise operation only to that window

#### Scenario: Exact window missing

- **WHEN** the backend cannot resolve an attached satellite's backend window id
- **THEN** it returns a per-window failure
- **AND** it does not fall back to pid-wide, app-wide, class-wide, or title-based control

### Requirement: KWin Bridge Provides Window Inventory

The KWin backend SHALL use the lmux KWin bridge to provide window inventory and exact-window control for KDE Wayland sessions.

#### Scenario: KWin window list

- **WHEN** the user opens the Linux attach picker on KDE Wayland
- **THEN** lmux queries the KWin bridge for current windows
- **AND** only windows with a usable backend identity are shown as attachable

#### Scenario: KWin attached window visibility

- **WHEN** anchor switching hides or shows a KWin attached window
- **THEN** the KWin backend targets the stored KWin window identity

### Requirement: X11/EWMH Best-Effort Backend

The X11 backend SHALL provide best-effort window listing and exact X11-window operations for X server sessions.

#### Scenario: X11 global window list

- **WHEN** lmux starts under X11 and EWMH is available
- **THEN** the X11 backend lists top-level windows from the X server and exposes them as attachable candidates

#### Scenario: X11 backend unavailable

- **WHEN** lmux starts under X11 and EWMH/window-control setup is unavailable
- **THEN** attach capability is reported as unsupported
- **AND** lmux does not crash or disable terminal/session features
