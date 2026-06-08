## ADDED Requirements

### Requirement: macOS managed app profiles

lmux SHALL support persistent managed app profiles for macOS-launched GUI satellites so user setup is preserved between lmux sessions while remaining isolated from the user's normal app profile.

#### Scenario: Chromium uses a persistent lmux profile

WHEN lmux launches a Chromium-family app on macOS
THEN the app SHALL receive a persistent lmux-managed user data directory
AND the directory SHALL NOT include the launch request id
AND repeated launches of the same app SHALL use the same lmux-managed profile.

#### Scenario: JetBrains uses persistent lmux paths

WHEN lmux launches a JetBrains-family app on macOS
THEN the app SHALL receive persistent lmux-managed config, system, plugin, and log paths
AND repeated launches of the same app SHALL reuse those paths.

### Requirement: macOS ownership safety

lmux SHALL NOT control macOS windows unless it can prove they are lmux-owned.

#### Scenario: ambiguous launch is unmanaged

WHEN a macOS launch creates windows that cannot be correlated to a lmux-owned process/profile or stable window id
THEN lmux SHALL leave the windows unmanaged
AND lmux SHALL NOT hide, minimize, or focus those windows as part of anchor switching.

#### Scenario: existing user windows are not controlled

WHEN a user has existing app windows outside lmux-managed profiles
AND lmux launches the same app through a managed profile
THEN anchor switching SHALL NOT hide, minimize, or focus the existing user windows.

### Requirement: macOS window identity supports multiple anchors per app

lmux SHALL track macOS satellite ownership per stable window identity, not per bundle id or process alone.

#### Scenario: two Chrome windows belong to different anchors

WHEN one lmux-managed Chrome window is assigned to anchor A
AND another lmux-managed Chrome window is assigned to anchor B
AND the user switches to anchor A
THEN only the Chrome window assigned to anchor A SHALL be restored
AND the Chrome window assigned to anchor B SHALL be minimized.

#### Scenario: transient candidate is replaced by primary window

WHEN a lmux launch first creates a transient setup or project chooser window
AND later creates a primary work window for the same launch request
THEN lmux SHALL replace the candidate ownership with the primary window ownership
AND subsequent anchor switches SHALL operate on the primary window.
