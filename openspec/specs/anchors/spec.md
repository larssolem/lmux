# anchors

## Purpose

An anchor is a pane the user has marked as long-running (dev server, AI agent, log tail). Anchors get an explicit lifecycle — tag/untag, pause (SIGSTOP), resume (SIGCONT), soft-hide (widget detach, PTY preserved), reattach — plus automatic tagging by command-pattern match and a compact workspace model where exactly one anchor is active at a time. Tab-edge glow colours mirror anchor and satellite state.

## Requirements

### Requirement: Tag and untag a pane as an anchor

The cockpit SHALL let the user tag any live pane as an anchor and later remove the anchor tag without affecting the underlying PTY or process.

#### Scenario: Tag the focused pane

- **WHEN** the user runs `lmux anchor tag <uuid>` against a running cockpit (or presses `prefix + a` with a fresh pane focused)
- **THEN** the pane's anchor metadata is set, the sidebar shows the anchor indicator, and subsequent `anchor.*` operations targeted at the pane's UUID are accepted

#### Scenario: Untag preserves the PTY

- **WHEN** the user runs `lmux anchor untag <uuid>` against a tagged pane
- **THEN** the anchor metadata is cleared, the sidebar indicator is removed, and the PTY and process continue running unaffected

### Requirement: Per-pane UUID identity

The cockpit SHALL assign a UUID to every pane it creates and SHALL maintain a reverse `pane_for_uuid` lookup so bus kinds can target a pane independently of its ephemeral `PaneId`.

#### Scenario: UUID survives layout changes

- **WHEN** a pane is split, moved, or its containing tab is reorganized
- **THEN** the pane's UUID is preserved across every insert, remove, drain, and rehydrate operation; `pane_for_uuid` continues to resolve to the same logical pane

### Requirement: Pause and resume via process-group signals

The cockpit SHALL pause a tagged anchor by sending `SIGSTOP` to the PTY's process group (with a single-PID fallback) and resume it by sending `SIGCONT`; the sidebar state glyph MUST reflect the transition within 100 ms.

#### Scenario: Pause halts the process group

- **WHEN** the user pauses a tagged anchor via `lmux anchor pause <uuid>` or the sidebar
- **THEN** the cockpit calls `kill(-pgid, SIGSTOP)` on the PTY's process group; the sidebar glyph transitions to "paused" within 100 ms

#### Scenario: Resume unblocks the process group

- **WHEN** the user resumes a paused anchor
- **THEN** the cockpit sends `SIGCONT` to the process group; the glyph returns to "live" and the pane resumes rendering output

#### Scenario: Pause is idempotent

- **WHEN** the user pauses an already-paused anchor
- **THEN** the operation is a no-op; no duplicate signal is sent and no error is returned

### Requirement: Soft-hide and reattach

The cockpit SHALL let the user hide a tagged anchor by detaching its widget from the rendering tree while keeping the PTY alive, and reattach later to the same or a different pane slot.

#### Scenario: Hide preserves PTY and scrollback

- **WHEN** the user hides a tagged anchor
- **THEN** the pane's widget is hidden (visibility toggled) and its anchor state transitions to "hidden"; the PTY stays open and libghostty continues to accumulate scrollback

#### Scenario: Reattach makes the pane visible again

- **WHEN** the user reattaches a hidden anchor
- **THEN** the anchor's widget becomes visible and its state transitions back to "live"; the accumulated scrollback is available via normal scrolling

#### Scenario: Hide transitions use the non-destructive path

- **WHEN** the cockpit transitions an anchor to hidden via `AnchorRegistry::set_hidden`
- **THEN** the pane binding is preserved; the destructive `hide` flavor (kill + respawn) is not used in this path

### Requirement: Auto-detect anchors by command pattern

The cockpit SHALL auto-tag new panes as anchors when their spawn argv matches any built-in or user-configured pattern prefix, within 1 second of first output.

#### Scenario: Built-in pattern triggers auto-tag

- **WHEN** a pane spawns a command whose argv starts with one of the built-in patterns (`npm run dev`, `pnpm dev`, `cargo watch`, `claude code`)
- **THEN** the pane auto-tags as an anchor within 1 second of first output, and the sidebar shows an "auto-detected" icon variant distinct from user-tagged anchors

#### Scenario: User-extensible patterns take effect after reload

- **WHEN** the user adds `"my dev cmd"` to `[anchors].auto_detect_patterns` in config and triggers a reload
- **THEN** subsequent panes spawning that command auto-tag; the pattern list is the union of built-in and user patterns, deduplicated

#### Scenario: Removing a pattern does not untag existing anchors

- **WHEN** the user removes a pattern from config and reloads
- **THEN** future panes matching that pattern are not auto-tagged; already-tagged panes remain anchors

### Requirement: Crash capture surfaces a dead anchor

When a tagged anchor's process exits unexpectedly the cockpit SHALL capture the exit status and recent output tail, transition the anchor to the "dead" state, and make the captured tail viewable from the sidebar.

#### Scenario: Exit is detected and the anchor marked dead

- **WHEN** a tagged anchor's child process exits (any signal or non-zero status)
- **THEN** the cockpit detects the exit via `waitpid`, records the exit status and the last 200 lines of output, and transitions the sidebar row to the "dead" state

#### Scenario: Captured tail is viewable

- **WHEN** the user opens the dead anchor's row in the sidebar
- **THEN** the captured tail (exit status + last 200 lines) is viewable and copyable; there is no modal interruption at the time of death

### Requirement: Per-anchor workspace ownership

The cockpit SHALL model a compact workspace where exactly one anchor is active at a time; non-active anchors' panes and their satellites are hidden, and `prefix + a` cycles between anchors.

#### Scenario: Switching the active anchor hides the others

- **WHEN** the user switches the active anchor from `A` to `B`
- **THEN** `B`'s panes become visible, `A`'s panes become hidden, and any satellites owned by `A`'s panes are minimized while `B`'s are shown

#### Scenario: `prefix + a` cycles anchors

- **WHEN** the user presses `prefix + a` with at least one anchor present
- **THEN** the active anchor advances to the next anchor in sort order; with no anchors present the binding creates a new one

### Requirement: Tab-edge state glow colours

The tab-edge glow SHALL convey non-focus pane state using distinct colours: blue for satellite-owned, orange for anchor-paused, muted orange for anchor-hidden, red for anchor-dead; focus ring composes with state glow.

#### Scenario: Paused anchor shows orange glow

- **WHEN** an anchor enters the paused state
- **THEN** its pane renders an orange tab-edge glow in addition to any focus ring

#### Scenario: Dead anchor shows red glow

- **WHEN** an anchor enters the dead state
- **THEN** its pane renders a red tab-edge glow until the anchor is respawned or dismissed

#### Scenario: Focused satellite-owning pane composes both glows

- **WHEN** a pane that owns a docked satellite holds focus
- **THEN** the pane renders both the focus ring and the blue satellite glow; the two are visually distinguishable

### Requirement: Anchor survival across session-window close

Hidden tagged anchors SHALL survive a session-window close so long as the cockpit process continues running; closing the cockpit itself kills hidden anchors per the v0.1 shutdown contract.

#### Scenario: Hidden anchor survives session close

- **WHEN** the user closes a session window that contains a tagged, hidden anchor
- **THEN** the PTY stays alive (the cockpit retains the master fd); reopening the session reattaches the anchor

#### Scenario: Cockpit shutdown reaps hidden anchors

- **WHEN** the cockpit itself shuts down
- **THEN** hidden anchors are terminated along with all other children within the 700 ms shutdown budget
