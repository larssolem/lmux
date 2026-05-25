# sessions

## Purpose

lmux has two session persistence layers today:

- a legacy JSON snapshot at `$XDG_DATA_HOME/lmux/last-session.json` used for
  startup restore and shutdown save;
- named TOML sessions under `$XDG_STATE_HOME/lmux/sessions/`, used by
  `lmux-cli session ...` and the fuzzy switcher.

Snapshots persist terminal pane layout, cwd map, and anchor pane ids. Live GUI
satellite panes are stripped from snapshots because they cannot be respawned
cleanly.

## Requirements

### Requirement: Legacy startup/shutdown snapshot

The cockpit SHALL restore and save the legacy JSON snapshot around process
startup/shutdown.

#### Scenario: Missing legacy snapshot starts fresh

- **WHEN** `last-session.json` is absent
- **THEN** lmux starts a fresh terminal session

#### Scenario: Corrupt legacy snapshot is moved aside

- **WHEN** `last-session.json` is malformed or has an unsupported schema version
- **THEN** it is renamed to `.bad.<unix-seconds>` when possible
- **AND** lmux starts fresh

#### Scenario: Shutdown writes current snapshot

- **WHEN** the cockpit shuts down cleanly
- **THEN** it writes the current terminal layout, cwd map, and anchor pane ids
  to the legacy JSON path through the atomic write helper

### Requirement: Snapshot shape

The JSON snapshot SHALL use schema version `1`, a recursive layout tree, a cwd
map, and legacy plus multi-anchor fields.

#### Scenario: Multi-anchor preferred over legacy singleton

- **WHEN** `anchor_pane_ids` is non-empty
- **THEN** readers use that list as the canonical anchor list
- **AND** otherwise fall back to `anchor_pane_id`

#### Scenario: Satellite panes are stripped before save

- **WHEN** `AppState::snapshot()` serializes the layout
- **THEN** live satellite pane leaves are removed from the layout before writing
  the snapshot

### Requirement: Named TOML session store

Named sessions SHALL be persisted under `$XDG_STATE_HOME/lmux/sessions/` or the
HOME fallback.

#### Scenario: Create named session

- **WHEN** `SessionStore::create(name, now)` succeeds
- **THEN** it writes `sessions/<name>.toml` and updates `sessions/index.toml`

#### Scenario: Rename named session

- **WHEN** `SessionStore::rename(from, to)` succeeds
- **THEN** it writes the renamed session file, removes the old file, and updates
  the index entry

#### Scenario: Delete named session

- **WHEN** `SessionStore::delete(name)` is called
- **THEN** the session file is removed if present and the index entry is removed

#### Scenario: List sessions by recency

- **WHEN** `SessionStore::list()` is called
- **THEN** it returns index entries sorted by last-opened time descending

### Requirement: Session name validation

Named sessions SHALL reject unsafe names.

#### Scenario: Valid names

- **WHEN** a name is non-empty, at most 64 chars, does not start with `.`, and
  contains only ASCII alphanumeric characters plus `.`, `_`, and `-`
- **THEN** it is accepted

#### Scenario: Invalid names fail before IO

- **WHEN** a name violates the validation rules
- **THEN** create, rename, delete, or load returns `InvalidName`

### Requirement: Named session switching

The fuzzy switcher and `lmux-cli session open` SHALL swap live terminal panes to
the target named session.

#### Scenario: Switching saves known outgoing session

- **WHEN** `AppState::switch_session(target)` runs and `current_session` is set
- **THEN** lmux snapshots the outgoing tree and saves it to the named TOML
  session before loading the target

#### Scenario: Switching tears down live panes

- **WHEN** a session switch proceeds
- **THEN** live panes are drained and terminated, in-memory pane/anchor/workspace
  maps are reset, and new panes are spawned from the target session or a fresh
  fallback

#### Scenario: Target without snapshot falls back fresh

- **WHEN** the target named session does not load into panes
- **THEN** lmux creates a fresh single-pane session at HOME

### Requirement: Fuzzy switcher

The cockpit SHALL provide a modal fuzzy switcher for named sessions.

#### Scenario: Prefix opens switcher

- **WHEN** the user presses `prefix + s`
- **THEN** a modal "Switch session" window opens, lists known sessions from the
  named store, and focuses the filter entry

#### Scenario: Enter switches selected session

- **WHEN** the user activates a selected session row
- **THEN** lmux calls `AppState::switch_session` with that session name

#### Scenario: Empty state points to CLI

- **WHEN** no named sessions exist
- **THEN** the switcher shows an empty-state message mentioning
  `lmux-cli session new <name>`

### Requirement: Session CLI

The implemented CLI SHALL expose named session CRUD through `lmux-cli session`.

#### Scenario: CLI commands route over bus

- **WHEN** the user runs `lmux-cli session list|new|rename|delete|open`
- **THEN** the CLI connects to the bus and sends the corresponding `session.*`
  kind
