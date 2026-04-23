# ADR-0016: Smart-open v0.2 event set — concrete `kind` values

- Status: Accepted
- Date: 2026-04-22
- Deciders: Lars
- Blocks: v0.2 (implementation), v0.3 (plugin SDK extends this set)
- Depends on: ADR-0015 (bus architecture / framing / version envelope)

## Context

ADR-0002 introduced the "smart-open bus" as the concept that glues panes, anchors, satellites, the CLI, and eventually editor plugins together. ADR-0008 picked the transport. Neither ADR pinned the concrete event schema.

v0.2 needs a frozen event `kind` list because:

- The cockpit server, the `lmux` CLI client, and the KWin script bridge are three independent clients that must agree on message shapes day one.
- The bus parser (ADR-0015) rejects unknown `kind` values with a structured error — but only if the server has an authoritative list of what's known.
- Forward-compat hinges on the envelope: unknown `kind` → structured rejection → client can probe for v0.3 features by checking the error.

The original ADR-0002/0008 examples (`open.url`, `open.path`) were drafted around the editor-plugin passive-tier use case, which is a v0.3 concern once the SDK story solidifies. v0.2 is a single-user cockpit with no third-party plugins; its event set is internal-only.

## Decision

v0.2 ships exactly the event `kind` values below. Every frame carries `{"v": 2, "kind": "...", "id": "<uuid>"}` plus kind-specific payload. Unknown kinds receive `{"v": 2, "kind": "error", "code": "unknown_kind", "kind_received": "<x>"}`.

### Session lifecycle (cockpit ↔ CLI)

- `session.list` *(req/resp)* — CLI lists all sessions. FR1, FR56.
- `session.new` *(req/resp)* — create a new named session. FR2, FR6.
- `session.rename` *(req/resp)* — rename an existing session. FR6.
- `session.delete` *(req/resp)* — remove a session. FR6.
- `session.open` *(req/resp)* — swap active session; persists outgoing. FR3–FR5.

### Pane inventory (cockpit ↔ CLI)

- `pane.list` *(req/resp)* — enumerate every live pane with its stable UUID, anchor UUID (when tagged), and cwd. Discoverability for `anchor.tag <uuid>`. Amended 2026-04-22.

### Pane & anchor (cockpit ↔ CLI, cockpit → subscribers)

- `anchor.tag` / `anchor.untag` *(req/resp)* — mark/unmark. FR17–FR18.
- `anchor.pause` *(req/resp)* — SIGSTOP the process group. FR19.
- `anchor.resume` *(req/resp)* — SIGCONT. FR20.
- `anchor.hide` *(req/resp)* — detach rendering, start ring-buffer capture. FR21.
- `anchor.reattach` *(req/resp)* — reattach rendering, replay buffer. FR22.
- `anchor.respawn` *(req/resp)* — kill + relaunch same argv/cwd/env. FR26.
- `anchor.status` *(event, cockpit → subscribers)* — state change; drives sidebar + tab-edge glow. FR37–FR39.

### Satellite (cockpit ↔ CLI, cockpit ↔ KWin script, cockpit → subscribers)

- `satellite.open` *(req/resp, CLI → cockpit)* — `lmux open <cmd>`. FR28.
- `satellite.detach` *(req/resp, sidebar/CLI → cockpit)* — release to floating. FR33.
- `satellite.reattach` *(req/resp, sidebar/CLI → cockpit)* — re-dock. FR34.
- `satellite.map` *(event, KWin script → cockpit)* — "new toplevel with wm-class `<tag>` appeared as kwin window `<id>`"; drives the Path-A correlation state machine. FR29, ADR-0003.
- `satellite.geometry` *(event, cockpit → KWin script)* — "place kwin window `<id>` at rect `<x,y,w,h>`"; emitted on pane move/resize. FR32.
- `satellite.status` *(event, cockpit → subscribers)* — state change (docked / detached / floating-fallback). FR40–FR43.

### Compositor health (cockpit ↔ sidebar, cockpit ↔ CLI)

- `compositor.status` *(event, cockpit → subscribers)* — `{ "state": "online" | "offline", "reason": "<str>" }`. FR51.
- `compositor.reinject` *(req/resp, sidebar/CLI → cockpit)* — trigger KWin script re-inject. FR50.

### Meta / infrastructure

- `hello` / `hello_ack` — handshake (ADR-0015).
- `subscribe` *(req/resp)* — client subscribes to one or more event kinds by exact-match or suffix pattern (`anchor.*`, `satellite.status`). Server replies with the subscription id; unsubscribing is done by closing the connection or sending `unsubscribe` with the id.
- `unsubscribe` *(req/resp)* — drop a specific subscription.
- `error` *(resp)* — protocol-level error envelope: `{"code": "...", "message": "...", "kind_received"?: "..."}`.
- `status.get` *(req/resp)* — drives `lmux status`. NFR28.

