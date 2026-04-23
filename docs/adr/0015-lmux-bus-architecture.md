# ADR-0015: lmux-bus architecture — one cockpit-wide socket, versioned protocol

- Status: Accepted
- Date: 2026-04-22
- Deciders: Lars
- Blocks: v0.2 (implementation), v0.3 (plugin SDK extends this surface)
- Supersedes (in part): ADR-0008 (per-session socket sketch — still authoritative for *transport choice* Unix socket + JSON; this ADR narrows the *topology* to a single cockpit-wide socket)

## Context

ADR-0008 picked a Unix domain socket with line-delimited JSON as the bus transport. It proposed a per-session socket at `$XDG_RUNTIME_DIR/lmux/<session-id>.sock`. That was drafted before the v0.2 cockpit model crystalized — when the mental model was still "one lmux process per session."

v0.2 makes the cockpit singular: one lmux process hosts *all* sessions. The per-session socket proliferation no longer matches the process model: one socket owner, many logical sessions inside it.

We also hit two protocol-level gaps when we started drafting the v0.2 event set (ADR-0016):

1. **Framing.** Line-delimited JSON is fragile once payloads include anchor scrollback, config dumps, or KWin script snippets — any embedded newline corrupts the stream.
2. **Version rejection.** FR59 (reject incompatible clients) is easy to forget when version is just a field; making it a first-class envelope concern turns version mismatch into a parser-level invariant.

## Decision

1. **One cockpit-wide Unix socket** at `$XDG_RUNTIME_DIR/lmux/bus.sock`. The cockpit process is the server; the CLI (`lmux <subcmd>`), the KWin script bridge, and (in v0.3) plugin clients connect as clients.
2. **Length-prefixed JSON framing.** Each frame is `u32-be length || JSON bytes`. Max frame: 4 MiB (rejected with `error.frame_too_large`). No embedded-newline hazards, no partial-line corner cases.
3. **Mandatory version envelope.** Every message carries `{"v": 2, "kind": "...", "id": "<uuid>"}` plus kind-specific payload. Any frame without `v: 2` (including `v: 1` from hypothetical pre-v0.2 clients) is rejected before the `kind` dispatcher runs.
4. **Handshake.** Client opens connection → sends `{"v": 2, "kind": "hello", "client": "lmux-cli|kwin-script|satellite", "pid": <n>}`. Server replies `{"v": 2, "kind": "hello_ack", "cockpit_version": "<semver>"}` or closes with an error envelope on incompatible clients.
5. **Peer authentication.** Server obtains peer UID via `SO_PEERCRED` on accept; rejects connections whose UID ≠ cockpit UID. Socket file mode 0600. No token handshake in v0.2.
6. **Stale-socket recovery.** On cockpit startup, if `bus.sock` already exists, check `bus.sock.pid` (atomically written adjacent file). If the PID is not a live cockpit process, unlink both and recreate. Otherwise refuse to start with a clear error.

## Alternatives considered

- **Keep per-session sockets (ADR-0008 as drafted).** Rejected: sessions are logical inside one cockpit process; multiple sockets add connection-management cost without benefit. Discoverability also suffers: a client has to know which session is active before connecting, but the "which session is active" state lives on the bus.
- **D-Bus.** Rejected (reaffirms ADR-0008): heavy for our message set; object-path ceremony for every event; every plugin language needs DBus bindings; we don't need introspection.
- **Line-delimited JSON (ADR-0008 framing).** Rejected for v0.2: fragile under large payloads; adopting length-prefix now is cheaper than later migration.
- **Length-prefixed bincode.** Rejected for v0.2: JSON is inspectable (`socat - UNIX-CONNECT:bus.sock` for debugging), forward-compatible by field addition, and the v0.2 message rate target (<100/s, NFR21) doesn't need a binary format. Revisit if telemetry-grade throughput becomes a v0.3+ concern.
- **HTTP / gRPC on the socket.** Rejected: massive dependency creep for a local IPC surface the cockpit fully controls.
- **Shared memory / mmap ring.** Rejected: vast over-engineering for our event rates.

## Consequences

- **+** Protocol version rejection is a parser-level invariant; FR59 can't be forgotten by implementors.
- **+** Length-prefixed framing is robust to any payload content; we can send binary-ish blobs (logs, config snippets) without escaping.
- **+** Single socket simplifies client discovery: `$XDG_RUNTIME_DIR/lmux/bus.sock` is the only endpoint.
- **+** SO_PEERCRED check makes the cross-UID attack vector explicit and easy to test.
- **−** Migrating from the ADR-0008 line-delimited sketch costs one codec rewrite; small, because we hadn't implemented ADR-0008 yet.
- **−** The bus is a single point of failure for the cockpit; if it panics, all clients lose their subscription. Mitigation: bus module is small and fuzz-tested (NFR21).
- **−** Fanout is implemented server-side (no built-in pub/sub). Fine at our rates; `subscribe` event is part of ADR-0016's event set.

## Follow-up

- ADR-0016 specifies the v0.2 event `kind` set.
- v0.3: evaluate whether the plugin SDK wants a token handshake on top of SO_PEERCRED (for session-sharing or remote-pairing scenarios).
- v0.3: decide whether `open.url` / `open.path` passive-tier intents (deferred from ADR-0008) join the bus or ride a separate surface.
