## Why

Anchor and launcher interactions currently feel far slower than their conceptual cost. Pressing `Ctrl+B l` can take seconds before the launcher appears, switching to an anchor with several satellites can produce macOS beachball stalls, and repeated anchor switches may appear to do nothing until the user retries.

The root problem is that latency-sensitive UI actions still perform blocking work on the GTK main loop:

- the launcher scans installed applications synchronously before rendering;
- macOS launch reconciliation runs synchronously during `set_active_anchor`;
- anchor switching tears down and rebuilds GTK pane trees;
- macOS window visibility can fall back to slow `System Events` / `osascript` paths.

Anchor switching and launcher opening are core cockpit gestures. They MUST feel immediate even when native app discovery, window reconciliation, or platform window control takes longer in the background.

## What Changes

- Make launcher opening non-blocking:
  - cache launch entries in the background;
  - render the launcher immediately using cached or loading state;
  - keep application scanning off the GTK main loop.
- Make anchor switching non-blocking:
  - remove synchronous macOS launch reconciliation from the active-anchor hot path;
  - perform platform window reconciliation asynchronously;
  - coalesce or cancel stale window-group operations when users switch anchors quickly.
- Reduce GTK churn during anchor switches:
  - avoid full pane-tree teardown/rebuild when only the active workspace changes;
  - update sidebar active state without rebuilding every row when possible.
- Tighten macOS window-control hot paths:
  - prefer native stable-window-id operations;
  - avoid AppleScript fallback after a successful native operation;
  - degrade per window rather than blocking or failing the whole switch.
- Add latency instrumentation and regression coverage for launcher open, anchor switch, reconciliation, and compositor bridge work.

## Capabilities

### Modified Capabilities

- `sidebar`: launcher and anchor row interactions become immediate UI actions; expensive scans/previews must not block input.
- `satellites`: anchor-owned satellite visibility reconciliation becomes asynchronous and coalesced.
- `compositor-control`: grouped window switches become latest-wins operations and must not block the cockpit UI thread.
- `observability`: user-visible latency-critical operations gain duration logging and regression thresholds.

## Impact

- Code: `crates/lmux/src/launcher.rs`, `crates/lmux/src/launcher/macos.rs`, `crates/lmux/src/state.rs`, `crates/lmux/src/sidebar.rs`, `crates/lmux/src/compositor_bridge.rs`, `crates/lmux-compositor/src/macos.rs`, `crates/lmux-macos-helper`.
- Runtime: launcher first-open may show cached/loading content while discovery finishes; anchor switches update cockpit focus immediately while native app windows are restored asynchronously.
- Safety: lmux must continue to control only lmux-owned windows; no change relaxes the ownership constraints from `macos-managed-launch-ownership`.
- Tests: add unit tests for no synchronous helper calls in `set_active_anchor`, cache-backed launcher behavior, compositor sequence coalescing, macOS native-window success without AppleScript fallback, and focused smoke tests for multi-anchor satellite switching.
