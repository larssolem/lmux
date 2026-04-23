## ADDED Requirements

### Requirement: Respawn a dead anchor

The cockpit SHALL let the user respawn a dead anchor in place using the same argv, cwd, and environment the anchor was originally started with; respawn is available from the sidebar context menu and from `lmux anchor respawn <uuid>`.

#### Scenario: CLI respawn relaunches in the same pane

- **WHEN** the user runs `lmux anchor respawn <uuid>` against a tagged anchor in the dead state
- **THEN** the cockpit re-forks the same argv + cwd + env in the pane's existing slot, transitions the anchor back to the live state, and preserves the anchor's UUID, metadata, and sort-key

#### Scenario: Sidebar Respawn action matches CLI semantics

- **WHEN** the user clicks "Respawn" in the sidebar context menu on a dead anchor row
- **THEN** the same code path runs as the CLI invocation; no difference in captured metadata or resulting pane state is observable

#### Scenario: Respawn on a live anchor is rejected

- **WHEN** `anchor.respawn` is dispatched against an anchor that is not in the dead state
- **THEN** the bus returns `error.anchor_not_dead` and no process is forked; the existing pane is unaffected

### Requirement: Destructive-hide with scrollback ring

The cockpit SHALL support a destructive hide flavor that terminates the anchor's child process on hide, preserves a capped output ring, and replays the ring to a freshly-spawned process on reattach.

#### Scenario: Destructive hide terminates the child

- **WHEN** the user hides an anchor with the destructive flavor (`lmux anchor hide --destructive <uuid>` or the explicit sidebar action)
- **THEN** the cockpit sends `SIGTERM` to the PTY's process group, enforces the v0.1 grace window + `SIGKILL` ceiling, reaps the child, and records the anchor as destructively-hidden
- **AND** the pane widget remains in place as a placeholder showing the ring's most recent lines

#### Scenario: Scrollback ring is capped

- **WHEN** an anchor is destructively hidden and its child emits output prior to termination
- **THEN** the captured ring is bounded by the smaller of 10 000 lines or 1 MiB; oldest lines are evicted first

#### Scenario: Reattach replays the ring

- **WHEN** the user reattaches a destructively-hidden anchor
- **THEN** a fresh child is forked with the original argv + cwd + env, the ring's tail is replayed into the terminal widget as scrollback before live output begins, and the anchor transitions to live

#### Scenario: Destructive-hide is opt-in per invocation

- **WHEN** the user hides an anchor without the `--destructive` flag or equivalent sidebar action
- **THEN** the soft-hide path is used instead; the child continues running as documented by the existing soft-hide requirement

## MODIFIED Requirements

### Requirement: Soft-hide and reattach

The cockpit SHALL let the user hide a tagged anchor with the default (soft) flavor by detaching its widget from the rendering tree while keeping the PTY alive, and reattach later to the same or a different pane slot. This is the default hide flavor; the destructive-hide flavor is opt-in and defined in its own requirement.

#### Scenario: Soft-hide preserves PTY and scrollback

- **WHEN** the user hides a tagged anchor with the default (soft) flavor
- **THEN** the pane's widget is hidden (visibility toggled) and its anchor state transitions to "hidden (soft)"; the PTY stays open and libghostty continues to accumulate scrollback

#### Scenario: Soft-reattach makes the pane visible again

- **WHEN** the user reattaches a soft-hidden anchor
- **THEN** the anchor's widget becomes visible and its state transitions back to "live"; the accumulated scrollback is available via normal scrolling

#### Scenario: Soft-hide transitions use the non-destructive path

- **WHEN** the cockpit transitions an anchor to hidden via `AnchorRegistry::set_hidden`
- **THEN** the pane binding is preserved and no `SIGTERM` is sent; the destructive flavor is defined separately
