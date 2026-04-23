## ADDED Requirements

### Requirement: Display-aware satellite placement

`spawn_satellite` SHALL accept an optional target display; `lmux open --display <id>` SHALL route the spawn to that display; restored satellites SHALL dock into their recorded display using `rect_on_display` translated to absolute coordinates via the display's current rect and scale.

#### Scenario: CLI --display targets the named display

- **WHEN** the user runs `lmux open --display ab12cd34:HDMI-A-1 firefox`
- **THEN** the cockpit resolves the id, spawns the satellite, and on correlation issues `set_geometry` with a rect positioned on that display; the satellite does not appear on any other display

#### Scenario: Unknown --display refuses the spawn

- **WHEN** the user runs `lmux open --display <nonexistent-id> <cmd>`
- **THEN** the cockpit returns `error.display_not_found` with the supplied id and does not spawn the satellite

#### Scenario: Restored satellite honors its recorded display

- **WHEN** a session containing a satellite record with `display_id = "<id>"` is restored and that display is present
- **THEN** the spawned satellite is docked into the display's rect at `rect_on_display` translated through the current layout

#### Scenario: Display reconfigure redocks satellites on that display

- **WHEN** a display's rect or scale changes
- **THEN** every docked satellite on that display receives exactly one `set_geometry` call with the new absolute rect within one frame; satellites on other displays are unaffected

#### Scenario: Default target is the cockpit's display

- **WHEN** the user runs `lmux open <cmd>` without `--display`
- **THEN** the satellite is placed on the same display as the cockpit window, matching v0.2 behaviour
