## 1. Data Model and Bus Contract

- [x] 1.1 Replace macOS-only window candidate payloads in `crates/lmux-bus/src/kinds.rs` with a platform-neutral candidate carrying backend, backend window id, optional pid, optional app identity, optional title, and optional workspace/output metadata.
- [x] 1.2 Update bus serde round-trip tests so `satellite.list_windows` and `satellite.attach_window` cover macOS, KWin, X11, and unsupported-backend payloads.
- [x] 1.3 Update `crates/lmux/src/bus.rs` so `satellite.list_windows` and `satellite.attach_window` dispatch through compositor/window-manager capabilities instead of macOS-only helper calls.
- [x] 1.4 Preserve macOS behavior by mapping existing macOS helper window info into the new platform-neutral candidate shape.

## 2. Compositor Capability Layer

- [x] 2.1 Add capability reporting for native window listing, exact-window attach, visibility, and raise/focus support.
- [x] 2.2 Add platform-neutral list/attach/window-operation methods to `CompositorControl` or introduce a focused sibling trait used by `AppState`.
- [x] 2.3 Ensure unsupported backends return explicit unsupported/degraded results without disabling terminal/session features.
- [x] 2.4 Update `SatelliteWindowId` construction so Linux attached windows store exact backend ids instead of PID-only identities.

## 3. KWin Backend

- [x] 3.1 Extend `share/lmux/kwin/lmux-dock.js` from diagnostics-only logging to maintain/query current KWin windows with stable backend identities.
- [x] 3.2 Implement KWin window listing in `crates/lmux-compositor/src/kwin.rs` using the KWin bridge.
- [x] 3.3 Implement KWin attach validation so only candidates with usable exact KWin identities can become managed satellite records.
- [x] 3.4 Implement KWin exact-window visibility/raise operations using stored backend window ids, with no pid/app/title fallback.
- [x] 3.5 Add unit tests for KWin candidate parsing and exact-identity command generation.

## 4. X11/EWMH Backend

- [x] 4.1 Add or select an X11/EWMH integration path for top-level window listing.
- [x] 4.2 Implement X11 candidate listing with X11 window id, title, pid, and WM_CLASS when available.
- [x] 4.3 Implement best-effort exact X11-window visibility/raise operations.
- [x] 4.4 Ensure X11 backend reports unsupported cleanly when EWMH/window-control setup is unavailable.
- [x] 4.5 Add tests for X11 candidate conversion and unsupported fallback behavior.

## 5. Cockpit State and Anchor Switching

- [x] 5.1 Add `AppState` attach logic for platform-neutral window candidates and move existing registrations when the same backend window is attached to a new anchor.
- [x] 5.2 Update grouped anchor-switch broadcasts to operate on exact attached window records for Linux and macOS.
- [x] 5.3 Remove or quarantine Linux PID-only spawn registration from launcher and `satellite.open` ownership paths.
- [x] 5.4 Ensure missing/stale Linux attached windows fail closed and never fall back to broad pid/app/title/class control.

## 6. Sidebar and User Workflow

- [x] 6.1 Generalize the macOS window picker into a platform-neutral attach picker.
- [x] 6.2 Show Attach Window as the primary Linux GUI action when the backend supports attachment.
- [x] 6.3 Show a clear degraded state when the active Linux compositor cannot list or attach windows.
- [x] 6.4 Make any remaining launch action visually and behaviorally secondary, with no implied ownership.
- [x] 6.5 Add tests for supported, unsupported, empty-list, and attach-failure picker states.

## 7. Validation

- [x] 7.1 Run `cargo test -p lmux-bus -p lmux-compositor -p lmux`.
- [x] 7.2 Manually verify KWin Wayland: attach an existing browser window to anchor A, attach another app/window to anchor B, switch anchors, and confirm only attached windows are affected.
- [x] 7.3 Manually verify unsupported Wayland compositor behavior: attach UI is degraded and terminal/session features still work.
- [x] 7.4 Manually verify X11/EWMH behavior where available, including multi-window apps with the same WM_CLASS.
- [x] 7.5 Update docs or ADR notes if the final implementation settles the KWin bridge transport or X11 dependency choice.

Validation notes:

- 7.1: `mise install` installed the repo-pinned Zig 0.15.2; `mise exec -- cargo test -p lmux-bus -p lmux-compositor -p lmux` passed.
- 7.2: KDE Wayland run used `KwinCompositor`; attached Chrome to anchor A and Slack to anchor B, then switched A/B. Logs showed exact `backend_window_id` hide/show sets for only the two attached records.
- 7.3: simulated unsupported Wayland with KDE env unset and `XDG_CURRENT_DESKTOP=GNOME`; lmux selected `NoopCompositor`, `status`/`pane list` still worked, and `satellite list-windows` returned an unsupported domain error.
- 7.4: simulated X11 with KDE env unset and `XDG_SESSION_TYPE=x11`; lmux selected `X11Compositor`. Two `GDK_BACKEND=x11 zenity` windows with the same `wm_class:zenity` listed as separate exact `x11:<window-id>` candidates. This host lacks `xdotool`, so attach/control correctly reported unsupported.
