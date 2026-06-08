## ADDED Requirements

### Requirement: Agent activity in anchor rows

The sidebar SHALL show compact agent activity for each anchor without replacing the anchor row's workspace identity.

#### Scenario: Agent-owned pane status is visible

- **WHEN** an anchor contains one or more agent-owned panes
- **THEN** the anchor row shows compact status text or chips with the agent name and pane purpose

#### Scenario: Activity updates when pane title changes

- **WHEN** an agent-owned pane is renamed
- **THEN** the sidebar updates the displayed agent activity to reflect the current visible title

### Requirement: Cross-anchor access prompts

The sidebar SHALL surface pending cross-anchor access requests.

#### Scenario: Pending request appears on target anchor

- **WHEN** an agent requests cross-anchor access
- **THEN** the target anchor row shows a pending request indicator
- **AND** opening the request shows agent identity, requested scope, target pane/window, reason, and allow/deny controls

#### Scenario: Grant state is revocable

- **WHEN** an anchor has an active agent grant
- **THEN** the sidebar exposes the grant in the anchor popover
- **AND** provides a revoke control

### Requirement: Separate anchor, terminal, and satellite surfaces

The UI SHALL keep workspace anchors, terminal tabs, and GUI satellites as separate navigation surfaces.

#### Scenario: Anchor rail remains workspace-only

- **WHEN** an anchor contains multiple terminal tabs and attached GUI satellites
- **THEN** the anchor rail still renders one row for the anchor

#### Scenario: Satellites are not terminal tabs

- **WHEN** a GUI window is attached to an anchor
- **THEN** it appears in the anchor's satellite surface or status
- **AND** it is not added to the terminal tab strip
