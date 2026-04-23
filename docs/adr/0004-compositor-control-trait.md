# ADR-0004: CompositorControl trait — identify/control separation

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.2

## Context

Path A (ADR-0003) means lmux talks to compositors via IPC. KDE and wlroots expose *different* primitives:

- **KDE/KWin:** does not advertise `wlr-foreign-toplevel-management-v1`. Instead: `ext-foreign-toplevel-list-v1` (identify, title/app_id only — no PID, no control) + `org.kde.KWin.Scripting` over D-Bus (full control, KWin-specific). Even on KDE, **identify** and **control** are on different transports.
- **Wlroots (Hyprland/Sway):** `wlr-foreign-toplevel-management-v1` exposes identify *and* control on one protocol, augmented by `hyprctl` / `swaymsg` socket IPC for dispatchers.

Spike 2 Phase 1 also discovered asymmetric identifiers (PID vs app_id+seq) and asynchronous geometry semantics (Wayland configure-ack is not synchronous with the request).

## Decision

Define a single trait in `compositor/src/lib.rs`:

```rust
trait CompositorControl {
    fn enumerate(&self) -> Result<Vec<ToplevelHandle>>;
    fn focus(&self, h: &ToplevelHandle) -> Result<()>;
    fn set_geometry(&self, h: &ToplevelHandle, g: Geometry) -> Result<Geometry>; // returns REQUESTED
    fn subscribe_lifecycle(&self) -> Stream<LifecycleEvent>;
}
```

Key shape rules:

1. **`ToplevelHandle` is an opaque `String`.** Backends encode whatever correlation strategy they use: PID on KDE, `app_id+seq` on wlroots. Callers must never parse it.
2. **`set_geometry` returns the requested geometry, not the committed one.** Configure-ack is async. Observation of the actual committed rect is a separate concern surfaced via `subscribe_lifecycle` (Geometry-changed event, v0.3 if needed).
3. **Identify and control may be backed by different protocols.** Backends are free to use one protocol for `enumerate` and another for `focus`/`set_geometry`. On KDE this is the rule, not the exception.
4. **Lifecycle synthesis is a backend concern.** On KDE, `LifecycleEvent::Closed` is synthesized from child-wait + long-lived KWin script (see ADR-0011). On wlroots, it comes from the Wayland protocol directly.

## Alternatives considered

- **Force a single-transport abstraction.** Rejected: KDE's reality breaks it. We'd have to re-wrap one side through a shim and lie about where events come from.
- **Expose typed handles (`KdeToplevelHandle` / `WlrToplevelHandle`).** Rejected: leaks backend into consumer code; prevents generic layout logic.
- **Block on commit-observed geometry.** Rejected: would block the UI thread on a round-trip that the compositor is free to modify or reject.

## Consequences

- **+** The hardest design signal from the spike (identify≠control) is encoded in the trait itself.
- **+** Backends are free to evolve independently — new compositors slot in without reshaping the trait.
- **+** Async/commit semantics are honest; consumers that need exact geometry subscribe, rather than fooling themselves with a sync return.
- **−** Opaque handles mean debug-printing a handle isn't meaningful across backends. Mitigation: add a `describe(h)` method returning a backend-specific Debug string in v0.2 if needed.
- **−** `subscribe_lifecycle` lumps several event kinds into one stream. Mitigation: acceptable for MVP; can split (Closed / GeometryChanged / TitleChanged) when demand appears.

## Follow-up

- Revisit after Phase 2 (wlroots) spike — confirm trait survives without amendment.
- If `GeometryChanged` becomes needed before v0.3, add as a variant of `LifecycleEvent`.
