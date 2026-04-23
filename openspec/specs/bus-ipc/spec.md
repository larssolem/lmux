# bus-ipc

## Purpose

Internal control plane for the cockpit: a Unix-socket bus at `$XDG_RUNTIME_DIR/lmux/bus.sock` speaking length-prefixed JSON with a versioned envelope. It lets a thin CLI client drive the running cockpit for session CRUD, `satellite.open`, anchor lifecycle, compositor ops, and status queries. Peer-authenticated to the cockpit UID, robust against stale sockets and malformed payloads.

## Requirements

### Requirement: Length-prefixed JSON wire format

The bus SHALL frame every message as a big-endian `u32` length followed by a JSON body; the maximum accepted frame is 4 MiB and oversized frames MUST be rejected without crashing the parser.

#### Scenario: Valid frame round-trips across partial reads

- **WHEN** a client writes a frame of length N split across multiple `write` syscalls
- **THEN** the server's reader reassembles exactly N bytes and decodes the JSON body; no frame boundaries are lost

#### Scenario: Oversize frame is rejected

- **WHEN** a client sends a length header greater than 4 MiB
- **THEN** the server closes the connection with `error.frame_too_large` and does not attempt to allocate the advertised buffer

#### Scenario: Fuzz harness exists and passes

- **WHEN** `cargo fuzz run parse_frame` is invoked against `crates/lmux-bus/fuzz/fuzz_targets/parse_frame.rs`
- **THEN** a 30-second corpus run completes without any crash, panic, or out-of-memory termination

### Requirement: Versioned envelope with explicit handshake

Every bus message SHALL carry `{"v": 2, "kind": "...", "id": "<uuid>"}`, and a client MUST complete a `hello` → `hello_ack` handshake before any domain kind is dispatched.

#### Scenario: Handshake succeeds on matching version

- **WHEN** a client opens a connection and sends `hello`
- **THEN** the server replies `hello_ack` with `cockpit_version` and the connection becomes ready for domain kinds

#### Scenario: Version mismatch is rejected

- **WHEN** a client sends a frame with `v` other than `2`
- **THEN** the server responds with `error.version_mismatch` and closes the connection before dispatching the payload

#### Scenario: Unknown kinds are rejected

- **WHEN** a client sends a frame whose `kind` is not present in the frozen schema
- **THEN** the server responds with `error.unknown_kind` including a `kind_received` field

### Requirement: Frozen kind schema

The bus SHALL expose the kind schema defined in ADR-0016: request/response kinds under `session.*`, `anchor.*`, `satellite.*`, `compositor.*`, plus meta kinds `hello`, `hello_ack`, `status.get`, generic `ok`, and `error.*`; unknown kinds are rejected deterministically.

#### Scenario: Every frozen kind round-trips through serde

- **WHEN** `cargo test -p lmux-bus` runs the kinds module round-trip suite
- **THEN** each defined kind serializes to JSON and deserializes back to an equal value; deny-list tests confirm unknown kinds fail to deserialize cleanly

#### Scenario: Required domain kinds are present

- **WHEN** the bus server initializes
- **THEN** it handles at minimum: `session.list`, `session.new`, `session.rename`, `session.delete`, `session.open`, `anchor.tag`, `anchor.untag`, `anchor.pause`, `anchor.resume`, `anchor.hide`, `anchor.reattach`, `satellite.open`, `compositor.status`, `status.get`

### Requirement: Peer authentication via SO_PEERCRED

The bus socket SHALL be created with mode `0600` and MUST reject any connecting client whose UID does not match the cockpit's UID.

#### Scenario: Same-UID connection is accepted

- **WHEN** a client process running under the cockpit's UID connects to `bus.sock`
- **THEN** the connection completes and the handshake proceeds

#### Scenario: Cross-UID connection is denied

- **WHEN** a client process running under a different UID connects
- **THEN** the server reads `SO_PEERCRED`, sees the UID mismatch, and closes the connection with `error.peer_denied`

#### Scenario: Socket file mode is 0600

- **WHEN** `ls -l $XDG_RUNTIME_DIR/lmux/bus.sock` is run while the cockpit is up
- **THEN** the mode bits show `srw-------`

### Requirement: Socket lifecycle and stale-socket recovery

The cockpit SHALL create `bus.sock` and a companion `bus.sock.pid` on startup, remove both on clean exit, and reclaim stale files on crash-restart without manual cleanup.

#### Scenario: Clean startup creates fresh socket

- **WHEN** the cockpit starts with no pre-existing bus files
- **THEN** it creates `$XDG_RUNTIME_DIR/lmux/bus.sock` (mode 0600) and `bus.sock.pid` containing its own PID

#### Scenario: Stale socket is reclaimed

- **WHEN** the cockpit starts and finds `bus.sock` + `bus.sock.pid` with a PID that no longer exists
- **THEN** it unlinks both files and creates fresh ones, logging the reclaim at INFO

#### Scenario: Refuse to start against a live cockpit

- **WHEN** the cockpit starts and `bus.sock.pid` references a PID still running
- **THEN** it exits with a non-zero status and a clear diagnostic naming the other PID, leaving the existing cockpit undisturbed

### Requirement: Thin CLI client

The `lmux` binary SHALL expose a thin CLI that connects to the bus, performs the handshake, and implements every user-facing subcommand in terms of exactly one bus kind.

#### Scenario: `lmux session list` round-trip

- **WHEN** the user runs `lmux session list`
- **THEN** the CLI connects to `bus.sock`, handshakes, sends `session.list`, and pretty-prints a table of sessions; exit code is `0` on success

#### Scenario: Machine-readable output

- **WHEN** the user runs `lmux session list --json`
- **THEN** the CLI emits a JSON document containing the server's response suitable for piping into `jq` or other shell tooling

#### Scenario: Missing cockpit yields a clear error

- **WHEN** the user runs any non-launch `lmux` subcommand with no cockpit process running
- **THEN** the CLI exits non-zero with a stderr message identifying the absent cockpit and pointing at the expected socket path

### Requirement: Hostile input safety

The bus SHALL never panic, crash, or leak unbounded memory from malformed input at any frame size up to the declared maximum.

#### Scenario: Malformed JSON is rejected per message

- **WHEN** a connected client sends a syntactically valid frame containing invalid JSON
- **THEN** the server responds with `error.malformed_body`, keeps the connection open, and awaits the next frame

#### Scenario: Schema-violating payloads are rejected

- **WHEN** a client sends JSON whose shape does not match the declared kind
- **THEN** the server responds with `error.schema_violation` naming the failing field; no dispatch occurs
