# ADR-0003: Historical Path A - spawn-and-track GUI satellites

- Status: Superseded
- Date: 2026-04-21
- Deciders: Lars
- Superseded by: current explicit native-window attach model in `openspec/specs/satellites/` and `openspec/specs/compositor-control/`
- Blocks: none

## Context

The original v0.2 satellite plan tried to make external GUI windows feel owned
by lmux. Wayland does not allow true inter-client reparenting, so the first
shippable approximation was to spawn a GUI app, identify its compositor window,
and then use compositor-specific APIs to move, resize, focus, and observe it.

That direction was useful as a spike: it proved that KWin scripting could see
and control windows. It is no longer the current user workflow.

The current product model is explicit attach:

1. The user opens a normal app window from the desktop, shell, or app launcher.
2. lmux lists attachable native windows through the active compositor backend.
3. The user picks the exact window to attach to the active anchor/workspace.
4. lmux stores that window identity and controls visibility/fronting on anchor
   switches.
5. The user remains responsible for placement, monitor choice, and ordinary
   window-manager layout.

## Decision

The old spawn-and-track geometry-control plan is superseded for native windows.

lmux SHALL NOT present native GUI windows as geometry-owned panes in the host
window manager. For host-compositor windows, lmux manages membership in a work
context: list, attach, hide/show, raise, and remove. It does not own placement
or monitor geometry.

The legacy `satellite.open` path may still launch a process where implemented,
but it is not the reliable ownership model. The reliable model is
`satellite.list_windows` plus `satellite.attach_window`, or the sidebar
add-window picker.

## Alternatives considered

- **True Wayland reparenting.** Still impossible by protocol design.
- **X11 XEmbed.** Not a useful primary model for a Wayland-first GTK4 app.
- **Screen capture plus input forwarding.** Too fragile and does not give real
  application ownership.
- **Host-compositor geometry ownership.** Superseded because multi-display and
  normal window-manager placement are better left to the user and desktop.

## Consequences

- Native app windows keep normal desktop behavior: decorations, shortcuts,
  accessibility, IME, and monitor placement remain owned by the OS compositor.
- Anchor switching is simpler and more predictable: lmux only needs to bring
  the active anchor's windows forward and hide or deprioritize inactive ones.
- Specs and user documentation must avoid promising spawned host windows that
  lmux owns geometrically.
- Historical docs that mention geometry-owned host windows should be read as
  spike history unless they explicitly refer to the nested Wayland host.

## Follow-up

- Keep the current behavior contract in `openspec/specs/satellites/` and
  `openspec/specs/compositor-control/`.
- If launch-and-own becomes reliable again, document it as a new capability
  instead of reviving this ADR's geometry-docking model.
