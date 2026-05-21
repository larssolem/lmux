## Summary

Replace macOS native app auto-launch/adoption with an explicit attach workflow. On macOS, users open GUI apps normally, focus the desired window, and attach that exact window to the active lmux anchor.

## Motivation

The current macOS launch ownership model relies on pid, bundle id, window index, and Accessibility visibility fallbacks. In practice this is not safe for apps such as Chrome and IntelliJ because macOS may reuse app processes, mutate window ids/indexes, and apply visibility at app/process scope. The result is that lmux can move, minimize, or raise windows it does not own.

We need a model with a clear ownership boundary: lmux may only control windows the user explicitly attached.

## Goals

- Disable macOS native app launching from the lmux program launcher.
- Stop auto-registering spawned macOS app windows as anchor satellites.
- Allow users to attach the currently focused macOS window to the active anchor.
- Anchor switching may only operate on explicitly attached windows by exact stable window id.
- If an exact attached window cannot be found, lmux must not guess via bundle, process, title, or index fallback.

## Non-Goals

- Do not implement app-specific Chrome/JetBrains integrations.
- Do not use macOS process-wide hide/show as workspace behavior.
- Do not introduce macOS Spaces support in this change.
- Do not build a new compositor/containment backend in this change.

## User Workflow

1. User opens a native macOS app normally.
2. User focuses the exact native window they want an anchor to own.
3. User activates the desired lmux anchor.
4. User clicks the attach button or sends `satellite.attach_focused`.
5. lmux records the exact focused window id under that anchor.

## Risks

- macOS window ids may still disappear when apps recreate windows. In that case lmux should mark/log the window as unavailable rather than guessing.
- Exact window minimize/raise may be limited by Accessibility permissions. Failures must be visible in logs and must not fall back to broad operations.
