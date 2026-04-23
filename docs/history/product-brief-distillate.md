---
title: "Product Brief Distillate: lmux"
type: llm-distillate
source: "product-brief-lmux.md"
created: "2026-04-21"
purpose: "Token-efficient context for downstream PRD + ADR creation"
---

# lmux Distillate

Dense context beyond the 1-2-page executive brief. Use as input for PRD, architecture, and ADR work.

## Identity

- **Working name:** `lmux`. Final name is an open ADR — defer until public v0.3 pitch.
- **Positioning sentence:** *"lmux is a Linux-native, Rust + libghostty dev cockpit for running multiple AI agents and editors in parallel. Each workstream centers on an anchor (agent CLI or GUI app) surrounded by satellite panes connected through a smart-open event bus. Smooth per-pane sandboxing, cohesive multi-workstream orchestration, pragmatic notifications — and a refusal to reimplement what git / editors / compilers / agents already handle. Workspace tenant, never a window manager."*
- **Public pitch line:** *"Cognitive overhead, not commands."*
- **Audience:** author (direction-anchor). Personal tool first, small OSS project second. No commercial intent.
- **Effort budget:** nights + weekends, ~10–15 h/week; timeline estimates carry 1.5–2× contingency.

## Design principles (taste filters — override features)

1. **Display, Don't Duplicate** — defer git/LSP/build/agent state to existing tools. lmux is the display surface, never the reimplementation.
2. **Editor-Agnostic > Vendor Deep Dive** — thin plugin protocol any editor can adopt beats a fat per-IDE integration.
3. **Cohesive Workspace Feel Is the Success Bar** — if it glitches on a normal resize, it fails.
4. Pragmatic, low-magic: reject auto-behaviours (per-project workspace auto-binding, anchor/worktree auto-pairing, idle-anchor suspend). User-explicit actions preferred.
5. Co-existence over takeover: WM is authoritative for global keys. lmux never rebinds global keybinds or acts as a WM replacement.
6. Terminal-first, editor/browser second. Agent-agnostic. User configurability non-negotiable.

## Architecture primitives

- **Anchor.** One canonical process per workstream. Owns identity: name, branch, notifications, "I'm waiting" signal. **Lifecycle rule:** anchor exit ⇒ workstream archives; satellites become orphaned but survivable.
- **Satellites.** Terminal (libghostty), browser, editor/IDE, log-watcher, GUI-app-embed. Richer types (browser, log-watcher) are v0.3+.
- **Smart-open event bus** (renamed from "tiered context bus" in the brief; original vocabulary retained here):
  - **Passive tier** = URL + file-path auto-routing between panes. Ships v0.1-ish. No editor cooperation required.
  - **Active tier** = opt-in plugin protocol for agent status, build status, richer events. Spec by v0.3; implementations v0.4+.
  - Transport: **open ADR** — D-Bus vs Unix socket vs stdio.
- **Pane taxonomy (first-class):** Shell, Agent Terminal, Browser, Editor/IDE, Log-watcher, GUI-app-embed.

## Compositor / GUI satellite story

