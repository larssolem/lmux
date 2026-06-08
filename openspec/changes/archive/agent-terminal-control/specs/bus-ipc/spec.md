## ADDED Requirements

### Requirement: Agent-control bus kinds

The bus SHALL expose versioned kinds for agent terminal control.

#### Scenario: Pane transcript kinds are available

- **WHEN** a client sends `pane.tail` or `pane.capture`
- **THEN** the cockpit returns transcript text, first sequence, last sequence, truncation metadata, and pane identity

#### Scenario: Pane creation kind supports tabs and splits

- **WHEN** a client sends `pane.new` with target anchor, placement, optional title, optional argv, and optional agent metadata
- **THEN** the cockpit creates a terminal pane in the requested anchor workspace when authorized
- **AND** returns the new pane UUID and owning anchor UUID

#### Scenario: Pane input kind is gated

- **WHEN** a client sends `pane.send_input`
- **THEN** the cockpit checks whether the requester may write to the target pane before injecting bytes into the PTY

#### Scenario: Pane rename kind tracks provenance

- **WHEN** a client sends `pane.rename`
- **THEN** the cockpit applies naming provenance rules before changing the visible pane title

#### Scenario: Anchor inventory kind is scriptable

- **WHEN** a client sends `anchor.list`
- **THEN** the cockpit returns anchor UUIDs, labels, active state, and compact agent/grant status suitable for CLI and MCP clients

### Requirement: Grant request bus kinds

The bus SHALL expose grant request and grant decision kinds for cross-anchor operations.

#### Scenario: Sensitive request returns pending grant

- **WHEN** a bus operation requires user approval
- **THEN** the cockpit returns or emits a pending grant id with requested scope and target metadata

#### Scenario: Grant decision resolves pending operation

- **WHEN** the user allows or denies the pending grant in the cockpit UI
- **THEN** the original bus operation completes according to that decision

### Requirement: MCP client role

The bus SHALL identify MCP adapter clients distinctly from the regular CLI role.

#### Scenario: MCP adapter handshakes with role

- **WHEN** `lmux-mcp` connects to the bus
- **THEN** it handshakes with a client role that identifies it as the MCP adapter
- **AND** the cockpit uses that role for audit/status text
