## Context

The current macOS managed-launch work improves correctness but still leaves expensive operations on user-input paths. The problematic paths are:

- `launcher::open` calls `scan_launch_entries()` before creating the GTK window.
- `set_active_anchor` calls macOS launch reconciliation before switching local state.
- `reconcile_macos_launches` can call helper window-listing for each pending launch.
- macOS visibility helpers may fall back to `osascript`, and group operations are still synchronous from the bridge worker's perspective.
- `rebuild_widget_tree` unparents all pane widgets and rebuilds the active tree on every anchor switch.

The desired product behavior is stricter than "eventually works": the cockpit should respond immediately to keyboard and sidebar gestures, even if the platform window server takes longer to catch up.

## Goals

- Keep `Ctrl+B l` launcher open on the GTK thread under 50 ms in normal conditions.
- Keep local anchor activation under 50 ms for typical cockpit state, independent of macOS helper latency.
- Ensure repeated anchor switches are latest-wins: stale native window operations must not make an old anchor active again or steal focus.
- Preserve macOS ownership safety: never hide or raise windows lmux cannot prove are lmux-owned.
- Add enough timing instrumentation to diagnose regressions without external profilers.

## Non-Goals

- Native macOS window embedding inside GTK.
- Rewriting the nested Wayland satellite host.
- Changing the anchor/satellite ownership model.
- Optimizing terminal rendering internals unrelated to anchor switching.
- Adding network telemetry.

## Decisions

### D1 - UI gestures update local state before platform work

Launcher open and anchor switch are treated as UI gestures first. They update GTK state synchronously and enqueue slow work afterwards.

For anchor switching:

1. validate target anchor;
2. update `active_anchor`, focus, CSS, and visible workspace;
3. notify sidebar active-state listeners;
4. enqueue a window-group switch with a monotonically increasing sequence;
5. return to the GTK main loop.

macOS launch reconciliation MUST NOT run inline inside `set_active_anchor`.

### D2 - Launcher entries are cached and refreshed in the background

The launcher owns an application-entry cache:

```text
LaunchEntryCache {
  entries: Vec<LaunchEntry>,
  status: Empty | Refreshing | Ready | Failed,
  last_refresh: Option<Instant>,
}
```

The cache is warmed during cockpit startup and refreshed from a background worker. Opening the launcher reads the current cache snapshot and renders immediately. If the cache is empty, the launcher displays a loading/empty state and updates rows when the worker completes.

macOS `.app` scanning and `plutil` calls MUST NOT run on the GTK main loop.

### D3 - Reconciliation is asynchronous and state-producing

macOS launch reconciliation moves behind an asynchronous boundary. A reconciliation worker receives a snapshot of pending launch intents and known satellite windows, performs helper listing off the GTK thread, and sends back a compact result:

```text
ReconcileResult {
  closed_windows,
  newly_bound_windows,
  title_updates,
  diagnostics,
  source_generation,
}
```

The GTK thread applies the result only if its generation is still current. Stale results are dropped.

### D4 - Compositor group switches are latest-wins

The compositor bridge already coalesces queued commands. This change extends latest-wins behavior to in-flight work:

- every window-group operation carries a sequence;
- the backend checks whether a newer sequence exists before expensive per-window steps;
- stale operations stop early and never raise/focus windows.

Backends return per-window results. One slow or failed window does not block the user's ability to switch anchors again.

### D5 - macOS native window-id success must not fall back to AppleScript

When the helper successfully unminimizes or raises a window by stable native window id, that operation is complete. It must not call process-wide AppleScript merely to make the app frontmost.

If app activation is required, it should use a native helper path that can target the owning application without scanning every `System Events` process. AppleScript remains a last-resort fallback for degraded environments, not the normal anchor-switch path.

### D6 - GTK workspace switching should preserve pane widgets

Full `rebuild_widget_tree` remains available for structural layout changes, but active-anchor changes should use a cheaper path. Preferred implementation:

- keep a stable container per anchor workspace, or use `GtkStack`;
- switch visible workspace containers instead of unparenting every pane;
- rebuild only when splits, closes, creates, or drag-reparent operations change the layout tree.

If a smaller first step is needed, add a fast path that skips rebuild when the pruned layout shape for the incoming workspace is already mounted.

### D7 - Sidebar active state updates are incremental

Anchor changes and anchor active-state changes are different events. The sidebar should not rebuild all rows when only `active_anchor` changes. It should toggle active row CSS and active indicator incrementally.

Preview refresh timers should skip hidden/inactive rows and must never render thumbnails while an anchor switch is in progress.

### D8 - Latency instrumentation is part of the feature

Latency-critical paths emit structured duration logs:

- `launcher.open`
- `launcher.scan`
- `anchor.switch.local`
- `anchor.reconcile`
- `gtk.workspace.switch`
- `compositor.group_switch`
- `macos.helper.list_windows`
- `macos.helper.set_visible`
- `macos.helper.apply_group`

Tests should assert behavioral non-blocking guarantees where direct wall-clock assertions would be flaky.

## Open Questions

- Should launcher cache refresh be time-based only, or also file-watcher based on application directories?
- What is the right user-facing state when launcher cache refresh fails: stale entries, empty state, or warning row?
- Should hidden inactive satellite windows be minimized, hidden, or simply left behind on macOS when restore latency is prioritized?
