## Design

### Ownership Model

macOS native windows become lmux-managed only through explicit user attachment. A managed macOS satellite record contains:

- backend: `Macos`
- generated lmux request id
- pid from the selected window
- bundle id from the selected window, when available
- exact AX window number encoded in `backend_window_id`, when available
- selected AX window index encoded in `backend_window_id` when AX does not expose a stable window number
- current title, if available

No launch intent is required for attachment.

### Program Launcher Behavior

On macOS, the GUI program launcher must not list `.app` entries. This prevents the old flow:

`launcher -> spawn -> wait for window -> infer ownership`

Linux behavior is unchanged.

The bus `satellite.open` operation must reject native macOS app spawning for now and direct callers to explicit attach commands.

### Attach Focused Window

`attach_focused_macos_window_to_active_anchor` should:

1. Require an active anchor.
2. Ask the macOS helper for the focused window.
3. Require a stable `window_id`.
4. Register that exact window under the active anchor.
5. Move any existing registration for the same backend window id to the new anchor.
6. Broadcast satellite visibility.

The method must not require a matching launch intent and must not read `macos_launches`.

### Attach Selected Window

When focus-based attach is unreliable because clicking lmux changes focus, lmux supports listing native macOS windows and attaching one selected by the user.

Selected-window attach should:

1. List candidate windows from native Accessibility window enumeration.
2. Attach the selected pid/window id when an AX window number exists.
3. Otherwise attach the selected pid/window index, optionally with title as a fingerprint.
4. Move any existing registration for the same backend window id to the new anchor.
5. Broadcast satellite visibility.

The pid/window-index path is not a bundle/process fallback: lmux still targets one AX window in one process and fails if that selected window cannot be found.

### Visibility Safety

macOS helper visibility must target only attached windows:

- Prefer native AX lookup by exact `AXWindowNumber`.
- If AX does not expose a window number, use the explicitly selected pid/window index and title fingerprint.
- Do not fall back to bundle id.
- Do not fall back to app/process visibility.
- Do not use process-wide `AXHidden` or System Events `visible of process`.
- If the selected window cannot be found, return an error and do nothing.

The compositor must not apply AppleScript fallback after helper failure for grouped macOS visibility. Partial failure is acceptable; moving unrelated windows is not.

### Future Extension

If exact-window minimize/raise remains inadequate, the next candidate is park/restore by exact AX position and size. That must be a separate change and must preserve the same explicit ownership rule.
