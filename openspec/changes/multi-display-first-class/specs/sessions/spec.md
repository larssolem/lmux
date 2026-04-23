## ADDED Requirements

### Requirement: Per-pane display placement

Every `PaneRecord` and `SatelliteRecord` in a persisted session SHALL optionally carry a stable `display_id` and a `rect_on_display` (display-local coordinates); on restore, the cockpit places the pane or satellite on the recorded display when present.

#### Scenario: Pane records its display on save

- **WHEN** the cockpit saves a session whose pane `P` is currently placed on display `D`
- **THEN** the TOML payload for `P` contains `display_id = "<D's id>"` and `rect_on_display = <P's rect relative to D>`

#### Scenario: Pane restores to its recorded display

- **WHEN** a session is restored and pane `P` has `display_id = "<id>"` matching a currently-present display
- **THEN** the cockpit places `P` on that display, using `rect_on_display` translated to absolute coordinates via the display's current rect and scale

#### Scenario: Absent fields parse to None

- **WHEN** a v0.2 session TOML without `display_id` is loaded
- **THEN** both `display_id` and `rect_on_display` parse as `None`; the cockpit places the pane using the v0.2 behaviour (primary display) and writes back fields on the first save

### Requirement: Display hotplug migration policy

The cockpit SHALL apply a deterministic migration policy when a display is removed, added, or reconfigured; the policy SHALL be configurable via `[display].on_removed` with default `"migrate_to_primary"`.

#### Scenario: Display removed migrates panes to primary

- **WHEN** a display carrying N panes is disconnected and `[display].on_removed = "migrate_to_primary"`
- **THEN** all N panes migrate to the current primary display, their `rect_on_display` is clamped to the primary's rect, and a single toast reports "Display <connector> disconnected — moved N panes to <primary connector>"

#### Scenario: Display added does not proactively move panes

- **WHEN** a new display is connected while the cockpit is running
- **THEN** no pane is moved; a one-time-per-session toast suggests "Use Move to display… to place panes on <connector>"

#### Scenario: Display reconfigured clamps rects in place

- **WHEN** a display changes resolution or scale
- **THEN** panes on that display remain on it; `rect_on_display` is clamped to the new rect; docked satellites on the display receive a `set_geometry` call with the clamped absolute coordinates

#### Scenario: Hide policy keeps panes out of view

- **WHEN** a display with N panes is disconnected and `[display].on_removed = "hide"`
- **THEN** the panes are soft-hidden (their PTYs continue running, the sidebar marks them "awaiting display"); reconnecting the display restores them in place

## MODIFIED Requirements

### Requirement: Atomic TOML persistence

The cockpit SHALL persist every session state change atomically under `$XDG_STATE_HOME/lmux/sessions/` using stage → `fsync` → rename, with file mode `0600`; the persisted schema MUST include the per-pane and per-satellite display placement fields (`display_id`, `rect_on_display`) when set, and MUST accept session files written without those fields as a valid v0.2 format.

#### Scenario: Atomic write survives power loss mid-operation

- **WHEN** the cockpit saves a session and a simulated crash occurs between stage and rename
- **THEN** the previously committed file remains intact and readable; on next launch the cockpit opens the last good version

#### Scenario: Session files are user-private

- **WHEN** a session file is created under `$XDG_STATE_HOME/lmux/sessions/<name>.toml`
- **THEN** its mode is `0600` and its index entry in `sessions/index.toml` is updated in the same atomic manner

#### Scenario: Display fields round-trip losslessly

- **WHEN** a session is saved with `display_id` and `rect_on_display` set and loaded back
- **THEN** both fields are present and structurally equal to the saved values; reordering of pane records does not affect field presence
