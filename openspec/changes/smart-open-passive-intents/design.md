## Context

The cockpit's v0.2 bus surface is internal-only: the CLI, the cockpit, and the KWin script are the only speakers, and they run as the cockpit's UID. ADR-0016 deliberately deferred `open.url` and `open.path` because they implied a story for *external* clients (plugins, shell tools) that didn't exist yet.

This change ships the **intents** themselves without claiming to solve the auth story. Same-UID clients (user shell scripts, CLI invocations from an editor plugin launched by the same user's sessoin) work immediately. External / cross-UID clients remain rejected until `plugin-sdk-public-bus` adds a token-auth model that the router can consult.

## Goals / Non-Goals

**Goals**
- Two intent kinds with a stable, forward-compatible payload shape.
- A routing model that honors explicit subscriptions, falls back through the focused-pane heuristic, and finally uses platform defaults (`xdg-open`, `$EDITOR`).
- A sidebar UI to edit per-pane handler patterns.
- CLI subcommands so shell aliases (`alias vo='lmux open-path'`) are a one-liner.

**Non-goals**
- External-client auth. Same-UID only; cross-UID stays denied.
- URL pattern matching beyond glob / prefix / exact. No regex to avoid ReDoS.
- Intent composition (`open.url` that redirects to `open.path`). Out of scope; routing is one-hop.
- Persistent subscription storage. Subscriptions are process-lifetime for v0.3; if a pane crashes and is respawned, the user re-subscribes. (Persistence can come in a follow-up if dogfooding demands.)

## Decisions

### D1 — Payload shape is minimal and fully-specified

`open.url { url: String, hint?: String }` and `open.path { path: String, line?: u32, column?: u32, hint?: String }`. The `hint` field is free-form and lets a sender express routing preference (`"prefer=editor"`, `"prefer=browser"`) without the router having to promise anything. The router treats `hint` as advisory; it's useful for logs even when no routing rule picks it up.

Paths are **not** auto-resolved to absolute form by the sender; the router resolves against the cockpit's cwd if relative. Rationale: senders may not know their own cwd relative to the target pane; the router is in the better position.

### D2 — Subscription model: per-pane pattern list

Each pane can hold a list of patterns, stored in `AppState` per-pane metadata. When the cockpit receives an intent, it iterates subscribers in (focused-first, otherwise undefined) order and dispatches to the first whose pattern matches. Patterns are globs compiled via `globset` (already a dependency through `crates/lmux/src/config`). No regex — ReDoS is a real risk.

Alternative considered: per-anchor subscriptions keyed by anchor capability (`editor`, `browser`, `terminal`). Rejected as too coarse for v0.3; users want "this specific `nvim` instance handles `*.rs`, the other handles `*.md`". Pane-level subs express that directly.

### D3 — Fallback chain

```
Explicit subscribers (globbed match)
    ↓ (none matched)
Focused pane, if its anchor's capability matches the intent
    ↓ (no match)
Platform default (xdg-open for URLs; $EDITOR in a new satellite pane for paths)
```

The final fallback uses `satellite.open` internally, so the new-satellite path already exists. If `$EDITOR` is unset and no capable pane is subscribed, the router returns `error.no_handler` and emits a toast.

### D4 — Anchor capability hints

A new optional field on the `anchors` spec's pane-metadata: `capabilities: Vec<String>`. For v0.3 the defined tokens are `"editor"` and `"browser"`. The router uses them for the focused-pane fallback (D3 step 2). The user sets the capability via the sidebar pane-row menu; nothing is inferred. This keeps the model explicit.

(This is a touch on the `anchors` capability, so the spec delta modifies `anchors` too. Kept small: one new requirement, no scenario changes on existing anchor requirements.)

### D5 — Routing metrics are counters, not logs

The router emits three counters, visible in `status.get`:

- `smart_open.intents_received` (by kind)
- `smart_open.subscriber_hits`
- `smart_open.fallback_hits`

Observability beyond counters (per-intent span) is optional; keep the trace stream quiet in the common case.

### D6 — CLI argument parsing

`lmux open-path vi.rs:42:10` — the `path:line:column` form is parsed by the CLI into a structured payload. `lmux open-url https://...` strictly passes through. The CLI does not perform URL or path validation — the router handles malformed inputs with `error.malformed_body`.

### D7 — Same-UID-only for v0.3

Enforce at the socket accept layer: if the connecting peer's UID differs from the cockpit's, close with `error.peer_denied` — which is already how the bus works today. No new auth code in this change. When `plugin-sdk-public-bus` lands, the policy changes at that boundary without touching the router or the kinds.

## Risks / Trade-offs

- *Risk: Globs per pane are user-visible config that users will get wrong.* → Mitigation: sidebar UI with live "test this path against my patterns" field; a default empty pattern list means "I don't handle anything" (safe default).
- *Risk: Sender floods the bus with intents.* → Mitigation: existing bus rate-limiting applies (the cockpit handler is single-threaded per connection); we don't add per-intent throttling but measure via D5 counters and revisit if needed.
- *Risk: `open.url` with a `file:///` URL conflicts with `open.path`.* → Mitigation: the router strips `file://` prefixes from URL intents and internally re-dispatches as `open.path`. Single-hop (D0 goals), same process.
- *Risk: Hardcoded fallback to `xdg-open` / `$EDITOR` doesn't match every environment.* → Mitigation: the two fallbacks are config-overridable under `[smart_open]` in `lmux.toml`.

## Migration Plan

- No migration. Same-UID clients that want to use the intents start sending them the day the change ships.
- Plugin authors waiting on cross-UID: tracked via `plugin-sdk-public-bus`.
- Config: `[smart_open]` section is optional; unset = built-in defaults. A comment at file-write time lists the available keys.

## Open Questions

- Should `hint` be structured (`{prefer: "editor"}`) instead of a string? Leaning string for now; promote to struct if a real plugin needs it.
- Per-pane subscriptions survive pane respawn? Leaning no for v0.3 (respawn requires re-subscribe); revisit if editor-plugin authors complain.
- Do we want a "last resort" catchall subscriber (e.g., a dedicated Zed pane that takes any path)? Can be expressed today by subscribing to `*` — no new feature needed.
