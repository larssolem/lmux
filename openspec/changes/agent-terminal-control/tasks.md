## 1. Bus and Data Model

- [x] 1.1 Add agent identity, pane title provenance, transcript range, grant, and pane placement payload structs to `lmux-bus`.
- [x] 1.2 Add bus kinds for `anchor.list`, `pane.new`, `pane.tail`, `pane.capture`, `pane.send_input`, `pane.rename`, grant requests/decisions, and MCP client role.
- [x] 1.3 Add serialization round-trip tests for every new bus kind and optional field behavior.
- [x] 1.4 Extend cockpit bus dispatcher routing so new read/write kinds are forwarded to GTK when they need live pane state.
- [x] 1.5 Add stable typed error strings or payloads for unauthorized, grant denied, transcript unavailable, stale sequence, and user-pinned title cases.

## 2. Transcript Capture

- [x] 2.1 Implement a bounded per-terminal transcript ringbuffer with sequence numbers, timestamps, decoded text, and truncation metadata.
- [x] 2.2 Wire transcript appends into the PTY reader path before terminal renderer feed.
- [x] 2.3 Expose transcript tail and capture helpers on terminal panes and AppState.
- [x] 2.4 Return a typed error when transcript requests target GUI satellite panes or missing panes.
- [x] 2.5 Add unit tests for sequence progression, tail line limits, capture-since behavior, and truncation reporting.

## 3. Terminal Tabs and Titles

- [x] 3.1 Extend layout/session model with an anchor-local terminal tab stack node and active tab selection.
- [x] 3.2 Restore legacy split layouts as a single implicit terminal tab.
- [x] 3.3 Add tab creation/switch helpers in AppState while preserving existing split-right and split-down behavior inside the active tab.
- [x] 3.4 Render a compact terminal tab strip for anchors with multiple terminal tabs.
- [x] 3.5 Add pane title state with default, agent-set, and user-pinned provenance.
- [x] 3.6 Persist user-pinned pane titles and tab-stack layout in snapshots/named sessions.
- [x] 3.7 Add restore and migration tests for legacy layouts, tab stacks, active tabs, and pinned titles.

## 4. Agent Ownership and Grants

- [x] 4.1 Parse agent identity from bus metadata and child environment variables without treating it as sole authorization.
- [x] 4.2 Store agent ownership metadata on panes created through agent-aware paths.
- [x] 4.3 Implement title rename rules: owned-agent auto rename allowed, user-pinned overwrite blocked, explicit request surfaced.
- [x] 4.4 Implement in-memory grant store with scope, requester, source anchor, target, expiry, and revoke state.
- [x] 4.5 Gate cross-anchor transcript reads, pane input, pane rename, and GUI attach against the grant store.
- [x] 4.6 Add tests for own-anchor access, cross-anchor pending grant, denial, expiry, revoke, and stronger `send-input` gating.

## 5. CLI Surface

- [x] 5.1 Add global `--json` output support to `lmux-cli` for new agent-control commands.
- [x] 5.2 Add `lmux-cli anchor list` and `anchor active`.
- [x] 5.3 Add `lmux-cli pane new --anchor <id|current> --tab|--split-right|--split-down --name <title> -- <argv...>`.
- [x] 5.4 Add `lmux-cli pane tail`, `pane capture`, `pane send`, and `pane rename`.
- [x] 5.5 Add tmux-compatible aliases where cheap: `capture-pane`, `send-keys`, and `new-window`.
- [x] 5.6 Add CLI integration tests for JSON output, text output, errors, and bus mapping.

## 6. MCP Adapter

- [x] 6.1 Create `crates/lmux-mcp` with a thin MCP stdio server or selected Rust MCP SDK integration.
- [x] 6.2 Implement MCP tools for anchor list, pane new, pane tail/capture, pane send, pane rename, list GUI windows, request attach GUI window, and launch-and-attach.
- [x] 6.3 Ensure every MCP tool calls the bus client directly and returns cockpit errors without bypassing grants.
- [x] 6.4 Add smoke tests for tool schema generation and mocked bus request mapping.

## 7. Sidebar and Workspace UI

- [x] 7.1 Show agent-owned pane chips/status under anchor rows without adding panes to the anchor rail.
- [x] 7.2 Show pending cross-anchor access requests on the target anchor row.
- [x] 7.3 Add allow once, allow timed, deny, and revoke controls in the anchor popover.
- [x] 7.4 Show pane title provenance and pinned-title state in the terminal tab UI or pane popover.
- [x] 7.5 Keep GUI satellites visually separate from terminal tabs in the active anchor surface.
- [x] 7.6 Add focused UI tests or smoke tests for request rendering and revoke state where feasible.

## 8. Agent GUI Satellite Attach

- [x] 8.1 Reuse native window listing candidates for agent attach requests.
- [x] 8.2 Implement grant-gated attach-existing-window flow with exact candidate metadata in the prompt.
- [x] 8.3 Implement launch-and-attach request flow with argv display, matching hints, timeout, and exact-window attach.
- [x] 8.4 Add tests for approved attach, denied attach, timeout, and no unrelated-window attach.

## 9. Documentation and Skill Workflow

- [x] 9.1 Document the lmux agent CLI workflow with `lmux-cli --json` examples.
- [x] 9.2 Add an agent skill draft that teaches anchor discovery, pane creation, transcript reading, and sensitive operation requests through CLI.
- [x] 9.3 Document MCP setup and explain that MCP and CLI use the same bus/grant model.
- [x] 9.4 Add security notes for terminal transcript sensitivity, `send-input`, and GUI attach approvals.

## 10. Verification

- [x] 10.1 Run `cargo test --workspace`.
- [x] 10.2 Run focused UI/e2e tests for anchors, panes, satellites, and session restore affected by tab stacks.
- [x] 10.3 Run `openspec status --change agent-terminal-control` and confirm the change is apply-ready.
- [x] 10.4 Manually smoke test: create agent pane, tail tests pane, deny cross-anchor read, allow timed cross-anchor read, rename pane, and attach GUI window.
- [x] 10.5 Split `state.rs` follow-up: extract agent pane control, grant state, and bus-facing pane helpers into focused modules once MVP behavior is verified.
