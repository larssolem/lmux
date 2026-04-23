# compositor-control

## Purpose

Abstraction that lets the cockpit talk to the display compositor without leaking compositor-specific types into the rest of the codebase. A `CompositorControl` trait with a `NoopCompositor` (X11 / non-KWin / CI) and a `KwinCompositor` (KWin 6.x D-Bus scripting) implementor keeps the v0.3 wlroots work honest and gives the user an honest, recoverable view of compositor integration state.

## Requirements

### Requirement: Compositor abstraction trait

The cockpit SHALL expose a `CompositorControl` trait whose method surface is compositor-agnostic (no raw D-Bus types, no KWin enums in the trait signatures) and SHALL provide at minimum two implementors: `NoopCompositor` and `KwinCompositor`.

#### Scenario: Trait compiles against both implementors

- **WHEN** both `NoopCompositor` and `KwinCompositor` are built
- **THEN** each implements `CompositorControl` without leaking compositor-specific types into the trait surface; callers depend only on `CompositorControl` and the supporting types `CompositorError`, `CompositorHealth`, `SpawnRequest`, `SpawnHandle`, `CompositorWindowId`, and `Rect`

#### Scenario: Cockpit chooses the implementor at runtime

- **WHEN** the cockpit starts on a Plasma 6 Wayland session with a discoverable `lmux-dock.js`
- **THEN** it instantiates `KwinCompositor`; on any other environment it falls back to `NoopCompositor`, with the decision logged at INFO

### Requirement: NoopCompositor degrades gracefully

`NoopCompositor` SHALL satisfy `ensure_script_loaded` trivially, report `Offline` with an explanatory reason from `health`, accept `spawn_satellite` as a floating-only spawn, and return `Unsupported` for geometry and docking operations — without ever crashing or erroring the caller.

#### Scenario: Health reports offline with reason

- **WHEN** `NoopCompositor::health()` is called
- **THEN** it returns `Offline { reason: <non-empty string describing why> }`

#### Scenario: Floating-only satellite spawn still works

- **WHEN** `NoopCompositor::spawn_satellite(argv, cwd)` is called
- **THEN** it returns `Ok(SpawnHandle)` for a floating satellite; the child runs without docking; `set_geometry`, `detach`, and `attach` subsequently return `Unsupported`

#### Scenario: Script-loaded and reinject are no-ops

- **WHEN** `ensure_script_loaded` or any re-inject operation is invoked on `NoopCompositor`
- **THEN** the call returns `Ok(())` without side effects; no error is raised

### Requirement: KWin script install and idempotent reconfigure

`KwinCompositor` SHALL ensure `lmux-dock.js` is installed into a known KWin scripts path and registered via KWin's scripting D-Bus surface; repeated calls MUST be idempotent.

#### Scenario: First-time install and register

- **WHEN** `KwinCompositor::ensure_script_loaded` is called and no prior registration exists
- **THEN** the script is installed into a discoverable location (env `LMUX_KWIN_SCRIPT`, `$XDG_DATA_HOME/lmux/kwin/`, `/usr/share/lmux/kwin/`, or dev-layout fallback), loaded via `org.kde.kwin.Scripting.loadScript`, and run

#### Scenario: Repeat call is a no-op

- **WHEN** `ensure_script_loaded` is called and the script is already registered and running
- **THEN** no duplicate load occurs; the call returns `Ok(())`

### Requirement: Live health probe over D-Bus

`KwinCompositor::health()` SHALL query KWin's scripting service over D-Bus in real time and map the result to `Online`, `ScriptMissing`, or `Offline { reason }`.

#### Scenario: Online when script is loaded and running

- **WHEN** KWin reports that `lmux-dock` is loaded
- **THEN** `health()` returns `Online`

#### Scenario: ScriptMissing when KWin is reachable but script is absent

- **WHEN** KWin's scripting service is reachable but reports `lmux-dock` is not loaded
- **THEN** `health()` returns `ScriptMissing`

#### Scenario: Offline when D-Bus is unreachable

- **WHEN** `DBUS_SESSION_BUS_ADDRESS` is unset or the scripting service cannot be reached
- **THEN** `health()` returns `Offline { reason: <specific cause> }`

### Requirement: Compositor status published on the bus

The cockpit SHALL publish a `compositor.status` event on the bus whenever the compositor health transitions between `Online`, `ScriptMissing`, and `Offline`.

#### Scenario: Offline transition surfaces within 500 ms

- **WHEN** the compositor transitions to `Offline` (e.g., KWin reload, script eviction)
- **THEN** a `compositor.status { state: "offline", reason: "..." }` event is published on the bus and the sidebar banner appears within 500 ms

#### Scenario: Online transition clears the banner

- **WHEN** the compositor transitions back to `Online`
- **THEN** a `compositor.status { state: "online" }` event is published and the offline banner is cleared

### Requirement: Re-inject from sidebar and CLI

The user SHALL be able to trigger a KWin script re-inject from the sidebar banner action and from the `lmux compositor reinject` CLI; the operation MUST be a single call to `ensure_script_loaded` over D-Bus.

#### Scenario: Sidebar re-inject path

- **WHEN** the user clicks the sidebar's "Re-inject" action while the compositor is offline
- **THEN** the cockpit calls `ensure_script_loaded`; on success, `health()` returns `Online` within 500 ms

#### Scenario: CLI re-inject path

- **WHEN** the user runs `lmux compositor reinject` against a running cockpit
- **THEN** the cockpit performs the same operation and returns the resulting health state; a failure surfaces as a toast naming the underlying `CompositorError` variant

### Requirement: Wayland-only; X11 degrades without error

The cockpit SHALL run its terminal and PTY surfaces on X11 sessions but MUST disable satellite docking with a single clear banner; `lmux open` on X11 still spawns the requested app as a floating window.

#### Scenario: X11 session shows one-time banner

- **WHEN** the cockpit starts inside an X11 session
- **THEN** a one-time sidebar banner explains that satellite docking is disabled; terminal and PTY features continue to work

#### Scenario: `lmux open` on X11 returns a floating satellite

- **WHEN** the user runs `lmux open <app>` inside an X11 session
- **THEN** the cockpit spawns the app as a floating window without attempting KWin correlation and without returning an error
