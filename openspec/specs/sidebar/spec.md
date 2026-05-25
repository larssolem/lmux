# sidebar

## Purpose

The sidebar is the always-installed workspace rail. It shows anchor workspaces,
lets the user create a new workspace, add an existing native window to the active
workspace, activate/rename/group/pause/remove anchors, reorder anchors within a
group, show low-res previews, and edit basic settings.

## Requirements

### Requirement: Always-installed sidebar shell

The cockpit SHALL wrap the pane tree in a horizontal `gtk::Paned` containing the
sidebar and the workspace area.

#### Scenario: Sidebar side follows config

- **WHEN** `[sidebar].position` is `left` or `right`
- **THEN** the sidebar is installed on that side of the paned layout

#### Scenario: Collapse button toggles rail

- **WHEN** the user clicks the collapse button
- **THEN** the sidebar switches between configured width and collapsed rail
  width

#### Scenario: Hover expands collapsed rail

- **WHEN** the sidebar is collapsed and the pointer enters it
- **THEN** it temporarily expands until the pointer leaves

### Requirement: Header actions

The sidebar header SHALL expose workspace creation and native window attach.

#### Scenario: New workspace button

- **WHEN** the user clicks the plus button
- **THEN** lmux creates a fresh terminal pane, tags it as a new anchor, and
  makes it active

#### Scenario: Add-window button follows compositor capability

- **WHEN** native attach is supported
- **THEN** the add-window button is enabled and opens the add-window picker
- **AND** otherwise it is disabled with a capability tooltip

### Requirement: Anchor row list

The sidebar SHALL render one row per tagged anchor, grouped and manually sorted.

#### Scenario: Empty state

- **WHEN** there are no anchors
- **THEN** the sidebar shows text telling the user to use `+` or `Ctrl+B a`

#### Scenario: Grouped rows

- **WHEN** anchors have group metadata
- **THEN** rows are grouped by group name, with ungrouped anchors under `No
  group`

#### Scenario: Active row updates without full rebuild

- **WHEN** only the active anchor changes
- **THEN** the sidebar updates active row CSS and active dot through the active
  anchor callback

### Requirement: Row activation and popover actions

Anchor rows SHALL activate workspaces and expose a small popover for metadata
and lifecycle operations.

#### Scenario: Left click activates anchor

- **WHEN** the user left-clicks an anchor row
- **THEN** that anchor becomes active

#### Scenario: Right click or more button opens popover

- **WHEN** the user right-clicks a row, long-presses it, or clicks the more
  button
- **THEN** a popover opens for that anchor

#### Scenario: Popover exposes implemented actions

- **WHEN** the popover opens
- **THEN** it shows the anchor UUID, editable name, editable group, `Pause` or
  `Resume`, `Remove`, and `Apply`

### Requirement: Drag reorder within group

The sidebar SHALL let the user reorder anchors inside their current group.

#### Scenario: Same-group drop rewrites sort keys

- **WHEN** an anchor row is dropped on another row in the same group
- **THEN** lmux rewrites sort keys in the new order and refreshes the list

#### Scenario: Cross-group drop ignored

- **WHEN** an anchor row is dropped on a row from another group
- **THEN** the drop is ignored; the user must use the group field in the
  popover to move groups

### Requirement: Mini previews

The sidebar SHALL optionally show low-resolution previews for visible terminal
anchor panes.

#### Scenario: Preview renders immediately and refreshes

- **WHEN** previews are enabled
- **THEN** each row attempts one immediate thumbnail render and then refreshes
  on the configured interval, clamped to at least 100 ms

#### Scenario: Hidden or inactive workspace preview is blank

- **WHEN** a pane is hidden or not in the active workspace
- **THEN** the preview timer does not render that pane's thumbnail

#### Scenario: Timer self-terminates

- **WHEN** the row or shared state has been dropped
- **THEN** the preview refresh timer stops

### Requirement: Add-window picker

The sidebar SHALL expose native window attach through a modal picker.

#### Scenario: Picker lists native candidates

- **WHEN** the picker opens
- **THEN** it asks the compositor backend for window candidates and renders a
  row for each candidate

#### Scenario: Picker previews windows best-effort

- **WHEN** the compositor can capture a preview for a candidate
- **THEN** the row replaces its fallback initials tile with that preview
- **AND** preview failures only log debug output

#### Scenario: Selecting candidate attaches to active workspace

- **WHEN** the user activates an unattached candidate row
- **THEN** lmux sends the candidate through compositor validation and registers
  the resulting window under the active anchor

### Requirement: Settings dialog

The application menu SHALL expose a settings dialog for the implemented config
surface.

#### Scenario: Settings loads config or defaults

- **WHEN** the settings dialog opens
- **THEN** it loads the config file if present or defaults if missing

#### Scenario: Settings can save prefix and general config

- **WHEN** the user applies valid settings
- **THEN** lmux saves the TOML file, updates the shared prefix cell, and applies
  config to live panes

#### Scenario: Invalid prefix blocks save

- **WHEN** the prefix entry is invalid
- **THEN** the dialog shows an error and does not write the config
