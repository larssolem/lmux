## ADDED Requirements

### Requirement: Platform-Neutral Window Candidate Payload

The bus SHALL expose a platform-neutral window candidate payload for native window attachment.

#### Scenario: List windows returns platform-neutral candidates

- **WHEN** a client sends `satellite.list_windows`
- **THEN** the response contains window candidates with backend, backend window id, optional pid, optional app identity, optional title, and optional workspace/output metadata

#### Scenario: Unsupported backend response

- **WHEN** a client sends `satellite.list_windows` and the active backend cannot list windows
- **THEN** the server returns a domain error or empty unsupported result that clearly indicates attach is unavailable for the current environment

### Requirement: Platform-Neutral Attach Window Request

The bus SHALL allow clients to attach a selected native window using the platform-neutral candidate identity.

#### Scenario: Attach selected candidate

- **WHEN** a client sends `satellite.attach_window` with a backend and backend window id from a prior list response
- **THEN** the cockpit registers that exact window under the active anchor
- **AND** replies with `ok { of: "satellite.attach_window" }`

#### Scenario: Attach unsupported backend

- **WHEN** a client sends `satellite.attach_window` for a backend that is not supported by the active compositor control
- **THEN** the cockpit returns a domain error
- **AND** no satellite registration is created

## MODIFIED Requirements

### Requirement: Frozen kind schema

The bus SHALL expose the kind schema defined in ADR-0016: request/response kinds under `session.*`, `anchor.*`, `satellite.*`, `compositor.*`, plus meta kinds `hello`, `hello_ack`, `status.get`, generic `ok`, and `error.*`; unknown kinds are rejected deterministically. The `satellite.list_windows` and `satellite.attach_window` kinds SHALL be platform-neutral and available to any backend that reports attach support.

#### Scenario: Every frozen kind round-trips through serde

- **WHEN** `cargo test -p lmux-bus` runs the kinds module round-trip suite
- **THEN** each defined kind serializes to JSON and deserializes back to an equal value; deny-list tests confirm unknown kinds fail to deserialize cleanly

#### Scenario: Required domain kinds are present

- **WHEN** the bus server initializes
- **THEN** it handles at minimum: `session.list`, `session.new`, `session.rename`, `session.delete`, `session.open`, `anchor.tag`, `anchor.untag`, `anchor.pause`, `anchor.resume`, `anchor.hide`, `anchor.reattach`, `satellite.open`, `satellite.list_windows`, `satellite.attach_window`, `compositor.status`, `status.get`
