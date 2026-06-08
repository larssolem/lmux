## ADDED Requirements

### Requirement: Explicit macOS Window Attachment

On macOS, lmux SHALL support attaching the currently focused native macOS window to the active anchor without requiring lmux to have launched the application.

#### Scenario: Attach focused native window

- **Given** an active lmux anchor
- **And** a native macOS window is focused
- **And** the helper can report a stable CoreGraphics window id
- **When** the user invokes attach focused
- **Then** lmux registers that exact window id under the active anchor
- **And** lmux does not inspect launch history to decide ownership

### Requirement: No Inferred macOS Native App Ownership

On macOS, lmux SHALL NOT infer native app anchor ownership from bundle id, process id, title, or window index when a stable attached window id is unavailable.

#### Scenario: Attached window id is stale

- **Given** an attached macOS window record
- **And** the recorded stable window id no longer resolves
- **When** lmux applies anchor visibility
- **Then** lmux logs a failure for that window
- **And** lmux does not apply bundle-wide or process-wide fallback visibility

### Requirement: macOS Launcher Disabled For Native Apps

On macOS, lmux SHALL NOT list native `.app` applications in the lmux program launcher.

#### Scenario: Invoke launcher on macOS

- **Given** lmux is running on macOS
- **When** the user invokes the program launcher
- **Then** lmux does not expose native `.app` launch choices
- **And** native app ownership is established only through explicit attach
