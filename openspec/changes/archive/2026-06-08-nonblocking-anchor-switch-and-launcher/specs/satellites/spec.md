### Requirement: Non-blocking anchor-owned satellite switching

Switching the active anchor SHALL update cockpit-local state immediately and SHALL perform satellite window reconciliation asynchronously. Platform window-control latency MUST NOT block the GTK main loop.

#### Scenario: Local anchor switch completes before reconciliation

- **WHEN** the user switches from anchor `A` to anchor `B`
- **THEN** the cockpit marks `B` active, updates local focus, and displays `B`'s workspace before macOS or compositor reconciliation completes
- **AND** satellite restore/minimize work is queued asynchronously

#### Scenario: Reconciliation result is stale

- **WHEN** reconciliation starts for anchor `B`
- **AND** the user switches to anchor `C` before that reconciliation finishes
- **THEN** the `B` reconciliation result is ignored or limited to non-focus bookkeeping
- **AND** it MUST NOT raise or focus `B`'s satellite windows after `C` is active

#### Scenario: One satellite window fails

- **WHEN** an anchor switch needs to restore multiple satellite windows
- **AND** one window operation fails or times out
- **THEN** the cockpit records the per-window failure
- **AND** continues applying other non-stale window operations
- **AND** remains responsive to further input

### Requirement: Fast workspace switching

The cockpit SHOULD switch active anchor workspaces without tearing down and rebuilding every pane widget when the layout structure has not changed.

#### Scenario: Active workspace changes without structural edit

- **WHEN** the user switches anchors without creating, closing, splitting, or rearranging panes
- **THEN** the cockpit uses a fast workspace switch path
- **AND** full pane-tree rebuild is not required for the switch

#### Scenario: Structural edit still rebuilds as needed

- **WHEN** the user splits, closes, creates, restores, or rearranges panes
- **THEN** the cockpit MAY rebuild the GTK widget tree to reflect the new layout
