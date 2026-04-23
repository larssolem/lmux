# sessions

## Purpose

Named, persistent sessions as first-class domain objects: each session owns a pane tree, per-pane cwds, anchor metadata, and a last-opened timestamp. Sessions are created, renamed, deleted, listed, and restored atomically from disk. The fuzzy switcher gives keyboard-only navigation between sessions and recently-closed panes.

## Requirements

### Requirement: Session domain model

The cockpit SHALL represent each session as a serializable value containing at minimum a name, creation and last-opened timestamps, a pane tree (with splits, focus, per-pane cwd), and anchor references.

#### Scenario: Session value round-trips through serde

- **WHEN** a `Session` value is serialized to TOML and deserialized back
- **THEN** the resulting value is structurally equal to the original, including pane tree shape, focus index, cwds, and anchor references

#### Scenario: Session index tracks recency

- **WHEN** `SessionIndex` is queried
- **THEN** it returns session entries sorted by `last_opened_at` descending, making the most-recently-used session the first candidate for the fuzzy switcher

### Requirement: Session CRUD from UI and CLI

The cockpit SHALL let the user create, rename, and delete sessions both from the in-process UI and from the `lmux session` CLI subcommand routed over the bus.

#### Scenario: Create a new named session

- **WHEN** the user runs `lmux session new client-acme` against a running cockpit
- **THEN** a new session named `client-acme` is created in the store, added to `SessionIndex`, and becomes available to the fuzzy switcher on next open

#### Scenario: Rename an existing session

- **WHEN** the user runs `lmux session rename client-acme acme` (or renames via the sidebar)
- **THEN** both the session's TOML file and its `SessionIndex` entry are renamed atomically; existing references to the old name fail with a clear "not found" error

#### Scenario: Delete a session

- **WHEN** the user runs `lmux session delete acme` (or triggers delete from the sidebar)
- **THEN** the TOML file and the `SessionIndex` entry are removed atomically; if the deleted session was currently active, the cockpit falls back to an empty unnamed session

#### Scenario: List all sessions

- **WHEN** the user runs `lmux session list` (with or without `--json`)
- **THEN** the CLI prints every known session in recency order, with name, last-opened timestamp, and active-session marker; `--json` emits machine-readable fields suitable for shell scripting

### Requirement: Atomic TOML persistence

The cockpit SHALL persist every session state change atomically under `$XDG_STATE_HOME/lmux/sessions/` using stage → `fsync` → rename, with file mode `0600`.

#### Scenario: Atomic write survives power loss mid-operation

- **WHEN** the cockpit saves a session and a simulated crash occurs between stage and rename
- **THEN** the previously committed file remains intact and readable; on next launch the cockpit opens the last good version

#### Scenario: Session files are user-private

- **WHEN** a session file is created under `$XDG_STATE_HOME/lmux/sessions/<name>.toml`
- **THEN** its mode is `0600` and its index entry in `sessions/index.toml` is updated in the same atomic manner

#### Scenario: Corruption never blocks cockpit startup

- **WHEN** the cockpit starts and finds a malformed session TOML file
- **THEN** it logs a warning and opens that session as empty (same name, empty pane tree); cockpit startup proceeds normally

### Requirement: Restore active session on launch

The cockpit SHALL restore the active session on launch by default, honoring the v0.1 `last-session.json` contract, with explicit `--session <name>` and "no session" overrides.

#### Scenario: Default launch restores last-active session

- **WHEN** the user runs `lmux` without arguments
- **THEN** the cockpit reads `last-session.json`; if it names a session, that session's pane tree and cwds are restored within 2 s of launch for sessions of up to 20 panes

#### Scenario: Explicit session selection

- **WHEN** the user runs `lmux --session client-acme`
- **THEN** the cockpit opens `client-acme` regardless of what `last-session.json` points at

#### Scenario: Missing last-session yields empty session

- **WHEN** the cockpit launches and no `last-session.json` exists
- **THEN** it opens an empty, unnamed session without erroring

#### Scenario: Graceful shutdown persists active session

- **WHEN** the cockpit shuts down cleanly
- **THEN** the active session's pane tree is written to its session TOML and `last-session.json` is rewritten to point at it

### Requirement: Migration from v0.1 last-session.json

The cockpit SHALL migrate an existing v0.1 `last-session.json` (pane tree without session name) to a named session on first v0.2 launch, idempotently.

#### Scenario: v0.1 state becomes `default` session

- **WHEN** a v0.1 user first runs v0.2 with a `last-session.json` containing an unnamed pane tree
- **THEN** the cockpit writes `sessions/default.toml`, rewrites `last-session.json` to reference `"default"`, and logs the migration at INFO level

#### Scenario: Re-running migration is a no-op

- **WHEN** the cockpit starts and `sessions/default.toml` already exists
- **THEN** no migration is performed and existing state is preserved

### Requirement: Fuzzy switcher opens within 50 ms

The cockpit SHALL provide a keyboard-only fuzzy switcher overlay, bound by default to `prefix + s`, that lists every session plus recently-closed panes and opens within 50 ms of the keybinding.

#### Scenario: Switcher opens on prefix chord

- **WHEN** the user presses `prefix + s`
- **THEN** a modal overlay appears centered in the cockpit window within 50 ms
- **AND** focus is captured by the overlay input; keystrokes do not reach the underlying pane until the overlay closes

#### Scenario: Filter latency stays under 16 ms

- **WHEN** the user types a character in the switcher with up to 50 sessions and 200 recent-pane entries loaded
- **THEN** the filtered list updates within 16 ms of the keystroke, ranked by substring match combined with recency

#### Scenario: Keyboard navigation and dismissal

- **WHEN** the switcher is open
- **THEN** `Enter` opens the top-ranked entry, arrow keys and `Tab` navigate between entries, and `Esc` closes the switcher without any session change

### Requirement: Switcher swap saves outgoing state

The cockpit SHALL save the outgoing session's state before swapping to the target session selected in the switcher; no pane state is lost in the transition.

#### Scenario: Session swap preserves outgoing state

- **WHEN** the user selects session `B` while session `A` is active with modified state
- **THEN** the cockpit saves `A`'s snapshot via the atomic store, tears down `A`'s live panes, and rehydrates `B` from its on-disk snapshot (or a fresh `$HOME` shell if `B` has no snapshot yet)
- **AND** the sidebar highlight, active-session marker, and tab-edge glow update to reflect `B`
