# ADR-0006: Wlroots primary backend — Hyprland over Sway

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.2 (spike), v0.3 (ship)

## Context

The wlroots family is one backend in lmux terms but two concrete compositors in the wild: **Hyprland** (modern, features-forward, ergonomic IPC) and **Sway** (mature, stable, i3-compatible IPC). Both implement `wlr-foreign-toplevel-management-v1`, but their dispatcher IPCs differ materially.

The spike spec (`docs/history/spike-compositor-ipc.md`, Design Notes) already flags Hyprland as preferred because `hyprctl`'s Unix-socket JSON IPC is more ergonomic than i3 IPC (Sway's transport), and Sway is an acceptable fallback if Hyprland doesn't install cleanly.

## Decision

Hyprland is the **primary** wlroots backend. Sway is an acceptable fallback for users whose system can't run Hyprland, but lmux does not guarantee feature parity on Sway through v0.3.

Concretely:

- The wlroots backend crate implements `CompositorControl` against Hyprland first (`hyprctl` + `wlr-foreign-toplevel-management-v1`).
- A Sway variant is shipped only if the Hyprland implementation factors cleanly and the i3-IPC layer costs <1 day of extra work. Otherwise Sway is v0.4+.
- GNOME / Mutter remain out of scope through v0.3.

## Alternatives considered

- **Sway first.** Rejected: i3 IPC is older and less permissive than Hyprland's socket; dispatchers we need for satellite control are harder to wire.
- **Both Hyprland and Sway from day one.** Rejected: spike budget doesn't allow; two wlroots backends is two hard problems, not one.
- **Wayfire or labwc.** Rejected: smaller user base; not enough pull on author's audience.

## Consequences

- **+** One clean wlroots target; spike can focus.
- **+** Hyprland users are culturally closest to the lmux target audience (enthusiast Linux devs who already compose their own tools).
- **−** Sway users feel second-class if Hyprland-first slips; they'll see "panes work, docking disabled" on the wlroots backend at v0.3. Mitigation: document the policy in README; Sway parity is an explicit v0.4 goal.
- **−** Hyprland itself has been known to make aggressive API changes. Mitigation: pin against a known-good version in the docs; watch for changelog breaks.

## Follow-up

- Phase 2 wlroots spike must target Hyprland by default; Sway only as a 1-day bonus if time.
- Re-evaluate Sway parity after v0.3 lands.
