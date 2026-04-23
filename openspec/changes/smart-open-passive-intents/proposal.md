## Why

ADR-0002 introduced "smart-open" as the concept that glues panes to editor plugins and external tools. ADR-0016 froze the v0.2 `kind` surface to internal cockpit/CLI/KWin-script clients only and **deferred** two intent kinds to v0.3:

- **`open.url`** — "open this URL somewhere sensible" (the focused pane's pattern handler, the default browser as fallback, or a subscribed plugin).
- **`open.path`** — "open this file path at this line/column" (the pane's associated editor satellite, or the user's default editor as fallback).

These passive intents are what make lmux useful to an editor plugin: a Neovim plugin on the user's machine can publish `open.path {path, line}` against the cockpit bus and lmux routes it to the right anchor's pane without the plugin needing to know anything about anchor UUIDs or pane ids. Without them, cross-tool navigation (ripgrep → editor, browser → editor, terminal stderr → editor) lives entirely outside lmux and the cockpit stays a terminal-shaped black box.

This change adds the two intent kinds, the subscription-and-routing model, and a built-in default-handler fallback. Plugin-author auth lives in a sibling change (`plugin-sdk-public-bus`) — this change assumes trusted same-UID clients only and fails closed on any cross-UID attempt.

## What Changes

- Two new bus kinds: `open.url { url: String, hint?: String }` and `open.path { path: String, line?: u32, column?: u32, hint?: String }`.
- A subscription model where a pane can register a handler pattern (e.g. `{ pane_id, patterns: ["*.rs", "*.toml"] }`) that the cockpit consults during routing.
- Routing order: (1) explicit subscribers whose pattern matches the intent payload; (2) the focused pane's anchor if it claims an editor capability; (3) a default-handler fallback (`xdg-open` for URLs, `$EDITOR` for paths, both opened in a new satellite if no pane can claim them).
- New CLI subcommands: `lmux open-url <url>` and `lmux open-path <path>[:line[:col]]` that each send exactly one bus kind.
- A sidebar UI on the focused pane row to edit the pane's handler patterns.
- Routing metrics exposed in `status.get`.

This change explicitly does **not** ship external-client auth. `open.url` and `open.path` from an out-of-UID process are rejected as they are today. The `plugin-sdk-public-bus` change removes that restriction behind an explicit plugin-token auth model.

## Capabilities

### New Capabilities

(none — extends `bus-ipc` with two new kinds and a routing model)

### Modified Capabilities

- `bus-ipc`: adds `Requirement: Smart-open intent kinds` and `Requirement: Intent routing with subscriber + fallback`, and modifies `Requirement: Frozen kind schema` to list the two new kinds and note that v0.3 is the first minor that extends the v0.2-frozen surface.

## Impact

- Code: new `crates/lmux/src/smart_open/` module with the router and subscription store; wiring in `crates/lmux-bus/src/kinds.rs` for the two new kinds; `crates/lmux-cli` for the two new subcommands; sidebar context menu additions.
- Schema churn: this is the first v0.3 bus-kind addition. ADR-0016's consequences section notes "v0.3 only *adds* kinds, never changes existing payload shapes"; this proposal obeys that.
- Cross-change: `plugin-sdk-public-bus` extends the auth model so these intents can accept trusted external clients. Until that change lands, smart-open remains a same-UID-only feature (which is still useful for user-level scripts and shell aliases).
- No changes to session, anchor, satellite, or compositor surfaces.
