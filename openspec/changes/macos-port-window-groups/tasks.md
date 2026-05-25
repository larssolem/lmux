## 1. Platform gating and build baseline

- [ ] 1.1 Gate `lmux-wayland-host` and nested-Wayland imports behind `cfg(target_os = "linux")`
- [ ] 1.2 Gate KWin-specific backend selection behind Linux/KDE detection so macOS builds do not require KWin or Wayland crates
- [ ] 1.3 Replace Linux-only `/proc/<pid>/cwd` usage in `lmux-pty` with a platform abstraction and macOS fallback
- [ ] 1.4 Replace Linux `PDEATHSIG` trampoline behavior with a platform-specific child-cleanup strategy on macOS
- [ ] 1.5 Add macOS peer-credential support for the bus (`getpeereid` / `LOCAL_PEERCRED`) while preserving Linux `SO_PEERCRED`
- [ ] 1.6 Add a macOS build job or documented local build command that compiles the terminal-only cockpit

## 2. macOS helper scaffold

- [ ] 2.1 Create `lmux-macos-windowctl` helper scaffold in Swift or Objective-C with a versioned JSON protocol
- [ ] 2.2 Implement helper handshake: protocol version, capabilities, permission state
- [ ] 2.3 Implement helper lifecycle in Rust: spawn, monitor, restart-once, and degraded fallback on crash
- [ ] 2.4 Add fake-helper test harness for Rust protocol tests
- [ ] 2.5 Document helper logs and failure messages under the existing tracing conventions

## 3. macOS permissions

- [ ] 3.1 Helper reports Accessibility trust state without blocking the cockpit
- [ ] 3.2 Cockpit status/sidebar exposes `accessibility = granted | denied | not_determined`
- [ ] 3.3 Add an actionable first-run banner for missing Accessibility permission
- [ ] 3.4 Detect permission changes at runtime and enable/disable managed satellite grouping without restart
- [ ] 3.5 Keep Screen Recording out of the required permission path unless preview work is added later

## 4. Compositor abstraction changes

- [ ] 4.1 Add cross-platform `SatelliteWindowId` / `ManagedWindowRef` type to `lmux-compositor`
- [ ] 4.2 Add identity-based visibility/focus/placement methods while keeping PID-based compatibility for existing KWin code
- [ ] 4.3 Add grouped active-anchor operation for hide/show/focus with per-window result reporting
- [ ] 4.4 Implement `MacWindowCompositor` that delegates operations to the helper
- [ ] 4.5 Update backend health/status to include backend name, degraded reason, and permission state
- [ ] 4.6 Unit-test trait compatibility for `NoopCompositor`, `KwinCompositor`, and `MacWindowCompositor`

## 5. macOS window observation and correlation

- [ ] 5.1 Helper observes app launches/activations via `NSWorkspace`
- [ ] 5.2 Helper enumerates candidate windows via Accessibility and CGWindow metadata
- [ ] 5.3 Correlate fresh launches by request id/process metadata where possible
- [ ] 5.4 Implement bounded observation fallback for single-instance apps
- [ ] 5.5 Emit `WindowCreated`, `WindowDestroyed`, `WindowTitleChanged`, and `FocusedWindowChanged` helper events
- [ ] 5.6 Mark ambiguous requests as `floating_fallback` with a clear bus/status event

## 6. Anchor-owned window groups

- [ ] 6.1 Change satellite registration in `AppState` from `anchor -> Vec<pid>` to `anchor -> Vec<SatelliteWindowId>` with migration-compatible KWin path
- [ ] 6.2 Register correlated macOS windows under the active anchor at spawn time
- [ ] 6.3 Implement manual "attach focused window to active anchor" in the bus layer
- [ ] 6.4 Add sidebar action for attaching the focused macOS window
- [ ] 6.5 Ensure removing/closing an anchor detaches or orphans its native macOS windows according to existing close policy

## 7. macOS visibility, focus, and placement

- [ ] 7.1 Helper minimizes/restores individual windows by Accessibility window reference
- [ ] 7.2 Helper avoids app-wide hide when a single app has windows owned by multiple anchors
- [ ] 7.3 Implement grouped switch operation: hide outgoing group, restore incoming group, apply focus policy
- [ ] 7.4 Implement default placement region adjacent to the cockpit
- [ ] 7.5 Debounce placement updates when the cockpit moves/resizes
- [ ] 7.6 Implement detach/reattach for macOS native windows and stop placement while detached

## 8. Launcher and app discovery on macOS

- [ ] 8.1 Add macOS app discovery for `.app` bundles in `/Applications`, `~/Applications`, and Spotlight/LaunchServices where available
- [ ] 8.2 Extend launcher entries to represent macOS apps alongside Linux `.desktop` entries
- [ ] 8.3 Launch macOS apps through helper or platform spawn while stamping request metadata where possible
- [ ] 8.4 Preserve `lmux open <argv>` for direct executable launches on macOS
- [ ] 8.5 Add fallback messaging for single-instance apps that do not inherit `LMUX_SATELLITE_ID`

## 9. Status, CLI, and observability

- [ ] 9.1 Add `lmux status` fields for `compositor.backend`, `compositor.health`, and macOS permission state
- [ ] 9.2 Add helper protocol spans for request/response latency and failure reason
- [ ] 9.3 Add counters for macOS correlated windows, floating fallbacks, attach-focused successes, and helper failures
- [ ] 9.4 Add clear user-facing diagnostics for helper unavailable, permission missing, ambiguous correlation, and window operation failed

## 10. Tests and manual smoke

- [ ] 10.1 Unit-test macOS helper protocol using the fake helper on all platforms
- [ ] 10.2 Unit-test anchor-switch grouping with stable window ids independent of platform
- [ ] 10.3 Add macOS VM/runner E2E lane; document that Xcode Simulator is not suitable for full macOS window-management tests
- [ ] 10.4 Add degraded-permission E2E: no Accessibility grant -> banner, terminal switching, unmanaged floating satellite
- [ ] 10.5 Add managed-window E2E on a dedicated Mac or macOS VM: launch TextEdit/Finder, attach to anchor, switch anchors, verify minimize/restore
- [ ] 10.6 Add macOS-only smoke test instructions: launch cockpit, grant Accessibility, open app, switch anchors, verify GUI group swaps
- [ ] 10.7 Add regression test that KWin PID-based visibility still works after identity abstraction changes
- [ ] 10.8 Add manual test for single-instance app fallback and manual attach

## 11. Documentation

- [ ] 11.1 README: platform matrix with Linux/KWin, Linux/nested-Wayland, macOS managed-native windows, and fallback behavior
- [ ] 11.2 BUILD.md: macOS prerequisites, helper build, GTK/libghostty notes, and permission setup
- [ ] 11.3 Add macOS UX note explaining managed-native windows versus true embedding
- [ ] 11.4 Add troubleshooting entries for Accessibility permission, helper crash, and ambiguous app correlation
