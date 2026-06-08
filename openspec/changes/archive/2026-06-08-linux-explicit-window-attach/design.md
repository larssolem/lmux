## Context

The macOS port has moved native GUI ownership away from launch inference and toward explicit window attachment. Linux still has a launcher-first flow that starts apps with lmux tags and then tries to own the resulting window by process correlation or nested Wayland embedding.

That model is fragile on Linux because "Linux window control" depends on the active display stack:

- KWin Wayland exposes useful control through KWin scripting, but only if lmux owns a script/helper bridge.
- X11 exposes global window inventory through EWMH, but that does not cover native Wayland clients.
- wlroots compositors vary: Hyprland and Sway have useful IPC, but there is no universal Wayland window-management API.
- GNOME Wayland intentionally does not expose broad window control through a stable public API.

The design therefore treats Linux as a set of compositor capabilities behind one product workflow.

## Goals / Non-Goals

**Goals:**

- Make Linux GUI ownership match macOS: lmux controls only windows explicitly attached by the user.
- Provide one platform-neutral attach picker and bus model for macOS and Linux.
- Make backend support capability-based so unsupported window managers degrade honestly.
- Implement a first backend path for KWin and a best-effort X11/EWMH path.
- Preserve terminal/session/anchor behavior when no attach-capable backend is available.

**Non-Goals:**

- Do not implement Hyprland, Sway, GNOME extensions, or generic wlroots support in this change.
- Do not embed arbitrary native Linux windows inside the GTK pane tree.
- Do not keep PID-only launch inference as the primary ownership model.
- Do not solve remote/SSH, plugin SDK, or packaging work.

## Decisions

### Decision: Explicit attach is the only ownership boundary

Linux native windows become lmux-managed only when the user selects a window from an attach picker or sends a platform-neutral `satellite.attach_window` request. The selected window record is registered under the active anchor and anchor switching applies only to registered records.

Alternative considered: keep launch inference and improve correlation. This fails the macOS safety lesson and remains weak for single-instance apps, browser profiles, and apps that reparent or respawn windows.

### Decision: Use platform-neutral `WindowCandidate`

The bus and UI should stop naming macOS in the data model. A candidate should carry:

- backend (`macos`, `kwin`, `x11`, future `hyprland`, etc.)
- backend window id
- pid when available
- app identity when available (`bundle_id`, `desktop_entry`, `wm_class`, `app_id`)
- title when available
- optional workspace/output metadata

`SatelliteWindowId` remains the managed-record type after attachment.

Alternative considered: add Linux-specific payloads next to macOS payloads. That duplicates UI and bus behavior and makes future backends harder.

### Decision: Capability-based compositor control

`CompositorControl` or a sibling trait should expose window-management capabilities:

- `list_windows`
- `attach_window`
- `set_window_visible`
- `focus_window` or `raise_window` when supported
- `capabilities`

The cockpit must check capabilities before showing controls or dispatching actions. Unsupported operations return explicit unsupported/degraded results, not silent success.

Alternative considered: assume KWin is the Linux backend. That is fine for the first implementation but not for the product model.

### Decision: KWin is first-class, X11 is best-effort

KWin Wayland should become the first Linux attach backend because the project already has `lmux-dock.js` and zbus integration. The KWin script must move beyond diagnostics and become the window inventory/control bridge.

X11/EWMH can be implemented as a best-effort backend for sessions where global window listing is available. It should not pretend to cover Wayland windows.

Alternative considered: start with X11 because it is easier. That would not help modern KDE Wayland, which is the existing Linux compositor target in this repo.

### Decision: KWin bridge uses D-Bus callback inventory

The KWin script publishes native-window snapshots and add/remove events to a Rust-owned session D-Bus service at `no.jpro.lmux.KWinBridge`. Rust keeps the current inventory in memory and asks the script for a fresh snapshot before serving `list_windows`. Exact KWin window ids use the `kwin:<internalId|windowId|uuid>` namespace and attach/visibility operations reject records without that exact id.

Alternative considered: write a JSON inventory file from KWin. KWin scripts do not have a dependable filesystem API, and the existing spike notes already point to `callDBus(...)` as the practical script-to-Rust stream path.

### Decision: X11 starts with tool-backed EWMH

The first X11 backend uses `xprop` for read-only `_NET_CLIENT_LIST`/window property discovery and `xdotool` for best-effort exact window operations. This avoids adding a Rust X11 dependency while keeping the backend capability-gated and easy to replace with a crate-backed implementation later.

Alternative considered: add an X11 crate immediately. That can be revisited if the tool-backed backend proves too slow or too inconsistent across distributions.

### Decision: Launcher becomes secondary

The Linux sidebar should prefer "Attach window". The existing app launcher can either be hidden during this change or retained as an explicitly separate "Launch externally" command that does not establish ownership until the user attaches a window.

Alternative considered: launch then automatically open the attach picker. This still encourages inferred ownership and mixes two workflows. It can be revisited later.

## Risks / Trade-offs

- KWin scripting may not expose every stable id we want → encode the most stable available KWin identity and fail closed when a stored window cannot be found.
- X11 minimize/raise behavior differs by WM → treat X11 as best-effort and surface partial failures in logs/toasts.
- GNOME Wayland users may see no attach support → show clear degraded UI instead of broken controls.
- Removing launcher-first behavior may feel like a regression → keep terminal/session behavior intact and explain the new attach flow in UI labels/toasts.
- Existing tests/specs assume `satellite.open` launch ownership → update specs and tests to separate "spawn app" from "own attached window".

## Migration Plan

1. Introduce platform-neutral bus/data types while keeping macOS compatibility mappings.
2. Generalize the existing macOS picker into an attach picker that can render any `WindowCandidate`.
3. Add capability reporting and degraded UI for unsupported Linux environments.
4. Implement KWin list/attach/show/raise over the KWin script bridge.
5. Add X11/EWMH list/attach/show/raise as best effort.
6. Change Linux launcher affordances so Attach Window is primary and launch ownership is no longer implied.
7. Remove or quarantine PID-only launch registration paths after the attach flow is covered.

Rollback is straightforward at the product level: keep the old launcher code path behind a feature flag or internal fallback until KWin attach is usable, but do not expose it as the primary ownership model.

## Open Questions

- Should `satellite.open` remain as "launch external app without ownership" or be deprecated on Linux?
- Should the fixed KWin bridge D-Bus name become per-instance before multi-cockpit support?
- Should the X11 tool-backed backend be replaced by a crate-backed EWMH implementation before release?
- Do we want separate UI labels for "Attach window" and "Launch app", or only the attach path in v0.2?
