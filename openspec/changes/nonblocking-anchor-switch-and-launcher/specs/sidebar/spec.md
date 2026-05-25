### Requirement: Non-blocking launcher open

The cockpit SHALL open the launcher UI without performing platform application discovery on the GTK main loop. Application discovery MAY continue asynchronously after the launcher is visible.

#### Scenario: Launcher opens from cached entries

- **WHEN** the user presses `Ctrl+B l` and a launcher cache snapshot exists
- **THEN** the launcher window appears using the cached entries
- **AND** the GTK main loop is not blocked by application directory traversal or metadata extraction

#### Scenario: Launcher opens while cache is cold

- **WHEN** the user presses `Ctrl+B l` before application discovery has completed
- **THEN** the launcher window appears immediately in a loading or empty-cache state
- **AND** entries are populated when the background scan completes

#### Scenario: macOS app scanning stays off the UI thread

- **WHEN** macOS application discovery scans `.app` bundles and reads `Info.plist` metadata
- **THEN** directory traversal and `plutil` execution run outside the GTK main loop

### Requirement: Incremental active-anchor sidebar updates

The sidebar SHALL distinguish anchor-list changes from active-anchor changes. Changing only the active anchor MUST update active row styling without rebuilding every sidebar row.

#### Scenario: Active anchor changes

- **WHEN** the active anchor changes from `A` to `B`
- **THEN** the sidebar removes the active styling from `A`
- **AND** applies the active styling to `B`
- **AND** does not rebuild rows whose anchor metadata did not change

#### Scenario: Preview refresh does not block switching

- **WHEN** an anchor switch is in progress or a row belongs to an inactive hidden workspace
- **THEN** sidebar preview refreshes skip expensive pane thumbnail rendering for that row
