## ADDED Requirements

### Requirement: Linux Attach Window Picker

On Linux, the sidebar SHALL expose an Attach Window action when the active compositor backend supports native window attachment.

#### Scenario: Attach action visible on supported backend

- **WHEN** lmux runs on Linux with an attach-capable backend
- **THEN** the sidebar shows an Attach Window action
- **AND** activating it opens a picker populated from `satellite.list_windows`

#### Scenario: Attach selected window

- **WHEN** the user selects a window in the Linux attach picker
- **THEN** the sidebar dispatches `satellite.attach_window` for that candidate
- **AND** closes the picker after a successful attach

### Requirement: Linux Attach Degraded State

On Linux, the sidebar SHALL clearly communicate when native window attachment is unavailable for the current display stack.

#### Scenario: Unsupported Wayland compositor

- **WHEN** lmux runs under a Wayland compositor without an attach-capable backend
- **THEN** the sidebar does not present a broken picker
- **AND** it shows a concise degraded-state message or disabled action naming unsupported window attachment

#### Scenario: Window listing failure

- **WHEN** opening the attach picker fails because the backend cannot list windows
- **THEN** the picker shows a non-crashing error state
- **AND** no attach request is sent

### Requirement: Launcher Is Secondary To Attach

On Linux, the sidebar SHALL present explicit attachment as the primary GUI ownership workflow.

#### Scenario: Header affordance

- **WHEN** a Linux backend supports window attachment
- **THEN** the primary sidebar GUI action is Attach Window, not Launch Program

#### Scenario: Launch does not imply ownership

- **WHEN** a separate launch action is available
- **THEN** its label or placement makes clear that ownership still requires attaching a window
