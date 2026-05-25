## Context

The current UX is built around anchors and satellites. The anchor is the terminal context; satellites are GUI apps owned by that context; switching anchors swaps the visible terminal pane tree and toggles satellite visibility via `CompositorControl::set_window_visible_by_pid`.

Linux can implement this with KWin scripting or a nested Wayland compositor. macOS cannot safely embed arbitrary foreign app windows into a GTK widget with public APIs. The portable product contract is therefore not "Wayland embedding"; it is "the active terminal context owns and switches a GUI window group." This change keeps that contract and implements it with native macOS window management.

## Goals / Non-Goals

**Goals**
- Build and run the cockpit on macOS with terminal panes, anchors, sessions, sidebar, and launcher.
- Preserve UX parity for anchor-owned GUI windows: spawn, group, hide on switch-away, restore on switch-back, focus, and placement.
- Use only public macOS APIs and degrade clearly when permissions are missing.
- Keep Linux KWin/nested-Wayland behavior unchanged.
- Introduce stable satellite window identity suitable for macOS single-instance applications.

**Non-goals**
- True embedding of arbitrary macOS app windows inside a GTK child widget.
- Private APIs, SIMBL-style injection, accessibility hacks that require disabling SIP, or window-server patching.
- Screen thumbnails in the first macOS port; Screen Recording permission is deferred unless previews are explicitly implemented.
- Full `.app` notarization and installer packaging beyond a developer build path.
- Changing the anchor/session mental model.

## Decisions

### D1 — macOS uses managed native windows, not embedded child windows

Decision: macOS satellites remain native top-level windows. lmux manages their lifecycle, grouping, focus, visibility, and placement so the workflow feels equivalent to docked satellites.

Reasons:
- Public macOS APIs support observing and controlling other app windows through Accessibility, but not robustly reparenting them inside GTK.
- Native windows preserve app behavior for IDEs, browsers, file dialogs, menus, tabs, and permission prompts.
- The core UX requirement is context switching, not a specific compositor implementation.

### D2 — A native helper owns AppKit/Accessibility calls

Decision: add a small macOS-only helper process (`lmux-macos-windowctl`) written in Swift or Objective-C. Rust talks to it over a versioned JSON protocol on stdin/stdout or a Unix socket.

Responsibilities:
- request/report Accessibility trust state;
- observe launched/running apps via `NSWorkspace`;
- enumerate windows via Accessibility and CGWindow metadata;
- correlate new windows to `LMUX_SATELLITE_ID` where available and fallback matching where not;
- hide/minimize/unminimize/raise/focus/place windows;
- emit window-created/window-destroyed/focus-changed events to the cockpit.

Reasons:
- AppKit and Accessibility APIs are much easier and safer from Swift/Objective-C than from raw Rust FFI.
- A helper boundary lets the Rust workspace test the protocol with a fake helper.
- A helper crash can degrade satellites without taking down the terminal cockpit.

### D3 — Stable `SatelliteWindowId` replaces PID-only visibility

Decision: add a cross-platform identity used by the cockpit and compositor backends:

```text
SatelliteWindowId {
  backend: "kwin" | "hyprland" | "macos" | "noop",
  request_id: Uuid,
  pid: Option<u32>,
  backend_window_id: String,
  bundle_id: Option<String>,
  title: Option<String>,
}
```

PID remains useful but is not authoritative. On macOS, one PID may own multiple windows and one app launch request may route to an existing process. The backend-specific window id is the AX/CGWindow identity the helper can act on.

### D4 — Correlation uses request id first, observation fallback second

Decision: all macOS satellite launches still stamp `LMUX_SATELLITE_ID=<uuid>` on the environment. When launching a fresh executable, the helper correlates windows by the app/process lineage and the request id. When macOS routes the request to an already-running single-instance app that does not inherit the environment, the helper uses a bounded observation window:

- bundle id / executable path;
- foreground app activation;
- new or title-changed AX windows;
- launch timestamp;
- active anchor at spawn time.

If correlation is ambiguous, the satellite becomes `floating_fallback` and the user can manually attach the focused window to the active anchor.

### D5 — Anchor switch is atomic at the product layer

Decision: `AppState::set_active_anchor` remains the source of truth. On each active-anchor transition, the cockpit sends one grouped command to the backend:

```text
set_active_group {
  show: [SatelliteWindowId],
  hide: [SatelliteWindowId],
  focus_policy: terminal | last_satellite
}
```

The macOS helper applies hide/minimize, restore, placement, and focus as a batch where possible. If a single window fails, the helper reports it but continues applying the rest of the group.

### D6 — Visibility policy: minimize by default, hide as fallback

