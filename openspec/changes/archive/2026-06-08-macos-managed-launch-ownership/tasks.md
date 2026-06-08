## 1. Cleanup and safety baseline

- [x] 1.1 Remove heuristic macOS window claiming from `crates/lmux/src/launcher.rs`
- [x] 1.2 Ensure macOS visibility operations never fall back from PID-specific control to bundle-wide app control
- [x] 1.3 Keep ambiguous macOS launches unmanaged until stable helper identity exists
- [x] 1.4 Add regression coverage that replacing a satellite with the same `request_id` removes older ownership records

## 2. Persistent lmux-managed profiles

- [x] 2.1 Change Chromium-family macOS launches to use a persistent lmux profile per app key
- [x] 2.2 Change JetBrains macOS launches to use persistent lmux `IDEA_PROPERTIES` per app key
- [x] 2.3 Document managed profile locations and reset instructions
- [x] 2.4 Add tests proving profile paths are stable across launch request ids

## 3. Helper stable window identity

- [x] 3.1 Extend `lmux-macos-helper::WindowInfo` with stable window id
- [x] 3.2 Replace `System Events` index identity with CGWindow/AX-backed identity in the helper
- [x] 3.3 Add helper protocol tests for listing multiple windows from one app process
- [x] 3.4 Add helper behavior for destroyed windows and title changes

## 4. Launch tracker

- [x] 4.1 Add a macOS launch tracker in `crates/lmux` for `request_id -> launch intent`
- [x] 4.2 Track launch state: pending, candidate, primary, closed, unmanaged
- [x] 4.3 Allow a primary window to replace a transient candidate for the same `request_id`
- [x] 4.4 Expire unresolved launch intents to unmanaged with a clear log/status event

## 5. Anchor reconciliation

- [x] 5.1 Add helper-backed listing of current lmux-owned windows
- [x] 5.2 On anchor switch, reconcile current helper windows with `AppState` ownership
- [x] 5.3 Minimize only lmux-owned windows not assigned to the active anchor
- [x] 5.4 Restore only lmux-owned windows assigned to the active anchor
- [x] 5.5 Add tests for two Chrome windows assigned to different anchors
- [x] 5.6 Add tests for two IntelliJ windows assigned to different anchors

## 6. Manual recovery path

- [x] 6.1 Add bus command to attach the focused lmux-owned macOS window to the active anchor
- [x] 6.2 Add sidebar or command-palette action for attach focused window
- [x] 6.3 Add diagnostics when a launch remains unmanaged and can be manually attached

## 7. Documentation and smoke tests

- [x] 7.1 Update `docs/macos-port.md` with managed profile and ownership model
- [x] 7.2 Update `BUILD.md` with managed profile reset instructions
- [x] 7.3 Add manual smoke: Chrome setup window -> real window -> anchor switch
- [x] 7.4 Add manual smoke: IntelliJ project chooser -> project window -> anchor switch
- [x] 7.5 Add manual smoke: two Chrome windows on two different anchors
- [x] 7.6 Add manual smoke: two IntelliJ windows on two different anchors
