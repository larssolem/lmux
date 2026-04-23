# observability

## Purpose

Structured tracing spans around every user-visible operation, a rotating on-disk log at `$XDG_STATE_HOME/lmux/logs/`, a `lmux status` CLI for live health, and in-memory counters for dogfooding signals (satellite dock success rate). No network telemetry; everything is local.

## Requirements

### Requirement: Structured spans per user-visible operation

The cockpit SHALL open a `tracing` span for every user-visible operation, carrying an id, duration, outcome, and operation-specific fields.

#### Scenario: Core operations emit spans

- **WHEN** the user triggers session-open, session-restore, anchor-pause, anchor-resume, anchor-hide, anchor-reattach, satellite-open, or compositor-reinject
- **THEN** a span is opened with `id`, `duration_ms`, `outcome`, and operation-specific fields; the span closes when the operation resolves

#### Scenario: v0.1 bell-to-toast span preserved

- **WHEN** a terminal bell is converted to a toast
- **THEN** the v0.1 `bell_to_toast` span is emitted with its existing fields; no regression from the v0.1 tracing surface

### Requirement: Rotating on-disk log

The cockpit SHALL write a rotating log to `$XDG_STATE_HOME/lmux/logs/lmux.<date>.log` with daily rotation and at most 5 retained files; `RUST_LOG` overrides the config-file verbosity when both are set.

#### Scenario: Daily rotation caps disk usage

- **WHEN** the cockpit runs across a calendar-day boundary
- **THEN** a new `lmux.<date>.log` is opened for the new day; at most 5 rolled files are retained; older files are pruned automatically

#### Scenario: RUST_LOG overrides config verbosity

- **WHEN** `RUST_LOG` is set in the environment and `[logging].level` is set in config
- **THEN** the effective verbosity is taken from `RUST_LOG`; the config value is used only when `RUST_LOG` is unset

### Requirement: `lmux status` CLI

The `lmux status` subcommand SHALL report cockpit uptime and version, compositor integration health, bus accept count, active pane count, active anchor count, active satellite count, and satellite spawn-success counters; `--json` emits machine-readable output.

#### Scenario: Status reports the baseline fields

- **WHEN** the user runs `lmux status` against a running cockpit
- **THEN** the output includes: `cockpit_version`, uptime, compositor state, bus accept count, active panes, active anchors, active satellites, and `satellites: ok=N fail=M`

#### Scenario: JSON output is machine-readable

- **WHEN** the user runs `lmux status --json`
- **THEN** the output is a single JSON document whose keys correspond to the pretty-printed fields, suitable for `jq` piping

#### Scenario: Missing cockpit yields non-zero exit

- **WHEN** the user runs `lmux status` with no cockpit process running
- **THEN** the CLI exits non-zero with a stderr message identifying the absent cockpit

### Requirement: Satellite spawn counters

The cockpit SHALL maintain atomic counters `satellite_spawn_ok` and `satellite_spawn_fail` that are incremented on every resolved `satellite.open` dispatch, MUST expose them through `status.get.result`, and MUST emit a tracing event on each spawn outcome.

#### Scenario: Counters increment per outcome

- **WHEN** a `satellite.open` dispatch resolves to success
- **THEN** `satellite_spawn_ok` increments by 1; on failure `satellite_spawn_fail` increments by 1

#### Scenario: Tracing event mirrors the outcome

- **WHEN** a spawn resolves
- **THEN** a `tracing` event is emitted carrying the request id, outcome, and (on failure) the reason

### Requirement: No network telemetry in v0.2

The cockpit SHALL NOT open any network sockets for telemetry, analytics, or crash reporting; observability MUST remain strictly local.

#### Scenario: No listening or outbound sockets from the cockpit

- **WHEN** `ss -lntp` and `ss -ntp` are run against the cockpit PID under normal operation
- **THEN** no listening TCP sockets and no outbound network connections are attributable to the cockpit process for observability purposes

### Requirement: Error messages name the recovery action

Every error surfaced to the user SHALL either name its recovery action inline or provide an expand affordance that reveals the recovery action.

#### Scenario: Compositor-offline toast offers re-inject

- **WHEN** the cockpit emits the `compositor.status { state: "offline" }` toast
- **THEN** the toast or its detail expansion names the "Re-inject" action and the CLI equivalent `lmux compositor reinject`

#### Scenario: Satellite fallback names the fallback explicitly

- **WHEN** a satellite falls back to floating because correlation timed out
- **THEN** the toast reads `Could not dock <app> — opened as floating window` and does not hide the cause behind a generic "error" phrasing
