# sidebar

## Purpose

Left-side panel that makes the cockpit's state legible and actionable: session list, per-session pane tree, anchor/satellite indicators, context actions, drag-to-reorder, mini-previews, and a toast channel for system events. Tab-edge glow on each pane mirrors focus state so the user always knows where keystrokes will land.

## Requirements

### Requirement: Toggleable sidebar reachable by keyboard

The cockpit SHALL render a sidebar that can be toggled on and off via a configurable keybinding (default `prefix + b`) and SHALL be navigable using the keyboard alone.

#### Scenario: Toggle hides and shows the sidebar

- **WHEN** the user presses the configured toggle key while a pane has focus
- **THEN** the sidebar hides or shows without stealing focus from the current pane

#### Scenario: Keyboard focus into the sidebar

- **WHEN** the user presses the sidebar-focus keybinding (default `prefix + s`)
- **THEN** focus moves into the sidebar; every sidebar control (session rows, pane rows, action menu, new-anchor button) is reachable with arrow keys, `Tab`, and `Enter`

### Requirement: Session list with active-session highlight

The sidebar SHALL list every known session from `SessionIndex`, visually highlight the active session, and open any session on click or keyboard activation.

#### Scenario: Sessions render in recency order

- **WHEN** the sidebar refreshes
- **THEN** session rows are drawn in `SessionIndex` recency order and the active session row has a visually distinct highlight (coloured dot and tint)

#### Scenario: Activate a session from the sidebar

- **WHEN** the user clicks a non-active session row (or presses `Enter` on it with keyboard focus)
- **THEN** the cockpit swaps to that session via the same code path used by the fuzzy switcher (outgoing state saved, target restored)

### Requirement: Pane tree with state glyphs

The sidebar SHALL display the active session's pane tree in nested form (splits render as nested lists) with a glyph on each row for focus, satellite ownership, and anchor state.

#### Scenario: Focused pane is unambiguous

- **WHEN** the active session has a pane with focus
- **THEN** exactly one pane row in the sidebar bears the focus glyph, matching the pane that currently receives keystrokes

#### Scenario: Anchor state glyphs are distinct

- **WHEN** a pane is tagged as an anchor
- **THEN** the row displays a glyph that visually distinguishes the four anchor states: live, paused, hidden, dead

#### Scenario: Satellite ownership is visible

- **WHEN** a pane owns a docked satellite
- **THEN** the row displays a satellite-ownership glyph distinct from the anchor glyphs

### Requirement: Context actions on pane rows

The sidebar SHALL expose context actions on each pane row, reachable by right-click and by a keyboard shortcut (`prefix + m` with the row focused), with enabled/disabled state matching the pane's current state.

#### Scenario: Pane context menu exposes minimum actions

- **WHEN** the user opens the context menu on a pane row
- **THEN** the menu offers at minimum: focus-pane, close-pane, pause-anchor, resume-anchor, detach-satellite, untag-anchor, rename-anchor, move-to-group
- **AND** actions inapplicable to the pane's current state are shown as disabled

#### Scenario: Action dispatches through AppState and bus

- **WHEN** the user triggers any context action
- **THEN** the action is applied via the in-process state store, and any observable side effect (anchor state change, pane close, satellite detach) also emits a corresponding status event on the bus

### Requirement: Drag-to-reorder anchors within a group

The sidebar SHALL let the user reorder anchor rows within the same group via drag-and-drop; the new order MUST persist across redraws.

#### Scenario: Drop reassigns sort keys

- **WHEN** the user drags an anchor row and drops it within the same group
- **THEN** `sort_key` values are rewritten `0..N` reflecting the post-drop order; the sidebar re-renders in the new order and preserves it across redraws

#### Scenario: Cross-group drops are ignored or moved explicitly

- **WHEN** the user drops an anchor row into a different group
- **THEN** either the drop is rejected (row snaps back) or a move-to-group action is applied with an explicit confirmation; in no case does the row silently disappear

### Requirement: Mini-preview per pane row

The sidebar SHALL render a live mini-preview for each pane row, derived from the terminal's cell grid downsampled to one pixel per cell, refreshing on a configurable interval (default 750 ms).

#### Scenario: Preview updates at the configured interval

- **WHEN** a pane's terminal content changes and the preview interval elapses
- **THEN** the pane row's `gtk::Picture` updates to reflect the new content

#### Scenario: Preview refresh self-terminates on widget drop

- **WHEN** the sidebar row or the owning `AppState` is dropped
- **THEN** the preview refresh timer detects the broken weak reference and unschedules itself

### Requirement: Toast channel for system events

The sidebar SHALL host a dedicated toast strip for non-modal system events with severity levels (info, warn, error), auto-dismiss, and a scrollable recent-events history.

#### Scenario: Toast shown on system event

- **WHEN** the cockpit fires an internal event (anchor crash, satellite fallback, config reload, compositor status change, first-run onboarding)
- **THEN** a toast appears in the toast strip with timestamp, severity, message, and an optional one-click action

#### Scenario: Toast auto-dismiss and history

- **WHEN** a toast has been visible for 8 seconds without user interaction
- **THEN** it is dismissed from the live strip but retained in a scrollable "recent events" list capped at 20 entries

#### Scenario: Error recoverability is explicit

- **WHEN** an error toast is surfaced
- **THEN** the toast either names the recovery action inline or offers an expand affordance revealing the recovery action

### Requirement: Tab-edge focus glow

Every pane SHALL carry a tab-edge glow indicating focus state; the outgoing pane's glow fades out and the incoming pane's glow appears within one frame of any focus change.

#### Scenario: Focus change repaints glow within one frame

- **WHEN** the user changes pane focus (keyboard or click)
- **THEN** within one frame (~16 ms) the outgoing pane's focus glow fades and the incoming pane's glow appears

#### Scenario: Glow is subtle, not neon

- **WHEN** the focus glow is rendered
- **THEN** its width is at most 2 px and its alpha is at most 0.4; it MUST NOT dominate the pane contents
