# terminal-core

## Purpose

Foundational terminal multiplexer capability inherited from lmux v0.1: pane tree, PTY lifecycle, libghostty rendering, tmux-style prefix dispatcher, clean shutdown contract. Every other capability builds on this.

## Requirements

### Requirement: Pane tree with recursive splits

The cockpit SHALL render a recursive horizontal/vertical split layout of terminal panes inside a single top-level window, with exactly one pane holding keyboard focus at any time.

#### Scenario: Split a pane horizontally

- **WHEN** the focused pane receives a `split-horizontal` action
- **THEN** the pane is replaced with a horizontal split whose left child is the previous pane and right child is a fresh pane running the user's login shell in `$HOME`
- **AND** focus moves to the new (right) pane

#### Scenario: Split a pane vertically

- **WHEN** the focused pane receives a `split-vertical` action
- **THEN** the pane is replaced with a vertical split (top = previous, bottom = new) and focus moves to the new pane

#### Scenario: Close the focused pane

- **WHEN** the user closes the focused pane with an already-exited shell
- **THEN** the pane is removed from the tree, its sibling collapses into the parent slot, and focus moves to the geometrically nearest remaining pane
- **AND** if the last pane is closed, the cockpit begins its shutdown sequence

### Requirement: PTY lifecycle and signal contract

The cockpit SHALL own each pane's pseudoterminal and process-group lifecycle, and MUST reap every spawned child during shutdown within a bounded time budget.

#### Scenario: Shutdown reaps every child within 700 ms

- **WHEN** the user closes the top-level window or the last pane exits
- **THEN** the cockpit sends `SIGTERM` to every pane's process group, waits up to 500 ms for graceful exit, then sends `SIGKILL`, and reaps all children within 700 ms of the shutdown trigger

#### Scenario: PTY resize propagates to the child

- **WHEN** a pane's widget is resized so its cell grid changes from (cols, rows) to (cols', rows')
- **THEN** the cockpit issues `TIOCSWINSZ` on the PTY master with the new dimensions before the next input is written to the child

#### Scenario: Child exit becomes a pane state transition

- **WHEN** a pane's child process exits (by any signal or normal exit)
- **THEN** the cockpit detects the exit via `waitpid` and marks the pane as exited without blocking the GTK main loop

### Requirement: libghostty terminal rendering

The cockpit SHALL render terminal cells through libghostty (dynamically linked) and MUST preserve per-keystroke input latency equivalent to the v0.1 baseline.

#### Scenario: Keystroke reaches the PTY within one frame

- **WHEN** the user presses a printable key in a focused pane under normal load
- **THEN** the keycode is written to the PTY master within one frame (≤16 ms) of the GTK key event

#### Scenario: libghostty is dynamically linked

- **WHEN** `ldd` is run against the built `lmux` binary
- **THEN** `libghostty` appears as a dynamic dependency (not statically embedded)

### Requirement: Tmux-style prefix key dispatcher

The cockpit SHALL provide a two-stroke prefix dispatcher (default `Ctrl+B` + key) that intercepts keybindings on a GTK Capture-phase controller before the focused pane receives the keys.

#### Scenario: Prefix arms, follower is consumed by the cockpit

- **WHEN** the user presses the configured prefix key
- **THEN** the dispatcher enters an armed state; the next non-modifier keystroke is captured by the cockpit instead of being forwarded to the PTY
- **AND** the armed state clears after the follower key or after a 1 s timeout

#### Scenario: A bare prefix key still reaches the terminal when escaped

- **WHEN** the user presses the prefix key twice in quick succession
- **THEN** the second keystroke is forwarded to the focused pane as a literal prefix character

### Requirement: Non-blocking UI event loop

The cockpit SHALL keep the GTK main loop responsive under PTY I/O and disk I/O load; long-running work MUST run on Tokio tasks and hand results back to the UI via `glib::MainContext::spawn_local`.

#### Scenario: A blocking write does not stall input

- **WHEN** a PTY child writes a large burst faster than the rendering pipeline consumes it
- **THEN** the UI continues accepting keyboard and mouse events and redraws within its normal frame budget; backpressure is applied at the read side of the PTY, not the UI

### Requirement: Bell-to-toast regression guard

The cockpit SHALL convert a terminal bell (`BEL`, 0x07) in any pane to a non-focus-stealing sidebar toast within one frame of arrival.

#### Scenario: Bell in a non-focused pane surfaces as a toast

- **WHEN** a non-focused pane's PTY emits `\a`
- **THEN** a toast appears in the sidebar toast strip tagged with the pane id, and no focus change occurs

### Requirement: Cargo workspace shape preserved

The project SHALL retain the v0.1 Cargo workspace structure; new capabilities are added as new crates under `crates/` without restructuring the existing ones.

#### Scenario: Workspace membership enumerable

- **WHEN** `cargo metadata --format-version=1 | jq '.workspace_members | length'` runs
- **THEN** it reports the current workspace member count; removal of any pre-existing v0.1 crate constitutes a breaking change
