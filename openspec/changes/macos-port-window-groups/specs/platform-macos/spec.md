## ADDED Requirements

### Requirement: macOS cockpit build and startup

The cockpit SHALL build and start on macOS with terminal panes, anchors, sessions, sidebar, launcher UI, config loading, logging, and bus IPC available. Linux-only compositor and Wayland-host code MUST be gated so it is not required for a macOS build.

#### Scenario: macOS build excludes Linux-only Wayland host

- **WHEN** the workspace is built for `target_os = "macos"`
- **THEN** `lmux-wayland-host` is not required by the `lmux` binary
- **AND** the cockpit compiles without KWin, Wayland, `/proc`, `PDEATHSIG`, or Linux `SO_PEERCRED` APIs

#### Scenario: macOS startup opens a terminal cockpit

- **WHEN** the user starts `lmux` on macOS
- **THEN** the cockpit opens with a shell pane, restores the active session when available, seeds an anchor when needed, and accepts the same prefix keybindings as Linux

### Requirement: macOS native window helper lifecycle

The cockpit SHALL start a macOS-only native helper for Accessibility/AppKit window operations when satellite management is enabled. The helper MUST handshake with a protocol version and permission state before the cockpit selects the macOS window backend.

#### Scenario: Helper handshake succeeds

- **WHEN** the cockpit starts on macOS and launches `lmux-macos-windowctl`
- **THEN** the helper replies with `{ protocol_version, capabilities, permission_state }`
- **AND** the cockpit selects the macOS backend when the protocol version is compatible

#### Scenario: Helper unavailable degrades to terminal-only satellites

- **WHEN** the helper binary is missing, crashes during handshake, or reports an incompatible protocol version
- **THEN** the cockpit continues running terminal features
- **AND** satellite launches open as unmanaged floating windows with a sidebar banner naming the helper failure

### Requirement: Accessibility permission gating

The cockpit SHALL treat macOS Accessibility permission as required for managed satellite hide/show/focus/placement, but MUST NOT require it for terminal multiplexing.

#### Scenario: Accessibility not granted

- **WHEN** the helper reports `accessibility = denied` or `not_determined`
- **THEN** the cockpit shows one actionable banner explaining that macOS Accessibility permission is needed for GUI window grouping
- **AND** satellites still launch as unmanaged floating windows
- **AND** anchor switching continues to switch terminal panes without errors

#### Scenario: Accessibility granted after startup

- **WHEN** the user grants Accessibility permission while the cockpit is running
- **THEN** the helper reports the updated permission state
- **AND** the cockpit clears the banner and enables managed satellite grouping without requiring restart

### Requirement: macOS bus and filesystem paths

The cockpit SHALL use macOS-appropriate runtime, state, and config paths while preserving the existing bus protocol semantics.

#### Scenario: Runtime socket path resolves on macOS

- **WHEN** `XDG_RUNTIME_DIR` is absent on macOS
- **THEN** the cockpit resolves its bus socket under a user-private macOS runtime directory
- **AND** `lmux-cli` can discover and connect to the same socket

#### Scenario: Peer credentials are validated on macOS

- **WHEN** a client connects to the cockpit bus on macOS
- **THEN** the server validates the peer user using a macOS-supported peer credential mechanism
- **AND** rejects connections from other users with the same `error.peer_denied` behavior as Linux

### Requirement: macOS end-to-end test environment

The project SHALL support end-to-end validation of the macOS port on a real macOS desktop session using either a dedicated Mac runner or a macOS virtual machine. The project MUST NOT rely on Xcode Simulator for this port because the required behavior depends on the macOS window server, native app windows, and Accessibility permissions.

#### Scenario: Fake-helper tests run without macOS desktop

- **WHEN** CI runs on Linux or on macOS without Accessibility permission
- **THEN** helper protocol, backend selection, degraded permission handling, and anchor-switch grouping are tested against a fake helper

#### Scenario: Real macOS E2E validates window management

- **WHEN** the macOS E2E lane runs on a dedicated Mac runner or macOS VM with Accessibility permission granted to the helper
- **THEN** the test opens the cockpit, launches a native app satellite, switches anchors, and asserts that outgoing windows are minimized/hidden and incoming windows are restored

#### Scenario: Missing Accessibility lane remains valid

- **WHEN** the macOS E2E lane runs without Accessibility permission
- **THEN** it verifies the degraded behavior: permission banner appears, terminal anchor switching works, and satellites launch as unmanaged floating windows without test failure
