# terminal-core

## Purpose

The terminal cockpit is the stable core of lmux: one GTK window, a recursive
split tree of PTY-backed terminal panes, libghostty-vt rendering, a tmux-style
prefix dispatcher, scrollback, copy/paste shortcuts, and coordinated shutdown.
The first terminal in a fresh cockpit is auto-tagged as the initial workspace
anchor so anchor-gated UI and attach flows always have a target.

## Requirements

### Requirement: Pane tree with recursive splits

The cockpit SHALL render a recursive horizontal/vertical split layout of
terminal panes inside one top-level GTK window.

#### Scenario: Split a pane right

- **WHEN** the focused pane receives the split-right action (`prefix + +`,
  `prefix + |`, `prefix + \`, or the terminal context action)
- **THEN** the focused leaf is replaced by a vertical split whose left child is
  the previous pane and whose right child is a fresh pane
- **AND** the fresh pane starts in the source pane's current working directory
- **AND** focus remains on the source pane

#### Scenario: Split a pane down

- **WHEN** the focused pane receives the split-down action (`prefix + -` or the
  terminal context action)
- **THEN** the focused leaf is replaced by a horizontal split whose top child is
  the previous pane and whose bottom child is a fresh pane
- **AND** the fresh pane starts in the source pane's current working directory
- **AND** focus remains on the source pane

#### Scenario: Close a non-last pane

- **WHEN** the user closes a focused pane and at least one other leaf remains
- **THEN** the pane is removed from the layout, its sibling collapses into the
  parent slot, the pane process is terminated, and focus moves to a surviving
  leaf

#### Scenario: Close the last pane is ignored

- **WHEN** the focused pane is the only leaf and receives close-pane
- **THEN** the cockpit leaves the pane alive and logs that the last pane close
  was ignored
- **AND** the user must quit through the quit action or window close path

### Requirement: PTY lifecycle and shutdown

The cockpit SHALL own each pane's PTY and child process lifecycle.

#### Scenario: Pane process is terminated on close

- **WHEN** a non-last pane is closed
- **THEN** the cockpit sends cooperative termination to the pane child and
  schedules a force-kill fallback

#### Scenario: Cockpit shutdown drains every pane

- **WHEN** the user quits lmux or the top-level window closes
- **THEN** the cockpit marks itself shutting down, saves the current snapshot,
  terminates every live pane, clears anchor/workspace state, and exits the GTK
  application

#### Scenario: PTY resize propagates to child

- **WHEN** a pane widget changes size so its terminal grid changes
- **THEN** the PTY window size is updated before subsequent input is written to
  the child process

### Requirement: libghostty-vt static rendering stack

The cockpit SHALL render terminal cells through the vendored libghostty-vt
library, built by Zig and statically linked as `ghostty-vt-static`.

#### Scenario: Build links the static Ghostty VT library

- **WHEN** `crates/lmux-libghostty/build.rs` runs
- **THEN** it runs `zig build --release=fast` under `vendor-ghostty`
- **AND** it emits `cargo:rustc-link-lib=static=ghostty-vt-static`
- **AND** bindgen generates bindings from `ghostty/vt.h` with
  `-DGHOSTTY_STATIC`

### Requirement: Prefix dispatcher

The cockpit SHALL provide a configurable two-stroke prefix dispatcher. Only the
prefix key itself is user-configurable today; the follower command table is
compiled in.

#### Scenario: Prefix arms and consumes follower

- **WHEN** the configured prefix key is pressed while a terminal pane has focus
- **THEN** the dispatcher enters an armed state, the window title and command
  hint reflect that state, and the next non-modifier key is consumed by the
  cockpit
- **AND** the armed state clears after the follower key or after one second

#### Scenario: Satellite focus bypasses cockpit shortcuts

- **WHEN** the focused pane is a Wayland/native satellite
- **THEN** the cockpit lets key events pass through and disarms any pending
  prefix state

#### Scenario: Built-in prefix commands

- **WHEN** the dispatcher is armed
- **THEN** `+`, `|`, and `\` split right, `-` splits down, `x` closes the
  focused pane, `a` cycles/creates the active workspace anchor, `s` opens the
  session switcher, `m` toggles rearrange mode, `q` quits, `o`/`n`/`]` cycle
  focus forward, and `p`/`[` cycle focus backward

### Requirement: Terminal input and scrollback shortcuts

Terminal panes SHALL translate keyboard input to PTY bytes and reserve
application-level scroll/copy/paste shortcuts.

#### Scenario: Page keys scroll the focused pane

- **WHEN** the user presses `PageUp` or `PageDown`
- **THEN** the focused pane scrolls by one page
- **AND** `Shift+PageUp` / `Shift+PageDown` scroll by one row

#### Scenario: Clipboard shortcuts are platform-aware

- **WHEN** the user presses `Ctrl+Shift+C` or `Ctrl+Shift+V`
- **THEN** the focused terminal pane copies or pastes
- **AND** on macOS, `Command+C` and `Command+V` do the same

#### Scenario: Clipboard image paste injects a file path

- **WHEN** terminal paste is invoked and the clipboard advertises an `image/*`
  MIME type
- **THEN** lmux reads the clipboard image as a texture, writes it as a PNG under
  `$XDG_RUNTIME_DIR/lmux/pastes/` or `/tmp/lmux-pastes-<uid>/`
- **AND** it injects the absolute file path into the PTY via bracketed paste
- **AND** it does not inject raw image bytes or inline terminal graphics

#### Scenario: Clipboard text paste remains the fallback

- **WHEN** terminal paste is invoked and no clipboard image can be read
- **THEN** lmux reads clipboard text and injects it through the normal
  bracketed-paste path

### Requirement: Bell notification

The cockpit SHALL convert terminal bell bytes into local desktop
notifications without blocking the GTK main loop.

#### Scenario: Bell replaces prior notification for same pane

- **WHEN** a pane emits BEL
- **THEN** the notifier sends a freedesktop notification labelled with the pane
  or anchor label
- **AND** later bells from the same pane reuse the previous notification id
  instead of stacking unlimited notifications

#### Scenario: Notification click raises lmux

- **WHEN** the notification daemon reports the default action for a notification
  created by lmux
- **THEN** the cockpit presents the lmux window
