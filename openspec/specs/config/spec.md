# config

## Purpose

User configuration lives at `$XDG_CONFIG_HOME/lmux/config.toml` with fallback
to `$HOME/.config/lmux/config.toml`. The current implemented schema is small:
`[general]`, `[keymap]`, `[sidebar]`, and repeated `[[autodetect]]` rules. The
cockpit provisions a default file, watches it when present, and applies safe
runtime settings without restarting panes.

## Requirements

### Requirement: Implemented TOML schema

The config loader SHALL accept the implemented sections `[general]`, `[keymap]`,
`[sidebar]`, and `[[autodetect]]`.

#### Scenario: Missing config loads defaults

- **WHEN** `lmux_config::load(path)` is called and `path` does not exist
- **THEN** it returns `Config::default()` and does not write a file

#### Scenario: First-run provisioning writes defaults

- **WHEN** `lmux_config::load_or_provision(path)` is called and `path` does not
  exist
- **THEN** it creates parent directories as needed, writes a complete default
  TOML file, and returns `ProvisionOutcome::Provisioned`

#### Scenario: Invalid TOML is an error

- **WHEN** the config file is syntactically invalid TOML
- **THEN** loading returns `LoadError::InvalidToml` with the path and parser
  error

### Requirement: General settings

The `[general]` section SHALL configure font family, font size, optional
compositor script path, and pointer focus mode.

#### Scenario: Font settings apply live

- **WHEN** config is applied at startup or after a watch reload
- **THEN** every live pane receives the configured font family and font size

#### Scenario: Focus mode applies live

- **WHEN** `[general].focus_mode` changes to `click` or `hover`
- **THEN** the shared focus-mode cell used by pane controllers is updated
  without recreating panes

### Requirement: Prefix-only keymap configuration

The `[keymap]` section SHALL currently expose only the prefix key. Individual
follower chords are not user-configurable.

#### Scenario: Prefix override takes effect

- **WHEN** `[keymap] prefix = "ctrl+shift+k"` is loaded
- **THEN** that chord arms the prefix dispatcher and the old prefix no longer
  does

#### Scenario: Invalid prefix in settings dialog is rejected

- **WHEN** the user tries to save an invalid prefix through the settings dialog
- **THEN** the dialog keeps focus on the prefix entry and shows a validation
  error

### Requirement: Sidebar configuration

The `[sidebar]` section SHALL configure sidebar side, expanded width, collapsed
width, collapsed initial state, preview enablement, preview refresh interval,
and default sort mode.

#### Scenario: Sidebar config affects installed sidebar

- **WHEN** the cockpit installs the sidebar
- **THEN** it uses the configured side, width, collapsed rail width, collapsed
  state, preview enablement, and preview refresh interval

#### Scenario: Sidebar may collapse and hover-expand

- **WHEN** the user clicks the collapse button
- **THEN** the sidebar switches between configured expanded width and collapsed
  rail width
- **AND** hovering the collapsed rail temporarily expands it

### Requirement: File-watch reload

When a config file exists at startup, the cockpit SHALL watch its parent
directory and apply debounced changes on the GTK main loop.

#### Scenario: Directory watch survives atomic writes

- **WHEN** an editor saves `config.toml` via stage-and-rename
- **THEN** the parent-directory watcher sees the change and reloads the config
  after the 150 ms debounce window

#### Scenario: Reload keeps live panes

- **WHEN** a valid watched config change is applied
- **THEN** live panes, PTYs, anchors, sessions, and attached windows remain
  alive
- **AND** runtime-applicable settings are updated in place

#### Scenario: Reload failure logs and preserves prior state

- **WHEN** watched reload fails due to IO or TOML parse error
- **THEN** the cockpit logs a warning and keeps the prior in-memory config

### Requirement: Autodetect rule data model

The config SHALL parse repeated `[[autodetect]]` rules with a `name`, a
`match` table, and `hide_on_session_close`.

#### Scenario: Command substring match

- **WHEN** an autodetect rule contains
  `match = { command_contains = ["cargo test"] }`
- **THEN** the rule matches a command line containing `cargo test`

#### Scenario: Environment variable match

- **WHEN** an autodetect rule contains `match = { env_set = ["LMUX_ANCHOR"] }`
- **THEN** the rule matches a pane environment containing `LMUX_ANCHOR`
