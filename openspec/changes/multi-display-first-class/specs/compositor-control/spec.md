## ADDED Requirements

### Requirement: Display enumeration and hotplug events

The `CompositorControl` trait SHALL expose a `displays()` method returning the current list of `Display` values with stable `DisplayId`s (EDID-hash prefix + connector name), and SHALL publish `compositor.displays` bus events on every add, remove, or reconfigure; backends MUST guarantee the `DisplayId` survives unplug/replug of the same physical monitor.

#### Scenario: Enumeration returns every connected display

- **WHEN** `compositor.displays()` is called on a machine with two connected monitors
- **THEN** the returned vector has exactly two entries, each with `rect`, `scale`, `primary`, `connector`, and a stable `DisplayId`; at most one entry has `primary = true`

#### Scenario: DisplayId is stable across replug

- **WHEN** a monitor is unplugged and re-plugged into any connector on the same machine
- **THEN** the `DisplayId` returned in the subsequent enumeration matches the id from before the unplug

#### Scenario: Hotplug publishes a bus event within 500 ms

- **WHEN** a display is connected or disconnected on a live cockpit
- **THEN** a `compositor.displays { displays, changed: Added|Removed|Reconfigured }` event is published on the bus within 500 ms of the compositor's own event

#### Scenario: Noop returns one synthetic display

- **WHEN** `NoopCompositor::displays()` is called
- **THEN** it returns a single `Display` at `(0,0, 1920x1080)` with `scale = 1.0` and `primary = true`; `DisplayId` is `"noop:default"`

#### Scenario: Enumeration timeout falls back to synthetic

- **WHEN** the compositor does not reply to the initial enumeration call within 500 ms at cockpit startup
- **THEN** the cockpit proceeds with a single synthetic display entry, logs a warning, and replaces it once a real enumeration event arrives
