# bus-ipc

## Purpose

The cockpit exposes a local Unix-socket control bus at
`$XDG_RUNTIME_DIR/lmux/bus.sock`. Frames are length-prefixed JSON envelopes.
`lmux-cli` is the implemented user-facing client. The bus handles read-only
queries on its Tokio thread and forwards cockpit mutations to the GTK main
thread.

## Requirements

### Requirement: Length-prefixed JSON wire format

Every bus message SHALL be framed as a big-endian `u32` length followed by one
JSON body. The maximum accepted body is 4 MiB.

#### Scenario: Partial reads reassemble one frame

- **WHEN** a client writes one frame across multiple writes
- **THEN** `read_frame` reassembles exactly the advertised body bytes

#### Scenario: Oversize frame is rejected

- **WHEN** a frame length is greater than 4 MiB
- **THEN** the parser returns `FrameTooLarge` without allocating the advertised
  body

### Requirement: Versioned envelope and handshake

Every request SHALL carry `v`, `kind`, and `id`. Clients MUST send `hello`
before any non-meta request.

#### Scenario: Handshake succeeds

- **WHEN** a same-UID client connects and sends `hello`
- **THEN** the server replies `hello_ack` with `cockpit_version`
- **AND** later domain requests on the same connection are accepted

#### Scenario: Domain request before hello is rejected

- **WHEN** a client sends any non-`hello` request before handshake completion
- **THEN** the server replies with `error.bad_request`

#### Scenario: Unknown kind is structured

- **WHEN** a request contains an unknown `kind`
- **THEN** the server replies with `error.unknown_kind` and includes
  `kind_received`

### Requirement: Implemented kind schema

The bus SHALL expose the kinds currently defined by `lmux_bus::Kind`.

#### Scenario: Core read kinds are handled on the bus thread

- **WHEN** a client sends `session.list`, `status.get`, or
  `satellite.list_windows`
- **THEN** the bus handler answers without borrowing GTK state except where the
  compositor backend itself is queried

#### Scenario: GTK mutation kinds are deferred

- **WHEN** a client sends `session.new`, `session.rename`, `session.delete`,
  `session.open`, `pane.list`, `anchor.*`, or `satellite.attach_window`
- **THEN** the request is sent to the GTK-thread dispatcher and the bus reply is
  sent after that dispatcher resolves it

#### Scenario: Unimplemented defined kinds fail explicitly

- **WHEN** a defined kind has no dispatcher implementation
- **THEN** the server returns a domain error beginning with `not_implemented`

### Requirement: Peer authentication and socket lifecycle

The bus socket SHALL be same-UID only and SHALL manage stale socket files on
startup.

#### Scenario: Socket is private

- **WHEN** the bus binds `bus.sock`
- **THEN** the socket mode is set to `0600`
- **AND** `bus.sock.pid` is written next to it

#### Scenario: Same UID accepted

- **WHEN** `SO_PEERCRED` reports the same UID as the cockpit
- **THEN** the connection may proceed to handshake

#### Scenario: Cross UID denied

- **WHEN** `SO_PEERCRED` reports a different UID
- **THEN** the server writes `error.peer_denied` and closes the logical request

#### Scenario: Stale socket reclaimed

- **WHEN** `bus.sock` and `bus.sock.pid` exist but the recorded PID is not live
- **THEN** the server removes stale files and binds a fresh socket

### Requirement: `lmux-cli` client

The implemented CLI binary SHALL be `lmux-cli`.

#### Scenario: Session commands map to one bus kind

- **WHEN** the user runs `lmux-cli session list`, `new`, `rename`, `delete`, or
  `open`
- **THEN** the CLI connects to the default bus socket, handshakes, sends the
  matching `session.*` kind, and exits non-zero on bus errors

#### Scenario: Anchor commands use UUIDs

- **WHEN** the user runs `lmux-cli anchor pause|resume|hide|reattach|untag|activate <uuid>`
- **THEN** the UUID is interpreted as an anchor UUID and resolved to the live
  pane on the GTK thread

#### Scenario: Pane list exposes pane UUIDs

- **WHEN** the user runs `lmux-cli pane list`
- **THEN** the CLI prints each live pane UUID, owning anchor UUID when the pane
  belongs to an anchor workspace or terminal tab stack, and best-effort cwd

#### Scenario: Native attach commands are available

- **WHEN** the user runs `lmux-cli satellite list-windows` or
  `satellite attach-window`
- **THEN** the request uses the implemented `satellite.list_windows` and
  `satellite.attach_window` bus kinds

### Requirement: Status snapshot

The bus SHALL expose a compact `status.get.result` snapshot.

#### Scenario: Status fields match implementation

- **WHEN** a client sends `status.get`
- **THEN** the response includes `cockpit_version`, `pid`, `session_count`,
  `anchor_count`, `compositor`, `satellite_spawn_ok`, and
  `satellite_spawn_fail`