Decision: macOS switch-away minimizes satellite windows by default and records whether they were minimized by lmux. If an app refuses minimize, the helper falls back to app hide only when the entire app belongs to the same anchor group. It MUST NOT hide a multi-anchor shared app if that would hide windows owned by the incoming anchor.

Reasons:
- Minimize operates at window granularity; app hide is process/app-wide.
- Many macOS apps are single-instance, so app-wide hiding can hide unrelated windows.

### D7 — Placement emulates docking

Decision: macOS placement keeps the cockpit and active satellite group in a predictable region:

- default: cockpit on the left, active GUI group in the remaining visible screen frame;
- if the user moves/resizes the cockpit, the helper repositions active satellites after a debounce;
- multiple GUI windows in one anchor use a simple cascade/tile policy in the satellite region;
- user detach stops lmux placement for that window until reattached.

This gives a docked workflow without pretending the windows are GTK children.

### D8 — Permissions are explicit runtime state

Decision: missing Accessibility permission is not a hard startup failure. The cockpit runs terminals normally, shows one clear banner, and satellites open as unmanaged floating windows until permission is granted.

The helper reports:

```text
PermissionState {
  accessibility: granted | denied | not_determined,
  screen_recording: granted | denied | not_required
}
```

Screen Recording is `not_required` for the first port.

### D9 — Platform gating avoids compiling Linux-only crates on macOS

Decision: Linux-only components (`lmux-wayland-host`, KWin D-Bus scripting path, Wayland-specific tests, `/proc` cwd lookup assumptions, `SO_PEERCRED` Linux path) are gated or replaced with macOS equivalents.

Examples:
- `lmux-wayland-host` is compiled only on Linux.
- bus peer credential checks use macOS `LOCAL_PEERCRED` / `getpeereid` equivalent.
- PTY cwd detection uses a portable fallback; `/proc/<pid>/cwd` is Linux-only.
- `PDEATHSIG` behavior becomes a platform-specific child-cleanup policy.

### D10 — E2E uses macOS VM or real macOS runner, not Xcode Simulator

Decision: end-to-end tests for the macOS port run on a real macOS desktop session: either a dedicated Mac runner or a macOS VM created through Apple's Virtualization Framework. Xcode Simulator is not a target because it simulates mobile/TV/watch/visionOS devices, not a full macOS desktop window server for arbitrary Mac apps.

Test tiers:
- fake-helper protocol tests on any platform;
- macOS headless-ish integration tests for helper protocol, permission-state handling, and app discovery;
- interactive macOS VM/runner E2E for Accessibility-controlled window operations;
- manual smoke for permission onboarding and real apps such as Finder/TextEdit/Safari/JetBrains.

The VM image MUST be resettable and preconfigured with a test user. Accessibility permission setup is documented as a manual gate first; if reliable TCC provisioning is possible in controlled CI, it can be automated later.

## Risks / Trade-offs

- *Risk: UX is not visually identical because macOS windows are not embedded.* Mitigation: define parity around workflow behavior: grouped visibility, focus, placement, and sidebar state. Avoid private APIs that would make the port fragile.
- *Risk: Accessibility permission friction.* Mitigation: first-run banner, clear status, terminal-only degradation, and no repeated permission prompts.
- *Risk: single-instance apps make correlation ambiguous.* Mitigation: `SatelliteWindowId`, bounded observation fallback, manual attach command, and clear `floating_fallback` state.
- *Risk: helper protocol drift.* Mitigation: versioned JSON handshake and fake-helper tests in Rust.
- *Risk: multi-window apps across anchors.* Mitigation: per-window identity and minimize-by-window default; app-wide hide only when safe.
- *Risk: hosted CI macOS runners cannot grant Accessibility permission for arbitrary binaries.* Mitigation: keep fake-helper tests in regular CI and run permission-dependent E2E on a dedicated Mac/VM lane.

## Migration Plan

- No Linux session migration.
- macOS sessions start with terminal/session/anchor persistence only. Live GUI windows are not restored after cockpit restart in the first port.
- Existing config keys remain valid. Add optional macOS placement keys only after the default policy is working.
- `status.get` gains backend and permission fields; clients that ignore unknown fields remain compatible.

## Open Questions

- Should the default focus after anchor switch be the terminal anchor or the last-focused GUI window in that anchor? Leaning: terminal by default, with a config option for last GUI.
- Should manual "attach focused macOS window to active anchor" be a CLI command, sidebar action, or both? Leaning: both, because it is the recovery path for single-instance apps.
- Should Screen Recording previews be part of this change or a follow-up? Leaning: follow-up.
