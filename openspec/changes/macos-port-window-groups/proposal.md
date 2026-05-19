## Why

lmux is currently Linux-first: terminals are portable in principle, but GUI satellites depend on Wayland/KWin or the nested Wayland host. macOS users should get the same product experience — a terminal anchor owns a set of GUI windows, and switching anchors switches the whole GUI context — without relying on Wayland mechanisms that do not exist on macOS.

## What Changes

- Add a first-class macOS platform path for building and running the cockpit with terminal panes, sessions, anchors, the sidebar, and the launcher.
- Introduce a macOS window-control backend behind the existing compositor abstraction. It manages native macOS windows through a helper using public AppKit, Accessibility, NSWorkspace, and CGWindow APIs.
- Preserve the existing user model: satellites spawned while anchor A is active belong to A; switching from A to B hides/minimizes A's GUI windows and restores/focuses B's GUI windows.
- Use "managed native windows" on macOS rather than true embedding. The cockpit controls placement, focus, hide/show, and grouping so the visible workflow matches today's docked satellite behavior, while the app windows remain normal macOS windows.
- Add macOS permission onboarding for Accessibility, and optional Screen Recording only if previews/thumbnails are enabled later.
- Add cross-platform data-model changes so a satellite is identified by a stable `SatelliteWindowId`, not only by PID. This is required because macOS apps are often single-instance and one process can own windows for multiple anchors.
- Keep Linux behavior unchanged: KWin/nested-Wayland paths remain the Linux implementations; unsupported environments still degrade through `NoopCompositor`.

## Capabilities

### New Capabilities

- `platform-macos`: macOS build/runtime behavior, native helper lifecycle, permissions, and graceful degradation.

### Modified Capabilities

- `compositor-control`: adds a macOS native window backend and refines the abstraction so window visibility/focus can be driven by stable window identity rather than PID-only matching.
- `satellites`: adds macOS managed-native satellite groups while preserving per-anchor satellite visibility and switch behavior.

## Impact

- Code: `crates/lmux` backend selection, `crates/lmux-compositor` trait/types, satellite registration in `crates/lmux/src/state.rs` and `launcher.rs`, new macOS helper code under `crates/` or `helpers/`, platform-gated build configuration, and docs.
- Runtime: macOS requires Accessibility permission for window hide/show/focus/placement. Without it, terminal features continue and satellites open as unmanaged floating windows with a clear banner.
- API: bus and CLI should continue to expose `satellite.open`, `status.get`, and anchor operations without platform-specific commands. Status gains the selected backend and macOS permission state.
- Dependencies: macOS helper may use Swift or Objective-C for public AppKit/Accessibility APIs; Rust communicates with it over stdin/stdout or a Unix socket using versioned JSON.
- Tests: Linux CI remains unchanged. macOS logic gets protocol/unit tests with a fake helper, plus manual or platform CI smoke tests for permission states and anchor-switch visibility.
