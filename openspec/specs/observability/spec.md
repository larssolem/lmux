# observability

## Purpose

Observability is local-only. The cockpit logs structured tracing to stderr and
to rotating files under `$XDG_STATE_HOME/lmux/logs/`, exposes a compact
`status.get` bus snapshot, records satellite spawn counters, and emits spans for
latency-sensitive operations.

## Requirements

### Requirement: Tracing setup

The cockpit SHALL initialize `tracing_subscriber` with an environment filter,
stderr logging, and an optional daily rolling file appender.

#### Scenario: RUST_LOG controls filtering

- **WHEN** `RUST_LOG` is set
- **THEN** the tracing filter is taken from the environment
- **AND** otherwise defaults to `info,lmux=info`

#### Scenario: File logs rotate locally

- **WHEN** `$XDG_STATE_HOME` or `$HOME` can resolve a state directory
- **THEN** lmux writes daily rotating logs under `$XDG_STATE_HOME/lmux/logs/`
  or `$HOME/.local/state/lmux/logs/`
- **AND** retains at most five files through the appender configuration

#### Scenario: Panic hook logs backtrace

- **WHEN** a panic reaches the installed hook
- **THEN** lmux logs the panic info and a forced backtrace before delegating to
  the default hook

### Requirement: Startup banner

The cockpit SHALL log a startup banner with useful environment fields.

#### Scenario: Startup metadata is logged

- **WHEN** the cockpit starts
- **THEN** it logs lmux version, libghostty version, current desktop,
  runtime dir, state dir, and data dir

### Requirement: Status snapshot

The implemented live status surface SHALL be `status.get` on the bus and
`lmux-cli status` on the CLI.

#### Scenario: Status reports implemented fields

- **WHEN** a client requests status
- **THEN** the snapshot includes cockpit version, PID, session count, anchor
  count, compositor online/offline state, `satellite_spawn_ok`, and
  `satellite_spawn_fail`

#### Scenario: Compositor health is probed live

- **WHEN** `status.get` is handled
- **THEN** the bus asks the current compositor backend for health before
  building the response

### Requirement: Satellite spawn counters

The cockpit SHALL maintain process-local counters for the legacy
`satellite.open` spawn path.

#### Scenario: Successful non-macOS spawn increments ok

- **WHEN** `satellite.open` launches a process successfully on a non-macOS
  build
- **THEN** `satellite_spawn_ok` increments
- **AND** the log says the process was launched without ownership and must be
  attached separately to be managed

#### Scenario: Failed or disabled spawn increments fail

- **WHEN** `satellite.open` receives empty argv, fails to spawn, or is called on
  macOS
- **THEN** `satellite_spawn_fail` increments

### Requirement: Bell notification span

The cockpit SHALL keep the `bell_to_toast` span around notification delivery.

#### Scenario: Bell delivery is measured

- **WHEN** a pane bell is converted to a desktop notification
- **THEN** the async notification call is wrapped in a `bell_to_toast` tracing
  span tagged with the pane id

### Requirement: Anchor switch latency logging

Anchor switches SHALL log local switch duration and compositor bridge duration.

#### Scenario: Local anchor switch logs duration

- **WHEN** `set_active_anchor` changes the active anchor
- **THEN** it logs `operation = "anchor.switch.local"`, `duration_ms`, target,
  active anchor, pane count, anchor count, satellite-window count, and whether a
  GTK rebuild happened

#### Scenario: Compositor group switch logs duration

- **WHEN** the compositor bridge applies a group switch
- **THEN** it logs `operation = "compositor.group_switch"`, duration, sequence,
  hide/show counts, attempted windows, and failure count

### Requirement: No network telemetry

lmux SHALL NOT implement analytics, crash upload, or network telemetry.

#### Scenario: Observability stays local

- **WHEN** lmux records logs, counters, status, spans, or notifications
- **THEN** the data remains on the local machine or local desktop session bus
