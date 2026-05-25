# ADR-0016: lmux bus v0.2 kind catalog

- Status: Accepted
- Date: 2026-04-22
- Updated: 2026-05-24
- Deciders: Lars
- Depends on: ADR-0015
- Blocks: `lmux-cli`, sidebar actions, session switcher, native-window attach

## Context

ADR-0002 introduced the bus as the glue between the cockpit, CLI, anchors,
satellites, and future plugins. ADR-0008 picked the Unix socket transport.
ADR-0015 fixed the length-prefixed JSON envelope.

The concrete `kind` catalog now lives in `crates/lmux-bus/src/kinds.rs`. The
current bus is a same-user local control surface for a running lmux process.
`lmux-cli` speaks this protocol; `lmux-cli status` is the documented status
interface.

## Decision

Bus frames carry `{"v": 2, "id": "<uuid>", "kind": "..."}` plus
kind-specific payload. Unknown kinds are rejected with a structured protocol
error.

The current v0.2 catalog is:

### Meta and status

- `hello`
- `hello_ack`
- `subscribe`
- `unsubscribe`
- `error`
- `ok`
- `status.get`
- `status.get.result`

### Sessions

- `session.list`
- `session.list.result`
- `session.new`
- `session.rename`
- `session.delete`
- `session.open`

### Panes

- `pane.list`
- `pane.list.result`

### Anchors

- `anchor.new`
- `anchor.activate`
- `anchor.tag`
- `anchor.untag`
- `anchor.pause`
- `anchor.resume`
- `anchor.hide`
- `anchor.reattach`
- `anchor.respawn`
- `anchor.status`

### Satellites

- `satellite.open`
- `satellite.detach`
- `satellite.reattach`
- `satellite.attach_focused`
- `satellite.list_windows`
- `satellite.list_windows.result`
- `satellite.attach_window`
- `satellite.map`
- `satellite.geometry`
- `satellite.status`

`satellite.list_windows` plus `satellite.attach_window` is the reliable native
window ownership flow. `satellite.attach_focused` is the macOS convenience path.
`satellite.open`, `satellite.map`, and `satellite.geometry` remain in the bus
catalog for legacy launch/spawn paths and compatibility; they are not the
primary native-window attach workflow.

### Compositor

- `compositor.status`
- `compositor.reinject`

## Response Shape

Every request receives exactly one response on the same connection. Side-effect
requests use `ok { of?: "<request_kind>" }`. Read requests use typed result
kinds such as `session.list.result`, `pane.list.result`,
`satellite.list_windows.result`, and `status.get.result`.

Events such as `anchor.status`, `satellite.status`, `compositor.status`,
`satellite.map`, and `satellite.geometry` are server-push messages and are not
used as direct request responses.

## Deferred

- `open.url`
- `open.path`
- plugin SDK auth and routing
- remote/shared session events

Those belong to later plugin or remote-work design, not the current local
cockpit control surface.

## Alternatives considered

- **JSON-RPC 2.0.** Rejected because the lmux envelope already provides version,
  id, kind, result, and error structure without extra ceremony.
- **Verb-object kind names.** Rejected in favor of dotted namespaces such as
  `session.open` and `satellite.attach_window`.
- **Separate sockets for request/response and events.** Rejected; the same
  socket can carry both because response ids and event kinds disambiguate them.

## Consequences

- `lmux-cli` can stay thin and map subcommands directly to bus kinds.
- Capability additions are explicit and grep-able.
- Existing legacy satellite kinds remain documented as legacy so their presence
  in code does not imply that host-window geometry ownership is the recommended
  workflow.

## Follow-up

- Keep `crates/lmux-bus/src/kinds.rs` and `openspec/specs/bus-ipc/` aligned.
- Add new bus kinds by amending the catalog rather than overloading payloads.
