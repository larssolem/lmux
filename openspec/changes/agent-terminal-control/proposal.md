## Why

AI agents need a reliable, visible way to work inside lmux without scraping rendered terminals or hiding side effects from the user. lmux already has anchors, panes, satellites, a local bus, and a CLI; this change makes those surfaces usable for long-running agent workflows while keeping the user in control.

## What Changes

- Add per-terminal-pane transcript capture so terminal output can be tailed or captured by sequence number through the bus, CLI, and MCP adapter.
- Add terminal tab stacks within an anchor workspace so multiple terminal processes can be organized without requiring visible splits for every pane.
- Add agent-owned pane metadata: agent identity, title provenance, automatic naming, and user-pinned names that agents cannot overwrite.
- Add bus and `lmux-cli` commands for pane creation, tail/capture, send-input, rename, and anchor inventory.
- Add a local `lmux-mcp` adapter whose tools call the same bus kinds as `lmux-cli`; MCP does not get a separate authority model.
- Add cross-anchor access grants with visible sidebar approval, expiry, and revoke controls.
- Add agent-requested GUI satellite attach flows for existing external windows and launch-and-attach flows, both with user-visible approval.
- Add a documented skill/CLI workflow so agents can use lmux through terminal commands when MCP is not configured.

## Capabilities

### New Capabilities

- `agent-terminal-control`: Agent identity, agent-owned panes, MCP/CLI parity, cross-anchor grants, transcript access, and safe terminal control semantics.

### Modified Capabilities

- `bus-ipc`: Add agent/control kinds and keep authorization decisions on the cockpit side.
- `terminal-core`: Add terminal tab stacks and per-pane transcript capture.
- `sidebar`: Show agent activity, pane ownership/provenance, and cross-anchor access prompts without mixing anchors with panes or satellites.
- `satellites`: Allow agent-requested GUI window attach and launch-and-attach flows with user approval.
- `sessions`: Persist terminal tab stack layout and user-pinned pane names where needed for restore.

## Impact

- Affected crates: `lmux`, `lmux-bus`, `lmux-cli`, `lmux-pty`, `lmux-session`, and a new `lmux-mcp` crate.
- Bus API: new `Kind` variants for agent identity, pane transcript, pane creation/naming/input, grant requests, and GUI attach requests.
- UI: sidebar rows gain compact agent/grant status; active anchor workspace gains a terminal tab strip when more than one terminal tab exists.
- Security: cross-anchor reads, cross-anchor input, and GUI attach use cockpit-owned prompts and revocable grants; no network telemetry is introduced.
