---
stepsCompleted: ['step-01-init', 'step-02-discovery', 'step-02b-vision', 'step-02c-executive-summary', 'step-03-success', 'step-04-journeys', 'step-05-domain', 'step-06-innovation', 'step-07-project-type', 'step-08-scoping', 'step-09-functional', 'step-10-nonfunctional', 'step-11-polish', 'step-12-complete']
classification:
  projectType: desktop-application
  domain: developer-tools-terminal-multiplexer
  complexity: high
  projectContext: brownfield
inputDocuments:
  - _bmad-output/planning-artifacts/prd-v0.1.md
  - _bmad-output/planning-artifacts/architecture.md
  - _bmad-output/planning-artifacts/product-brief-lmux.md
  - _bmad-output/planning-artifacts/product-brief-lmux-distillate.md
  - spikes/compositor-ipc/FINDINGS.md
  - docs/adr/0001-rendering-stack.md
  - docs/adr/0002-anchor-satellites-bus.md
  - docs/adr/0003-path-a-spawn-and-track.md
  - docs/adr/0004-compositor-control-trait.md
  - docs/adr/0005-kwin-mvp-compositor.md
  - docs/adr/0006-wlroots-backend-hyprland.md
  - docs/adr/0007-config-format-toml.md
  - docs/adr/0008-bus-transport-unix-socket.md
  - docs/adr/0009-sandbox-primitive-bubblewrap.md
  - docs/adr/0010-sandbox-defaults.md
  - docs/adr/0011-kwin-script-lifecycle.md
  - docs/adr/0012-session-persistence-earn-it.md
  - docs/adr/0013-distribution-static-binary.md
  - docs/adr/0014-product-name.md
workflowType: 'prd'
---

# Product Requirements Document - lmux v0.2

**Author:** Lars
**Date:** 2026-04-21

## Executive Summary

lmux v0.2 is the milestone that makes the cockpit a daily driver. v0.1 shipped the pane/PTY/shutdown foundation with a single session scoped to one KWin window. v0.2 extends that foundation into a multi-session, GUI-aware workspace: named sessions that persist across reboots, a fuzzy switcher to jump between them, a sidebar that makes the pane graph visible and manipulable, anchors that pause/hide/resume long-running work (manual now, auto-detected where safe), and — critically — GUI satellites. A JetBrains IDE, a browser, or Figma can be spawned *into* a pane so the compositor dockss them to lmux geometry instead of floating in the WM's separate reality.

The primary user is an editor-agnostic Linux developer who already lives in tmux-style splits but loses focus every time they alt-tab between terminals and IDE. v0.2 solves the split-brain: the cockpit owns the terminal panes *and* the GUI surfaces that belong to the same context. Multi-session + the fuzzy switcher solve the "where did I leave that debug session?" problem; anchors solve the "don't let the dev server die just because I'm closing the laptop" problem.

### What Makes This Special

**Compositor-aware satellite docking.** v0.2 implements Path A (spawn-and-track) on KWin: lmux launches the GUI app with an lmux-tagged wm-class, then a KWin script places that toplevel into the pane's screen geometry and follows it through moves/resizes. The KWin spike returned GO in v0.1. The user sees "JetBrains in pane 2"; the compositor sees a normal toplevel whose placement happens to be scripted. No reparenting, no fork of the compositor, no fragile X11 embedding tricks.

**Editor-agnostic by design.** Unlike IDE-integrated terminals or tiling-WM configs locked to a single stack, lmux sits at the compositor layer. JetBrains, VS Code, Neovim, Zed, or a browser dev-tools window are interchangeable satellites — the cockpit doesn't care which editor you use this week.

**Multi-session without a daemon-monster.** Sessions are persisted as TOML snapshots per workspace; the `lmux` process that restores them is the same single-binary process that runs v0.1. No background agent, no server/client split. A fuzzy switcher bound to a prefix shortcut (the tmux-prefix mechanic we validated in v0.1) moves between them.

**Anchors = first-class pause/resume.** An "anchor" is a pane the user has marked as long-running (dev server, AI agent, log tail). v0.2 gives anchors an explicit pause/hide/resume lifecycle — manual in the first cut, auto-detected for well-known commands (`npm run dev`, `cargo watch`, `claude code`, etc.). Anchors let the user close a session window without killing its critical processes.

## Project Classification

- **Project Type:** Native desktop application (Rust, GTK4, direct Wayland + KWin scripting integration)
- **Domain:** Developer tools — terminal multiplexer + window-compositor cockpit
- **Complexity:** High — multi-process IPC (bus for satellite discovery), compositor-script lifecycle (KWin JS), Wayland foreign-toplevel-management, PTY lifecycle with signal contracts, libghostty FFI, sandboxed subprocess spawn (bubblewrap)
- **Project Context:** Brownfield — v0.1 is implemented, smoke-tested, and archived as `prd-v0.1.md`; v0.2 builds on the existing Cargo workspace (`crates/lmux`, `crates/lmux-pty`, `crates/lmux-control`)

## Success Criteria

### User Success

- **Daily-driver threshold:** Lars (dogfooder, primary user) replaces his tmux + floating-WM workflow with lmux for at least 5 consecutive working days without falling back to tmux, with zero "I lost my work" incidents attributable to lmux.
- **Session survival:** Closing the lmux window and relaunching it restores every named session's pane tree, PTY cwds, and anchor state within 2 seconds of launch. No user-visible difference between "new launch" and "resume" other than latency.
- **Cross-session context switch:** The fuzzy switcher lets the user jump from session A to session B in ≤200 ms of keystroke-to-paint, without mousework, starting from the tmux-prefix key.
- **Satellite trust:** When a user types `lmux edit .` in a pane, the JetBrains IDE opens docked to that pane's screen geometry on ≥95% of launches on KWin. The remaining ≤5% fall back gracefully (the IDE opens as a normal floating window with a toast "docking unavailable, open anyway") without aborting.
- **Anchor peace of mind:** A user can mark a `claude code` or `npm run dev` pane as an anchor, close its session window, and find the process still running — and resumable into a new session — 10 minutes later with no data loss.
- **Graph visibility:** The sidebar shows all sessions + pane trees + anchor state at a glance; the user reports (qualitatively, in dogfooding notes) that the sidebar is the feature they cannot imagine going back without.

### Business Success

