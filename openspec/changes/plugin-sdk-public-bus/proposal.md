## Why

v0.2 ships the cockpit bus as a same-UID, SO_PEERCRED-gated Unix socket. That's a good internal-control plane but a non-starter for the plugin story ADR-0002 promised. Editor plugins, third-party automation, and the existing `smart-open-passive-intents` kinds are all stuck behind one fence: there is no authenticated path for an external client to talk to the cockpit.

The design constraint is that **same-UID ≠ same-trust**. A browser-extension helper running as the user shouldn't have the same authority as the cockpit's own CLI. The answer is **capability tokens**: the cockpit mints scoped tokens; clients present them on the handshake; the router gates kinds by scope. Same-UID is still enforced at the socket (defence in depth), but authority is the token's, not the UID's.

This change lands:
- A token-issuance surface (CLI + config) for the user to mint, list, and revoke tokens.
- A scoped-subscription model: tokens carry a set of `scope` strings (e.g. `open.url`, `pane.list`, `anchor.*`); the router rejects unscoped kinds.
- A persistent revocation list under `$XDG_STATE_HOME/lmux/`.

A plugin SDK (docs + example clients in Rust and TypeScript) ships alongside; the SDK is code+documentation and doesn't itself carry requirements — the capability spec is the bus surface.

## What Changes

- New bus kinds on the handshake: `hello { v, token? }` — tokens are optional for same-UID internal clients (preserves the v0.2 CLI path) and required for anything that sends a gated kind.
- `token.scopes` on the `hello_ack` — the server echoes the token's scope set so the client can self-check capability before sending.
- A `TokenStore` persisted at `$XDG_STATE_HOME/lmux/tokens.toml` (mode 0600), rotating daily-logged, with per-entry `{id, name, scopes, created_at, last_used_at, revoked?}`.
- CLI: `lmux token new <name> --scope <s> [--scope <s> ...]`, `lmux token list`, `lmux token revoke <id>`.
- Router gate: a kind is accepted iff the connection's token scope set matches the kind's required scope (or the connection is a same-UID CLI client using the unscoped internal kinds listed in `bus-ipc`).
- Error kinds: `error.unauthorized` (no token when one was needed), `error.insufficient_scope { required, granted }`.
- Config key: `[bus].public = false` default. Setting to `true` binds an additional TCP or abstract-socket endpoint for machines where same-UID isolation is insufficient (e.g. container sidecars); still token-gated. TLS out of scope for v0.3.

## Capabilities

### New Capabilities

(none — extends `bus-ipc` and `config`)

### Modified Capabilities

- `bus-ipc`: adds `Requirement: Capability tokens and scoped authorization`, `Requirement: Token lifecycle management via CLI`, and modifies `Requirement: Peer authentication via SO_PEERCRED` to clarify that SO_PEERCRED is a defence-in-depth check, not the primary authorization.
- `config`: adds `Requirement: Bus public-mode opt-in` for the `[bus].public` toggle.

## Impact

- Code: new `crates/lmux-bus-auth/` crate with `TokenStore`, `Scope`, and token-format helpers; wiring in `crates/lmux-bus/src/server.rs` for handshake + per-kind gate; `crates/lmux-cli` for the three new subcommands; `crates/lmux` config loader.
- Cross-change: removes the same-UID restriction on smart-open intents from `smart-open-passive-intents`; that change becomes accessible to scoped external clients the day this one ships.
- No changes to existing v0.2 internal-client behaviour: CLI without a token continues to work for all unscoped kinds.
- Security: tokens are 256-bit URL-safe base64 strings; the on-disk store records only the SHA-256 of each token plus metadata. Revocation is an append-only operation.
