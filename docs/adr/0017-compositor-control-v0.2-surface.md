# ADR-0017: CompositorControl - current method surface

- Status: Accepted
- Date: 2026-04-22
- Updated: 2026-05-24
- Deciders: Lars
- Depends on: ADR-0004, ADR-0005, ADR-0003 as historical spike input
- Blocks: native-window attach and anchor-driven window visibility

## Context

ADR-0004 established a compositor abstraction so the cockpit can work against a
trait while each desktop backend handles its own platform details.

The old v0.2 trait was built around spawn-and-track geometry ownership:
`spawn_satellite`, `set_geometry`, `detach`, and `attach`. That no longer
describes the product surface by itself. The current native-window workflow is
explicit attach of already-open windows, followed by anchor-driven visibility
and fronting. macOS also needs exact per-window identity because one app process
can own several windows that belong to different anchors.

The trait therefore has to support two eras at once:

- legacy launch/spawn hooks that still exist in code;
- current native-window operations: list, preview, attach, hide/show, raise,
  and grouped anchor switches.

## Decision

`CompositorControl` is the cockpit-facing trait for host-compositor windows.
Implementations are async, shareable behind `Arc<dyn CompositorControl>`, and
must keep terminal/session behavior alive even when native window control is
unsupported.

The current method surface is:

```rust
#[async_trait]
pub trait CompositorControl: Send + Sync {
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError>;
    async fn health(&self) -> Health;

    fn window_control_capabilities(&self) -> WindowControlCapabilities;

    async fn spawn_satellite(
        &self,
        argv: &[String],
        cwd: Option<&str>,
    ) -> Result<Uuid, CompositorError>;

    async fn set_geometry(&self, window: &WindowId, rect: Rect)
        -> Result<(), CompositorError>;
    async fn detach(&self, window: &WindowId) -> Result<(), CompositorError>;
    async fn attach(&self, window: &WindowId) -> Result<(), CompositorError>;

    async fn list_windows(&self) -> Result<Vec<WindowCandidate>, CompositorError>;
    async fn window_preview(
        &self,
        candidate: &WindowCandidate,
        max_width: u32,
        max_height: u32,
    ) -> Result<Option<WindowPreview>, CompositorError>;
    async fn attach_window(
        &self,
        candidate: &WindowCandidate,
    ) -> Result<SatelliteWindowId, CompositorError>;
    async fn raise_window(&self, window: &SatelliteWindowId)
        -> Result<(), CompositorError>;
    async fn set_window_visible_by_pid(
        &self,
        pid: u32,
        visible: bool,
    ) -> Result<(), CompositorError>;
    async fn set_window_visible(
        &self,
        window: &SatelliteWindowId,
        visible: bool,
    ) -> Result<(), CompositorError>;
    async fn apply_window_group_switch(
        &self,
        switch: WindowGroupSwitch,
    ) -> Result<Vec<WindowOpResult>, CompositorError>;
    async fn apply_window_group_switch_latest(
        &self,
        switch: WindowGroupSwitch,
        sequence: u64,
        latest_sequence: Arc<AtomicU64>,
    ) -> Result<Vec<WindowOpResult>, CompositorError>;
}
```

The native-window identity is `SatelliteWindowId`: backend, optional request id,
optional PID, backend window id, optional bundle id, and optional title.

`WindowControlCapabilities` reports whether a backend can list windows, attach
windows, set visibility, and raise windows. Unsupported backends return false
for unsupported capabilities rather than disabling the terminal cockpit.

`WindowGroupSwitch` is the current anchor-switch primitive: hide windows from
inactive anchors, show windows from the incoming anchor, and apply the focus
policy. Backends may batch this; the default implementation performs per-window
operations and returns per-window results.

## Current implementors

- **KWin**: primary Linux/KDE backend. Uses D-Bus and the installed KWin script
  for health, listing, attach, preview, visibility, and raise where supported.
- **X11**: best-effort backend using X11 tooling and window ids.
- **macOS**: uses Accessibility/CoreGraphics identity for exact per-window
  attach and grouped visibility/fronting.
- **Noop**: fallback for unsupported desktops and tests. Keeps lmux usable while
  reporting native-window operations as unsupported.

## Alternatives considered

- **Keep only the six-method spawn/dock trait.** Rejected by implementation
  reality: current UI and CLI need listing, exact attach, visibility, and raise.
- **Make lmux own monitor geometry.** Rejected for the current product. Users
  place native windows; lmux controls context membership and fronting.
- **One backend-specific escape hatch per platform.** Rejected because it would
  leak platform logic into sidebar, bus, sessions, and anchor switching.

## Consequences

- The trait is larger than the original v0.2 surface, but it matches the actual
  behavior users exercise.
- Backend capability checks become user-facing: the sidebar can disable attach
  or show diagnostics when a compositor cannot list or attach native windows.
- Anchor switching can be implemented uniformly across KDE, X11, macOS, and
  Noop by applying a `WindowGroupSwitch`.
- The old `spawn_satellite`/geometry methods remain for compatibility and
  legacy paths, but they are not the reliable native-window attach model.

## Follow-up

- Keep `openspec/specs/compositor-control/` as the behavior contract.
- Remove or further isolate legacy spawn/geometry calls when no current feature
  depends on them.
