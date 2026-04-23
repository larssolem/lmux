# ADR-0017: CompositorControl — v0.2 method surface

- Status: Accepted
- Date: 2026-04-22
- Deciders: Lars
- Blocks: v0.2 (implementation), v0.3 (wlroots backend must implement this trait)
- Depends on: ADR-0004 (trait-first compositor abstraction), ADR-0005 (KWin MVP), ADR-0003 (Path A spawn-and-track)

## Context

ADR-0004 established `CompositorControl` as a trait: the cockpit programs against the trait, and each compositor (KWin now, Hyprland later) ships its own implementor. The ADR deliberately did not pin the method set because v0.1 didn't yet know which operations were load-bearing.

v0.2 implements the satellite subsystem. The trait's method set must now be frozen because:

- `lmux-satellite` depends on the trait object; every method gap becomes an awkward backend-specific escape hatch.
- The v0.3 wlroots backend is planned as a port of the same trait — if a method only KWin can implement sneaks in at v0.2, the v0.3 spike becomes a partial rewrite.
- Testing needs a `NoopCompositor` (for X11 / non-KWin Wayland / CI without a display) — the trait must be small enough that a no-op version is genuinely useful, not a pile of `todo!()`.

FINDINGS.md from the Phase-1 KWin IPC spike gives us the empirical set: the cockpit needs script injection, health probe, spawn+track, geometry update, detach, and reattach. That's the v0.2 surface.

## Decision

```rust
#[async_trait]
pub trait CompositorControl: Send + Sync + 'static {
    /// Inject or refresh the lmux KWin script. Called on startup,
    /// after re-inject requests (FR50), and after config reload if
    /// the script asset path changed. Idempotent.
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError>;

    /// Health probe. Returns Online unless a prior call observed
    /// offline state (script missing, DBus gone, etc.). Powers
    /// FR51 and the sidebar banner.
    async fn health(&self) -> CompositorHealth;

    /// Spawn a satellite via the sandbox-or-direct launcher and
    /// begin Path-A correlation. Returns when the launcher has
    /// fork/exec'd; correlation completion (or timeout) is
    /// delivered as `satellite.map` / `satellite.status` events
    /// on the bus.
    async fn spawn_satellite(&self, req: SpawnRequest) -> Result<SpawnHandle, CompositorError>;

    /// Move/resize a previously-placed satellite. Must be
    /// idempotent — the cockpit calls this on every pane layout
    /// tick, most of which are no-ops.
    async fn set_geometry(
        &self,
        window: CompositorWindowId,
        rect: Rect,
    ) -> Result<(), CompositorError>;

    /// Release a satellite back to free-floating state (FR33).
    async fn detach(&self, window: CompositorWindowId) -> Result<(), CompositorError>;

    /// Re-dock a previously detached satellite at the given rect (FR34).
    async fn attach(
        &self,
        window: CompositorWindowId,
        rect: Rect,
    ) -> Result<(), CompositorError>;
}
```

### Supporting types (in `lmux-compositor`)

- `CompositorError` — closed enum: `ScriptInstallFailed`, `DbusUnavailable`, `WindowNotFound`, `Unsupported`, `Timeout`, `Io(std::io::Error)`.
- `CompositorHealth` — `{ state: Online | Offline, reason: Option<String>, last_checked: Instant }`.
- `SpawnRequest` — `{ wm_class_tag: String, argv: Vec<OsString>, env: Vec<(OsString, OsString)>, sandbox: SandboxPolicy, target_rect: Rect }`.
- `SpawnHandle` — `{ pid: Pid, request_id: Uuid }`. The cockpit correlates `satellite.map` events to the request_id via the wm-class tag.
- `CompositorWindowId` — opaque `NewType(String)` wrapping the backend-specific window id (KWin window uuid in v0.2).
- `Rect` — `{ x: i32, y: i32, w: u32, h: u32 }` in screen coordinates.

### v0.2 implementors

- **`KwinCompositor`** (real). Uses DBus + the injected `lmux-dock.js` KWin script + the bus to drive correlation. Implements every method.
- **`NoopCompositor`** (fallback). Used on X11, non-KWin Wayland compositors, and in tests/CI without a display.
  - `ensure_script_loaded` → `Ok(())` (no-op).
  - `health` → `Offline { reason: "No supported compositor detected" }`.
  - `spawn_satellite` → succeeds, returns `SpawnHandle` marked floating-only; no correlation. Honors FR43.
  - `set_geometry` / `detach` / `attach` → `Err(CompositorError::Unsupported)`.

## Alternatives considered

- **Add `list_windows()` / `raise_window()` / `focus_window()` now.** Rejected: not needed in v0.2; if we add them speculatively, the v0.3 wlroots spike inherits them. Add when a concrete FR demands.
- **Split into two traits (`CompositorLifecycle` and `CompositorWindowing`).** Rejected: over-abstraction for a 6-method surface. Revisit if v0.3 adds 10+ methods.
- **Model spawn as fire-and-forget + bus-only correlation** (no `SpawnHandle` return). Rejected: the caller needs the request_id *before* the bus event arrives, to wire up its per-pane state.
- **Sync trait with blocking IO.** Rejected: KWin DBus calls must be async; sync wrapper would force a separate thread for each call.
- **Let backends define their own error enum.** Rejected: a closed common enum forces backends to map their errors to stable, user-surface-able variants (good for the UI; good for v0.3 portability).

## Consequences

- **+** `lmux-satellite` holds `Arc<dyn CompositorControl>` and programs against the trait; the `KwinCompositor` / `NoopCompositor` swap is a one-line factory call.
- **+** `NoopCompositor` gives us a clean X11 / no-display story (FR43) without branching inside satellite logic.
- **+** The small method count (six) means the v0.3 wlroots port is a focused task, not a rewrite.
- **−** Every new capability (window focus, workspace placement, multi-monitor hints) adds one trait method and requires every backend to implement or return `Unsupported`. Acceptable cost; if this trait grows past ~15 methods, revisit the split.
- **−** Closed `CompositorError` means any genuinely backend-specific error needs a mapping decision at the boundary. Mitigation: `Io` variant is a safety valve; `Unsupported` carries an optional message.

## Follow-up

- Implement the trait + both impls in `lmux-compositor` (step 8 of the v0.2 implementation sequence).
- Wire `KwinCompositor` to ADR-0016 events (`satellite.map` in, `satellite.geometry` out).
- v0.3: `HyprlandCompositor` implements this exact trait; any method it can't support reveals a gap we'd rather find now than later.
