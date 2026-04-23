---
title: "Product Brief: lmux"
status: "complete"
created: "2026-04-21"
updated: "2026-04-21"
inputs:
  - _bmad-output/brainstorming/brainstorming-session-2026-04-20-23-28.md
  - _bmad-output/implementation-artifacts/spec-compositor-ipc-spike.md
  - spikes/compositor-ipc/FINDINGS.md
  - spikes/libghostty-ffi/ (working FFI + GTK4 + PTY spike)
effort_assumption: "Nights + weekends, ~10–15 h/week; all timeline estimates carry an implicit 1.5–2× contingency multiplier."
audience: "Author, as direction-anchor. Personal tool first, small OSS project second. No commercial ambition at this time."
---

# Product Brief: lmux

> **Working name.** `lmux` is the project handle used in spike work. Final product name is an open decision — see ADR backlog.

## Executive Summary

lmux is a Linux-native **dev cockpit** for developers running multiple AI coding agents and parallel workstreams side-by-side. Built in Rust on top of libghostty (Ghostty's terminal core), it organises work around an **anchor + satellites** model: every workstream has one *anchor* — an agent CLI like Claude Code, or a GUI app like a JetBrains IDE — surrounded by *satellite* panes (terminals, browsers, editors) that share its identity, its notifications, and an auto-routing event bus.

The category exists on macOS (cmux, Conductor, Crystal, Claude Squad, Vibe Kanban, Antigravity) and has educated developers on the workflow. **Linux has no equivalent.** lmux fills that gap with a compositor-native implementation (KWin first), pragmatic per-pane sandboxing, and a principled refusal to reimplement what git, editors, compilers, and agents already handle.

Two highest-risk technical spikes have already returned **GO**: libghostty FFI + GTK4 + PTY rendering (working end-to-end), and KWin compositor IPC for spawn/identify/focus/move-resize/lifecycle (sub-5 ms steady-state). The build is tractable — the remaining risk is scope discipline and DX taste, not technology.

## The Problem

**Cognitive overhead, not commands.** Developers in 2026 routinely run 3–10 asynchronous coding agents in parallel — each with its own branch, its own context window, its own long-running build or test loop. The real tax is not the shell plumbing; it is keeping track of what every agent is doing and whether it needs you. A concrete, recurring failure mode: silent cross-agent overwrites and merge conflicts between worktrees.

The tooling shape is already clear:

- **macOS:** cmux, Conductor (YC S24), Crystal, Claude Squad, Vibe Kanban, Antigravity. Mature, monthly releases.
- **Linux:** nothing comparable. Users wire `tmux`/Zellij + git worktrees + custom scripts, and pay the cognitive tax.

Worktrees are the de-facto substrate *and* the de-facto weak point: ports, databases, `node_modules`, seeded environments, and auth tokens leak between parallel workspaces. JetBrains users on Linux are doubly underserved — existing orchestrators are CLI-only or VS Code / Cursor-centric.

## The Solution

lmux organises concurrent work around three primitives:

1. **Anchor.** One canonical process per workstream — usually an agent CLI or a GUI IDE. Owns the workstream's identity: name, branch, notifications, *"I'm waiting for you"* signal. **Lifecycle rule:** when the anchor exits, the workstream archives; satellites become orphaned but survivable.
2. **Satellites.** Terminals, browsers, editors, log-watchers *docked* side-by-side inside lmux. Terminals render via libghostty; GUI panes dock via compositor-orchestrated window control.
3. **Auto-routing event bus.** *Smart open* by default: a URL clicked in a terminal opens in the workstream's browser pane; a file path opens in its editor. Users should discover this by noticing it works, not by reading about it. An opt-in **editor plugin protocol** exposes richer events (agent status, build status) — spec first, implementations later.

Layered on top:

- **Per-pane sandboxing** (v0.3): Bubblewrap-backed isolation with named templates, overlay filesystem, `lmux commit` / `lmux revert`, network deny-by-default with palette-granted allowlist. Composes with the tooling agents already use.
- **Real notifications.** Bell → freedesktop notification (v0.1). Priority tiers (v0.3). Cross-session reach so notifications escape the current workspace.
- **Cohesive workspace feel.** Minimal chrome. No magic. lmux is a **tenant** in your compositor, **never** a window-manager replacement.

**Day in the life (v0.3 target):** Open lmux. Super+N → new workstream, anchor = Claude Code, satellites auto-attached (terminal, browser, JetBrains IDE docked). Claude finishes a task and rings the bell; a notification says `"[checkout-api] Claude is waiting for input"`; clicking jumps focus back. Meanwhile workstream `[billing-fix]` is in a sandbox template with a scratch Postgres schema and no outbound network. Close it; overlay diff appears — commit or revert.

## What Makes This Different

- **The high-risk bits already work.** libghostty FFI + PTY renders correctly from Rust. KWin compositor IPC for spawn/identify/focus/move-resize/lifecycle returned GO with sub-5 ms steady-state. Receipts in `spikes/compositor-ipc/FINDINGS.md` and `spikes/libghostty-ffi/`. Solo OSS projects build trust by showing evidence, not claiming conclusions.
- **Single static Rust binary.** One binary, no runtime, no daemon, no Electron. Opens instantly. For the target audience (Linux power users burned by Electron and Python dependency hell) this is a trust signal, not a footnote.
- **Anchor + satellites is composable.** Competitors flatten everything into "workspaces" (Conductor) or "agent cards on a kanban" (Vibe Kanban, Antigravity). Anchor + satellites lets any CLI agent or GUI IDE be an anchor, and any terminal / browser / editor be a satellite.
- **Sandboxing that's actually smooth.** Bubblewrap is already the Linux consensus (Claude Code defaults to it). lmux's differentiator is the *UX* around it: outcome-named templates (`isolated`, `networked`, `untrusted`), commit / revert from the pane, network policy as a palette action. The hard part is getting shared Postgres, shared `node_modules`, and shared auth tokens *right by default* — that is ADR-7 and will be decided by the Bubblewrap spike.
- **Editor-agnostic, with JetBrains windows that dock cleanly.** JetBrains + Linux + AI agents is a genuinely abandoned intersection. Pseudo-embedding makes a JetBrains IDE a first-class *docked anchor* from v0.2 onward. A Kotlin plugin that speaks the active event bus is v0.4 work — not v0.3.
- **Display, Don't Duplicate** as a promise, not a constraint: **lmux will never become a kanban board, never ship its own git UI, never embed an LLM, never become a secrets vault.** A published anti-roadmap is rare, memorable, and compounds trust.

The honest moat is execution speed in an open lane. The lane *will* close — Conductor has YC money, Antigravity is Google, any of them can ship a Linux build in a quarter if they see pull.

## Who This Serves

**Primary user (v0.1 → v0.2):** The author. Linux developer running multiple Claude Code / Codex / Aider sessions in parallel, alongside a JetBrains IDE, on KDE Plasma Wayland.

**Secondary user (v0.3+):** Linux developers who know the Mac tools exist and want the same thing on their desktop — especially JetBrains, Hyprland / Sway, and existing Ghostty users.

**Not targeted:** Windows, macOS, casual terminal users content with `tmux`, non-developers.

## Success Criteria (falsifiable)

- **v0.1 (MVP, ~3 weeks calendar):** `konsole` + Claude Code anchor + one terminal satellite + bell → freedesktop notification works end-to-end on KWin. Author runs **2 Claude Code anchors + JetBrains IDE for a full working day** and logs zero fallbacks to `tmux` for that day.
- **v0.2 (dogfood, ~1 month after v0.1):** Multi-session, fuzzy switcher, sidebar, GUI satellite docking on KWin. Author uses lmux for **10 consecutive working days**, ≥3 parallel agents, zero fallback sessions, ≤2 crash-restarts/week, pain log written.
- **v0.3 (first recommendable release, ~3 months after v0.2):** Sandbox shipped with at least the `isolated` template (read-only host, writable overlay, no network) + palette-granted network toggle. Compositor docking works on KWin without visible glitches. Wlroots = best-effort (panes work, docking may be disabled). A second Linux developer can honestly be pitched to try it.

**Anti-metrics (fail-signals):** lmux processes outlive the compositor session. Sandbox defaults need a 5-step README to be productive. Docked GUI windows visibly detach, flicker, or lag on a normal resize.

## Kill Criteria (when to stop or pivot)

- libghostty embed API changes incompatibly **twice in 3 months**.
- Wlroots compositor docking still glitches after **3 focused weeks** of effort → cut to KWin-only for v0.3.
- Bubblewrap spike returns NO-GO on "shared Postgres / `node_modules` / auth tokens" defaults → cut sandbox scope to opt-in isolated-only; defer commit/revert.
- Author stops using lmux for **2 consecutive weeks** during dogfood → halt and review.

## Spike Confidence Map

| Status | Item |
|--------|------|
| **Proven** | libghostty FFI + GTK4 + PTY (full render, keyboard, resize). KWin compositor IPC spawn/identify/focus/move-resize/lifecycle (GO, sub-5 ms). |
| **Plausible** | Wlroots (Hyprland or Sway) compositor IPC — Phase 2 spike deferred but designed. Bubblewrap sandbox with smart defaults — consensus primitive. |
| **Assumed** | Auto-routing event bus protocol spec. JetBrains-on-Linux wedge is large enough to matter. Freedesktop notification behaviour is consistent across dunst / mako / swaync. |

## Scope

**In (v0.1 → v0.3):**

- Rust host, libghostty FFI rendering, portable-pty, GTK4 surface
- Anchor + satellites + auto-routing bus primitives
- KWin-first compositor IPC; wlroots backend best-effort by v0.3
- Bubblewrap sandbox: `isolated` template + network toggle (v0.3)
- Bell → freedesktop notifications (v0.1); priority tiers (v0.3)
- Editor plugin protocol **spec** (v0.3) — reference implementation deferred

**Out (through v0.3):**

- Conductor-style multi-agent kanban or dashboard UI
- Remote / SSH / multi-host sessions (v0.4+; pane graph design stays host-agnostic)
- GNOME / Mutter; X11 (v0.4+ backlog)
- JetBrains Kotlin plugin (v0.4; context-bus spec ships first)
- Click-to-teleport notifications, tab-edge glow (v0.4)
- Sandbox `commit` / `revert`, multi-template palette (v0.4)
- Window-manager-replacement behaviours: global keybind capture, summon overlay, per-project workspace auto-binding
- Reimplementation of git, LSP, build tooling, agent-specific chrome
- Task tracker, prompt manager, agent router, secrets vault — **lmux never becomes any of these**

## Vision

If v0.3 lands and is adopted, the 12-month picture: lmux becomes the default Linux workspace for developers running AI agents in parallel. Context-bus plugins exist for Helix, Neovim, Zed, VS Code, Cursor — and a Kotlin plugin for JetBrains. Sandbox backends extend to Podman and optionally Firecracker. Remote spawning lands as first-class; JetBrains Gateway becomes a pane type. The long ambition is not to compete with IDEs or agents — it is to be the boring, cohesive *display surface* they all plug into. The workspace tenant. Never the window manager.

## Open Decisions (ADR Backlog)

Grouped by the milestone each one blocks.

**Blocks v0.1:**
1. Config format: TOML vs KDL.
2. KWin script lifecycle: does a long-lived registration for `windowAdded` / `windowRemoved` leak across lmux restarts? Prototype required.

**Blocks v0.2:**
3. Wlroots backend choice: Hyprland or Sway as primary, based on IPC ergonomics and author device.
4. Auto-routing event bus transport: D-Bus vs Unix socket vs stdio; minimum v0.1 event set.

**Blocks v0.3:**
5. Sandbox defaults (ADR-7): what is shared vs isolated by default — the UX battle of the sandbox story. Drives the Bubblewrap spike.
6. Session state persistence: append-only log vs snapshot — marked "earn it or drop."

**Deferred (does not block v0.3):**
7. Product name: `lmux` is working; final name needed before public v0.3 pitch.
8. Distribution channel: static binary vs Flatpak vs AUR as authoritative.
