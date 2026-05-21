## 1. Measurement baseline

- [x] 1.1 Add structured duration logging for launcher open, launcher scan, anchor switch local work, GTK workspace rebuild/switch, macOS reconciliation, compositor group switch, helper list, helper set-visible, and helper apply-group.
- [x] 1.2 Add debug counters for active anchors, tracked satellite windows, pending macOS launches, sidebar rows, and mounted GTK pane widgets in the relevant duration logs.
- [x] 1.3 Document a manual baseline recipe: three anchors, Chrome and IntelliJ satellites on at least two anchors, repeated launcher open and anchor switch.

## 2. Non-blocking launcher

- [x] 2.1 Introduce a `LaunchEntryCache` owned by the cockpit or launcher module.
- [x] 2.2 Warm the launcher cache from a background worker during startup.
- [x] 2.3 Change `launcher::open` so it renders immediately from the current cache snapshot and never calls platform scanning synchronously.
- [x] 2.4 Update macOS scanning so `.app` traversal and `plutil` calls run only on the worker path.
- [x] 2.5 Update the launcher dialog when a cache refresh completes while it is open.
- [x] 2.6 Add tests proving `launcher::open` does not invoke the scanner directly and can render with an empty/loading cache.

## 3. Non-blocking anchor activation

- [x] 3.1 Split `set_active_anchor` into a fast local activation step and asynchronous platform reconciliation work.
- [x] 3.2 Remove direct calls to `reconcile_macos_launches` from the local activation path.
- [x] 3.3 Add a generation/sequence token for macOS reconciliation results and drop stale results on the GTK thread.
- [x] 3.4 Ensure repeated anchor switches enqueue only the latest desired satellite visibility command.
- [x] 3.5 Add tests proving active anchor state changes without macOS helper calls.

## 4. Compositor latest-wins behavior

- [x] 4.1 Extend the compositor bridge/backend contract so in-flight group switches can observe newer sequence numbers.
- [x] 4.2 Stop stale macOS group operations before per-window raise/focus work.
- [x] 4.3 Return per-window results without aborting an entire anchor switch when one window fails.
- [x] 4.4 Add tests for rapid A -> B -> C anchor switching where only C is raised/focused.

## 5. macOS helper hot-path cleanup

- [x] 5.1 Change successful stable `window_id` show/raise operations so they do not fall through to process-wide AppleScript.
- [x] 5.2 Replace normal-path `raise_visible_windows` AppleScript with a native helper operation or remove it from the fast path.
- [x] 5.3 Keep AppleScript as an explicit degraded fallback with timeout and structured logging.
- [x] 5.4 Add tests proving native helper success does not invoke AppleScript fallback.

## 6. GTK and sidebar switch cost

- [x] 6.1 Add a cheap active-workspace switch path that avoids full pane-tree teardown when layout structure has not changed.
- [x] 6.2 Keep full `rebuild_widget_tree` for structural changes: split, close, create anchor, restore session, and rearrange.
- [x] 6.3 Split sidebar callbacks into anchor-list changes and active-anchor changes.
- [x] 6.4 Update sidebar active row styling incrementally instead of rebuilding every row for active-anchor changes.
- [x] 6.5 Pause or skip sidebar preview rendering for hidden/inactive rows during anchor switching.

## 7. Verification

- [x] 7.1 Run Rust unit tests for `lmux`, `lmux-compositor`, and `lmux-macos-helper`.
- [x] 7.2 Run targeted macOS smoke from the baseline recipe and capture before/after duration logs.
- [x] 7.3 Verify `Ctrl+B l` opens immediately with warm cache and with cold cache.
- [x] 7.4 Verify rapid anchor switching remains responsive and does not restore a stale anchor's windows after the final switch.
