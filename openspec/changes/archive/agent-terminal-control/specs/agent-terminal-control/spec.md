## ADDED Requirements

### Requirement: Agent identity and provenance

The cockpit SHALL track agent identity and provenance for agent-created or agent-managed panes.

#### Scenario: Agent identity is discovered from environment

- **WHEN** a bus request or child pane process includes `LMUX_AGENT_ID`
- **THEN** the cockpit records that id as the requesting agent identity
- **AND** uses `LMUX_AGENT_NAME` when present as the display name

#### Scenario: Agent-created pane records ownership

- **WHEN** an agent creates a terminal pane through the bus, CLI, or MCP adapter
- **THEN** the pane records the agent id, display name, creation time, and requested purpose if provided

#### Scenario: User-created pane has no agent owner

- **WHEN** the user creates a pane through normal keyboard or sidebar UI
- **THEN** the pane has no agent owner unless an agent later receives an explicit grant for it

### Requirement: Pane naming provenance

The cockpit SHALL distinguish user-set, agent-set, and default pane names.

#### Scenario: Agent auto-names owned pane

- **WHEN** an agent sets the title of a pane it owns
- **THEN** the cockpit updates the visible pane title
- **AND** records the title provenance as agent-set

#### Scenario: User-pinned title blocks automatic agent rename

- **WHEN** a user manually renames or pins a pane title
- **THEN** later automatic agent rename requests for that pane are rejected or ignored
- **AND** the existing user-set title remains visible

#### Scenario: Explicit agent rename request is visible

- **WHEN** an agent asks to rename a user-pinned pane
- **THEN** the cockpit presents the old title, proposed title, and agent identity before applying the rename

### Requirement: Cross-anchor grants

The cockpit SHALL require visible grants before an agent can read output, send input, rename panes, or attach GUI windows outside its own anchor.

#### Scenario: Own anchor read succeeds without prompt

- **WHEN** an agent requests transcript output from a pane it owns in its current anchor
- **THEN** the cockpit returns the requested output without asking for a cross-anchor grant

#### Scenario: Cross-anchor read requires approval

- **WHEN** an agent requests transcript output from a pane in another anchor
- **THEN** the cockpit creates an access request showing the agent, source anchor, target anchor, target pane, scope, and reason
- **AND** the operation waits for user allow or deny

#### Scenario: Denied grant blocks operation

- **WHEN** the user denies an access request
- **THEN** the pending operation fails with an authorization error
- **AND** no transcript, input, rename, or attach side effect is applied

#### Scenario: Timed grant expires

- **WHEN** a timed grant reaches its expiry
- **THEN** subsequent operations using that grant fail until a new grant is approved

#### Scenario: Revoked grant stops future access

- **WHEN** the user revokes an active grant from the UI
- **THEN** later operations that require that grant fail with an authorization error

### Requirement: MCP adapter parity

The MCP adapter SHALL expose tools that call the same bus operations as the CLI.

#### Scenario: MCP tool maps to bus kind

- **WHEN** an MCP client invokes a lmux tool for pane creation, transcript capture, pane input, pane rename, anchor listing, or GUI attach
- **THEN** `lmux-mcp` sends the corresponding `lmux_bus::Kind` to the cockpit
- **AND** returns the cockpit's structured result or error

#### Scenario: MCP has no separate authority

- **WHEN** a cockpit grant is required for the underlying bus operation
- **THEN** the MCP tool waits for the same grant decision as the CLI path
- **AND** cannot bypass cockpit approval

### Requirement: CLI workflow for agents

The CLI SHALL expose the agent-control operations in a scriptable form.

#### Scenario: JSON output is available

- **WHEN** the user or an agent runs an agent-control `lmux-cli` command with `--json`
- **THEN** the command prints a machine-readable response containing stable ids and error fields

#### Scenario: Skill can operate without MCP

- **WHEN** an agent follows the documented lmux CLI workflow without MCP configured
- **THEN** it can discover anchors, create an owned pane, read transcript output, and request sensitive operations through `lmux-cli`