v0.2 is a solo-developer dogfooding milestone, not a commercial launch. Business success is defined as:
- **Milestone completion:** all v0.2 FRs land in `master` with green CI by 2026-07-31 (12-week budget from PRD sign-off).
- **Narrative readiness:** v0.2 produces a demo-able story — screen recording of multi-session + GUI satellite docking — suitable for a public announce post when v0.3 graduates the wlroots backend.
- **ADR debt stays bounded:** each of the three v0.2-blocking ADRs (wlroots backend pick, bus transport finalization, smart-open v0.1 event set) lands before the matching epic starts; no epic starts "pending an ADR."
- **No regressions in v0.1 contracts:** the shutdown-within-700ms and restore-from-last-session.json contracts from v0.1 remain green in CI throughout v0.2 work.

### Technical Success

- **Multi-session data model:** sessions are first-class domain objects (not just multiple windows of the same state), persisted to `~/.local/state/lmux/sessions/<name>.toml`, restored idempotently.
- **Fuzzy switcher latency:** open-to-first-paint ≤50 ms for up to 50 sessions; filter-to-selection ≤16 ms per keystroke.
- **KWin script lifecycle:** the lmux KWin script loads on lmux start, survives KWin reload, and unloads cleanly on lmux exit (per ADR-0011).
- **Bus transport stable:** the satellite/anchor IPC bus (ADR-0008: Unix socket) handles ≥100 events/sec without backpressure issues and survives satellite crash/restart without wedging the cockpit.
- **Path A placement race-free:** spawn-and-track on KWin correctly places ≥99% of satellite toplevels within 500 ms of their initial map event, with a deterministic fallback if the map event is missed.
- **Sandbox defaults hold:** satellites launched via the bus respect ADR-0010 sandbox defaults (bubblewrap profile; no new capabilities escape).
- **Test coverage:** every new pub API in `lmux-control`, `lmux-session`, and `lmux-satellite` has unit tests; end-to-end integration tests cover restore-session, spawn-satellite-and-dock, pause-and-resume-anchor.

### Measurable Outcomes

| Outcome | Target | How measured |
|---|---|---|
| Session restore time | ≤2 s | instrumented log span from launch to "all panes ready" |
| Switcher open latency | ≤50 ms | frame-timing probe in debug build |
| Switcher keystroke latency | ≤16 ms | same |
| Satellite dock success rate | ≥95% on KWin | dogfooding log + KWin script success signal |
| Anchor survival across session close | 100% (for user-marked anchors) | integration test + dogfooding |
| Daily-driver continuous use | ≥5 days without fallback | Lars's dogfooding journal |
| v0.1 contract regressions | 0 | existing CI suite stays green |

## Product Scope

### MVP — v0.2 Minimum

Scoped to exactly what the brief/distillate locks for v0.2. Everything below must ship for v0.2 to be considered complete:

1. **Multi-session** — named sessions, create/rename/delete, persist to disk, restore on launch.
2. **Fuzzy switcher** — prefix-bound overlay for jumping between sessions and recently-closed panes. Rebound from the v0.1 Super+K proposal to a prefix-key chord (e.g., prefix + `k`) since v0.1 migrated to a tmux-prefix model after Super-key KDE conflicts.
3. **Sidebar** — left-side panel showing session list + pane tree + anchor state; toggle-able; keyboard-navigable.
4. **Anchors — manual lifecycle** — user can tag a pane as anchor; pause (SIGSTOP), hide (detach from rendering), resume (SIGCONT, reattach). Anchors survive session-window close if the anchor was "hidden" before close.
5. **Anchors — auto-detection + tags** — well-known commands (`npm run dev`, `cargo watch`, `claude code`, user-configurable patterns) get auto-tagged as anchors with a recognizable icon in the sidebar.
6. **GUI satellite docking via Path A on KWin** — `lmux open <app>` spawns the app with lmux wm-class; KWin script tracks the new toplevel, places it to match the pane's screen geometry, and follows moves/resizes. Close-with-pane and detach-from-pane semantics defined.
7. **Tab-edge glow (lightweight)** — visual affordance on panes that have focus vs. panes that own a satellite vs. panes whose anchor is paused. Subtle, not neon.
8. **Configurable TOML keybinds** — users can override prefix key and all prefix-chord mappings in `~/.config/lmux/config.toml` (per ADR-0007).

### Growth Features (Post-v0.2)

Explicitly *not* in v0.2. Parked for v0.3+:

- **Wlroots backend** (ADR-0006 locks Hyprland as the first wlroots target) — v0.3.
- **Plugin system / satellite SDK** — third-party apps registering as satellites over the bus.
- **Remote / SSH session integration** — sessions that span a local + remote shell pair.
- **Full AI-agent orchestration UI** — beyond anchor-tag for Claude Code, a panel to inspect agent outputs.
- **Multi-workspace / multi-display first-class support** — v0.2 assumes single primary display; multi-display is best-effort.

### Vision (Future, post-daily-driver)

- **Cross-compositor cockpit** — Hyprland, Sway, and KWin all first-class backends with no feature drift.
- **Shared session over network** — pair-programming-grade session sharing.
- **Public announce + packaging** — AUR, Flatpak, static binary (ADR-0013) distributed to a broader audience beyond the single dogfooder.
- **"Workspace" abstraction above session** — e.g., "Work" vs "OSS" superset containing multiple sessions each.

## User Journeys

lmux v0.2 is an individual-developer tool. There is one primary user (the solo developer / dogfooder) and a small set of secondary roles the *same person* plays (configurator, satellite author, "compositor operator" when something breaks). There are no admin, moderator, or API-consumer personas — the bus IPC is internal, not a public API surface in v0.2.

### Persona: Lars the dogfooding developer

**Situation:** Polyglot developer on Manjaro KDE Plasma Wayland, Norwegian keyboard, works across a Rust cockpit codebase (lmux itself), personal OSS, and paid client projects. Daily flow involves ~6 terminal panes per project, 1 JetBrains IDE, 1 browser, and 1 AI assistant in a terminal. Currently lives in tmux + a messy floating-WM window pile; loses 10+ minutes per day to "where did I put that window" and "is the dev server still running?"

**Goal:** Run the whole day — multiple projects, two AI agents, a running dev server, and a JetBrains session — without alt-tab roulette and without losing state when he closes the laptop.

**Obstacle:** Tmux owns terminals but can't talk to the WM. The WM owns windows but knows nothing about terminal context. Every existing "dev cockpit" ties him to one editor.

**Solution:** lmux v0.2. Sessions persist the whole context. Anchors keep dev servers and AI agents alive across session-window close. GUI satellites (his JetBrains IDE) dock to the pane that "owns" them. Fuzzy switcher replaces the WM alt-tab.