### Response shapes (amendment 2026-04-22)

Every request gets exactly one reply on the same connection, carrying the
same envelope `id`. Responses are themselves `Kind` values, so the bus
parser uses one enum for both directions.

- `session.list.result` *(resp to `session.list`)* — `{ "sessions": [SessionSummary, ...] }`.
  `SessionSummary` is `{ "name": str, "created_at_unix_seconds": u64, "last_active_unix_seconds": u64? }`.
- `status.get.result` *(resp to `status.get`)* — `StatusSnapshot = { "cockpit_version": str, "pid": i32, "session_count": u32, "anchor_count": u32, "compositor": "online"|"offline" }`.
- `pane.list.result` *(resp to `pane.list`)* — `{ "panes": [PaneSummary, ...] }` where `PaneSummary = { "pane_id": uuid, "anchor_id"?: uuid, "cwd"?: str }`.
- `ok` *(resp for side-effect-only requests)* — optional `{ "of": "<request_kind>" }`; used
  by `session.new`, `session.rename`, `session.delete`, `session.open`,
  `anchor.tag`, `anchor.untag`, `anchor.pause`, `anchor.resume`,
  `anchor.hide`, `anchor.reattach`, `anchor.respawn`, `subscribe`,
  `unsubscribe`, `satellite.detach`, `satellite.reattach`,
  `compositor.reinject`. Server MAY omit `of` when the envelope `id`
  round-trip already disambiguates.
- `satellite.open.result` — **not yet defined**; the `satellite.open` reply
  shape needs the pane-id scheme from Epic 9 before it can be frozen. For
  v0.2-alpha servers return `ok { of: "satellite.open" }` and emit a
  subsequent `satellite.status` event once mapping completes.
- Events (`anchor.status`, `satellite.status`, `compositor.status`,
  `satellite.map`, `satellite.geometry`) remain one-way server-push; they
  never appear as responses.
- Errors use the existing `error` kind regardless of which request failed.

#### Consequence

The v0.2 bus protocol now has a frozen request **and** response surface
for everything the cockpit/CLI need to ship. `lmux session list` can
route through the bus by default; the SessionStore fallback remains for
offline invocations (cockpit not running). A bus-first CLI is unblocked
once the cockpit binds `Server::bind` with a `Handler` backed by the
live `SessionStore` + `AppState`.

### Deferred to v0.3

- `open.url` — the original passive-tier smart-open intent (ADR-0002/0008) for URL routing between panes.
- `open.path` — file-path intent for editor integration.

v0.2's `lmux open` is **only** for spawning GUI satellites via `satellite.open`. URL and file-path intents ride with the plugin SDK in v0.3 once the external-client auth model is resolved.

## Alternatives considered

- **Ship `open.url`/`open.path` in v0.2.** Rejected: without a plugin SDK and an external-client auth story, these are server-side intents looking for a client. Better to add them alongside the first plugin in v0.3.
- **Use verb-object kinds (`open_session`, `close_anchor`) instead of dotted namespaces.** Rejected: dotted namespaces (`session.open`) scale to sub-kinds (`session.open.cancel`) and group cleanly by domain. Consistent with D-Bus / MQTT topic conventions the team knows.
- **Push all events as JSON-RPC 2.0.** Rejected: JSON-RPC adds method/params/result/id ceremony that mostly duplicates our envelope. Net cost > benefit for a closed internal protocol.
- **Two sockets, one for req/resp and one for events.** Rejected: one socket carries both; the `id` field disambiguates responses from server-initiated events.

## Consequences

- **+** Explicit event list means the bus parser can validate by exhaustive match — compiler-assisted correctness when kinds grow in v0.3.
- **+** Deferring `open.url`/`open.path` keeps v0.2's surface stable; when the SDK lands, new kinds slot into the unknown-kind-rejection story cleanly.
- **+** Namespacing by domain (`session.*` / `anchor.*` / `satellite.*` / `compositor.*`) makes subscription patterns obvious.
- **−** Bus schema changes in v0.3 must version carefully; adding required payload fields to existing kinds is a breaking change. Mitigation: policy that v0.3 only *adds* kinds, never changes existing payload shapes.
- **−** KWin script is now a first-class bus client; any KWin runtime change that breaks its socket support would break us. Mitigation: KWin's Unix-socket support in JS is stable since 5.24; we pin to that.

## Follow-up

- Write schema types in `lmux-bus/src/kinds.rs` — one struct per `kind`, `#[serde(tag = "kind")]` enum wrapper.
- Fuzz the parse-envelope and per-kind parse-payload steps (`lmux-bus/fuzz/`).
- v0.3: ADR for `open.url` / `open.path` / plugin-SDK auth model.
- v0.3: ADR for session-sharing events (if multi-user lmux ever becomes real).
