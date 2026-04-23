---
stepsCompleted: [1, 2, 3, 4]
inputDocuments: []
session_topic: 'Rust-based spiritual successor to cmux for Linux, built on libghostty, with deeper JetBrains tooling integration'
session_goals: 'Generate a broad and deep idea pool (100+) covering architecture, feature set, UX, JetBrains integration, and differentiation, to later inform a product brief/PRD'
selected_approach: 'progressive-flow'
techniques_used: ['What If Scenarios', 'Mind Mapping', 'SCAMPER Method', 'Resource Constraints']
ideas_generated: 60
design_principles: 3
mvp_decisions: 7
risks_flagged: 2
phase1_complete: true
phase2_complete: true
phase3_complete: true
phase4_complete: true
session_complete: true
context_file: ''
---

# Brainstorming Session Results

**Facilitator:** Lars
**Date:** 2026-04-20

## Session Overview

**Topic:** Rust-based spiritual successor to cmux for Linux, built on libghostty, with deeper JetBrains tooling integration.

**Goals:** Generate a broad, divergent pool of 100+ ideas across architecture, feature set, UX, JetBrains integration, and differentiation — enough raw material to later inform a product brief / PRD.

### Session Setup

**Founding constraints (from the user):**

- Host language: **Rust**.
- Terminal rendering: **libghostty** (Ghostty's library, Zig-authored — FFI from Rust).
- Not a port of cmux — a **spiritual successor** with roughly similar capabilities.
- **Differentiator:** deeper integration with **JetBrains tools** (IDEs, Gateway, Toolbox, Fleet, plugin ecosystem).
- Target OS: **Linux** (DE + Wayland/X11 considerations in scope).

---

## Technique Selection

**Approach:** Progressive Technique Flow
**Journey:** Divergent → Pattern Recognition → Development → Action

| Phase | Technique | Purpose |
|-------|-----------|---------|
| 1 — Expansive Exploration | What If Scenarios | Maximize creative breadth |
| 2 — Pattern Recognition | Mind Mapping | Cluster raw ideas into branches |
| 3 — Idea Development | SCAMPER | Refine top concepts |
| 4 — Action Planning | Resource Constraints | Ruthless MVP scoping |

---

## Phase 1 — Expansive Exploration (What If Scenarios)

**Total ideas generated in Phase 1:** 34

**Emergent taste filter (from Lars's reactions):**

- **Pragmatic and low-magic** — reject clever auto-behaviors; keep mental model simple.
- **Co-existence over takeover** — lmux is a well-behaved tenant inside your existing WM.
- **Terminal first, editor/browser second and third** — don't pretend to be an IDE.
- **Agent-agnostic** — Claude Code / Codex / Aider / anything. No vendor lock-in.
- **User configurability is non-negotiable** — keybinds, mouse, layout.

### Idea Pool

#### Desktop / Workspace Topology

- **[Desktop #1] ✓** *Workspace Workbench (minimal)* — lmux is a normal app window that's happiest when given a whole virtual desktop/workspace to itself. No WM takeover. **(User-endorsed as the direction.)**
- **[Desktop #2]** *Per-Project Workspace Binding* — auto-create workspace per project. *(Parked: too magical.)*
- **[Desktop #3]** *Multi-Desktop Spillover* — single session spans two adjacent workspaces. *(Parked.)*
- **[Desktop #4]** *Summon Overlay (Wayland layer-shell)* — Super+Space system-wide dev launcher. *(Parked.)*
- **[Desktop #5]** *Focus-Mode Keybinds* — rebind globals when lmux has focus. *(Parked.)*
- **[Desktop #6]** *WM-Agnostic via Freedesktop Protocols* — target ext-workspace-v1, freedesktop.Notifications, etc. **(Retained as a design principle.)**

#### Session Model

- **[Session #1]** *One Window, Many Sessions (lmux-internal switcher)* — sessions swap inside lmux, not via WM.
- **[Session #2]** *Session = Everything* — atomic swap of pane tree + processes + anchor + satellites + scrollback.
- **[Session #3]** *Fuzzy Session Switcher (Super+K)* — palette over sessions by name, recency, last command, last agent activity.
- **[Session #4]** *Optional Chrome, Not a Status Bar* — because libghostty gives us a GPU surface, chrome is real UI, not ANSI tricks.

#### Anchor + Satellites Model (core architectural insight)

- **[Anchor #1]** *Anchor-and-Satellites Session Model* — every session has one anchor + N satellites.
- **[Anchor #2]** *Agent-Agnostic Chat Detection* — auto-tag agent CLIs; explicit tag override.
- **[Anchor #3]** *First-Class Pane Types* — Shell, Agent Terminal, Browser, Editor/IDE, Log-watcher, GUI-app-embed.
- **[Anchor #4]** *Satellite Browser* — per-session browser with scoped cookies/auth, auto-opens agent URLs.
- **[Anchor #5]** *Session Switcher = Anchor Switcher* — pick by live work, not abstract name.
- **[Anchor #6]** *GUI App as Anchor* — IDE / Figma / Chromium as anchor with terminals grouped around it.
- **[Anchor #7]** *Context Bus — Satellites See the Anchor* — pub/sub of anchor events; satellites subscribe.
- **[Anchor #8]** *Tiered Context Bus* — passive default (auto-wire URLs/files); active opt-in (agents address satellites).

#### Remote / Multi-Host

- **[Remote #1]** *Machine-Agnostic Pane Graph* — each pane has a `host` attribute; session tree spans hosts natively.
- **[Remote #2]** *Half-and-Half Sessions* — local anchor + remote satellites (or vice-versa).
- **[Remote #3]** *Gateway-Native Pane* — JetBrains Gateway thin-client as a pane type.
- **[Remote #4]** *Don't Reinvent Mosh* — plain SSH + reconnect; pragmatism ceiling.

#### Notifications & Attention

- **[Notif #1]** *Native Linux Notifications (freedesktop)* — uses existing notification stack.
- **[Notif #2]** *Attention-Required Signals* — per-pane state: idle/active/waiting-for-input/errored/finished.
- **[Notif #3]** *Actionable Toasts* — click toast, teleport to pane.
- **[Notif #4]** *Priority Tiers + DND* — critical / info / chatter; per-session and global mute.
- **[Notif #5]** *In-UI Subtlety: Tab-Edge Glow* — colored edges, no screaming popups.
- **[Notif #6]** *Cross-Session Reach* — notifications escape the current session.
- **[Notif #7]** *Design Principle — Minimal Chrome + Real Notifications* — codifies the trade-off.

#### Input / Control

- **[Input #1]** *Configurable Keybindings as Foundation* — plain-file config (TOML/KDL). Nothing hardcoded-only.
- **[Input #2]** *First-Class Mouse* — click, drag, right-click menus, scroll — all native via libghostty surface.
- **[Input #3]** *Mouse + Keybinds Co-Exist, No "Modes"* — move mouse, mouse works; type, keyboard takes over.

#### Agent Safety / Sandbox

- **[Sandbox #1]** *Per-Pane Sandbox Flag* — opt-in per pane; user-namespace + bubblewrap isolation; distinct visual border.
- **[Sandbox #2]** *Overlay Filesystem* — writes go to overlay; `lmux commit` promotes, `lmux revert` throws away.
- **[Sandbox #3]** *Network Policy per Sandbox* — deny-by-default outbound; palette-grant per host.
- **[Sandbox #4]** *Sandbox Templates* — named presets (build-safe, test-net-ok, untrusted-full).
- **[Sandbox #5]** *Commit / Revert from the Pane* — close-pane diff review, gated promotion.

#### Persistence (lukewarm)

- *Session state as append-only log — syncable, replayable, fallback-readable.* Retained as a **Phase-3 "earn it" candidate.**

---

## Phase 2 — Pattern Recognition (Mind Map)

### Branch Summary

| # | Branch | Ideas | Quality | Status |
|---|--------|-------|---------|--------|
| 1 | **Anchor + Satellites** ⚡ | 8 | Deep + novel | **CORE architectural insight** |
| 2 | Session Model | 4 | Grounded | Foundation — decisions locked |
| 3 | Desktop Topology | 1 keep + 5 parked | Clarified by subtraction | Decided: minimal tenant |
| 4 | Remote / Multi-Host | 4 | Strong | User-endorsed as first-class |
| 5 | Notifications | 7 | Surprisingly rich | Strong design principle emerged |
| 6 | Input / Control | 3 | Fundamentals | Must-have, not negotiable |
| 7 | Sandbox / Safety | 5 | User-endorsed after parking | Feature, not narrative |
| 8 | JetBrains Integration | 2 | **Under-explored** | Gap to plug in Phase 3 |
| 9 | Persistence | 1 lukewarm | Weak | Phase 3 "earn it or drop" |

### Phase 3 Candidates (SCAMPER targets)

1. **Anchor + Satellites + Context Bus** — core architectural insight; must flesh out
2. **Sandbox / Safety Model** — user-endorsed, buildable, differentiating
3. **JetBrains Integration (re-scoped)** — fill the under-explored gap with the satellite/anchor primitive
4. Notifications + Minimal Chrome Design Principle *(optional, revisit if time permits)*

---

## Phase 3 — Idea Development (SCAMPER)

### Concept #1: Anchor + Satellites + Context Bus — developed

**SCAMPER Substitute — Anchor = "the thing this workstream is about":**

- **[SCAMPER-S #1]** Anchor generalizes — agent CLI, GUI app, JetBrains IDE, any focal process
- **[SCAMPER-S #2]** Parallel workstreams (2–5+ anchors running simultaneously) as the first-class default
- **[SCAMPER-S #3]** Worktree-bound anchor is a common pattern, but lmux doesn't manage the worktree itself

**SCAMPER Combine:**

- **[SCAMPER-C ✓]** Agents sandboxed by default; GUI apps not (opt out, not opt in)
- **[SCAMPER-C ✗]** Anchor+worktree auto-pairing — rejected (git is not lmux's job)

**SCAMPER Modify/Magnify — scale to 5–10 parallel workstreams:**

- **[SCAMPER-M #1]** Persistent sidebar with attention indicators — click to switch, color/badge states
- **[SCAMPER-M #2]** Manual Pause / Hide / Resume — user-explicit lifecycle
- **[SCAMPER-M ✗]** Auto-suspend of idle anchors — rejected (too magical, loses overview)
- **[SCAMPER-M #3]** Anchor tags for filter/grouping
- **[Remote #5]** Attach to remote running process (Waypipe / xpra / SSH+reattach / Gateway) — mechanism per pane type, open research question

### Concept #2: Sandbox / Safety Model — developed

**SCAMPER Adapt — pluggable backends:**

- **[Sandbox #6]** Tiered sandbox backends: None → bubblewrap → LXC/Podman/Docker → microVM (firecracker/crosvm)
- **[Sandbox #7]** Defer backend choice to implementation — design API abstract; pick tech during build

### Concept #3: JetBrains Integration — re-scoped

- **[JetBrains #1]** JetBrains as anchor AND satellite — both modes
- **[JetBrains #2]** Editor-agnostic context-bus protocol (VSCode, Cursor, Helix, Neovim, Zed, JetBrains all equal)
- **[JetBrains #3]** JetBrains-specific Kotlin plugin — deferred until anchor semantics mature
- Verdicts on the 5 candidate features — see below:

| # | Feature | Verdict |
|---|---------|---------|
| 1 | IDE as satellite | **CORE** (generic anchor/satellite) |
| 2 | Context bus ↔ plugin (passive) | **CORE, editor-agnostic** |
| 3 | Gateway as pane type | Nice to have, later |
| 4 | Run configs from palette | Nice to have, deferred |
| 5 | Replace JB terminal panel | Out |

### Design Principles Crystallized

- **[Principle #1]**: *Display, Don't Duplicate* — lmux adds multiplexing, sandboxing, context bus, notifications. Git / LSP / editing / build / test stay with the tools that already do them well.
- **[Principle #2]**: *Editor-Agnostic > Vendor Deep Dive* — a thin plugin protocol any editor can adopt beats a fat plugin for one vendor.

### Differentiator Re-Framing

**Original pitch:** "Linux cmux clone with JetBrains integration."

**Post-Phase-3 pitch:** "A Linux-native, Rust+libghostty dev cockpit for running multiple AI agents and editors in parallel — with smooth per-pane sandboxing, an anchor-and-satellites workstream model, and editor-agnostic integration (including JetBrains)."

---

## Phase 4 — Action Planning (Resource Constraints)

### MVP Decisions Locked

- **[MVP Decision #1]** Anchor/Satellite is core identity — in from v0.1, not deferred.
- **[MVP Decision #2]** Bell forwarding is the minimum attention signal (every terminal app already rings it).
- **[MVP Decision #3]** Limited satellite pane types are OK for MVP; richer types (browser, editor, log-watcher) are v0.3+.
- **[MVP Decision #4]** GUI satellites via compositor IPC (Path A — spawn-and-track, not reparenting).
- **[MVP Decision #5]** True embedding is a stretch goal; revisit if Wayland support ever matures.
- **[MVP Decision #6]** v0.2 is the personal-dogfood bar — Lars switches to lmux here.
- **[MVP Decision #7]** v0.3 must include Sandbox + cohesive-embedding story — the differentiators.

### Risks Flagged

- **[Risk #1]** Compositor fragmentation — Hyprland / Sway / KWin / Mutter / niri each have different IPC. Pick minimum viable set per release; document supported compositors honestly.
- **[Risk #2]** "True embedding" is a design illusion on Wayland — Wayland's isolation model rules out real reparenting. Pseudo-embedding via compositor window-rules is the closest you can ship.

### The Roadmap

| Release | Scope Summary | Time | Success Criterion |
|---------|---------------|------|-------------------|
| **v0.1** | Anchor/satellite minimum + libghostty FFI + bell→freedesktop-notify. One session, many terminal panes, manual anchor tag, H+V splits, mouse focus/resize, hardcoded keybinds. | ~3 weeks | **Proof of life:** "I can run Claude + tests side-by-side and get pinged." |
| **v0.2** | Multi-session + fuzzy switcher (Super+K) + sidebar + manual pause/hide/resume + anchor auto-detection + anchor tags + GUI satellites (Path A) on **Hyprland + Sway** + tab-edge glow + configurable TOML keybinds. | ~1 month | **Personal dogfood ready** (Lars switches from cmux/tmux). |
| **v0.3** | Sandbox (bubblewrap + templates + commit/revert) + cohesive embedding (compositor-orchestrated pseudo-embedding on Wayland + XEmbed for X11 apps) + KWin + Mutter support + notification tiers + DND + passive context bus (if time) + minimal remote SSH spawn (if time). | ~3 months | **First recommendable release.** |
| **v0.4+** | Context-bus active tier, editor plugins (VSCode, Cursor, Helix, Neovim, Zed, JetBrains Kotlin), LXC/Podman/microVM sandbox backends, remote attach (Waypipe/xpra/tmux-attach), JetBrains Gateway as pane type. | TBD | **Serious multi-agent cockpit.** |

**Total zero → v0.3:** ~4–5 months focused solo dev.

### v0.1 Detailed Slice (the weekend-plus-some)

- Rust host binary
- libghostty FFI — PTYs render
- One window, one session, many terminal panes (H + V splits)
- Mouse: click-to-focus, drag-to-resize dividers
- Keybinds: hardcoded defaults (spawn, close, split, focus)
- Manual anchor tag via `lmux-cli mark-anchor` or `Super+A`
- Visual highlight on the anchor pane's border
- Bell (`\a`) from any pane → freedesktop notification (pane-named, clickable)
- Save/restore pane layout on quit/launch (single session)

**Explicitly NOT in v0.1:** sandbox, context bus, remote, sidebar, session switcher, GUI satellites, editor/browser pane types, JetBrains plugin.

### Pre-Code Validation Spikes

Before locking the roadmap with any real confidence, prototype these (1–3 days each):

1. **libghostty FFI from Rust** — confirm Zig library linking, PTY rendering, input handling work. This is the riskiest dependency.
2. **Hyprland + Sway compositor IPC spike** — confirm you can spawn an external window, track its toplevel, focus it on demand, position/resize it. This validates Path A entirely.
3. **Bubblewrap sandbox prototype** — confirm you can run a shell in an overlay-FS sandbox with net deny, then promote writes. Validates v0.3's flagship differentiator.
4. **freedesktop notifications integration** — confirm actionable toasts work across GNOME/KDE/Hyprland's notification daemons (dunst/mako/swaync).

**If any of these spikes fail or are dramatically harder than expected, the roadmap changes.** Don't skip them.

---

## Final Synthesis

### The Project in One Paragraph

**lmux** is a Linux-native, Rust + libghostty dev cockpit for running **multiple AI agents and editors in parallel**. Each parallel workstream centers on an **anchor** (an agent CLI like Claude Code / Codex / Aider, or a GUI app like RustRover) surrounded by **satellite panes** (terminals, browsers, editor/IDE windows) connected through a tiered context bus. lmux adds the things nobody else does well on Linux — **smooth per-pane sandboxing, cohesive multi-workstream orchestration, pragmatic notifications** — and deliberately refuses to reimplement what good tools (git, editors, compilers, agents) already handle. It's a **workspace tenant, not a window manager** — claims a virtual desktop and owns it, leaving the rest of your system untouched. Editor-agnostic from day one; JetBrains happens to be well-supported because the author uses it.

### Design Principles

1. **[Principle #1] Display, Don't Duplicate** — lmux adds multiplexing, sandboxing, context bus, and notifications. Git, LSP, editing, build/test stay with the tools that already do them well.
2. **[Principle #2] Editor-Agnostic > Vendor Deep Dive** — a thin plugin protocol any editor can adopt beats a fat plugin for one vendor. JetBrains is first-class but not privileged.
3. **[Principle #3] Cohesive Workspace Feel Is the Success Bar** — mechanism is invisible; experience is the product. If it glitches, it fails; if it feels seamless, the mechanism doesn't matter.

**Implicit taste filters that emerged through the session:**

- *Pragmatic and low-magic* — reject clever auto-behaviors (no auto-suspend, no auto-worktree, no magical "AI decides what to do").
- *Co-existence over takeover* — lmux is a well-behaved tenant inside your WM, not a replacement for it.
- *Terminal first, editor/browser second and third* — don't pretend to be an IDE.
- *Agent-agnostic* — Claude / Codex / Aider / anything. No vendor lock-in at the anchor layer.
- *User configurability is non-negotiable* — keybinds, mouse, layout, notifications.

### The Differentiators (Post-Phase-3 Pitch)

1. **Multi-workstream parallel agent orchestration** — 2+ agents + IDE running side-by-side, each with its own satellite constellation, manually paused/hidden/resumed.
2. **Smooth per-pane sandboxing with tiered backends** — nothing else does this cleanly on Linux.
3. **Anchor + satellites + tiered context bus** — novel mental model.
4. **Minimal chrome + real notifications** — clean by default, alerts only when they matter.
5. **Linux-native, Rust, libghostty** — performance, static binary distribution, modern rendering.
6. **Editor-agnostic** — any editor with a thin plugin can play; JetBrains included.

### Open Questions for a Future PRD

- Which compositor IPC(s) ship at v0.2? (Hyprland + Sway is my proposal — needs validation spike.)
- Fuzzy switcher UX: spec'd as Super+K palette, but exact information density / sorting not settled.
- Sandbox template distribution: bundled-in-binary vs. user-config-file-shipped defaults vs. fetched-from-registry.
- Session persistence: how much, if any, ships in v0.3? (Currently marked lukewarm, Phase 3 "earn it" candidate.)
- Config format: TOML or KDL? (Both pragmatic; needs one decision.)
- Distribution: single-static-binary + optional Flatpak + optional AUR — which is the authoritative channel for non-devs?
- Economics: OSS only? Paid tier (remote sync, team features)? Sponsorware? Not discussed, needs a separate brief.
- Project name: "lmux" is a working name. Is it the final name? (Likely needs branding exercise before public launch.)

### Recommended Next Steps

1. **Run the 4 validation spikes** before committing to the roadmap. 1–3 days each, ~2 weeks total.
2. **Create a Product Brief** (consider the `bmad-product-brief` skill) capturing this session's synthesis — differentiators, taste filters, roadmap.
3. **Create a PRD** (consider the `bmad-create-prd` skill) focused specifically on v0.1 scope — one detailed document per release, not a monster all-versions PRD.
4. **Set up the Rust project** with libghostty FFI as the first integration test. v0.1 starts the moment the FFI demo renders one PTY.
5. **Name decision** — do a short brand/name pass if "lmux" isn't final. Matters for public communication but not for building.

### Session Highlights

**User Creative Strengths:**
- Decisive subtractive editing — rejected 5 "fancy" desktop variants to land on the clean minimal one; killed git management, auto-suspend, and JetBrains maximalism in turn.
- Quick reframing ability — when the satellite-browser + anchor-agent combo clicked, it turned into a clean "multi-workstream" product vision in a single turn.
- Clear taste signals — "low magic," "display don't duplicate," "cohesive feel" all emerged as design principles from individual reactions.

**AI Facilitation Approach:**
- Matched user's short-burst energy with short-burst provocations.
- Pivoted domains aggressively to combat semantic clustering bias.
- Used subtraction-positive framing — celebrating rejections as scope-sharpening, not failure.
- Re-scoped phases when user reframed (e.g., Anchor-and-Satellites absorbed the JetBrains integration angle).

**Breakthrough Moments:**
- "Anchor + Satellites + Context Bus" emerging from the agent-connect-to-browser thread.
- "Display, Don't Duplicate" crystallizing as a design principle after the git rejection.
- The differentiator re-framing after the editor-agnostic scope call.

**Energy Flow:**
- High through Phase 1 and 2, focused through Phase 3, decisive through Phase 4. Never depleted; just concluded cleanly.

