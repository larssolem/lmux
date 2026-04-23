# ADR-0003: Path A — spawn-and-track GUI satellites (no Wayland reparenting)

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.2

## Context

Users expect a JetBrains IDE to sit *inside* the lmux window alongside a terminal, the way a tmux pane does. On X11 this would be XEmbed-style reparenting. On Wayland, reparenting is **impossible by design** — the protocol isolates clients from each other's surfaces, and no extension makes inter-client reparenting legitimate.

The design choice is therefore not "reparent or embed" but "accept the constraint and find the shippable approximation."

## Decision

**Path A — spawn-and-track.** lmux does not embed GUI satellites; it *docks* them by controlling the compositor:

1. lmux spawns the GUI app as a normal compositor client with a known identifier (PID on KDE, app_id + first-seen heuristic on wlroots).
2. lmux uses compositor-specific IPC to enumerate, focus, move, resize, and observe lifecycle of the spawned toplevel.
3. lmux positions the spawned window so it *appears* docked relative to its own chrome (sidebar, tabs).
4. User-facing language is **"docked"** or **"attached"**, never "pseudo-embedded" — the latter signals "not quite working" to a cold reader.

Validated by the KWin Phase 1 spike (commit `f67c37d`, `spikes/compositor-ipc/FINDINGS.md`): all five capabilities (spawn, identify, focus, move/resize, lifecycle) work with sub-5 ms steady-state round-trips.

## Alternatives considered

- **True Wayland reparenting.** Rejected: forbidden by the protocol. No extension changes this.
- **X11 XEmbed for GUI satellites.** Rejected for v0.1–v0.3: X11 is deprecated on modern Linux desktops, adds a second code path, and doesn't help the KWin/wlroots MVP audience. Parked for v0.4+ backlog.
- **Screen-capture + in-process compositing** of the GUI satellite's surface. Rejected: loses input routing, breaks HiDPI, enormous implementation cost, legally and technically fragile.
- **Launch GUI satellites as truly separate windows** with no docking. Rejected: defeats "cohesive workspace feel" — which is a named success bar.

## Consequences

- **+** Shippable on Wayland with existing protocols.
- **+** The GUI satellite is a normal compositor client; keyboard, IME, accessibility, HiDPI all work natively.
- **+** Clean separation: lmux chrome is one client, each satellite is another; compositor handles rendering/compositing.
- **−** Anti-metric risk: "docked window visibly detaches, flickers, or lags on resize." Kill criterion applies.
- **−** Each compositor needs its own backend (KWin D-Bus script; wlroots protocol + hyprctl/swaymsg). Abstracted via `CompositorControl` (see ADR-0004).
- **−** Some interactions remain imperfect: the docked IDE's own title-bar/shadow still reads as a separate window to attentive users.

## Follow-up

- Verify resize-follow latency on KWin under heavy load before v0.2 dogfood commitment.
- Phase 2 (wlroots) spike before v0.3.
