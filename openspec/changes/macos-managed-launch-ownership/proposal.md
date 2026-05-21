## Why

The current macOS launcher work proved that PID-only matching, bundle-wide fallback, and `System Events` window indices are not safe enough for anchor-owned GUI workflows. Chrome and JetBrains can open transient setup/project windows before the actual working window, and existing windows may be indistinguishable from new ones when titles are missing or window indices reorder. This can cause lmux to claim windows it did not start or lose the real window that should belong to the active anchor.

We need a safer foundation: lmux-managed app profiles isolate lmux-owned app instances from the user's normal app state, while a native helper provides stable per-window identity so multiple windows from the same app can belong to different anchors.

## What Changes

- Introduce a macOS managed-launch ownership model:
  - lmux starts supported apps with a persistent lmux-managed profile per app, not a throwaway profile per launch.
  - lmux never controls windows outside lmux-managed app processes/profiles.
  - anchor ownership is tracked at window identity level, not at app or bundle level.
- Replace heuristic `System Events` window claiming with a planned native helper flow based on stable macOS window identity.
- Reconcile lmux-owned windows during anchor switches:
  - discover current lmux-owned windows;
  - attach new windows to pending launch requests when unambiguous;
  - minimize non-active anchor windows;
  - restore active anchor windows.
- Keep uncertain windows unmanaged instead of guessing.

## Capabilities

### Modified Capabilities

- `platform-macos`: adds managed app profiles and a safe ownership model for lmux-started native app windows.
- `satellites`: refines macOS satellite ownership so multiple windows from the same lmux-managed app can belong to different anchors.
- `compositor-control`: requires stable macOS window identity and grouped reconciliation for anchor switching.

## Impact

- Code: `crates/lmux-compositor/src/spawn.rs`, `crates/lmux-compositor/src/macos.rs`, `crates/lmux-macos-helper`, `crates/lmux/src/launcher.rs`, `crates/lmux/src/state.rs`.
- Runtime: Chrome/JetBrains and other supported apps use persistent lmux-managed profiles. Users configure the lmux profile once; normal user profiles are not modified.
- Safety: lmux must not hide/minimize app windows that it cannot prove are lmux-owned.
- Tests: add fake-helper tests for stable window identity, ownership reconciliation, and multi-anchor windows from the same app.
