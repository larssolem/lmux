## Context

The v0.2 bus's authorization is a single line: `if peer_uid != cockpit_uid { deny }`. That's a perimeter check — inside the perimeter every client is root-equivalent. The plugin story breaks this model: a Firefox helper, an editor plugin, a CI script — each runs as the user, each has different *needs*, none should be authorized to issue `session.delete`.

Capability-based authorization (scoped tokens) is the cleanest answer. The token is the bearer credential, the scope set is the authority. The socket-level SO_PEERCRED gate becomes a second-line defence rather than the load-bearing check.

## Goals / Non-Goals

**Goals**
- Tokens that the user mints, scopes, and revokes without restarting the cockpit.
- Scope-gated dispatch: each bus kind maps to a required scope; the router enforces it.
- Per-token audit trail: when a scoped kind is dispatched, log the token id (not the secret).
- A path to open the bus beyond same-UID (`[bus].public = true`), still token-gated.
- Documentation + example SDK clients in Rust and TypeScript so a plugin author can ship a prototype in a day.

**Non-goals**
- TLS on the public socket. Ships in a later change if the abstract-namespace or TCP endpoint sees real use.
- OIDC / OAuth integration. Out of scope; bearer tokens are enough for v0.3.
- Per-scope rate limiting. Can be added later without schema changes.
- A plugin runtime / sandbox. Plugins are external processes talking to the bus; the cockpit has no hosting role.

## Decisions

### D1 — Token format: opaque 256-bit bearer + SHA-256 on disk

Tokens are 32 random bytes encoded as URL-safe base64 (no padding). The on-disk store records only `{ id: Uuid, name, sha256: [u8; 32], scopes, created_at, last_used_at, revoked }`. The raw token is shown to the user exactly once at creation and never again. This matches the pattern GitHub, AWS, and Cloudflare use for personal access tokens — familiar and safe-by-default.

Alternatives considered: signed JWTs (too much machinery for a single-cockpit issuer); HMAC'd tokens with embedded scopes (stateless, but revocation requires a blacklist anyway, so we might as well use a store).

### D2 — Scope vocabulary mirrors the bus kind namespace

Scope strings use the same dotted namespace as kinds, with wildcard suffixes allowed:

- Exact: `open.url`, `pane.list`, `anchor.tag`
- Prefix: `anchor.*`, `satellite.*`
- Catchall: `*` (equivalent to all kinds — discouraged but available)

A token with scope `anchor.*` can send `anchor.tag`, `anchor.pause`, `anchor.hide`, etc., but not `session.open`. Rationale: scope strings are user-facing; making them mirror the kind namespace means one concept to learn.

### D3 — Unscoped internal kinds are same-UID-only; everything else requires a token

The bus has two tiers:

- **Internal kinds** (CLI-only, no scope gate, must be same-UID): `hello`, `hello_ack`, `session.*`, `status.get`, `compositor.reinject`. Same-UID CLI users keep full power without needing tokens.
- **Scoped kinds** (require a token with matching scope regardless of UID): `open.url`, `open.path`, `pane.list`, `anchor.*`, `satellite.*`, `subscribe.smart_open`, `unsubscribe.smart_open`.

The split is explicit in a `kind_authorization.rs` table and tested exhaustively. Rationale: the CLI never needs scoped tokens (bad UX to make the user mint one for a local tool); plugins always need them.

### D4 — Token on handshake, not per-frame

The token is presented on the `hello` frame (`{"v": 2, "kind": "hello", "token": "<b64>"}`) and validated once. The server stores the resulting scope set on the per-connection state. No per-frame revalidation — if the token is revoked mid-session, existing connections continue until close. Revocation takes effect for new connections immediately.

Alternative considered: per-frame validation. Rejected — the auth hot-path matters for geometry-follow-style bursty traffic; connection-level gate is sufficient.

### D5 — Revocation is append-only

The on-disk token store is never mutated in place; revocation appends a new entry with `revoked: true, revoked_at: <ts>`. Audit-friendly, crash-safe (no half-writes can lose tokens), and the store's size stays bounded by the number of tokens the user has created.

### D6 — Public-mode transport is an abstract Unix socket

Setting `[bus].public = true` binds `@lmux-public-<uid>` (abstract-namespace Unix socket; no filesystem artefact, user-scoped by name). Anybody under the same Linux user can reach it; the token gate is the sole authorization. Abstract sockets are Linux-native and behave well across the container boundary (shared PID namespace, typical devcontainer setup).

Alternatives considered: loopback TCP (needs a port; leaks to any local user without netns isolation); filesystem-namespaced Unix socket outside XDG_RUNTIME_DIR (same story as the internal socket but worse ergonomics for cross-UID). Abstract socket is the sweet spot.

### D7 — Error kinds

Two new kinds, narrow and actionable:

- `error.unauthorized` — no token presented on a scoped kind, OR token invalid/revoked.
- `error.insufficient_scope { required: String, granted: [String] }` — token present but doesn't cover the attempted kind.

Both close the connection after response. Clients that suspect token rotation can reconnect and retry.

### D8 — Backward compatibility for the `smart-open-passive-intents` change

That change shipped `open.url` / `open.path` as same-UID-only. This change flips those kinds into the **scoped** tier:

- Same-UID CLI (no token) → denied via `error.unauthorized`. CLI shortcuts like `lmux open-path` must pick up a token.
- We ship an auto-minted CLI token at first cockpit boot under `$XDG_STATE_HOME/lmux/cli-token` with scopes `open.*` so the CLI works out of the box. (The CLI implicitly presents this token when it has no user-supplied one.)

Rationale: flipping smart-open into the scoped tier means the same auth story works for CLI and plugin alike, with no special-casing in the router. The CLI's auto-token is invisible UX: the user never sees it unless they run `lmux token list`.

## Risks / Trade-offs

- *Risk: users leak tokens (commit to dotfiles, paste in screenshots).* → Mitigation: token-new output flags "treat this like an SSH key"; revocation is one command.
- *Risk: CLI auto-token file leaks.* → Mitigation: mode 0600 on the file; its scope is limited to `open.*` so worst-case blast radius is an attacker opening arbitrary URLs/paths on the user's cockpit, not deleting sessions.
- *Risk: scope vocabulary drift as kinds are added.* → Mitigation: the `kind_authorization.rs` table is the single source of truth; adding a kind requires adding a scope entry in the same PR.
- *Risk: abstract sockets on non-Linux platforms.* → lmux is Linux-only per ADR-0001; non-issue.

## Migration Plan

- First-run after upgrade: cockpit mints the CLI auto-token if it doesn't exist, with scope `open.*`. Logged at INFO; the user sees one line about the mint.
- `[bus].public` defaults to `false`. Existing configs continue to work.
- Token store is new; no migration from v0.2.
- CLI without a token file: smart-open subcommands fail with a clear message pointing at `lmux token new` — shouldn't happen in practice because the auto-token is created on first boot.

## Open Questions

- Should we allow token "expiry" in addition to revocation? Skew toward yes for externally-minted tokens; tasks.md adds an optional `expires_at` on `lmux token new`.
- Should the CLI auto-token be rotateable? Leaning yes via `lmux token rotate cli`; tracked in tasks.md.
- Does the SDK need to ship packaged (crate / npm), or just as example code? Start with example code; promote to published packages once the surface settles.
