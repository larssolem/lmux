# ADR-0004: CompositorControl backend abstraction

- Status: Accepted
- Date: 2026-04-21
- Updated: 2026-05-24
- Deciders: Lars
- Amended by: ADR-0017
- Blocks: native-window attach and compositor health

## Context

lmux needs to interact with host desktop compositors without hard-coding KDE,
X11, macOS, or future backends into the cockpit UI. Different platforms expose
different primitives:

- KDE/KWin uses a mix of D-Bus scripting and window-listing/control APIs.
- X11 can be driven best-effort through X11 window ids and tools.
- macOS uses Accessibility and CoreGraphics window ids.
- unsupported desktops must still run the terminal cockpit with native attach
  disabled.

The exact method surface has changed since the original spike. The important
decision that remains is the abstraction boundary: app state, sidebar, sessions,
and bus code should call a compositor trait instead of containing
platform-specific control logic.

## Decision

lmux keeps a single cockpit-facing compositor abstraction.

The current method list is defined by ADR-0017 and the implementation in
`crates/lmux-compositor/src/lib.rs`. Backends report capabilities explicitly:
listing windows, attaching windows, setting visibility, raising windows, and
providing previews are independent capabilities.

Compositor handles and native window ids are opaque outside the backend layer.
Callers may display them for diagnostics or pass them back to the backend, but
must not parse backend-specific ids in generic cockpit logic.

Unsupported platforms use `NoopCompositor`: health/capability reporting keeps
the UI honest while terminal panes, sessions, and anchors continue to work.

## Alternatives considered

- **Inline platform branches in the sidebar and state layer.** Rejected because
  every new desktop would leak into unrelated product code.
- **One universal backend API with no capability flags.** Rejected because
  native attach support is intentionally partial across desktops.
- **Geometry ownership as the core abstraction.** Superseded. Current native
  attach treats monitor placement as user/window-manager state, while lmux
  manages context membership and visibility/fronting.

## Consequences

- Native attach can grow backend by backend without destabilizing terminal
  behavior.
- The sidebar can disable or explain unsupported actions instead of failing
  mysteriously.
- Anchor switching can be expressed as a backend operation over attached window
  identities.
- The trait will evolve, but the boundary stays: compositor-specific details
  belong in `lmux-compositor`.

## Follow-up

- Keep ADR-0017 and `openspec/specs/compositor-control/` as the current method
  and behavior references.
