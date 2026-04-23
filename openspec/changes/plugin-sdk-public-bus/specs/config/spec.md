## ADDED Requirements

### Requirement: Bus public-mode opt-in

The config schema SHALL support a `[bus].public` boolean (default `false`) that, when `true`, opens an abstract-namespace Unix socket at `@lmux-public-<uid>` alongside the filesystem bus socket for token-gated non-same-UID clients.

#### Scenario: Default is false

- **WHEN** a user upgrades a v0.2 config that has no `[bus]` section
- **THEN** `[bus].public` evaluates to `false`; only the filesystem bus socket is bound; behaviour matches v0.2

#### Scenario: Enabling public mode binds the abstract socket

- **WHEN** the user writes `[bus] public = true` and reloads the config
- **THEN** the cockpit binds `@lmux-public-<uid>`, logs the bind at INFO, and surfaces `bus.public = true` in `lmux status`

#### Scenario: Public mode toggles without restart

- **WHEN** the user flips `[bus].public` from `true` to `false` in a live config
- **THEN** the cockpit unbinds the abstract socket on the next hot-reload; in-flight public connections are closed with a reason toast; filesystem-socket connections are unaffected
