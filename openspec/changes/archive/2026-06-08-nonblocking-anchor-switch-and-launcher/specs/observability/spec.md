### Requirement: Latency instrumentation for interactive cockpit gestures

The cockpit SHALL emit structured duration logs for latency-critical interactive operations so regressions in launcher open and anchor switching can be diagnosed from normal trace output.

#### Scenario: Launcher latency is logged

- **WHEN** the launcher is opened or application discovery runs
- **THEN** the cockpit logs operation name, duration, entry count when available, cache state, and whether the work ran on the UI path

#### Scenario: Anchor switch latency is logged

- **WHEN** the active anchor changes
- **THEN** the cockpit logs local switch duration, active anchor id, pane count, satellite window count, and whether a full GTK rebuild was performed

#### Scenario: Platform window operation latency is logged

- **WHEN** macOS reconciliation, helper listing, helper visibility, or compositor grouped switching runs
- **THEN** the cockpit logs operation name, duration, sequence/generation where applicable, window count, and failure count

### Requirement: Regression thresholds for responsiveness

The test suite SHALL include behavioral checks that guard against reintroducing synchronous application discovery or synchronous macOS helper calls into launcher open and local anchor activation paths.

#### Scenario: Launcher open does not scan synchronously

- **WHEN** a test opens the launcher with a fake scanner that would block
- **THEN** the launcher open path returns without waiting for that scanner

#### Scenario: Anchor activation does not call helper synchronously

- **WHEN** a test activates an anchor with a fake macOS helper that would block
- **THEN** local active-anchor state changes without waiting for the helper