- **Path A = spawn-and-track** via compositor IPC. **Not** window reparenting — Wayland isolation rules that out. This is the design-honesty point: the "embedding" is cohesive *docking*, not true embedding.
- **KWin MVP backend** (user's daily driver):
  - `ext-foreign-toplevel-list-v1` is **NOT** advertised by KWin 6.5.6 → fall back to `org.kde.KWin.Scripting` (D-Bus) for **both** enumeration and control.
  - KWin scripts return via D-Bus signals on `/Scripting/Script<N>`, not synchronous returns. Use `zbus::Proxy::receive_signal` with correlation-id-keyed dispatch.
  - Gotchas: `Qt.rect` unavailable in plain `.js` (use plain `{x,y,width,height}` literal → QRectF coercion); `workspace.windowList()` includes ~30 OSDs/panels — client-side PID filter required; `konsole` needs `--separate --hold` or D-Bus activation steals the PID.
  - Identifier correlation: child PID via KWin script (reads `client.pid`).
  - **Phase 1 spike timings:** KWinScripting init ~2 ms; spawn→PID-match 221–323 ms (konsole launch dominates); focus 2–4 ms; move/resize 1–4 ms. All well under UI budget.
- **Wlroots backend** (Hyprland or Sway):
  - Uses `wlr-foreign-toplevel-management-v1` + `hyprctl` / `swaymsg`.
  - Correlation: app_id + first-seen-after-spawn heuristic.
  - **Phase 2 spike deferred** this session; rerun before v0.2 PRD lock.
- **Trait shape (v0 from spike, amended):**
  - `trait CompositorControl` with `enumerate`, `focus`, `set_geometry`, `subscribe_lifecycle`.
  - `ToplevelHandle = opaque String` (PID on KDE, `app_id + seq` on wlroots).
  - `set_geometry` returns **requested** geometry only — configure-ack is async, observation is a separate stream primitive.
  - Identify and control may be backed by **different protocols** per backend (Wayland extension for identify + D-Bus for control on KDE; Wayland-only on wlroots).
- **Open ADR:** long-lived KWin script for `windowAdded` / `windowRemoved` lifecycle stream — unknown if registrations leak across lmux restarts. Must prototype before v0.2 PRD lock.

## Rendering / FFI stack (libghostty-ffi spike, working)

- Rust host; libghostty statically linked via `ghostty-vt-static` (Zig, vendored as subproject).
- `build.rs` runs `zig build --release=fast`; bindgen generates Rust FFI from `ghostty/vt.h` with `DGHOSTTY_STATIC` + allowlist `ghostty_*` / `Ghostty*` / `GHOSTTY_*`.
- GTK4 0.9 + pangocairo 0.20 for rendering surface.
- portable-pty 0.9 for PTY; async-channel 2 for reader→UI.
- Distribution implication: **single static binary is achievable.** Elevate this as a trust signal.

## Sandbox story (v0.3)

- Tiered pluggable backends: **None → Bubblewrap → LXC/Podman/Docker → microVM (Firecracker / crosvm)**. v0.3 ships Bubblewrap only.
- **Outcome-named templates** (briefing convention; original policy-intent names retained as aliases):
  - `isolated` (alias: `build-safe`): read-only host, writable overlay, no network.
  - `networked` (alias: `test-net-ok`): `isolated` + palette-granted outbound allowlist.
  - `untrusted` (alias: `untrusted-full`): v0.4+ — ephemeral, stricter, opt-in only.
- **v0.3 scope cut:** only `isolated` template + network toggle ships. Defer `lmux commit` / `lmux revert`, multi-template palette, and commit-diff-review to v0.4.
- Overlay FS with commit/revert is the risk center — shared Postgres / `node_modules` / auth tokens is the UX battle.
- Per-sandbox network deny-by-default with palette-granted allowlist.
- ADR-7 is the single most important design decision: **what is shared vs isolated by default**.

## Notifications

- Attention-required signals per-pane: idle / active / waiting-for-input / errored / finished.
- Actionable toasts, click-to-teleport-to-pane.
- Priority tiers: critical / info / chatter; per-session and global mute/DND.
- Tab-edge glow as subtle in-UI signal.
- Cross-session reach — notifications escape the current session.
- **v0.1:** bell → freedesktop notification (pane-named, clickable).
- **v0.3:** priority tiers.
- **v0.4:** click-to-teleport, tab-edge glow. Freedesktop notification spike still pending (dunst / mako / swaync).

## Roadmap slices (detailed)

### v0.1 (MVP, ~3 weeks)
- Rust host binary; libghostty FFI PTY rendering.
- One window, one session, many terminal panes with H+V splits.
- Mouse click-to-focus + drag-to-resize dividers.
- Hardcoded keybinds.
- Manual anchor tag via `lmux-cli mark-anchor` or `Super+A`.
- Visual anchor border highlight.
- Bell → freedesktop notification (pane-named, clickable).
- Save/restore layout on quit.
- **NOT in v0.1:** sandbox, context bus, remote, sidebar, session switcher, GUI satellites, editor/browser pane types, JetBrains plugin.

### v0.2 (dogfood, ~1 month)
- Multi-session + fuzzy switcher (Super+K).
- Sidebar.
- Manual pause / hide / resume of anchors.
- Anchor auto-detection + anchor tags.
- **GUI satellites via Path A on KWin** (brainstorm said Hyprland+Sway; spec update made KWin MVP).
- Tab-edge glow (lightweight variant).
- Configurable TOML (or KDL, open ADR) keybinds.

### v0.3 (first recommendable release, ~3 months)
- Sandbox: Bubblewrap, `isolated` template, network toggle (scope cut from brainstorm's full template set + commit/revert).
- Cohesive docking: compositor-orchestrated for KWin; wlroots = best-effort (panes work, docking may be disabled).
- Notification priority tiers + DND.
- Passive smart-open bus (if time).
- Minimal remote SSH spawn (if time, ambitious).
- Editor plugin protocol **spec** (markdown). No reference plugin in v0.3.

### v0.4+ backlog
- Active context bus tier implementation.
- Editor plugins: VSCode, Cursor, Helix, Neovim, Zed, JetBrains Kotlin.
- Sandbox backends: LXC, Podman, Firecracker / crosvm.
- Sandbox `commit` / `revert`, multi-template palette, commit-diff-review.
- Click-to-teleport notifications, tab-edge glow (richer).
- GNOME / Mutter support. X11 via XEmbed.
- Remote attach (Waypipe / xpra / tmux-attach).
- JetBrains Gateway as pane type.
- Wlroots compositor docking (if deferred from v0.3).
- Resource broker: auto-allocated ports, per-worktree Postgres schema, scoped secrets (opportunity-reviewer adjacent-value idea).
- Per-anchor replay / audit trail from PTY ownership.
- Workstream bundle export (`lmux export` — shareable snapshot).

## Rejected ideas (do not re-propose)

- **Per-project workspace auto-binding / magical workspace behaviours** — violates low-magic taste filter.
- **Summon overlay** (Super+Space system-wide launcher via Wayland layer-shell) — overreach; lmux is a tenant, not a WM.
- **Focus-mode global keybind rebinding** when lmux has focus — hostile to co-existence; WM is authoritative.
- **Auto-suspend of idle anchors** — too magical; user-explicit pause/hide/resume preferred.
- **Anchor ↔ worktree auto-pairing / lmux managing git worktrees** — violates Display-Don't-Duplicate; git is not lmux's job.
- **Replace JetBrains terminal panel** with lmux-embedded ghostty — out of scope; contradicts terminal-first and editor-agnostic.
- **Conductor-style multi-agent kanban / dashboard UI** — out of scope through v0.3; would violate "tenant, not platform" positioning.
- **JetBrains Kotlin plugin ahead of anchor-semantics maturity** — deferred to v0.4+.

## Open decisions (ADR candidates, grouped by milestone)

**Blocks v0.1:**
- Config format: TOML vs KDL.
- KWin script lifecycle: does long-lived `windowAdded` / `windowRemoved` registration leak across lmux restarts?

**Blocks v0.2:**
- Wlroots backend primary: Hyprland vs Sway (IPC ergonomics + author device).
- Smart-open event bus transport: D-Bus vs Unix socket vs stdio.
- Smart-open v0.1 passive event set (URL intent, file-path open intent).

**Blocks v0.3:**
- Sandbox defaults (ADR-7): what is shared vs isolated by default in `isolated` template.
- Session state persistence: append-only log vs snapshot ("earn it or drop").

**Deferred:**
- Product name.
- Distribution channel: static binary vs Flatpak vs AUR authoritative.

## Competitive intelligence (from web research)

- **cmux (manaflow-ai):** macOS only; Ghostty-native; vertical-tab workspaces; socket/CLI control; positioned as "primitive, not solution" for parallel Claude Code / Codex / Aider / OpenCode. No Linux story, no per-pane sandboxing, no JetBrains/GUI-anchor pattern.
- **Conductor (conductor.build, YC S24):** Mac app; isolated worktree workspaces per agent with checkpoints; spotlight-testing sync back to main; multi-model tabs. Closed source, workspace-centric (not pane-composable).
- **Crystal:** MIT-licensed Conductor analogue. Same macOS framing.
- **Claude Squad / Vibe Kanban / Antigravity / Cursor Background Agents:** "Orchestrator tier" UIs (kanban boards, dashboards) managing 3–10 async agents. Treat agents as backend jobs, not composable anchors with live satellites.
- **Terminal multiplexers repurposed (Zellij, Wezterm, tmux, Warp Agents, Ghostling, Kytos):** Power users wire worktrees + layouts + scripts. All leave orchestration, context wiring, and sandboxing as DIY.
- **Category shift 2026:** narrative moved from "conductor" (one agent, real-time) → "orchestrator" (3–10 async agents, independent context windows) — lmux's anchor+satellites model fits this exactly.
- **Consensus Linux agent sandbox:** Bubblewrap (Claude Code default) + Landlock/seccomp (OpenAI Codex default). HN thread early 2026 catalogued 20+ sandbox wrappers launched in prior year — Cambrian moment.
- **libghostty-vt state:** shipping, zero-deps, cross-platform (macOS/Linux/Windows/WASM); stable functionality, API signatures still in flux. Embedders are early adopters.
- **KWin 6 Wayland** is default on major distros; scripting/IPC surface is richer than GNOME's for tiling-style cockpits.

## User pain points (with competitor citations)

- "Cognitive overhead, not commands" — keeping track of what each agent is doing across N worktrees is the real tax.
- N parallel workspaces → N copies of `node_modules`, N Postgres instances, port collisions.
- Silent cross-agent overwrites / merge conflicts rank as top failure modes. Conductor's checkpoints frequently cited as killer feature.
- Re-auth friction with Claude Code + slow rebuilds called "workflow killers" → argues for persistent, pre-warmed per-pane environments over ephemeral containers.
- Windows/Linux users feel left out ("worktrees on Windows", "sandboxing AI agents in Linux" 2026 blog posts).

## Risks & mitigations (prioritized)

1. **libghostty API churn** (high): pre-1.0, API signatures in flux, Kitty graphics + OSC clipboard on roadmap not shipped. Budget for churn. Kill criterion: incompatible break ≥2× in 3 months.
2. **KWin script lifecycle leakage** (high, ADR-4): could invalidate v0.2 pseudo-embedding mid-build. Prototype before v0.2 PRD lock.
3. **Solo-dev burnout / scope fatigue** (high): ~4–5 months unpaid, no external accountability. Kill criterion: author stops using lmux 2 consecutive weeks during dogfood.
4. **Category crowding / Linux-lane closing** (medium): Conductor (YC money) or Antigravity (Google) could ship Linux in a quarter. Execution speed is the only defense.
5. **Sandbox UX defaults** (medium): shared Postgres/`node_modules`/auth + overlay FS + network policy is research-grade. Bubblewrap spike may surface limits. Cut scope if spike returns GO-with-caveats.
6. **Wlroots docking glitches** (medium): kill criterion — still glitches after 3 focused weeks → v0.3 ships KWin-only, wlroots moves to v0.4.
7. **Ghostty upstream policy change** (low-medium): "no embedders" stance would kill rendering strategy. No fallback designed.
8. **JetBrains-on-Linux-with-AI wedge** (medium): intersection might be narrow — if ~dozens of devs globally, doesn't pull v0.3 adoption.

## Anti-metrics (fail signals)

- lmux processes outlive the compositor session.
- Sandbox defaults need a 5-step README to be productive.
- Docked GUI windows visibly detach, flicker, or lag on normal resize.

## Scope signals

- **MVP committed:** anchor + satellites + smart-open (passive) + bell-notify + KWin docking + `isolated` sandbox.
- **Maybe for v0.3:** wlroots docking (best-effort), minimal remote SSH spawn, editor plugin spec.
- **Not in v0.3:** kanban UI, remote sessions, GNOME/X11, JetBrains Kotlin plugin, commit/revert, click-to-teleport, tab-edge glow (rich), resource broker, replay/audit, bundle export.
- **Never:** task tracker, prompt manager, agent router, secrets vault, WM replacement, git/LSP/build-tool reimplementation.
