## Manual Baseline Recipe

Use this recipe before and after implementation when validating responsiveness on macOS.

## Setup

1. Start `lmux` with trace logging enabled.
2. Create three anchors: `A`, `B`, and `C`.
3. Under anchor `A`, launch Chrome and IntelliJ.
4. Under anchor `B`, launch another Chrome window and another IntelliJ project/window.
5. Leave anchor `C` with only its terminal pane.

## Launcher Baseline

1. Focus a terminal pane.
2. Press `Ctrl+B l`.
3. Record the `launcher.open` duration and whether `launcher.scan` appears on the same user-input path.
4. Close the launcher and repeat twice: once with a warm cache and once after restarting `lmux`.

Expected target after this change: the launcher window appears immediately. A cold scan may finish later, but application traversal and `plutil` work must not block the GTK main loop.

## Anchor Switch Baseline

1. Switch `A -> B -> C -> A` using the sidebar.
2. Repeat the same sequence quickly, without waiting for native app windows to finish raising.
3. Record `anchor.switch.local`, `gtk.workspace.switch`, `anchor.reconcile`, `compositor.group_switch`, and macOS helper durations.
4. Watch for stale focus: after a rapid final switch to `C`, windows from `A` or `B` must not come forward later.

Expected target after this change: local anchor focus and workspace display update immediately. Native app windows may restore asynchronously, but the cockpit must remain responsive and the final active anchor must win.
