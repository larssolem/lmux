# ADR-0005: KWin as MVP compositor; wlroots secondary

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.2

## Context

The original brainstorm assumed Hyprland/Sway as the MVP compositor target. The author's daily driver is KDE Plasma / KWin on Wayland. Shipping for a compositor the author doesn't use daily means dogfood is faked; v0.2 "personal dogfood ready" becomes meaningless.

Simultaneously, "editor-agnostic and multi-compositor" is a positioning promise that suffers if lmux is seen as KDE-only. KWin's scripting/IPC is richer than GNOME's for cockpit-style work but is unusual — it does not advertise `wlr-foreign-toplevel-management-v1`, forcing a KWin-specific D-Bus + JS-script approach.

## Decision

- **MVP compositor:** **KDE Plasma / KWin** on Wayland.
  - Primary control transport: `org.kde.KWin.Scripting` over D-Bus.
  - Primary identify transport: `ext-foreign-toplevel-list-v1` when available; otherwise same KWin script.
  - Required KWin version: 6.0+ (tested against 6.5.6).
- **Secondary compositor:** wlroots (Hyprland primary; see ADR-0006). Best-effort docking by v0.3; KWin must not regress while wlroots lands.
- **Deferred:** GNOME / Mutter and X11. v0.4+ backlog.

The compositor abstraction (ADR-0004) must be designed from day one so the secondary path is never a rewrite.

## Alternatives considered

- **Hyprland/Sway first (brainstorm default).** Rejected: author doesn't daily-drive either; would either fake dogfood or force a compositor switch mid-project.
- **GNOME first.** Rejected: Mutter's scripting surface is weaker; the primary audience (enthusiast Linux devs) skews away from GNOME.
- **Compositor-agnostic from day one, no priority.** Rejected: real protocol quirks surface only in one backend at a time; spreading attention produces two half-working backends and no dogfood target.

## Consequences

- **+** Author can dogfood on the real target, every day.
- **+** KWin's richer scripting surface gives us an easier first backend; the trait is battle-tested against the harder compositor first.
- **−** KWin-first narrows the initial beachhead. Hyprland/Sway users discovering lmux at v0.2 will hit "docking disabled here." Mitigation: degrade gracefully (panes still work), document clearly.
- **−** Porting debt emerges the moment wlroots users show up before v0.3. Mitigation: the `CompositorControl` trait contains the damage; only the backend crate needs to evolve.
- **−** KWin-specific JS scripts are a fragile dependency on an internal API. Mitigation: ADR-0011 lifecycle investigation + kill criterion if KWin breaks us across updates.

## Follow-up

- Phase 2 (wlroots) compositor IPC spike before v0.3 PRD lock.
- Watch KWin changelogs for scripting-API changes each KDE release.
