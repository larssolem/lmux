## ADDED Requirements

### Requirement: Smart-open intent kinds

The bus SHALL accept two passive intent kinds — `open.url { url: String, hint?: String }` and `open.path { path: String, line?: u32, column?: u32, hint?: String }` — and respond with a structured `open.result` whose payload names the tier that handled the intent.

#### Scenario: `open.url` is accepted with a valid payload

- **WHEN** a same-UID client sends `open.url { url: "https://example.com", hint: "prefer=browser" }` after handshake
- **THEN** the server dispatches the intent through the router and responds with `open.result { routed_to: <tier> }` on the same envelope id

#### Scenario: `open.path` supports line and column

- **WHEN** a client sends `open.path { path: "src/lib.rs", line: 42, column: 10 }`
- **THEN** the server parses all three fields, resolves the path against the cockpit cwd if relative, and routes the intent with the full triple preserved

#### Scenario: `hint` is advisory, not mandatory

- **WHEN** a client sends an intent with a `hint` string that no subscriber claims
- **THEN** the router ignores the hint for correctness purposes but logs it at DEBUG for future routing-rule tuning; routing still proceeds through the standard fallback chain

#### Scenario: Malformed intents are rejected per-frame

- **WHEN** a client sends `open.url` with a missing `url` field or a non-string URL
- **THEN** the server responds with `error.schema_violation` naming the failing field; no dispatch occurs; the connection stays open

#### Scenario: Cross-UID intents stay rejected

- **WHEN** a process running under a different UID opens the bus socket and attempts to send `open.url` or `open.path`
- **THEN** the connection is closed with `error.peer_denied` at the SO_PEERCRED check — before any kind is parsed

### Requirement: Intent routing with subscriber + fallback

The cockpit SHALL route accepted intents through a three-tier fallback chain — explicit pane subscribers (glob-matched), focused-pane anchor capability, platform default (`xdg-open` / `$EDITOR`) — returning a typed `RoutedTo` that names the tier that handled the intent.

#### Scenario: Subscriber tier takes precedence

- **WHEN** a pane has subscribed with pattern `"*.rs"` and an `open.path { path: "src/lib.rs" }` arrives
- **THEN** the router dispatches to that pane's subscriber handler and returns `RoutedTo::Subscriber { pane_id }`

#### Scenario: Focused anchor fallback when no subscriber matches

- **WHEN** no subscriber pattern matches and the focused pane's anchor has capability `"editor"` set
- **THEN** the router dispatches `open.path` to that anchor and returns `RoutedTo::FocusedFallback { pane_id }`

#### Scenario: Platform default fallback

- **WHEN** no subscriber matches and the focused pane has no matching capability
- **THEN** the router invokes `xdg-open` for URLs or spawns a satellite running `$EDITOR` for paths, and returns `RoutedTo::PlatformDefault`

#### Scenario: No handler is a typed error

- **WHEN** no subscriber matches, the focused pane lacks the required capability, and the platform default is unset or missing (`$EDITOR` empty, `xdg-open` not on PATH)
- **THEN** the router returns `RoutedTo::Rejected { reason }` and emits a cockpit toast naming the absent handler; the response uses the `error.no_handler` kind

#### Scenario: `file://` URLs rewrite to path intents

- **WHEN** an `open.url { url: "file:///home/me/src/lib.rs" }` arrives
- **THEN** the router strips the scheme and re-dispatches internally as `open.path { path: "/home/me/src/lib.rs" }`; the returned `RoutedTo` reflects the tier that handled the path

## MODIFIED Requirements

### Requirement: Frozen kind schema

The bus SHALL expose the kind schema defined in ADR-0016, extended in v0.3 with the smart-open intent kinds (`open.url`, `open.path`, `open.result`) and the subscription kinds (`subscribe.smart_open`, `unsubscribe.smart_open`); request/response kinds under `session.*`, `anchor.*`, `satellite.*`, `compositor.*`, plus meta kinds `hello`, `hello_ack`, `status.get`, generic `ok`, and `error.*`, remain unchanged. Unknown kinds are rejected deterministically.

#### Scenario: Every frozen kind round-trips through serde

- **WHEN** `cargo test -p lmux-bus` runs the kinds module round-trip suite
- **THEN** each defined kind — including the v0.3 additions — serializes to JSON and deserializes back to an equal value; deny-list tests confirm unknown kinds fail to deserialize cleanly

#### Scenario: Required domain kinds are present

- **WHEN** the bus server initializes
- **THEN** it handles at minimum: `session.list`, `session.new`, `session.rename`, `session.delete`, `session.open`, `anchor.tag`, `anchor.untag`, `anchor.pause`, `anchor.resume`, `anchor.hide`, `anchor.reattach`, `satellite.open`, `compositor.status`, `status.get`, `open.url`, `open.path`, `subscribe.smart_open`, `unsubscribe.smart_open`

#### Scenario: v0.3 adds only new kinds, never mutates existing payloads

- **WHEN** a v0.2 client connects and uses any pre-existing kind after the v0.3 cockpit upgrade
- **THEN** the payload shape of every v0.2 kind is accepted unchanged; no field becomes newly-required, no field changes type
