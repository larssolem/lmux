## ADDED Requirements

### Requirement: Terminal transcript ringbuffer

Each PTY-backed terminal pane SHALL maintain a bounded live transcript ringbuffer.

#### Scenario: PTY output is recorded before render

- **WHEN** the PTY reader receives bytes for a terminal pane
- **THEN** the cockpit appends decoded transcript entries with monotonically increasing sequence numbers before feeding the terminal renderer

#### Scenario: Tail returns recent output

- **WHEN** a client requests the last N transcript lines for a pane
- **THEN** the cockpit returns no more than N recent lines with sequence metadata

#### Scenario: Capture since sequence

- **WHEN** a client requests transcript output since a known sequence number
- **THEN** the cockpit returns entries newer than that sequence when still retained
- **AND** reports truncation when older entries have fallen out of the ringbuffer

#### Scenario: Satellite panes have no terminal transcript

- **WHEN** a transcript request targets a GUI satellite pane
- **THEN** the cockpit returns a typed error explaining that only PTY-backed terminal panes have transcript output

### Requirement: Anchor-local terminal tab stacks

The cockpit SHALL support tab stacks for terminal panes inside an anchor workspace.

#### Scenario: New pane as tab

- **WHEN** a user or authorized client creates a terminal pane with tab placement in an anchor workspace
- **THEN** the cockpit adds the pane as a tab in that anchor's terminal tab stack
- **AND** the active tab selection is updated according to the request

#### Scenario: Existing split behavior remains available

- **WHEN** a user creates a split right or split down
- **THEN** the cockpit keeps using the existing recursive split behavior inside the current tab

#### Scenario: Tab can contain split layout

- **WHEN** the active tab has a split layout
- **THEN** switching away and back to that tab restores the same split tree and focused pane where possible

#### Scenario: Anchor rail does not list terminal tabs

- **WHEN** multiple terminal tabs exist under an anchor
- **THEN** the anchor rail continues to show one row for that anchor rather than one row per terminal tab

### Requirement: Terminal pane titles

Terminal panes SHALL have visible titles independent of anchor labels.

#### Scenario: Default title from command

- **WHEN** a pane has no user-set or agent-set title
- **THEN** the cockpit derives a best-effort title from the running command or fallback pane label

#### Scenario: Title visible in tab strip

- **WHEN** an anchor has multiple terminal tabs
- **THEN** the tab strip shows each tab's title and indicates the active tab
