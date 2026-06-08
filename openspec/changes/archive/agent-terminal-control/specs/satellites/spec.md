## ADDED Requirements

### Requirement: Agent-requested native window attach

Agents SHALL be able to request attaching existing native windows to an anchor, subject to user approval.

#### Scenario: Agent lists attachable windows

- **WHEN** an authorized agent requests native window candidates
- **THEN** the cockpit returns the same candidate identities used by the user-facing add-window picker

#### Scenario: Attach request shows exact target

- **WHEN** an agent requests attaching a native window to an anchor
- **THEN** the cockpit shows the agent identity, target anchor, window title, app identity, backend id, and reason before applying the attach

#### Scenario: Approved attach uses existing attach path

- **WHEN** the user approves an agent native window attach request
- **THEN** lmux validates the candidate through the compositor backend
- **AND** registers the resulting window under the target anchor

#### Scenario: Denied attach has no side effect

- **WHEN** the user denies an agent native window attach request
- **THEN** no native window ownership is changed

### Requirement: Agent launch-and-attach request

Agents SHALL be able to request launching a GUI app and attaching the matching native window, subject to user approval.

#### Scenario: Launch request shows command

- **WHEN** an agent requests launch-and-attach
- **THEN** the cockpit shows the command argv, target anchor, matching hints, and agent reason before launching

#### Scenario: Matching window is attached after approval

- **WHEN** a launch-and-attach request is approved and a matching window appears
- **THEN** lmux attaches that exact window to the target anchor through the explicit native attach path

#### Scenario: No matching window times out

- **WHEN** no matching native window appears before the request timeout
- **THEN** the request fails with a timeout error
- **AND** no unrelated window is attached
