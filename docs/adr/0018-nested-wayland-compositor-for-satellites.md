# ADR-0018: Nested Wayland compositor for GUI satellites

- Status: Proposed
- Date: 2026-04-22
- Deciders: Lars
- Blocks: v0.2 (Epic 9 — if accepted) OR v0.3 (if deferred)
- Depends on: ADR-0004 (CompositorControl trait), ADR-0017 (v0.2 method surface)
- Supersedes (partially): ADR-0005 (KWin MVP — spawn-external-window model)

## Context

The initial v0.2 satellite model was: fork-exec the GUI app, let KWin manage its window, then nudge it into position via KWin scripting. This is what `KwinCompositor::spawn_satellite` + the one-shot placement script does today.

Empirically this model fails the "useful cockpit" bar:

- External windows fall behind the cockpit the moment focus moves elsewhere. `keepAbove` mitigates stacking but not focus; `transient-for` helps stacking but Wayland doesn't let us forcibly set a foreign client's parent.
- Terminal panes and GUI satellites get *structurally different* treatment: one is a GTK widget child of the cockpit, the other is a top-level window slaved by geometry hints. Anchor workspace semantics (hide on switch-away, re-show on switch-back) only work reliably for the embedded kind.
- The user's explicit requirement: **"terminal og whatever gui verktøy må behandles likt"** — identical treatment end-to-end. An external-window model cannot satisfy this on Wayland.

On Wayland, there is exactly one way to truly embed a foreign client's surface as a child of ours: **host a nested Wayland compositor inside the cockpit** and have satellite clients connect to it (via `WAYLAND_DISPLAY=lmux-<socket>`). Smithay is the mature Rust library for this.

XEmbed (X11-only) is ruled out: GTK4 dropped `GtkSocket`, and Wayland-native apps don't speak the XEmbed protocol anyway.

## Decision

Host an in-process Wayland compositor inside lmux using `smithay`. GUI satellites spawn with `WAYLAND_DISPLAY` pointing at lmux's nested socket; their `wl_surface`s render into a GTK widget that sits in the pane tree on equal terms with a terminal pane.

### v0.2 MVP scope (what MUST land to unblock Epic 9)

| Area                | In scope                                                                                                             | Deferred                                                           |
| ------------------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| Protocols           | `wl_compositor`, `wl_shm`, `wl_subcompositor`, `xdg_shell` (xdg_wm_base, toplevel, popup), `wl_seat` (ptr + kbd)     | `linux_dmabuf`, `wl_data_device` (DnD), input-method, xdg-decoration, layer-shell, xdg-foreign, primary-selection |
| Rendering           | Software: read `wl_shm` buffers (ARGB8888 / XRGB8888) and blit into a GTK4 `Picture` / `GLArea` via `MemoryTexture`  | GPU path via `linux_dmabuf` + GL-textures-from-handle              |
| Surface widget      | One `SatelliteSurface` widget per xdg_toplevel; lives in `lmux::pane::Pane` on equal footing with the ghostty widget | Multiple toplevels per satellite (popups excepted)                 |
| Input               | Pointer (motion / button / axis) + keyboard focus follows GTK focus; `wl_keyboard` keymap from xkbcommon             | Tablet, touch, gesture, IME (xdg_input_method)                     |
| Output              | Single `wl_output` matching the GTK widget allocation; scale = 1                                                     | Multi-output, fractional scale (wp_fractional_scale_v1)            |
| Spawning            | `spawn_tagged_with_pid` stays, but sets `WAYLAND_DISPLAY=<lmux-socket>` and unsets `DISPLAY`                          | X11 satellites (XWayland-in-lmux) — out of MVP                     |
| Workspace semantics | Satellite widget obeys `pane_workspace` like terminals: hide = `set_visible(false)` on widget; same for anchor switch | Full respawn-on-hide (Epic 6)                                      |
| CompositorControl   | `KwinCompositor` / `NoopCompositor` drop `spawn_satellite` geometry-slave path; replaced by a `NestedCompositor` impl that owns the smithay server and produces `SatelliteWidget`s | Dock/undock to external window (v0.3)                              |

### Out of MVP, needed for v0.3

- X11 client support via nested XWayland.
- GPU buffer path (`linux_dmabuf` → GL texture) for apps that refuse `wl_shm`.
- Drag-and-drop (`wl_data_device`) across satellites and terminals.
- xdg-decoration: satellite decides server-side vs client-side decorations.
- Layer-shell for panels/overlays.
- Dock-out: detach a satellite into a standalone window (reverse of embedding).
- Proper clipboard bridge between satellites + cockpit.

### Architecture sketch

```
┌──────────────────────────────── lmux (GTK4 app) ─────────────────────────────────┐
│                                                                                  │
│   gtk4::Paned  ──────────────────────────────────────────────────────────────    │
│     │                                                                            │
│     ├─  GhosttyWidget  (existing terminal pane)                                  │
│     │                                                                            │
│     └─  SatelliteWidget (xdg_toplevel #1)  ─ draws ARGB from wl_shm buffer       │
│           │                                                                      │
│           └── forwards GTK input events → smithay Seat → wl_keyboard/wl_pointer  │
│                                                                                  │
│   ┌─── smithay compositor (tokio task on a dedicated thread) ─────────────┐     │
│   │                                                                        │     │
│   │   wl_display @ $XDG_RUNTIME_DIR/lmux/wayland-0                         │     │
│   │   xdg_wm_base · wl_compositor · wl_shm · wl_seat · wl_subcompositor   │     │
│   │                                                                        │     │
│   │   event loop: calloop → dispatches client requests →                   │     │
│   │     new xdg_toplevel → notifies GTK thread (async_channel) →           │     │
│   │       GTK thread creates a SatelliteWidget + registers in AppState     │     │
│   │                                                                        │     │
│   └────────────────────────────────────────────────────────────────────────┘    │
│                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────┘
                                   ▲
                                   │ WAYLAND_DISPLAY=lmux/wayland-0
                                   │
                              satellite clients
                              (Chromium, Kate, ...)
```

