# Architecture Decision Records — lmux

Compact Nygard-style ADRs for lmux. One decision per file, numbered sequentially.

Current behavior lives in [`../../openspec/specs/`](../../openspec/specs/).
ADRs record why a direction was chosen at the time. Some older Accepted ADRs
describe historical implementation plans that have since been superseded by the
living capability specs or by later implementation work.

## Template

```
# ADR-NNNN: Title

- Status: Accepted | Proposed | Deferred | Superseded
- Date: YYYY-MM-DD
- Deciders: …
- Supersedes: — / ADR-NNNN
- Blocks: v0.1 | v0.2 | v0.3 | none

## Context
## Decision
## Alternatives considered
## Consequences
## Follow-up (optional)
```

## Status legend

- **Accepted** — decision taken; may be revisited only with supersession.
- **Proposed** — decision needed but blocked on a spike / prototype / external signal. Hypothesis recorded.
- **Deferred** — decision intentionally postponed until a named trigger.
- **Superseded** — replaced by a later ADR. Kept for history.

## Index

### Foundational

- [ADR-0001](0001-rendering-stack.md) — Rendering stack: Rust + libghostty + GTK4 + portable-pty — **Accepted**
- [ADR-0002](0002-anchor-satellites-bus.md) — Anchor + Satellites + Smart-open bus — **Accepted**
- [ADR-0003](0003-path-a-spawn-and-track.md) — Historical Path A: spawn-and-track (no Wayland reparenting) — **Superseded**
- [ADR-0004](0004-compositor-control-trait.md) — CompositorControl backend abstraction — **Accepted**

### Platform

- [ADR-0005](0005-kwin-mvp-compositor.md) — KWin as MVP compositor; wlroots secondary — **Accepted**
- [ADR-0006](0006-wlroots-backend-hyprland.md) — Wlroots primary backend: Hyprland over Sway — **Accepted**
- [ADR-0011](0011-kwin-script-lifecycle.md) — KWin script lifecycle strategy — **Accepted** (lifecycle-probe spike; restart scenario deferred)

### Stack choices

- [ADR-0007](0007-config-format-toml.md) — Config format: TOML — **Accepted**
- [ADR-0008](0008-bus-transport-unix-socket.md) — Smart-open bus transport: Unix domain socket — **Accepted**
- [ADR-0009](0009-sandbox-primitive-bubblewrap.md) — Bubblewrap as v0.3 sandbox primitive — **Accepted**
- [ADR-0010](0010-sandbox-defaults.md) — Sandbox shared-vs-isolated defaults — **Accepted** (single-tier bubblewrap default; spike → validation during v0.3)

### Operations / policy

- [ADR-0012](0012-session-persistence-earn-it.md) — Session state persistence: earned snapshot layers — **Accepted**
- [ADR-0013](0013-distribution-static-binary.md) — Distribution channel: static binary authoritative — **Accepted**
- [ADR-0014](0014-product-name.md) — Product name — **Deferred**

### v0.2 — bus, satellites, compositor pivot

- [ADR-0015](0015-lmux-bus-architecture.md) — lmux-bus architecture (transport, framing, kinds) — **Accepted**
- [ADR-0016](0016-smart-open-event-set-v0.2.md) — lmux bus v0.2 kind catalog — **Accepted**
- [ADR-0017](0017-compositor-control-v0.2-surface.md) — `CompositorControl` current method surface — **Accepted**
- [ADR-0018](0018-nested-wayland-compositor-for-satellites.md) — Nested Wayland host for pane-native satellites — **Accepted**

### v0.2 — UX

- [ADR-0019](0019-rearrange-mode-dnd-on-split-tree.md) — Rearrange mode: drag-and-drop on the split tree — **Accepted**
- [ADR-0020](0020-clipboard-image-paste-via-tempfile.md) — Clipboard image paste via tempfile + path injection — **Accepted**
