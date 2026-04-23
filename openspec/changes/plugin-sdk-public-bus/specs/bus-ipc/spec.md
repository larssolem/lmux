## ADDED Requirements

### Requirement: Capability tokens and scoped authorization

The bus SHALL authorize every scoped kind by matching the connection's token scope set against the kind's required scope; unscoped internal kinds remain same-UID-gated. Tokens are 256-bit bearer credentials stored on disk as SHA-256 hashes only, validated once at handshake and cached per-connection.

#### Scenario: Scoped kind with matching scope is dispatched

- **WHEN** a client handshakes with `hello { token: "<b64>" }` whose token carries scope `anchor.*` and then sends `anchor.tag { ... }`
- **THEN** the cockpit dispatches the `anchor.tag` as it would for an internal client; the span logs the token id and the granted scope

#### Scenario: Scoped kind without a token returns error.unauthorized

- **WHEN** a client handshakes without a token and sends a scoped kind such as `open.url`
- **THEN** the server responds with `error.unauthorized` and closes the connection after the response

#### Scenario: Insufficient scope names required and granted

- **WHEN** a client presents a token with only `open.*` scope and sends `anchor.pause`
- **THEN** the server responds with `error.insufficient_scope { required: "anchor.pause", granted: ["open.*"] }` and closes the connection

#### Scenario: Revoked token fails validation on new connections

- **WHEN** a token is revoked via `lmux token revoke <id>` and a new client attempts to handshake with that token
- **THEN** the server rejects the handshake with `error.unauthorized`; existing connections authenticated under the same token before revocation continue until close

#### Scenario: Wildcard scope matches the kind namespace

- **WHEN** a token carries scope `satellite.*` and the client sends `satellite.detach`
- **THEN** the server accepts; when the same client sends `session.open`, the server rejects with `error.insufficient_scope`

#### Scenario: Exhaustive kind-to-scope mapping is compile-time checked

- **WHEN** a new bus kind is added to the `Kind` enum in `crates/lmux-bus`
- **THEN** the build fails unless an authorization entry for the new kind is added to `kind_authorization.rs`; this prevents unscoped-by-default regressions

### Requirement: Token lifecycle management via CLI

The user SHALL be able to mint, list, and revoke capability tokens via the `lmux token` CLI subcommand group; minted tokens are shown exactly once, stored only as SHA-256 hashes afterward, and revocations are append-only.

#### Scenario: `lmux token new` prints the raw token once

- **WHEN** the user runs `lmux token new "firefox" --scope open.url`
- **THEN** the CLI prints the raw base64 token on stdout exactly once with a clear "save this now, it will not be shown again" note, and persists only the SHA-256 hash plus metadata in `$XDG_STATE_HOME/lmux/tokens.toml` (mode 0600)

#### Scenario: `lmux token list` omits the raw token

- **WHEN** the user runs `lmux token list`
- **THEN** the CLI prints `{id, name, scopes, created_at, last_used_at, revoked?}` for each token record; no raw token values are emitted

#### Scenario: `lmux token revoke` takes effect for new connections

- **WHEN** the user runs `lmux token revoke <id>`
- **THEN** the CLI appends a revocation record; subsequent handshake attempts using that token are rejected; no in-place mutation of prior records occurs

#### Scenario: CLI auto-token is minted on first boot

- **WHEN** the cockpit starts and `$XDG_STATE_HOME/lmux/cli-token` does not exist
- **THEN** the cockpit mints a token with scope `open.*`, writes the raw value to `cli-token` (mode 0600), and surfaces the record in `lmux token list` as `name = "cli"`

#### Scenario: CLI auto-token is picked up implicitly

- **WHEN** the user runs a scoped CLI subcommand such as `lmux open-url` without setting `LMUX_TOKEN`
- **THEN** the CLI reads `$XDG_STATE_HOME/lmux/cli-token` and presents it on the handshake automatically

## MODIFIED Requirements

### Requirement: Peer authentication via SO_PEERCRED

The bus SHALL require the connecting client's UID to match the cockpit's UID as a defence-in-depth check on the filesystem socket; primary authorization is the capability-token gate. The public abstract-namespace socket bypasses the SO_PEERCRED check and relies entirely on token scope.

#### Scenario: Same-UID connection is accepted at the socket layer

- **WHEN** a client process running under the cockpit's UID connects to `bus.sock`
- **THEN** the SO_PEERCRED gate passes; authorization of dispatched kinds is still governed by the token scope rules (internal kinds need no token, scoped kinds do)

#### Scenario: Cross-UID connection is denied at the filesystem socket

- **WHEN** a client process running under a different UID connects to `bus.sock`
- **THEN** the server reads `SO_PEERCRED`, sees the UID mismatch, and closes the connection with `error.peer_denied` before any handshake

#### Scenario: Socket file mode is 0600

- **WHEN** `ls -l $XDG_RUNTIME_DIR/lmux/bus.sock` is run while the cockpit is up
- **THEN** the mode bits show `srw-------`

#### Scenario: Public socket accepts connections regardless of UID but still requires a token

- **WHEN** `[bus].public = true` is set and a connection arrives on `@lmux-public-<uid>`
- **THEN** the server skips SO_PEERCRED, requires the handshake to present a token, and rejects with `error.unauthorized` if none is presented
