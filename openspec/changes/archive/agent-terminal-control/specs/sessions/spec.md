## ADDED Requirements

### Requirement: Terminal tab stack persistence

Session snapshots SHALL preserve terminal tab stack structure for PTY-backed panes.

#### Scenario: Save tab stack layout

- **WHEN** a session containing terminal tab stacks is saved
- **THEN** the snapshot records each tab's layout, active tab, pane ids, cwd map, and pane titles

#### Scenario: Restore tab stack layout

- **WHEN** a session containing terminal tab stacks is restored
- **THEN** lmux recreates the tab stacks and restores the active tab selection where possible

#### Scenario: Legacy layout restores as one tab

- **WHEN** a legacy session snapshot contains only a recursive split layout
- **THEN** lmux restores it as a single terminal tab containing that layout

### Requirement: Pane title persistence

Session snapshots SHALL preserve user-pinned pane titles.

#### Scenario: Save user-pinned title

- **WHEN** a pane has a user-pinned title during session save
- **THEN** the snapshot records that title and provenance

#### Scenario: Agent automatic titles are not treated as user-pinned

- **WHEN** a pane title was only agent-set automatically
- **THEN** restore may use the saved title as a display hint
- **AND** it does not mark the title as user-pinned
