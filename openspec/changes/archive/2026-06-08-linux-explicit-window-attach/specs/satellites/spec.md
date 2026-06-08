## ADDED Requirements

### Requirement: Explicit Linux Window Attachment

On Linux, lmux SHALL support attaching an already-open native window to the active anchor without requiring lmux to have launched that application.

#### Scenario: Attach selected Linux window

- **WHEN** the user selects an attachable Linux window from the attach picker
- **THEN** lmux registers that exact backend window identity under the active anchor
- **AND** anchor switching operates only on that registered window record

#### Scenario: No active anchor

- **WHEN** the user tries to attach a Linux window with no active anchor
- **THEN** lmux rejects the attach request with a clear error
- **AND** the selected window is not registered

### Requirement: No Inferred Linux Launch Ownership

On Linux, lmux SHALL NOT infer anchor ownership for native GUI windows from launch history, process id, desktop entry, app id, title, or WM_CLASS alone.

#### Scenario: Spawned app opens a matching window

- **WHEN** lmux launches an app and a compositor reports a new window with a matching pid or app identity
- **THEN** lmux does not register that window under an anchor unless the user explicitly attaches it

#### Scenario: Attached window identity is stale

- **WHEN** an attached Linux window record no longer resolves in the active compositor backend
- **THEN** lmux logs or surfaces a failure for that window
- **AND** lmux does not fall back to controlling every window with the same pid, title, desktop entry, app id, or WM_CLASS

### Requirement: Linux Launching Is Separate From Ownership

Linux app launching, if exposed, SHALL be separate from window ownership. Launching an app MUST NOT by itself create a managed satellite window record.

#### Scenario: Launch external app

- **WHEN** the user launches an app from lmux on Linux
- **THEN** the app may open as a normal external window
- **AND** lmux does not treat that window as owned until the user attaches it

#### Scenario: Attach after launch

- **WHEN** a launched app creates a window and the user attaches that window
- **THEN** lmux registers the selected window under the active anchor using the same explicit attach path as any pre-existing window

## REMOVED Requirements

### Requirement: KWin best-effort placement

**Reason**: Linux native window ownership is moving to explicit user attachment. Launch-time PID placement encourages inferred ownership and is not portable across Linux display stacks.

**Migration**: Use explicit attach for KWin-managed native windows. A future launch command may open an app externally, but ownership begins only after attachment.

### Requirement: Correlation timeout and floating fallback

**Reason**: Correlation timeout is part of launch-inferred docking. Explicit attach does not wait for a spawned process to map a window.

**Migration**: If no attach-capable backend exists, lmux reports attach as unsupported/degraded; it does not start a correlation timer.

## MODIFIED Requirements

### Requirement: Per-anchor satellite visibility

The cockpit SHALL bind each explicitly attached satellite window to the anchor that was active at attach time; switching the active anchor MUST hide or lower windows bound to inactive anchors and show or raise windows bound to the active anchor using the compositor backend's exact stored window identity.

#### Scenario: Switching anchor away hides attached windows

- **WHEN** the active anchor transitions from `A` to `B`
- **THEN** the cockpit broadcasts a grouped visibility switch for attached windows owned by `A` and `B`
- **AND** each backend operation targets the stored backend window identity, not a broad pid/app/title match

#### Scenario: Missing attached window fails closed

- **WHEN** the compositor backend cannot resolve an attached window identity during an anchor switch
- **THEN** lmux reports that window operation as failed
- **AND** lmux does not control unrelated windows as fallback