### Thread model

- Smithay runs on its own OS thread with a `calloop` event loop (not tokio — smithay's protocol dispatch is blocking synchronous around the wl_display fd).
- GTK main thread owns the widget tree and `AppState`.
- Cross-thread: existing `async_channel` pattern — smithay → GTK `glib::MainContext::spawn_local`, GTK → smithay via calloop `ping`.

### Buffer path (MVP, software)

1. Client commits a `wl_shm` buffer to a surface.
2. Smithay handler releases the previous buffer, copies the new one's `memmap` into a `Vec<u8>` (ARGB → RGB reshuffle if needed), and posts `FrameReady { surface_id, w, h, rgba }` on the async_channel.
3. GTK thread receives, promotes the `Vec<u8>` into a `gdk::MemoryTexture`, and sets it on the `Picture` inside the `SatelliteWidget`.
4. `wl_callback::done` sent back so the client paints the next frame at the target rate.

(Zero-copy via dmabuf is v0.3; v0.2 software path is sufficient for Chromium / Kate / VS Code / the target personas. Measured Chromium wl_shm path on an 8-core Linux box is ~30–60 MB/s at 1080p — well within memory-copy budget for a cockpit tool.)

### Input path

GTK events on `SatelliteWidget` → translated to smithay `Seat` events:

- `GestureClick` → `PointerHandle::button`.
- `EventControllerMotion` → `PointerHandle::motion`.
- `EventControllerScroll` → `PointerHandle::axis`.
- `EventControllerKey` → `KeyboardHandle::input` (raw keycode).
- Focus: `SatelliteWidget::focus-in` → `Seat::set_keyboard_focus(Some(surface))`.

The smithay `Seat` serialises and forwards to the currently-focused surface's `wl_pointer` / `wl_keyboard`.

## Consequences

**Positive**

- Terminals and GUI satellites get *structurally identical* treatment: both are GTK widgets inside `pane::Pane`. Workspace ownership, focus, anchor switching, layout splits — all Just Work without per-kind branching.
- Removes the KWin placement hack entirely. `KwinCompositor` can drop `spawn_satellite`'s geometry script and the matching `set_window_visible_by_pid` script.
- Makes the v0.3 Hyprland/wlroots port cheaper: the compositor backend is lmux's own nested compositor, not the host's, so lmux is desktop-environment-agnostic for GUI satellites.
- Unblocks Epic 9 (docking) meaningfully: "dock" and "undock" become "reparent widget" operations rather than window-manager remote control.

**Negative**

- Significantly expands v0.2 scope. Realistic effort: **2–3 engineering weeks** for the MVP surface listed above (smithay plumbing: ~3 days; xdg_shell handler: ~2 days; GTK widget + wl_shm blit: ~3 days; input routing: ~2 days; focus/anchor integration: ~2 days; polish + tests + e2e: ~3 days). Assumes familiarity with smithay's type system, which is not trivial.
- Adds `smithay` (+ its transitive deps: `wayland-server`, `wayland-protocols`, `calloop`, `xkbcommon`) to the dependency tree. These are well-maintained but meaningful binary-size and compile-time costs.
- Software-only buffer path: GPU-accelerated apps (OBS, video players, blender) will be laggy at 1080p+. Acceptable for the v0.2 personas (editor + browser + chat) but a known limitation.
- Risk: smithay's API churns between minor versions. Pin a version and gate upgrades.

**Rollback**

If the MVP doesn't land within the timebox: remove `NestedCompositor` + `SatelliteWidget`, restore the old `KwinCompositor::spawn_satellite` placement path, and defer Epic 9 to v0.3. The v0.2 story list accommodates this because terminal-only functionality (Epics 1-8, 10, 11) doesn't depend on satellites.

## Alternatives considered

- **Keep external-window model with `keepAbove` + `transient-for`**: rejected — fails the "treated like terminals" requirement, and stacking/focus quirks remain.
- **X11 + XEmbed (GtkSocket re-implementation)**: rejected — GTK4 removed GtkSocket; Wayland-native apps don't support XEmbed; project target is Wayland-first.
- **Defer Epic 9 to v0.3 entirely**: still on the table as a rollback. The user's most recent signal is "move this into v0.2" (2026-04-22).
- **Use an external nested compositor (cage, weston-nested)**: rejected — introduces a separate process the cockpit has to manage, and the embedding surface is still a top-level window, not a GTK widget.

## Open questions

1. **smithay version pin**: 0.3 is the current stable branch; 0.4 is in-flight. Pick 0.3 for v0.2 stability.
2. **GTK surface widget**: `Picture` with `MemoryTexture` (simplest, software), vs. `GLArea` with smithay's GL renderer (harder, future-proof). v0.2 MVP picks `Picture`; the widget type is encapsulated behind a module boundary so upgrading to GL is a local change.
3. **Input focus ownership**: GTK's focus model vs. smithay's seat model. MVP: GTK is authoritative; smithay seat mirrors the GTK focus of the `SatelliteWidget`.
4. **Keyboard shortcuts (Ctrl+B prefix)**: must intercept the key event *before* forwarding to the satellite, otherwise Ctrl+B lands in Chromium. Handled by GTK capture-phase controller on the top-level window (already how terminal panes do it).
