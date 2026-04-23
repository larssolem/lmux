# config

## Purpose

User configuration lives in `~/.config/lmux/config.toml` with a shipped default, documented schema, and hot-reload semantics. The user overrides the prefix key, chord bindings, anchor auto-detect patterns, sandbox toggle, and UI defaults without a restart; pane state, PTYs, and docked satellites survive a reload.

## Requirements

### Requirement: TOML schema with documented sections

The cockpit SHALL read `$XDG_CONFIG_HOME/lmux/config.toml` at startup and accept at minimum the sections `[keybindings]`, `[anchors]`, `[satellites]`, `[sandbox]`, and `[ui]`; a missing file is equivalent to the shipped default.

#### Scenario: Missing config is equivalent to defaults

- **WHEN** the cockpit starts and no `config.toml` exists in `$XDG_CONFIG_HOME/lmux/`
- **THEN** the cockpit operates with the shipped default values for every schema key and does not error out

#### Scenario: Unknown keys are surfaced, not fatal

- **WHEN** the config contains a key outside the documented schema
- **THEN** the cockpit logs a warning naming the unrecognised key and proceeds using defaults for the rest of the schema

### Requirement: First-run provisioning

On a first launch with no existing config the cockpit SHALL write a commented default `config.toml`, mark the user as onboarded via `$XDG_STATE_HOME/lmux/onboarded.v0.2`, and surface a one-time onboarding toast pointing at the path.

#### Scenario: First launch writes a default config

- **WHEN** the cockpit starts with no `config.toml` and no `onboarded.v0.2` marker
- **THEN** a commented default config is written to `~/.config/lmux/config.toml`, the marker file is created, and a sidebar toast points at the config path with an "open in editor" action

#### Scenario: Subsequent launches do not re-provision

- **WHEN** the cockpit starts with the `onboarded.v0.2` marker present
- **THEN** the default config is not rewritten and no onboarding toast appears, even if the user has since deleted the config file

### Requirement: Keybinding overrides

The user SHALL be able to override the prefix key and individual chord mappings via `[keybindings]`; invalid binding strings MUST be rejected at load time with a descriptive error surfaced as a toast.

#### Scenario: Prefix override takes effect on load

- **WHEN** `[keybindings] prefix = "ctrl+space"` is set and the cockpit (re)loads config
- **THEN** `ctrl+space` arms the prefix dispatcher; the previous prefix binding is released

#### Scenario: Chord overrides are independent

- **WHEN** individual chord overrides (for example `switcher = "prefix+s"`) are present
- **THEN** each override takes effect on load without requiring other chords to be present in the file

#### Scenario: Invalid binding surfaces a toast

- **WHEN** the config contains a keybinding string that fails to parse
- **THEN** the load aborts for that specific key, the prior value (or default) is retained, and a sidebar toast names the failing key and reason

### Requirement: Hot-reload preserves live state

The user SHALL be able to trigger a config hot-reload from within the cockpit (default `prefix + C`, or `lmux config reload`); applying a reload MUST NOT kill pane PTYs, docked satellites, or mutate session state.

#### Scenario: Reload applies without killing panes

- **WHEN** the user triggers a reload and the new config is valid
- **THEN** keybindings, anchor patterns, sidebar toggle, sandbox policy, and font are re-applied live; all open PTYs and docked satellites continue running

#### Scenario: Reload failure surfaces a toast

- **WHEN** a reload attempt parses invalid TOML or fails schema validation
- **THEN** the previous config remains in force, and a toast shows the parse error or the failing key

#### Scenario: Font propagates to every pane

- **WHEN** `[ui] font` changes across a reload
- **THEN** every pane calls `Pane::set_font(family, size)`, re-measures cell metrics, and redraws at the new size without losing scrollback

### Requirement: File-watch driven reload

The cockpit SHALL watch the parent directory of `config.toml` with a `notify` recursive watcher, debounced at 150 ms, and apply any on-disk change through the same reload path as the explicit trigger.

#### Scenario: Editing the file applies a reload

- **WHEN** the user saves an edit to `config.toml` from any editor
- **THEN** within 150 ms of the write settling, the cockpit re-reads and re-applies the config, with the same success/failure toasts as the explicit trigger

#### Scenario: Watcher survives atomic-write editors

- **WHEN** an editor writes `config.toml` via stage + rename (vim, Neovim default)
- **THEN** the watcher still fires and the reload applies; watching the parent directory (not the file directly) is what makes this work

### Requirement: User-extensible anchor patterns

The `[anchors].auto_detect_patterns` array SHALL be a user-extensible list; the effective pattern set is the union of built-in and user patterns and reloads when the config reloads.

#### Scenario: New user pattern triggers auto-tag

- **WHEN** the user adds `"pnpm dev"` to `auto_detect_patterns` and the config reloads
- **THEN** subsequent panes spawning `pnpm dev` auto-tag as anchors

#### Scenario: Removing a pattern does not untag existing anchors

- **WHEN** the user removes a pattern from `auto_detect_patterns`
- **THEN** future panes matching that pattern are not auto-tagged; panes already tagged under the pattern remain tagged

### Requirement: Keyboard-layout-respecting defaults

Default keybindings SHALL respect a non-US keyboard layout (specifically Norwegian): primary bindings MUST use unshifted alphanumeric keys, not US-centric shifted symbols like `|` or `\`.

#### Scenario: Defaults use unshifted keys

- **WHEN** the cockpit boots with the shipped defaults
- **THEN** the primary chord bindings use keys reachable without `Shift` on a Norwegian keyboard (letters and `+`/`-`, not US-shifted symbols)
