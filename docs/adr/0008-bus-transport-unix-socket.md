# ADR-0008: Smart-open bus transport — Unix domain socket

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.2 (passive tier), v0.3 (active tier spec)

## Context

The smart-open bus (ADR-0002) needs a transport for:

- **Passive tier (v0.1-ish):** lmux-internal — URL and file-path routing between panes inside one session. Single producer/consumer surface per session.
- **Active tier (v0.3 spec, v0.4+ plugins):** bidirectional messaging between external editor plugins and the anchor — agent status, build status, focus intents.

Candidates: D-Bus, Unix domain socket with a line-delimited JSON protocol, stdio pipes, TCP/localhost.

## Decision

**Unix domain socket per lmux session**, at `$XDG_RUNTIME_DIR/lmux/<session-id>.sock`. Protocol is **line-delimited JSON** — one message per line, each an envelope:

```json
{"kind": "open.url", "url": "https://…", "source_pane": "…"}
{"kind": "open.path", "path": "…", "line": 42, "source_pane": "…"}
{"kind": "agent.status", "anchor": "…", "status": "waiting_for_input"}
```

- lmux is the server; panes and editor plugins are clients.
- Authentication: filesystem permissions on the socket path (user-private `$XDG_RUNTIME_DIR`). No token handshake in v0.1.
- Versioning: one envelope field `"v": 1`; unknown kinds are ignored (forward-compat).
- D-Bus is **not** the primary transport, but the active-tier spec will leave room for a D-Bus bridge later if a specific integration needs it.

## Alternatives considered

- **D-Bus (session bus) as primary.** Rejected: heavier than required for URL/path routing; every event becomes an object-path/interface ceremony; introspection machinery we don't need; harder to version informally. Good for *specific integrations* (e.g. KDE desktop actions); bad as a general plugin bus.
- **stdio pipes per plugin.** Rejected: doesn't scale beyond one plugin; awkward for GUI editors that launch independently.
- **TCP on localhost.** Rejected: weaker default security than Unix sockets; needs port management; no reason to prefer it.
- **Shared memory / mmap ring.** Rejected: vast over-engineering for event rates we'll actually see.

## Consequences

- **+** Cheap to implement in Rust (`tokio::net::UnixListener`) and any editor (every plugin language has socket bindings).
- **+** Per-session sockets give free process-isolation between lmux sessions.
- **+** JSON is trivially versioned, extended, and inspected with `socat - UNIX-CONNECT:…`.
- **+** Stays compatible with a future D-Bus bridge for compositor-level integrations that need it.
- **−** No built-in broadcast / pub-sub. We implement fanout server-side. Mitigation: fine for our event rates.
- **−** Filesystem permissions are the only AuthN. Acceptable for session-scoped sockets under `$XDG_RUNTIME_DIR`.
- **−** Socket path collisions if lmux crashes uncleanly. Mitigation: on startup, unlink stale sockets whose owner-PID doesn't exist.

## Follow-up

- Define v0.1 passive event set (URL intent, file-path open intent) in the plugin spec.
- Draft active-tier envelope schema during v0.3 planning.
