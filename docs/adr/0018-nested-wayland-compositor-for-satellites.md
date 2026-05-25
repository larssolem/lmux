# ADR-0018: Nested Wayland host for pane-native satellites

- Status: Accepted
- Date: 2026-04-22
- Updated: 2026-05-24
- Deciders: Lars
- Blocks: pane-native Wayland satellite work
- Depends on: ADR-0001, ADR-0004, ADR-0017

## Context

There are two different satellite paths in lmux today:

- **Native window attach**: the reliable user workflow. The user opens a normal
  desktop window, attaches the exact window to an anchor, and lmux controls
  visibility/fronting while the OS compositor owns placement.
- **Nested Wayland host**: an implementation path for pane-native Wayland
  clients. The satellite process connects to a Wayland compositor hosted by
  lmux so its surfaces can be represented as cockpit-managed widgets.

The old host-compositor geometry-ownership model tried to make external windows
look embedded. That model is superseded for native windows. True pane-native
satellites require lmux to host the Wayland connection itself.

The repository now contains `lmux-wayland-host`, which binds the nested socket,
runs the smithay/calloop event loop on a dedicated thread, advertises Wayland
globals, tracks xdg toplevels/popups/child toplevels, handles SHM/dmabuf frame
events, and exposes command/event channels to the GTK cockpit.

## Decision

Accept the nested Wayland host as the direction for pane-native Wayland
satellites.

The host is separate from native-window attach:

- Native-window attach remains the supported flow for ordinary already-open app
  windows on KDE, X11, and macOS.
- The nested host is for clients launched into lmux's own Wayland display.
- Documentation must not describe nested host support as the same thing as
  controlling arbitrary host-compositor windows.

The nested host owns a Wayland display name and emits `HostEvent::Ready` when
clients can connect. New toplevels, title/app-id changes, child toplevels,
popups, frame events, and close events flow from the compositor thread to the
cockpit. GTK sends resize, focus, close, and shutdown commands back through the
host command channel.

## Scope

Current implementation surface:

- socket binding and dedicated compositor thread;
- `wl_compositor`, `wl_shm`, xdg toplevel/popup tracking, and related registry
  plumbing;
- SHM and dmabuf frame event surfaces;
- child-toplevel and popup event reporting;
- command/event channels between compositor thread and GTK;
- Linux-only implementation with non-Linux stubs.

Still implementation-dependent and not to be confused with general native
window attach:

- full GTK widget integration for every client class;
- cross-satellite clipboard and drag-and-drop;
- IME/input-method completeness;
- XWayland-in-lmux;
- broad app compatibility and performance tuning.

## Alternatives considered

- **Host-compositor geometry ownership.** Superseded for native windows; it fights
  the user's monitor/window-manager placement and is not true embedding.
- **XEmbed/X11 embedding.** Not suitable for Wayland-first GTK4.
- **Screen capture plus input forwarding.** Too fragile and not real client
  ownership.
- **External nested compositor process.** Rejected for the cockpit path because
  lmux still needs tight event, focus, and widget integration.

## Consequences

- The project has a clean conceptual split: explicit attach for ordinary native
  desktop windows, nested Wayland for future pane-native GUI clients.
- Specs that describe host-compositor native attach must avoid promising
  embedded/pane-native behavior.
- Specs that describe nested Wayland must be clear about implementation status
  and platform scope.
- The implementation cost lives in `lmux-wayland-host` and GTK integration, not
  in host window-manager geometry hacks.

## Follow-up

- Keep native-window attach documented in `openspec/specs/satellites/`.
- Keep compositor-control documented in `openspec/specs/compositor-control/`.
- When pane-native satellite UX becomes user-facing, add or extend capability
  specs for the nested-host flow instead of overloading native attach language.
