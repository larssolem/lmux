## 1. Bus kind schemas

- [ ] 1.1 Add `OpenUrl { url: String, hint: Option<String> }` to `crates/lmux-bus/src/kinds.rs` with serde tag `"open.url"`
- [ ] 1.2 Add `OpenPath { path: String, line: Option<u32>, column: Option<u32>, hint: Option<String> }` with serde tag `"open.path"`
- [ ] 1.3 Add `OpenResult { routed_to: RoutedTo }` where `RoutedTo ∈ { Subscriber { pane_id }, FocusedFallback { pane_id }, PlatformDefault, Rejected { reason } }`; response kind for both intents
- [ ] 1.4 Unit tests: round-trip every payload shape through serde; deny-list tests for malformed payloads

## 2. Router module

- [ ] 2.1 Create `crates/lmux/src/smart_open/mod.rs` with `Router { subscribers, capabilities, defaults }`
- [ ] 2.2 `SubscriberStore` backed by `HashMap<PaneId, Vec<GlobMatcher>>` compiled via `globset`
- [ ] 2.3 `Router::route_url(url, hint) -> RoutedTo`: subscriber → focused browser capability → `xdg-open` fallback
- [ ] 2.4 `Router::route_path(path, line, col, hint) -> RoutedTo`: subscriber → focused editor capability → `$EDITOR` in new satellite fallback
- [ ] 2.5 `file://` URLs route through `route_path` after strip
- [ ] 2.6 Unit tests covering each fallback tier and the precedence rules

## 3. Subscription surface

- [ ] 3.1 `subscribe.smart_open { pane_id, patterns: [String] }` bus kind that writes into `SubscriberStore`
- [ ] 3.2 `unsubscribe.smart_open { pane_id }` kind clears all patterns for a pane
- [ ] 3.3 Pane close-pane handler removes its subscriptions
- [ ] 3.4 Unit tests: pattern registration + teardown idempotence

## 4. Capability tokens on anchors

- [ ] 4.1 Add `capabilities: Vec<String>` field to the anchor metadata struct in `crates/lmux-anchor/src/registry.rs`
- [ ] 4.2 Persist `capabilities` in the session TOML (additive, backward-compatible)
- [ ] 4.3 Sidebar pane-row menu: "Set as editor" / "Set as browser" toggles
- [ ] 4.4 Migration from v0.2 sessions: empty list on first read

## 5. CLI subcommands

- [ ] 5.1 `lmux open-url <url> [--hint <str>]`: one bus call → `open.url`
- [ ] 5.2 `lmux open-path <path[:line[:col]]> [--hint <str>]`: parse the trailing `:line[:col]` form
- [ ] 5.3 `--json` flag on both prints the `OpenResult` payload unmodified
- [ ] 5.4 Exit code: `0` routed, `1` no handler, `2` bus error
- [ ] 5.5 Round-trip integration test against a running cockpit

## 6. Default-handler fallbacks

- [ ] 6.1 `xdg-open` spawn for URL fallback; capture exit code into the toast on failure
- [ ] 6.2 `$EDITOR` spawn inside a fresh satellite for path fallback; preserve `line`/`column` where the editor supports `file:line:col` syntax
- [ ] 6.3 Config keys `[smart_open] url_fallback = "xdg-open"` and `path_fallback = "$EDITOR"` in `lmux.toml`
- [ ] 6.4 If `$EDITOR` unset AND no capable pane, return `error.no_handler` and toast

## 7. Observability

- [ ] 7.1 Span `smart_open.route { kind, hint }` with fields for matched tier
- [ ] 7.2 Counters: `smart_open.intents_received` (by kind tag), `smart_open.subscriber_hits`, `smart_open.fallback_hits`
- [ ] 7.3 `lmux status` prints the smart-open counters when non-zero
- [ ] 7.4 `error.no_handler` is a typed kind distinct from `error.malformed_body`

## 8. Sidebar UI

- [ ] 8.1 Pane-row context menu: "Smart-open patterns…" opens a popover editing the glob list
- [ ] 8.2 Live pattern-test input so the user can paste a sample path and see "matches" / "doesn't match"
- [ ] 8.3 "Set anchor capability" submenu toggles `editor` / `browser`
- [ ] 8.4 The popover persists changes through the bus (`subscribe.smart_open`)

## 9. Documentation

- [ ] 9.1 README section: "Using lmux smart-open from the shell and editor plugins"
- [ ] 9.2 Add an example `.zshrc` alias snippet (`alias vo='lmux open-path'`)
- [ ] 9.3 Cross-link from `plugin-sdk-public-bus` proposal: "smart-open intents are the first kinds external clients will want to send"
- [ ] 9.4 Update ADR-0016 "Deferred to v0.3" section: mark `open.url` / `open.path` as shipping in v0.3 via this change
