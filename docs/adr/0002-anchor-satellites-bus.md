# ADR-0002: Anchor + Satellites + Smart-open bus

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.1

## Context

Existing Mac products model parallel agent work as flat "workspaces" (Conductor, Crystal) or as kanban cards on a board (Vibe Kanban, Antigravity, Cursor Background Agents). Flat workspaces conflate identity with layout; kanbans remove the live terminal/editor surface that's the whole point of using an agent in the first place.

lmux needs a model that (a) gives each workstream a single identity for notifications, branch context, and "I'm waiting" signals, (b) keeps live terminals / editors / browsers composable inside that workstream, (c) stays neutral across CLI agents and GUI IDEs, (d) works without scope-creeping into orchestration.

## Decision

Three primitives:

1. **Anchor.** Exactly one canonical process per workstream, normally a CLI agent (Claude Code, Codex, Aider) or a GUI app (JetBrains IDE). Owns the workstream's name, branch, notifications, and "waiting-for-input" state. **Lifecycle rule:** anchor exit archives the workstream; satellites become orphaned but survivable.
2. **Satellites.** Terminals, browsers, editors, log-watchers attached to the anchor and *displayed* side-by-side. Richer pane types (browser, log-watcher) are v0.3+.
3. **Smart-open event bus.** Two tiers:
   - *Passive* (v0.1): URL and file-path auto-routing between panes. No cooperation required from the satellite.
   - *Active* (v0.3 spec, v0.4+ impl): opt-in plugin protocol for agent/build status. One spec; any editor can implement.

User-facing vocabulary: **anchor**, **satellites**, **smart open**. Internal term "tiered context bus" is kept only in architecture docs.

## Alternatives considered

- **Flat workspaces (Conductor-style).** Rejected: loses single-identity focus; notifications and "whose turn is it" get muddy across N panes of equal weight.
- **Kanban / agent cards (Vibe Kanban-style).** Rejected: removes the live surface; violates "tenant, not platform" and Display-Don't-Duplicate.
- **Pane-group with no identity** (tmux / Zellij). Rejected: the author's real pain is not layout, it's tracking what each agent is doing across N workstreams.
- **One-anchor-per-pane-type** (e.g. separate anchors for agent + IDE). Rejected: each workstream is one unit of attention; multiple anchors per workstream defeats the notifications model.

## Consequences

- **+** Novel mental model; differentiates from macOS incumbents.
- **+** Composable — any CLI agent or GUI IDE can be an anchor.
- **+** Notification, sandbox, and context-bus semantics have a natural owner (the anchor).
- **−** New vocabulary to teach. Mitigation: passive smart-open ships without ever naming the bus; users discover it by noticing it works.
- **−** "Anchor exit archives workstream" is a strong rule; users may want to replace a crashed anchor without losing satellites. Mitigation: satellites survive as orphans, can be re-adopted by a new anchor on restart (v0.2 behaviour).

## Follow-up

- Define the *minimum* passive smart-open event set before v0.1 (URL intent, file-path open intent).
- Draft the active-tier protocol spec during v0.3 planning.
