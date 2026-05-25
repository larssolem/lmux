## 1. Spec and Safety

- [x] 1.1 Create OpenSpec proposal, design, and implementation tasks for explicit macOS window attachment.
- [x] 1.2 Remove or disable macOS native app listing in the lmux launcher.
- [x] 1.3 Reject `satellite.open` on macOS for native app spawning.

## 2. Explicit Attach

- [x] 2.1 Change `attach_focused_macos_window_to_active_anchor` so it attaches any focused macOS window with a stable window id.
- [x] 2.2 Ensure attaching a window already registered under another anchor moves it instead of duplicating it.
- [x] 2.3 Keep sidebar attach affordance wired to the updated attach behavior.

## 3. Exact-Only macOS Visibility

- [x] 3.1 Remove process-wide/bundle-wide visibility fallback from macOS helper show/hide paths.
- [x] 3.2 Remove compositor AppleScript fallback after macOS helper group failures.
- [x] 3.3 Ensure stale or missing window ids produce logged failures without touching unrelated windows.

## 4. Tests

- [x] 4.1 Add/update unit tests for reattaching an existing backend window to a new anchor.
- [x] 4.2 Update macOS launcher tests to preserve scanner internals while public launcher entries are disabled.
- [x] 4.3 Run `cargo test -p lmux -p lmux-compositor -p lmux-macos-helper`.

## 5. Legacy macOS Launch Cleanup

- [x] 5.1 Remove stale macOS launch-intent/reconciliation state from `AppState`.
- [x] 5.2 Remove stale macOS launch baseline/reconciliation call sites from launcher and bus paths.
- [x] 5.3 Add bus/CLI support for listing macOS windows and attaching a selected window when focus-based attach is unreliable.
- [x] 5.4 Replace the sidebar attach-focused action with a macOS window picker.
- [x] 5.5 Ensure the picker shows every discovered window instance, including multiple windows from the same app/process.
- [x] 5.6 Add visual window identifiers in the picker so selection is not based on title alone.

## 6. Manual Verification

- [x] 6.1 Start lmux on macOS with pre-existing Chrome/IntelliJ windows open.
- [x] 6.2 Attach a pre-existing Chrome window to anchor 1 and verify lmux logs selected window ownership.
- [x] 6.3 Switch anchors and verify lmux hides/shows the attached Chrome window without group failures.

## 7. Cleanup and Settings

- [x] 7.1 Remove obsolete macOS offscreen/minimize tracking from the helper visibility path.
- [x] 7.2 Keep inactive macOS attached windows in place and only raise active-anchor windows during anchor switching.
- [x] 7.3 Stabilize sidebar anchor ordering so rename does not reorder anchors and new anchors append within their group.
- [x] 7.4 Add an in-app settings dialog for terminal font family and font size.
- [x] 7.5 Persist settings changes through the lmux config writer and apply them to open panes immediately.
- [x] 7.6 Populate the font setting from the system font family list instead of free-form text.
- [x] 7.7 Move Settings from the sidebar header into a cross-platform Window menu.
- [x] 7.8 Add a terminal right-click action menu with shortcut labels.
- [x] 7.9 Add editable keymap prefix support in Settings and apply it without restarting.
