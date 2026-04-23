## 1. Token store

- [ ] 1.1 New `crates/lmux-bus-auth/` crate with `Scope`, `Token`, `TokenStore`
- [ ] 1.2 `TokenStore` backed by `$XDG_STATE_HOME/lmux/tokens.toml` (mode 0600), append-only on mutation
- [ ] 1.3 `mint(name, scopes, expires_at?) -> (Token, TokenRecord)`; record stores only SHA-256 of the token
- [ ] 1.4 `verify(presented: &str) -> Option<TokenRecord>`: constant-time compare; returns `None` for revoked or unknown
- [ ] 1.5 `revoke(id)`: appends a revocation entry
- [ ] 1.6 Unit tests: round-trip mint/verify/revoke; audit-log append-only invariant

## 2. Scope vocabulary + kind authorization table

- [ ] 2.1 `crates/lmux-bus/src/kind_authorization.rs`: exhaustive `KIND -> RequiredScope` table
- [ ] 2.2 Scope matching: exact, prefix (`foo.*`), catchall (`*`); reject malformed scope strings at mint time
- [ ] 2.3 Internal-only kinds enumerated explicitly: `hello`, `hello_ack`, `session.*`, `status.get`, `compositor.reinject`
- [ ] 2.4 Scoped kinds enumerated: `open.*`, `pane.list`, `anchor.*`, `satellite.*`, `subscribe.smart_open`, `unsubscribe.smart_open`
- [ ] 2.5 Compile-time exhaustiveness: match against the `Kind` enum fails to compile if a kind lacks an authorization entry

## 3. Handshake + gate

- [ ] 3.1 Extend `hello` payload with optional `token: String`
- [ ] 3.2 `hello_ack` echoes `token_scopes: [String]` so the client can self-inspect
- [ ] 3.3 Per-connection `ConnectionAuth { uid, scopes }` populated at handshake; all dispatches consult it
- [ ] 3.4 Gate: scoped kind + no/invalid token → `error.unauthorized`; valid token but scope mismatch → `error.insufficient_scope`; close connection after response
- [ ] 3.5 Integration test: each scope/kind combo exercised by a test matrix

## 4. CLI

- [ ] 4.1 `lmux token new <name> [--scope <s>]... [--expires-in <duration>]` — prints the raw token once
- [ ] 4.2 `lmux token list` — prints `id`, `name`, `scopes`, `created_at`, `last_used_at`, `revoked`
- [ ] 4.3 `lmux token revoke <id>` — appends revocation; a confirmation required unless `--yes`
- [ ] 4.4 `lmux token rotate cli` — rotates the auto-minted CLI token in place
- [ ] 4.5 `--json` on all three for scripting

## 5. CLI auto-token bootstrapping

- [ ] 5.1 On first boot, if `$XDG_STATE_HOME/lmux/cli-token` is absent, mint a token with scope `open.*` and write the raw value to that file (mode 0600)
- [ ] 5.2 CLI picks up the auto-token when no user-supplied `LMUX_TOKEN` env var is set
- [ ] 5.3 The auto-token appears in `lmux token list` like any other
- [ ] 5.4 Rotation via `lmux token rotate cli` updates the file and revokes the old entry
- [ ] 5.5 Integration test: delete the file, restart cockpit → new file appears; smart-open CLI commands succeed

## 6. Public-mode opt-in

- [ ] 6.1 Config key `[bus].public = false` (default) / `true`
- [ ] 6.2 On `true`, bind `@lmux-public-<uid>` abstract socket in addition to the filesystem socket
- [ ] 6.3 Public socket connections MUST present a token on `hello`; no auto-token accepted
- [ ] 6.4 `lmux status` surfaces `bus.public = true|false` and the bound endpoint

## 7. Observability

- [ ] 7.1 Structured span on dispatch: `bus.dispatch { token_id, kind, granted_scope }`; never logs the raw token
- [ ] 7.2 Counter `bus.auth_failures` split by reason (`unauthorized` / `insufficient_scope`)
- [ ] 7.3 Sidebar toast on repeated auth failures from the same connection (hints at a misconfigured plugin)

## 8. Smart-open hand-off

- [ ] 8.1 Remove the same-UID-only paragraph from the smart-open router (intents are now purely scope-gated)
- [ ] 8.2 Verify `lmux open-url` / `lmux open-path` still work via the auto-token path after this change lands
- [ ] 8.3 Update the `smart-open-passive-intents` proposal-linked README section to reference token-based access for external clients

## 9. Example SDK clients

- [ ] 9.1 `examples/plugin-rust/`: minimal Rust client that handshakes with a token, subscribes to `anchor.status`, and prints events
- [ ] 9.2 `examples/plugin-typescript/`: equivalent TypeScript client for Node
- [ ] 9.3 README cross-link: "Want to write a plugin? Start here."
- [ ] 9.4 Both examples document token minting as a two-line shell snippet

## 10. Documentation

- [ ] 10.1 New `docs/plugin-sdk.md`: bus architecture primer, handshake, kind list with required scopes, error kinds
- [ ] 10.2 Security note: "tokens are bearer credentials; treat like SSH keys"
- [ ] 10.3 Add ADR-0021 (new) for the capability-token model if ADR-0015 doesn't already cover it; cross-link from `bus-ipc` spec
- [ ] 10.4 Release notes entry: "v0.3 adds capability tokens; existing CLI workflow is unchanged thanks to the auto-token"
