## Context

lmux already has the right ownership boundary for agent terminal control: the cockpit owns PTYs, panes, anchors, satellites, sidebar state, and the local Unix-socket bus. `lmux-cli` already speaks bus kinds for sessions, anchors, panes, status, and native window attach. AI support should extend that model instead of introducing a separate terminal manager or letting MCP become the source of truth.

The user-facing product model is:

```text
Anchor = workspace
Terminal tabs/splits = lmux-owned PTYs inside that workspace
GUI satellites = external windows attached to that workspace
Agent status = visible metadata and grants around panes/windows
```

## Goals / Non-Goals

**Goals:**

- Let agents create, name, tail, capture, and optionally send input to terminal panes through the same bus semantics exposed by `lmux-cli`.
- Let users script the same operations directly from a shell without MCP.
- Provide a local MCP adapter for clients that support MCP, with no extra authority beyond the lmux bus.
- Make agent-created panes, automatic naming, cross-anchor access, and GUI attach requests visible in the sidebar/workspace UI.
- Add terminal tab stacks so agent-created long-running commands do not force visual splits.
- Keep cross-anchor reads, cross-anchor input, and GUI attach behind cockpit-owned grants.

**Non-Goals:**

- A general plugin SDK or public remote bus. This can later integrate with `plugin-sdk-public-bus`, but this change stays local-first.
- Remote/SSH session management.
- Full terminal transcript persistence across restarts. MVP transcript is live process memory.
- OCR or screen scraping of external GUI windows.
- Automatic direct control of other anchors without user-visible grants.

## Decisions

### D1 - One operation model, three frontends

Every agent-relevant operation is represented as an `lmux_bus::Kind`. `lmux-cli` and `lmux-mcp` call those kinds; a documented skill can use `lmux-cli --json` when MCP is not configured.

Alternatives considered:

- MCP-only tools. Rejected because users and non-MCP agents need the same capabilities from a terminal.
- CLI shelling out from MCP. Rejected as the primary implementation because it creates quoting/error ambiguity; the MCP adapter should use the bus client directly.

### D2 - Cockpit-owned grants

Permission checks happen in the cockpit when bus requests are handled. The MCP client may show its own consent UI, but lmux still prompts when an operation crosses a sensitive boundary.

Grant dimensions:

- requester identity: agent id/name or same-UID user process
- source anchor/pane, when known
- target anchor/pane/window
- scope: `read-output`, `send-input`, `rename`, `attach-window`
- expiry: once, timed, or revoked

Alternatives considered:

- Trust same-UID clients. Rejected for agent flows because same UID does not mean the user intended cross-anchor access.
- Implement OAuth in the MVP. Rejected; local cockpit prompts solve the immediate trust boundary and can later compose with capability tokens.

### D3 - Transcript is captured before rendering

Terminal output is recorded in a bounded ringbuffer at the PTY reader path before feeding libghostty. Entries carry a monotonically increasing sequence number, timestamp, and decoded text lossily enough for logs. Raw bytes may be retained in-memory internally, but CLI/MCP returns text plus sequence metadata.

Alternatives considered:

- Read rendered scrollback from libghostty. Rejected because it is display-oriented and loses stream ordering.
- Capture only stderr/stdout of commands started by agents. Rejected because users also need to tail existing panes and dev servers.

### D4 - Terminal tabs are workspace-local

The active anchor workspace may contain tab stacks. A tab stack selects one child terminal layout at a time; each tab can itself contain splits. Anchor rail entries remain workspaces only, and GUI satellites remain separate from terminal tabs.

Alternatives considered:

- Put every terminal under the anchor rail. Rejected because it mixes workspace navigation with process navigation.
- Replace splits with tabs. Rejected because splits remain useful for side-by-side workflows.

### D5 - Agent naming is visible and user-overridable

Pane titles have provenance: user-set, agent-set, or process/default. Agents may auto-name panes they own. Once a user manually renames or pins a pane title, agents cannot overwrite it without an explicit rename request.

Alternatives considered:

- Let agents silently rename panes. Rejected because names are navigation UI and need user trust.
- Disable auto-naming. Rejected because agent-created panes are otherwise hard to identify.

### D6 - GUI satellite attach is a request flow

Agents can list attachable native windows, request attaching an existing window, or request launch-and-attach. The cockpit shows the exact window/app/command and target anchor before granting attach.

Alternatives considered:

- Reuse `satellite.open` as the managed flow. Rejected because current specs define it as legacy spawn without ownership.
- Let agents attach focused windows silently. Rejected because it can move an external window into an anchor workspace unexpectedly.

## Risks / Trade-offs

- Transcript ringbuffer may expose secrets printed in terminals -> Cross-anchor reads require grants; own-pane reads are limited to panes the agent created or is attached to, and transcript is in-memory for MVP.
- `send-input` can be destructive -> Treat as a stronger scope than read and require explicit grants for cross-anchor targets.
- Terminal tabs add layout complexity -> Implement tab stack as a new layout node with a single active child, preserving existing split nodes inside tabs.
- Agent identity from environment can be spoofed by same-UID processes -> Use it for provenance and prompt text in MVP, not as sole authorization for sensitive cross-anchor actions.
- MCP SDK/API churn -> Keep `lmux-mcp` thin and isolate protocol code in a crate; bus kinds remain the stable internal API.

## Migration Plan

- Existing sessions restore as a single implicit tab containing the current split layout.
- Existing `pane.list`, anchor, and satellite commands keep working.
- New transcript and agent metadata are optional at runtime; absence is interpreted as no transcript or user/default provenance.
- Rollback is local: ignore tab-stack metadata and load the first tab's layout as the workspace layout if a downgrade path is needed.

## Open Questions

- Which MCP Rust SDK should be used, or should MVP start with a minimal stdio JSON-RPC implementation?
- Should `lmux-cli` expose tmux-compatible aliases (`capture-pane`, `send-keys`, `new-window`) in the first MVP or after lmux-native commands land?
- Should transcript redaction hooks exist in MVP, or is grant gating enough until dogfooding reveals concrete needs?
