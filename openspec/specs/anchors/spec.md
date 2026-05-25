# anchors

## Purpose

An anchor is a terminal pane tagged as a workspace root. Exactly one anchor is
active on screen at a time. Non-active anchors and panes owned by them are hidden
from the GTK workspace view; native windows attached to those anchors are sent
through the compositor bridge for hide/show handling. Anchors can be named,
grouped, reordered, paused, hidden, reattached, activated, and removed.

## Requirements

### Requirement: First terminal auto-anchor

The cockpit SHALL ensure a fresh session has an anchor target.

#### Scenario: Fresh startup creates initial anchor

- **WHEN** lmux starts with one fresh terminal and no restored anchor metadata
- **THEN** the cockpit tags that terminal as an anchor
- **AND** makes it the active anchor

#### Scenario: Restored snapshots keep restored anchors

- **WHEN** a snapshot restores one or more anchor pane ids
- **THEN** each restored pane is tagged through the restore path
- **AND** the first restored anchor becomes active

### Requirement: Manual anchor tagging

The cockpit SHALL let the user tag a live terminal pane as an anchor.

#### Scenario: Prefix `a` creates the first anchor

- **WHEN** no anchors exist and the user presses `prefix + a`
- **THEN** the focused pane is tagged as an anchor and becomes active

#### Scenario: Tagging absorbs unowned panes

- **WHEN** a pane is tagged as an anchor
- **THEN** all currently unowned panes are assigned to that anchor's workspace
- **AND** existing panes owned by other anchors are left alone

#### Scenario: Satellite pane cannot become anchor

- **WHEN** a pane is already owned by a different anchor workspace
- **THEN** anchor tagging refuses to promote it

### Requirement: Anchor identity

Every live pane SHALL have a process-local UUID, and every tagged anchor SHALL
have a separate anchor UUID.

#### Scenario: Pane UUID addresses non-anchor panes

- **WHEN** `pane.list` returns a pane
- **THEN** the returned `pane_id` is the pane UUID used by `anchor.tag`

#### Scenario: Anchor UUID addresses anchor operations

- **WHEN** `anchor.pause`, `anchor.resume`, `anchor.hide`, `anchor.reattach`,
  `anchor.untag`, or `anchor.activate` is called
- **THEN** the UUID is resolved as an anchor UUID to its live pane

#### Scenario: UUIDs are process-local

- **WHEN** panes are restored from session snapshots
- **THEN** fresh pane UUIDs and anchor UUIDs are assigned for the new cockpit
  process

### Requirement: Active anchor workspace switching

The cockpit SHALL render only one anchor workspace at a time.

#### Scenario: Activating anchor changes visible workspace

- **WHEN** `set_active_anchor(Some(anchor))` succeeds
- **THEN** panes whose workspace owner is that anchor are visible
- **AND** panes owned by other anchors and unowned panes are hidden from the
  mounted GTK workspace view
- **AND** focus moves to the active anchor pane

#### Scenario: Prefix `a` cycles anchors

- **WHEN** one or more anchors exist and the user presses `prefix + a`
- **THEN** active anchor advances through anchors in pane-id order

#### Scenario: Sidebar activation sets active anchor

- **WHEN** the user left-clicks an anchor row in the sidebar
- **THEN** that anchor becomes active

### Requirement: Native windows follow anchor ownership by visibility/fronting

Attached native windows SHALL be associated with the owning anchor and switched
through the compositor bridge when the active anchor changes. lmux does not own
their physical display placement.

#### Scenario: Active anchor windows are shown

- **WHEN** an anchor becomes active
- **THEN** native windows registered under that anchor are sent in the `show`
  side of an `ApplyWindowGroupSwitch` command

#### Scenario: Other anchor windows are hidden

- **WHEN** an anchor becomes inactive
- **THEN** native windows registered under that anchor are sent in the `hide`
  side of the group-switch command

#### Scenario: Window placement is preserved

- **WHEN** the compositor handles an anchor switch
- **THEN** lmux does not rewrite display placement, monitor assignment, or saved
  geometry for the native windows

### Requirement: Pause and resume

The cockpit SHALL pause and resume tagged anchor processes by sending process
group signals and updating the in-memory anchor registry.

#### Scenario: Pause sends SIGSTOP

- **WHEN** a tagged live anchor is paused
- **THEN** lmux sends `SIGSTOP` to the pane child's process group using the
  negative-pid path
- **AND** the registry state becomes `paused`

#### Scenario: Resume sends SIGCONT

- **WHEN** a paused anchor is resumed
- **THEN** lmux sends `SIGCONT` to the process group
- **AND** the registry state becomes `live`

### Requirement: Hide and reattach

The cockpit SHALL hide and reattach anchor panes without killing the PTY.

#### Scenario: Hide preserves process

- **WHEN** a tagged anchor is hidden
- **THEN** its pane widget is hidden, its pane and PTY remain alive, and the
  registry state becomes `hidden`

#### Scenario: Reattach preserves workspace filter

- **WHEN** a hidden anchor is reattached
- **THEN** its registry state becomes `live`
- **AND** the widget becomes visible only if the anchor's workspace is active

### Requirement: Remove anchor

The cockpit SHALL let the user remove the anchor tag without killing the pane.

#### Scenario: Remove clears metadata

- **WHEN** an anchor is removed through the sidebar or `anchor.untag`
- **THEN** the anchor set, registry entry, hidden state, active CSS class, and
  workspace ownership records for that anchor are cleared
- **AND** the underlying terminal pane remains alive

#### Scenario: Removing active anchor promotes another

- **WHEN** the removed anchor was active and other anchors remain
- **THEN** lmux activates the next remaining anchor
- **AND** if no anchors remain, active anchor becomes `None`

### Requirement: Sidebar metadata

Anchors SHALL carry editable presentation metadata for the sidebar.

#### Scenario: Rename and regroup

- **WHEN** the user edits name or group in the row popover
- **THEN** the anchor registry updates `name` and `group`
- **AND** the sidebar refreshes

#### Scenario: Drag reorder within group

- **WHEN** the user drags an anchor row onto another row in the same group
- **THEN** lmux rewrites sort keys for that group's anchors in the new order

#### Scenario: Cross-group drag is ignored

- **WHEN** the user drops an anchor row onto a row in another group
- **THEN** the drop is ignored; regrouping is done through the popover

### Requirement: Autodetect matcher library

The repo SHALL provide a pure autodetect matcher for configured rules, even
though cockpit auto-tag wiring is not the primary runtime path today.

#### Scenario: First matching rule wins

- **WHEN** multiple `[[autodetect]]` rules could match a command/env pair
- **THEN** `lmux_anchor::match_rule` returns the first matching rule
