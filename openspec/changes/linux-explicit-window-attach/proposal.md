## Why

Linux currently treats GUI satellite ownership as a launch-and-dock flow: lmux starts an app, tries to correlate the resulting window, and may render or control it as part of the cockpit. After the macOS port moved to explicit attachment for safety, Linux should follow the same ownership model so lmux only controls windows the user intentionally attaches to an anchor.

Linux also spans multiple display stacks and window managers. A safe plan must make compositor support capability-based instead of assuming one generic Linux window-control API exists.

## What Changes

- Replace the primary Linux GUI satellite workflow from "launch program into lmux" to "attach an already-open window to the active anchor".
- Add a platform-neutral window candidate model for listing attachable windows across macOS, KWin, X11/EWMH, and future Linux compositors.
- Generalize `satellite.list_windows` and `satellite.attach_window` so they are no longer macOS-only.
- Add Linux compositor backend capabilities for listing, identifying, attaching, hiding, showing, and raising windows.
- Implement Linux support incrementally:
  - KWin Wayland first, using the existing `lmux-dock.js` path as the authoritative window inventory/control bridge.
  - X11/EWMH as a best-effort backend for traditional X sessions.
  - Hyprland/Sway/GNOME Wayland remain explicit future work unless a backend capability can be implemented safely.
- Change Linux sidebar UI to prefer an Attach Window picker, matching macOS.
- Keep app launching as non-primary/degraded behavior unless a later change reintroduces it as "open externally, then attach".
- **BREAKING**: Linux `satellite.open` / launcher behavior will no longer be the main path for owning GUI windows under anchors.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `satellites`: Satellite ownership changes from inferred launch ownership to explicit user attachment on Linux.
- `compositor-control`: Linux compositor integrations become capability-based and expose window listing/attach/control primitives.
- `bus-ipc`: Existing satellite window list/attach kinds become platform-neutral instead of macOS-only.
- `sidebar`: Linux shows an attach-window affordance and picker instead of the launcher-first workflow.

## Impact

- Affected crates:
  - `crates/lmux`: sidebar attach UI, app state registration, bus dispatch, launcher behavior, anchor switching.
  - `crates/lmux-bus`: platform-neutral window candidate payloads.
  - `crates/lmux-compositor`: trait surface and backend implementations for KWin and X11/EWMH.
- Affected scripts:
  - `share/lmux/kwin/lmux-dock.js`: must evolve from diagnostics-only logging toward window inventory/control for KWin.
- Affected specs:
  - `openspec/specs/satellites/spec.md`
  - `openspec/specs/compositor-control/spec.md`
  - `openspec/specs/bus-ipc/spec.md`
  - `openspec/specs/sidebar/spec.md`
