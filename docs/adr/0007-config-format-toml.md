# ADR-0007: Config format — TOML

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.1

## Context

lmux needs a config file for keybinds, colours, sandbox template overrides, and eventually pane-layout hints. The brainstorm left "TOML vs KDL" open; both are pragmatic, both have Rust tooling.

Decision criteria:

- How familiar is the format to Linux devs (the target audience)?
- How shallow/deep is our config — does nesting matter?
- How mature is the Rust tooling?
- How much editor tooling (syntax highlighting, formatters) ships by default across IDEs?

## Decision

**TOML** is the config format. Config files live under `$XDG_CONFIG_HOME/lmux/` (default `~/.config/lmux/`), starting with `config.toml` and `keybinds.toml`.

Use `toml` + `serde` for parsing. All config fields are optional and have defaults in code.

## Alternatives considered

- **KDL.** Nicer syntax for nested node-style structures (think: layout trees). Rejected: lmux config is shallow (flat keybinds, flat colours, flat template overrides). KDL's nesting advantage is unrealized, and TOML is vastly more familiar to Linux devs. `kdl-rs` is solid but less universally supported by editor tooling.
- **YAML.** Rejected: the community has repeatedly rejected YAML for devtool config (indent bugs, implicit typing); anti-pattern.
- **JSON.** Rejected: no comments; hostile for hand-editing.
- **Lua / Fennel / custom DSL.** Rejected: massive scope creep for a config file; conflicts with "pragmatic, low-magic" taste filter.
- **Ron.** Rejected: Rust-ecosystem-only; bad for external tools and plugin authors.

## Consequences

- **+** Familiar to every Cargo user; zero onboarding friction.
- **+** `serde` + `toml` is the most battle-tested path in Rust.
- **+** Config can be read by non-lmux tools (IDE plugins, `awk`, `yq-via-toml`) trivially.
- **−** Deeper structures (e.g. a layout tree) will be awkward in TOML. Mitigation: if/when that arises, nest as arrays-of-tables or switch the *one* section to a side file.
- **−** KDL would have been slightly nicer for the active-tier plugin protocol config (if it ever grows nesting). Acceptable: the plugin protocol has its own spec (not user config).

## Follow-up

- Document config precedence: built-in defaults → system config → user config → env overrides.
- Keep config surface small through v0.3; resist adding config for every behaviour.