### Journey 1 — Primary happy path: "Resume a project across a reboot"

**Opening scene.** Monday, 08:30. Lars boots his laptop. Over the weekend he rebooted into Windows to play a game; the lmux process from Friday is long gone. He runs `lmux` from his terminal emulator launcher.

**Rising action.** The cockpit window opens. The sidebar appears on the left listing three sessions: `lmux`, `client-acme`, `oss-experimental`. The `lmux` session is highlighted as "last active." Lars presses the prefix key, then `k`. The fuzzy switcher overlay appears in the center. He types "acm" — `client-acme` filters to top. Enter. The cockpit swaps in the restored session: five panes in the exact split layout he left, each with the correct cwd (`~/code/acme/api`, `~/code/acme/web`, `~/code/acme/infra`, etc.).

**Climax.** One pane's tab-edge glows orange — the sidebar marks it as "anchor: paused." It's his `npm run dev` pane, marked anchor and hidden on Friday. He selects the pane, presses the prefix then `r` (resume anchor). The process unpauses, reattaches, and starts streaming logs. He types `lmux open idea .` in another pane. A JetBrains window opens — not on a floating desktop, but fitted to the screen area of pane 4. The tab-edge of pane 4 now glows blue: "owns satellite."

**Resolution.** Ninety seconds from `lmux` launch, Lars is typing code. No alt-tab, no "which terminal had the vite server," no "where did I leave the editor." Emotional shift: from the residual dread of "how much will I have to rebuild this morning" to "the cockpit remembered everything."

*Requirements this journey reveals:* multi-session persistence + restore (FR1–FR6), fuzzy session switcher (FR7–FR10), sidebar (FR11–FR16), anchor manual pause/hide/resume (FR17–FR22), GUI satellite spawn + dock on KWin (FR28–FR35), tab-edge glow (FR36–FR39).

### Journey 2 — Primary edge case: "An anchor crashes while hidden; a satellite refuses to dock"

**Opening scene.** Wednesday afternoon. Lars closes his `client-acme` session window to switch context to `oss-experimental`. Two of the `client-acme` panes are anchors: `cargo watch` and `claude code`. Both are marked "hide on close" so the processes keep running in the background even though no window shows them.

**Rising action.** Two hours later, he opens the fuzzy switcher, picks `client-acme`, and the session restores. The sidebar shows `cargo watch` as "anchor: exited (code 101)" — it crashed while hidden. A small red dot on the anchor row draws his eye but nothing pops up modally. He clicks the row and sees a log tail of the last 200 lines captured before death. He decides to respawn it with the recorded command: prefix, `R`. The anchor respawns in place.

**Climax.** He also types `lmux open figma-linux ./design.fig`. Figma is not a KWin-friendly app — its wm-class is weird and the KWin script fails to correlate the new toplevel to the spawn request within the 500 ms budget. Instead of leaving him hanging, a toast appears in the cockpit: "Could not dock figma-linux — opened as a floating window." The pane it was meant for gets a muted "satellite: detached" indicator.

**Resolution.** He gets his work done. The cockpit didn't pretend nothing happened; it also didn't block him. Failure was legible and recoverable.

*Requirements this journey reveals:* anchor crash capture + log retention (FR23–FR25), anchor respawn (FR26), satellite docking timeout + fallback + toast (FR40–FR43), sidebar state indicators (FR13).

### Journey 3 — Configurator: "Rebind the prefix and add a custom anchor pattern"

**Opening scene.** Second week of dogfooding. Lars' wrists don't like `Ctrl+B` (the v0.1 default, carried over). He wants to map the prefix to `Ctrl+Space`. He also wants `pnpm dev` auto-detected as an anchor in addition to the built-in `npm run dev`.

**Rising action.** He opens `~/.config/lmux/config.toml` in his editor (via the cockpit, via a satellite, of course). He edits:
```toml
[keybindings]
prefix = "Ctrl+Space"
[anchors]
auto_detect_patterns = ["npm run dev", "pnpm dev", "cargo watch", "claude code"]
```
Back in the cockpit pane he presses prefix (still Ctrl+B until reload) + `C` (capital C, "reload config"). The sidebar briefly shows a toast "config reloaded." Pressing Ctrl+Space now arms the prefix; the old Ctrl+B is released.

**Climax.** He starts a `pnpm dev` in a pane. Within 1 s, the sidebar tags the pane as anchor (auto-detected), with the same icon the built-in `npm run dev` pattern uses.

**Resolution.** Config changes apply without a restart. Auto-detection extends to his custom patterns.

*Requirements this journey reveals:* config file format + schema (FR44–FR46), config hot-reload (FR47), anchor auto-detection patterns (FR24, FR27), user-extensible pattern list (FR48).

### Journey 4 — Operator-of-last-resort: "KWin reloaded and broke everything"

**Opening scene.** Lars updates his system; a Plasma update restarts KWin mid-session. The lmux KWin script is evicted and lmux's satellite-docking capability dies silently.

**Rising action.** He notices because a new `lmux open` attempt produces the "opened as floating window" toast. The sidebar also now shows a small red banner near the top: "Compositor integration offline." He clicks it. A detail panel explains: "KWin script failed to reload. Click 'Re-inject' to attempt reconnection."

**Climax.** He clicks Re-inject. lmux re-installs the script via `kwriteconfig5` + DBus `KWin.reconfigure()` per ADR-0011. 500 ms later, the banner turns green and the capability returns. Already-open floating satellites don't retroactively dock (out of scope for v0.2), but *new* `lmux open` calls dock correctly.

**Resolution.** The cockpit is honest about compositor state and self-heals on demand.

*Requirements this journey reveals:* KWin script lifecycle + health check (FR49), re-inject command (FR50), compositor-offline banner (FR51), graceful degradation to floating satellites (FR43).

### Journey Requirements Summary

Capabilities revealed across the four journeys — grouped by domain, with FR ranges that the next step will flesh out:

- **Session domain** (FR1–FR10): create/name/delete sessions, persist/restore pane trees with cwd, default/last-session boot behavior, fuzzy switcher with recency + score ordering.
- **Sidebar + visibility** (FR11–FR16): session list, pane tree per session, anchor state indicators, satellite-ownership indicators, toggle + keyboard nav, toast channel.
- **Anchors** (FR17–FR27): manual tag/untag, pause (SIGSTOP) / hide (detach rendering) / resume (SIGCONT, reattach), crash capture + log retention, respawn, auto-detection + user patterns.
- **GUI satellites** (FR28–FR43): `lmux open` command, wm-class tagging, KWin-script correlation by PID+class, geometry follow on move/resize, detach/reattach, timeout + floating fallback, per-pane ownership.
- **Config + keybinds** (FR44–FR48): TOML schema, prefix + chord mapping, hot-reload, user-extensible anchor patterns.
- **Compositor integration health** (FR49–FR51): script injection lifecycle, health probe, re-inject command, offline banner.
- **Visual affordances** (FR36–FR39): tab-edge glow for focus / satellite-owned / anchor-paused / anchor-dead states.

## Domain-Specific Requirements

lmux isn't a regulated product (no HIPAA/PCI/GDPR data flow — session state is local-only). "Domain complexity" here is technical, not legal: Wayland + compositor-scripting integration and sandboxed subprocess management.

### Compositor & Wayland Constraints

- **KWin only for v0.2 satellite docking** (ADR-0005). wlroots (Hyprland) compositor support is deferred to v0.3 (ADR-0006) to keep the v0.2 satellite-docking surface small. v0.2 ships a `CompositorControl` trait (ADR-0004) with a KWin impl; a wlroots impl may exist as a stub returning "unsupported" so the trait boundary is real.
- **Wayland version target:** Plasma 6.x on stable Manjaro/Arch (the dogfooder's environment). Plasma 5.27 LTS is best-effort but not a release gate.
- **Foreign-toplevel-management compatibility:** v0.2 uses KWin-script-driven placement, not `wlr-foreign-toplevel-management-v1` directly. The protocol dependency arrives in v0.3 with the wlroots backend.
- **No X11 fallback.** lmux is Wayland-native. If the session is X11, the cockpit still runs but satellite docking is disabled with a clear banner.

### Process & Sandbox Constraints

- **Subprocess sandbox defaults** (ADR-0009, ADR-0010): satellites spawned via `lmux open` run under a bubblewrap profile by default (no access to `~/.ssh`, `~/.gnupg`, `~/.aws`, or `~/.config/lmux/config.toml`; read-only bind of the project directory; network allowed). User can opt out per-command with `lmux open --no-sandbox <app>`.
- **PID + wm-class are the only satellite correlation keys in v0.2.** No kernel cgroups, no seccomp-bpf of the satellite. Sandbox is about filesystem isolation, not syscall containment.
- **PTY lifecycle:** v0.1 shutdown contract (SIGTERM → 500 ms grace → SIGKILL, reap within 700 ms) extends unchanged to v0.2. Anchors add SIGSTOP/SIGCONT but do not change the shutdown path.

### Local Data & Privacy

- **Session files live in `$XDG_STATE_HOME/lmux/sessions/` (default `~/.local/state/lmux/sessions/`)**, mode 0600. Content includes pane cwds, command histories (if configured), and anchor metadata — no credentials, but enough context to be treated as private.
- **No telemetry in v0.2.** No analytics, no crash reporting upload. Future versions may opt-in; v0.2 ships zero network by default for the cockpit process itself.
- **Config and state files are TOML** (ADR-0007), human-editable.

### Integration Requirements

- **libghostty** (dynamic link, not vendored) for rendering. v0.1 ships this; v0.2 does not change the integration surface.
- **KWin scripting API** (JavaScript, loaded via `kwriteconfig5` + DBus `org.kde.KWin.reconfigure`). Script lifecycle per ADR-0011.
- **Bus transport = Unix socket** (ADR-0008) at `$XDG_RUNTIME_DIR/lmux/bus.sock`. v0.2 uses it for satellite-spawn ack and anchor lifecycle events; v0.3+ opens it to third-party subscribers.
- **No D-Bus service name in v0.2** for the lmux cockpit itself (we don't export a public service). We *call* KWin's D-Bus endpoints; we don't *register* one.

### Domain-Specific Risks & Mitigations

| Risk | Mitigation |
|---|---|
| KWin API change between Plasma minor versions breaks satellite docking | KWin script version-gated; health probe (Journey 4) surfaces breakage; script file shipped alongside binary, not pulled from internet |
| Satellite toplevel never appears (app crashed, `wm-class` wrong) | 500 ms spawn-to-map timeout; toast + floating fallback; no pane-ownership state committed until map seen |
| Anchor "hide" loses output when compositor kills the offscreen surface | Anchors keep their PTY running regardless of rendering state; a ring buffer (≥10 000 lines) captures output while hidden so "resume" can replay the scrollback tail |
| Session file on disk corrupts mid-write (power loss) | Atomic write: stage to `sessions/<name>.toml.new`, fsync, rename. Restore tolerates a missing or malformed file by logging + falling back to "fresh session with the same name" |
| Bubblewrap missing on the user's system | Detect at startup; if bwrap is missing, degrade `lmux open` to direct spawn + log a warning; don't crash |
| bwrap profile blocks a satellite that legitimately needs the blocked path | `--no-sandbox` escape hatch, documented in config |

## Innovation & Novel Patterns

### Detected Innovation Areas

**1. Compositor-scripted Path A docking as an editor-agnostic cockpit primitive.** Docking GUI applications into tmux-like panes is not new in spirit (tmux for GUI has been attempted many times), but most prior attempts either (a) required a custom compositor / WM (e.g., i3-with-custom-patches, Sway-based forks), or (b) used X11 reparenting hacks. Path A — spawn the app with a tagged wm-class, then use the mainstream compositor's own scripting API to place the resulting toplevel — is a novel combination that runs on *the user's existing KDE desktop* with no fork, no reparenting, and no privilege escalation. The KWin spike validated this end-to-end (FINDINGS.md → GO verdict).

**2. Anchor as a first-class lifecycle primitive.** tmux has "detached sessions." Window managers have "minimize." Neither has a unified concept of "this is a long-running process I want to pause / hide / resume without killing." Anchors combine: pause (SIGSTOP), hide (detach rendering but keep PTY), resume (SIGCONT + reattach), scrollback capture while hidden, auto-detection by command pattern. The compound primitive is the innovation, not any single operation.

**3. Editor-agnostic cockpit at the compositor layer.** Competitors (Warp, Wave, Zellij, WezTerm multiplex) try to own the terminal surface. JetBrains, VS Code, Zed try to own the editor surface. lmux is the first attempt (that we're aware of) to be the *layer above both* — leveraging the compositor as the arbiter — without being a full compositor itself.

### Market Context & Competitive Landscape

- **tmux / Zellij / WezTerm multiplex** — pure terminal multiplexers. No GUI-window awareness. Users alt-tab for their IDE.
- **Warp / Wave Terminal** — "modern terminal" category. Beautiful UX inside the terminal process; cannot host arbitrary GUI apps.
- **JetBrains Projector / JetBrains Fleet** — remote/thin JetBrains. IDE-specific. Not a multiplexer, not editor-agnostic.
- **Tiling WMs (i3, Hyprland, Sway, river)** — can tile terminals and GUI apps in the same tree but have no per-project session semantics, no anchor abstraction, and no fuzzy project switcher. They're desktops; lmux is a cockpit that runs on any desktop.
- **tmuxp / zellij layouts** — project-scoped terminal layouts. No GUI awareness; no compositor integration.
- **The v0.1 spike (KWin-scripts)** found no public prior art for a general-purpose compositor-scripted app-docking tool. This is the seam lmux sits in.

### Validation Approach

- **v0.1 already validated** the PTY/cockpit foundation and the KWin-scripting spike (compositor-ipc).
- **v0.2 validation is dogfooding** — 5 consecutive days of daily-driver use (per Success Criteria) is the load-bearing proof that the anchor + satellite model works in practice, not just in demo.
- **KWin script lifecycle** will have a real integration test (start lmux → inject script → spawn test app → assert placement → teardown) as part of the compositor-integration crate's CI.
- **Path A placement success rate** tracked via instrumentation logs during dogfooding; target ≥95% or redesign the correlation key before v0.3.

### Risk Mitigation (Innovation-Specific)

| Innovation risk | Mitigation |
|---|---|
| Path A turns out to have a race on first-map for fast-starting apps | 500 ms timeout → floating fallback + toast. The feature degrades, the cockpit doesn't break. Retry-on-configure-event for slow mappers. |
| Anchor hide/resume has edge cases with TUI apps that hate SIGSTOP (e.g., some fullscreen readline UIs) | Anchor-pause per-command opt-out in config; default pattern list excludes known-fragile commands. Scrollback ring captures output regardless. |
| Compositor-agnostic story fails when wlroots backend lands (v0.3) because KWin assumptions leaked | `CompositorControl` trait (ADR-0004) forces the abstraction in v0.2 even with one implementor. A wlroots stub that returns `Unsupported` keeps the trait honest. |
| "Editor-agnostic" slogan fails because JetBrains docking works but VS Code / Zed don't | v0.2 explicitly tests JetBrains + Firefox + Figma. If any of these three fails, the PRD's satellite claims get scoped down. Zed / VS Code tested best-effort, not gating. |

## Desktop-Application Specific Requirements

### Project-Type Overview

lmux is a single-process, single-binary native desktop application written in Rust, targeting Linux on Wayland (Plasma 6.x as the v0.2 release target; wlroots deferred to v0.3). The UI is GTK4 (reusing v0.1's widget tree), rendering is libghostty for terminal cells, and the compositor integration is a JS script injected into KWin plus Unix-socket IPC for satellite/anchor coordination. Distribution is a statically-ish linked binary (ADR-0013) packaged alongside the KWin script asset.

### Technical Architecture Considerations

v0.2 preserves the v0.1 crate layout and adds new crates for new concerns:

- **Existing (v0.1, unchanged in ownership, extended in surface):**
  - `lmux` — GTK4 application shell, window/tab plumbing, keybinding dispatcher.
  - `lmux-pty` — PTY lifecycle (spawn, resize, signal, reap), trampoline, portable-pty wrapper.
  - `lmux-control` — local IPC endpoint for the cockpit's control channel (already used by the test trampoline hook).
- **New in v0.2 (proposed; architecture step will confirm):**
  - `lmux-session` — session data model, TOML (de)serialization, atomic writes, restore logic.
  - `lmux-satellite` — satellite spawn/track/follow, `lmux open` CLI entry, Path-A correlation.
  - `lmux-compositor` — `CompositorControl` trait (ADR-0004) + KWin implementation (script loader, DBus caller, health probe).
  - `lmux-anchor` — anchor lifecycle (pause/hide/resume), scrollback ring buffer for hidden anchors, auto-detection pattern matcher.
  - `lmux-bus` — Unix-socket event bus (ADR-0008) between cockpit and satellites; internal in v0.2 but the public-API boundary for v0.3.
- **Process topology:** single cockpit process + N PTY children (one per terminal pane) + M satellite processes (one per GUI app). No daemon. No background service.

Cross-cutting:

- **Event loop** stays on the GTK main context; long IO runs on Tokio tasks dispatched via `glib::MainContext::spawn_local` back to the UI.
- **State ownership:** cockpit process owns all session/pane/satellite/anchor state. Satellites are dumb clients over the bus. No shared-memory, no mmap tricks.
- **Concurrency model:** exactly one writer per pane PTY, readers are UI + log-capture tap; anchor pause/hide mutates the rendering side only, never the PTY side.
- **Error surfaces:** every external seam (KWin DBus, bus accept, sandbox bwrap) wraps failure in a `CompositorOffline` / `SatelliteDockFailed` / `AnchorOperationFailed` enum; the sidebar toast channel is the default user-visible surface.

### Platform & Dependency Requirements

- **OS:** Linux with kernel ≥ 5.15 (for robust pidfd support). Tested on Manjaro/Arch; Ubuntu 24.04 best-effort.
- **Display server:** Wayland only. X11 users get the cockpit + PTY features but no satellite docking.
- **Compositor:** KWin 6.x (Plasma 6). Plasma 5.27 best-effort. No GNOME/Mutter support in v0.2 (Mutter's scripting surface doesn't support the Path-A pattern we need).
- **Runtime deps:** libghostty (dyn), GTK4 ≥ 4.14, pango, glib ≥ 2.78, bwrap (bubblewrap) ≥ 0.8, DBus, KWin JS engine (shipped by Plasma).
- **Build deps:** Rust ≥ 1.79, libghostty headers, GTK4 dev headers, pkg-config.
- **Static-ish binary:** everything except libghostty/GTK4/glib (system libs) statically linked per ADR-0013.

### Configuration & Persistence

- **Config:** `$XDG_CONFIG_HOME/lmux/config.toml` (default `~/.config/lmux/config.toml`). Schema per ADR-0007. Hot-reload supported (Journey 3).
- **State:** `$XDG_STATE_HOME/lmux/` holding `sessions/<name>.toml`, `last-session.json` (v0.1 contract preserved), and `anchors/` scrollback ring buffers.
- **Runtime:** `$XDG_RUNTIME_DIR/lmux/` for `bus.sock` and PID file.
- **Atomic writes everywhere** state leaves RAM (stage+fsync+rename).

### CLI & Entry Points

- `lmux` — launches the cockpit (default opens last session, or empty if none).
- `lmux --session <name>` — launch with a specific session.
- `lmux open <app> [args...]` — requests the running cockpit to spawn `app` as a satellite docked to the focused pane. If no cockpit is running, errors with a hint. This is the satellite UX entry point.
- `lmux session list | new | rename | delete` — session CRUD from the shell.
- `lmux anchor pause | hide | resume | respawn <pane-id>` — anchor CRUD (parallel to prefix-key bindings).

All non-interactive `lmux` subcommands talk to the running cockpit over `bus.sock`.

### Packaging & Distribution

v0.2 ships:
- Static binary `lmux`.
- KWin script asset (`share/lmux/kwin/lmux-dock.js`) installed to `~/.local/share/kwin/scripts/lmux/` on first run.
- Default config template copied to `~/.config/lmux/config.toml` if absent.
- No package yet (AUR / Flatpak targeted for v0.3 once wlroots backend lands and "one binary, all compositors" is the story).

### Implementation Considerations

- **Trait-first for the compositor seam.** ADR-0004's `CompositorControl` isn't optional scaffolding; it's the discipline that keeps v0.3's wlroots work from becoming a rewrite. Every KWin-specific call in v0.2 goes through the trait.
- **libghostty dynamic link and vendored shim.** v0.1 established a shim; v0.2 must not regress to static-linking ghostty (build time pain, licensing considerations).
- **GTK4 `EventControllerKey` propagation.** The v0.1 tmux-prefix mechanic uses Capture-phase interception. v0.2 adds the fuzzy switcher overlay which takes focus; the prefix dispatcher must cooperate (prefix arms, switcher consumes).
- **KWin-script ↔ Rust correlation** over DBus. v0.2 sends `{request_id, expected_wm_class, spawner_pid}` to the script; script posts back `{request_id, kwin_window_id}` when match succeeds or `{request_id, timeout}` after 500 ms. Retry & fallback live in `lmux-satellite`.
- **Anchor SIGSTOP + PTY.** Pausing a process that owns a PTY is well-defined (SIGSTOP doesn't kill the PTY, just halts the process group). Subtle: `cargo watch` and similar spawn child processes; SIGSTOP on the leader of a process group stops children too if we send to `-PGID`. Default to process-group signalling; document the edge.
- **Scrollback ring while hidden.** Anchors that hide detach from the GTK rendering path. We still read from the PTY master into a ring buffer (10k lines, ~1 MiB) so resume can replay the tail to the terminal widget.

### Skipped Sections (Not Applicable to Desktop App v0.2)

- API versioning / rate limiting — no public API.
- Multi-tenancy, RBAC — single-user desktop app.
- Mobile device permissions — not a mobile app.
- Browser/SEO/accessibility-as-WCAG-audit — not a web app. (Basic keyboard-navigation accessibility is in scope as a UX concern, not a WCAG audit.)

## Project Scoping & Phased Development

### MVP Strategy & Philosophy

**MVP approach: experience MVP on a single compositor.** v0.2's definition of MVP is "lmux is the tool Lars uses for 5 straight days without fallback." That is an *experience* threshold, not a feature-count threshold. Concretely: every v0.2 FR below must work; the v0.2 backend story is KWin-only. Multi-compositor support, plugin SDK, and polish features are deferred.

**Resource model.** Solo developer (Lars), ~12 calendar weeks budget, assisted by Claude Code / agentic workflows (v0.1 established the pattern). Target delivery of v0.2 milestone: **2026-07-31**.

**Sequencing rationale.** Session persistence is the spine; anchors and satellites bolt onto panes that already persist; fuzzy switcher is a UI on top of the session store; sidebar is a UI on top of both; config + keybind reload is horizontal infrastructure. Therefore: sessions → anchors (manual) → satellites → sidebar → auto-detection → switcher → config hot-reload → tab-edge glow polish.

### MVP & Phasing (Cross-Reference)

The v0.2 MVP feature list and Post-MVP / Vision roadmap are the canonical lists in **[§ Product Scope](#product-scope)**. All four user journeys (Journey 1–4 in **[§ User Journeys](#user-journeys)**) are fully supported by the v0.2 MVP. v0.1 contracts (shutdown ≤ 700 ms, `last-session.json` restore, PTY signal semantics) remain binding — captured as FR55, FR63, NFR10.

### Risk Mitigation Strategy

**Technical risks.**
- *KWin scripting API instability across minor versions.* Mitigation: pin tested Plasma versions, version-probe on startup, keep the script vendored alongside the binary. If Plasma 6.N ships a breaking API change, the cockpit degrades satellite docking (floating fallback) rather than crashing.
- *Path A correlation race.* Mitigation: 500 ms timeout + retry-on-configure. If we see <90% success during week-2 dogfooding, bring forward Path-B (child-of-cockpit) design notes from v0.3 backlog.
- *Libghostty ABI churn.* Mitigation: pin a known-good tag in the workspace; v0.1 already solved the shim problem.
- *Anchor SIGSTOP edge cases.* Mitigation: per-command opt-out list in config; default detection patterns exclude known-fragile commands.

**Product/adoption risks (scoped to a solo dogfooder).**
- *Lars gets tired of the product before 5 days of continuous use.* Mitigation: dogfooding journal, weekly retro against this PRD. If the 5-day threshold fails, the failure itself is the v0.3 backlog input — the cockpit is supposed to survive contact with real work.
- *v0.2 scope creep erodes the 2026-07-31 target.* Mitigation: FRs below are the locked surface; any new feature request goes on the v0.3 list, not into v0.2.

**Resource risks.**
- *Solo developer with variable availability.* Mitigation: epic breakdown (bmad-create-epics-and-stories in a later step) sequenced so "half-done v0.2" is still a useful product — e.g., sessions + anchors without satellites is shippable for self-use.
- *Agentic coding productivity regression (AI assistant degrades).* Mitigation: explicit v0.2 crate boundaries keep the blast radius of any one module bounded; tests per v0.1 standard keep regressions observable.

## Functional Requirements

> **Capability contract.** The FRs below are the binding surface of v0.2. Any capability not listed here will not ship in v0.2. FRs are implementation-agnostic — the architecture and epic breakdown steps translate them into concrete design and stories. Actor "User" = the dogfooding developer; "Cockpit" = the lmux process; "Compositor" = KWin in v0.2.

### Sessions (FR1–FR10)

- **FR1.** User can create a named session from the cockpit UI and from the CLI (`lmux session new <name>`).
- **FR2.** User can rename an existing session.
- **FR3.** User can delete a session (prompts confirmation when the session is currently open or has anchors).
- **FR4.** Cockpit persists each session's pane tree (splits, focus, pane cwds, PTY commands if configured) to disk atomically.
- **FR5.** Cockpit restores the pane tree of the last-active session on launch by default.
- **FR6.** User can override launch behavior with `lmux --session <name>` or a "no session" mode that starts empty.
- **FR7.** User can open a fuzzy switcher overlay via the configured keybinding (default: prefix + `k`).
- **FR8.** Fuzzy switcher lists all known sessions plus recently-closed panes, each with a brief context label (cwd, last command).
- **FR9.** Fuzzy switcher filters entries as the user types and selects by Enter; arrow/Tab navigation also supported.
- **FR10.** Selecting a session in the switcher swaps the cockpit to that session, saving the outgoing session's current state before the swap.

### Sidebar & Visibility (FR11–FR16)

- **FR11.** User can toggle the sidebar on/off via keybinding and via a visual toggle control.
- **FR12.** Sidebar displays the list of sessions, with the active session highlighted.
- **FR13.** Sidebar displays the pane tree of the active session, including per-pane indicators for: focus, satellite ownership, anchor state (live / paused / hidden / dead).
- **FR14.** User can navigate the sidebar via keyboard only (select session, expand/collapse pane tree, focus a pane).
- **FR15.** User can act on sidebar items via context actions (right-click or keyboard shortcut) — minimum actions: focus-pane, close-pane, pause-anchor, resume-anchor, detach-satellite.
- **FR16.** Cockpit emits toasts in a dedicated channel visible from the sidebar for: anchor crashes, satellite docking fallback, config reload result, compositor health changes.

### Anchors (FR17–FR27)

- **FR17.** User can mark any live pane as an anchor.
- **FR18.** User can unmark an anchored pane (anchor-tag removed; pane stays live).
- **FR19.** User can pause an anchor (PTY process group receives SIGSTOP; pane rendering is frozen).
- **FR20.** User can resume a paused anchor (SIGCONT; rendering resumes).
- **FR21.** User can hide an anchor (pane rendering detaches; the PTY stays live; output is captured to a scrollback ring buffer).
- **FR22.** User can re-attach a hidden anchor to a pane, replaying the tail of captured output.
- **FR23.** Cockpit retains the last 10 000 lines (or ≥1 MiB, whichever is smaller) of output from a hidden anchor in-memory.
- **FR24.** Cockpit captures the exit status and last 200 lines of output when an anchor's PTY process exits while the anchor is tagged.
- **FR25.** Cockpit marks a dead anchor clearly in the sidebar with a visible state change, and makes its last output viewable.
- **FR26.** User can respawn a dead anchor using the same command + cwd that it was originally started with.
- **FR27.** Cockpit auto-detects new panes whose commands match configured patterns (default: `npm run dev`, `pnpm dev`, `cargo watch`, `claude code`; user-extensible) and auto-tags them as anchors without prompting.

### GUI Satellites (FR28–FR43)

- **FR28.** User can run `lmux open <app> [args...]` to request the cockpit spawn `app` as a satellite docked to the currently-focused pane.
- **FR29.** Cockpit spawns satellite processes with a unique, lmux-generated wm-class that carries a request id.
- **FR30.** Cockpit (via its compositor driver) tracks new toplevels and correlates them to outstanding spawn requests by wm-class + PID.
- **FR31.** Cockpit sets initial satellite geometry to match the screen-space geometry of the owning pane.
- **FR32.** Cockpit follows the owning pane's geometry changes (move, resize, show/hide of containing tab) and updates the satellite position accordingly.
- **FR33.** User can explicitly detach a satellite from its pane; the satellite becomes a normal floating toplevel and the pane's "satellite owned" indicator clears.
- **FR34.** User can re-attach a detached satellite to a pane via the sidebar context action.
- **FR35.** When the owning pane closes, the cockpit gives the satellite the configured close behavior: SIGTERM it, detach it to floating, or leave it running (user-configurable; default: detach to floating).
- **FR36.** Pane tab-edge color/glow indicates focus state.
- **FR37.** Pane tab-edge color/glow indicates that a satellite is currently docked.
- **FR38.** Pane tab-edge color/glow indicates that the pane's anchor is paused or hidden.
- **FR39.** Pane tab-edge color/glow indicates a dead anchor until the user respawns or dismisses it.
- **FR40.** Cockpit times out a satellite-spawn/correlation attempt after 500 ms (configurable) and surfaces a toast describing the failure.
- **FR41.** On correlation timeout or compositor-offline, the cockpit does not retroactively dock the satellite but lets it remain as a floating window.
- **FR42.** Cockpit retries Path-A correlation on later `configure`/map events so slow-starting satellites can still dock within a configurable retry window (default: 2 s).
- **FR43.** When running on a non-KWin compositor or under X11, `lmux open` succeeds in spawning the app but marks the satellite as "docking unavailable" without erroring out.

### Compositor Integration Health (FR49–FR51)

- **FR49.** Cockpit injects its KWin script on startup and detects successful script load.
- **FR50.** User can trigger a re-inject of the KWin script from the sidebar when the cockpit reports compositor integration as offline.
- **FR51.** Cockpit displays a persistent banner in the sidebar when compositor integration is offline, clearly labeled and one-click recoverable.

### Configuration & Keybindings (FR44–FR48)

- **FR44.** Cockpit reads `~/.config/lmux/config.toml` on startup; missing file is equivalent to the shipped default config.
- **FR45.** User can override the prefix key and all prefix-chord bindings in `config.toml`.
- **FR46.** Config schema includes at minimum: `[keybindings]`, `[anchors]` (auto-detect patterns), `[satellites]` (close behavior), `[sandbox]` (bwrap enabled/disabled), `[ui]` (sidebar default-visible).
- **FR47.** User can trigger a config hot-reload from within the cockpit (keybinding + sidebar command); reload applies without losing pane state or killing satellites.
- **FR48.** User can extend the anchor auto-detect pattern list via config without a code change.

### Sandbox & Process Hygiene (FR52–FR55)

- **FR52.** Cockpit runs `lmux open` satellites under the bubblewrap sandbox profile defined by ADR-0010 when bwrap is present on PATH.
- **FR53.** User can bypass the sandbox per-invocation with `lmux open --no-sandbox <app>`.
- **FR54.** Cockpit falls back to direct spawn (with a warning toast) if bwrap is missing; `lmux open` never fails solely because of a missing bwrap.
- **FR55.** Cockpit propagates the v0.1 PTY shutdown contract (SIGTERM → 500 ms grace → SIGKILL → reap ≤ 700 ms) to all panes including anchors and satellite-owning panes.

### CLI & Bus (FR56–FR60)

- **FR56.** A user-facing CLI exists for session CRUD, `open`, and anchor CRUD (`lmux session ...`, `lmux open ...`, `lmux anchor ...`).
- **FR57.** CLI subcommands other than `lmux` and `lmux --session <name>` require a running cockpit and communicate with it over `$XDG_RUNTIME_DIR/lmux/bus.sock`.
- **FR58.** Cockpit creates, owns, and removes `bus.sock` across its lifecycle (created on startup, deleted on clean exit, reclaimable from a stale lockfile on crash-restart).
- **FR59.** Bus messages are versioned; the cockpit rejects incompatible client versions with a clear error rather than undefined behavior.
- **FR60.** CLI exit codes are non-zero on failure, zero on success, with diagnostic text on stderr suitable for shell scripting (`lmux session list` machine-readable format).

### State & Atomicity (FR61–FR63)

- **FR61.** Session state writes to disk are atomic (stage → fsync → rename) and safe against power loss.
- **FR62.** Cockpit tolerates a missing or malformed session file by logging and starting that session empty with the same name; corruption never prevents cockpit startup.
- **FR63.** v0.1's `last-session.json` restore contract is preserved unchanged: closing the cockpit writes the active session state; next launch reads it.

## Non-Functional Requirements

Only the NFR categories that actually bite a local desktop-cockpit are listed. Scalability (millions-of-users growth), web-accessibility-WCAG, and payment-grade security are explicitly out-of-scope.

### Performance (NFR1–NFR10)

- **NFR1. Cold start to usable cockpit ≤ 400 ms** on reference hardware (2020-era x86_64 laptop, SSD, Plasma 6). Measured from `exec lmux` to first input accepted in a restored pane.
- **NFR2. Session restore ≤ 2 s** for a session of up to 20 panes, including PTY respawn with cwd set.
- **NFR3. Fuzzy switcher open latency ≤ 50 ms** from keybinding to overlay visible and taking input.
- **NFR4. Fuzzy switcher filter latency ≤ 16 ms** per keystroke for up to 50 sessions + 200 pane entries.
- **NFR5. Satellite dock window ≤ 500 ms** from `lmux open` invocation to first placement (matches FR40 timeout budget).
- **NFR6. Pane input latency ≤ one frame (≤ 16 ms)** from keystroke to PTY write under normal load; no regression from v0.1 baseline.
- **NFR7. Idle cockpit CPU usage < 1%** on reference hardware with 8 live panes + 1 docked satellite.
- **NFR8. Idle cockpit memory usage < 200 MiB RSS** with 8 live panes + 2 anchors (including their scrollback rings) + 1 docked satellite.
- **NFR9. Anchor pause/resume round-trip ≤ 100 ms** keystroke-to-state-visible.
- **NFR10. Shutdown completes within 700 ms** of user-initiated close (preserves v0.1 contract under v0.2 conditions — more panes, anchors included).

### Reliability (NFR11–NFR16)

- **NFR11.** Cockpit MTBF ≥ 1 working day (8 h of use) across 5-day dogfooding week; crashes → 0 tolerated during the 5-day daily-driver window.
- **NFR12.** No session data loss on graceful shutdown — 100% of active-session state is present on next launch.
- **NFR13.** Session data loss on abrupt kill (SIGKILL of cockpit, power loss) bounded to the last 10 seconds of state changes (due to atomic-write-on-change policy with a debounce window).
- **NFR14.** Anchor processes survive cockpit restart *only if* the anchor was hidden — explicit model, not an accidental one. Non-hidden anchors die with the cockpit (same semantics as v0.1 panes).
- **NFR15.** KWin script outage is recoverable without cockpit restart (re-inject per FR50).
- **NFR16.** bus.sock reclaims correctly from a stale file if the cockpit is killed ungracefully; next launch succeeds without manual cleanup.

### Security & Sandbox (NFR17–NFR21)

- **NFR17.** Cockpit process opens no network sockets in v0.2 — verifiable with `ss -lntp` on a running cockpit.
- **NFR18.** Default bubblewrap profile for satellites blocks read access to `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/lmux/config.toml`, and `$XDG_RUNTIME_DIR/lmux/`.
- **NFR19.** Session state files and config files are created with mode 0600.
- **NFR20.** `bus.sock` is created with mode 0600 and rejects connections from other UIDs.
- **NFR21.** Bus messages rejected for schema-violating or oversized payloads; no parser crashes from hostile input (fuzz-tested at crate-test level for the bus codec).

### Compatibility (NFR22–NFR25)

- **NFR22.** Runs on Plasma 6.x as the release-gate configuration; Plasma 5.27 best-effort and warned in config docs.
- **NFR23.** Runs on Wayland sessions only for satellite docking; X11 sessions run the cockpit + PTY features but disable satellite docking with a one-time banner.
- **NFR24.** v0.1 Cargo workspace layout preserved; v0.2 adds new crates without restructuring existing ones.
- **NFR25.** libghostty ABI pin documented in workspace; unintended ABI break caught at compile time.

### Observability (NFR26–NFR29)

- **NFR26.** All user-visible operations (session create, restore, anchor pause/resume, satellite spawn) emit a structured tracing span with id, duration, and outcome.
- **NFR27.** Cockpit writes a rotating log to `$XDG_STATE_HOME/lmux/logs/lmux.log` (debug verbosity configurable via `RUST_LOG` and config).
- **NFR28.** Key health signals (compositor integration state, bus accept-rate, active pane count, active anchor count) are readable via a `lmux status` CLI call.
- **NFR29.** Satellite-dock success/failure events are counted in-memory so dogfooding can report a dock-success rate (target ≥ 95%, per Success Criteria).

### Usability (NFR30–NFR33)

- **NFR30.** All v0.2 features are reachable by keyboard alone (mouse optional, not required for any workflow).
- **NFR31.** Default keybindings respect the user's keyboard layout considerations from v0.1 (Norwegian-friendly, unshifted keys for primary bindings, per established user preference).
- **NFR32.** Every error state surfaced to the user also explains the recovery action (either inline or via a sidebar "details" expansion).
- **NFR33.** Onboarding: a first-run without `~/.config/lmux/config.toml` writes a commented default config and displays a one-time sidebar toast pointing at it.
