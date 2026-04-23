---
title: 'Validation Spike 2: Compositor IPC (KWin + wlroots)'
type: 'chore'
created: '2026-04-21'
status: 'in-progress'
baseline_commit: 'f67c37d'
context:
  - '{project-root}/_bmad-output/brainstorming/brainstorming-session-2026-04-20-23-28.md'
  - '{project-root}/spikes/libghostty-ffi/Cargo.toml'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** The brainstorm's pre-code validation spikes listed "Hyprland + Sway compositor IPC" as risky. The MVP target has since shifted to KDE Plasma / KWin (user's daily driver), with wlroots relegated to multi-compositor support. Before the v0.2 PRD is written, we need empirical proof that lmux can **spawn, identify, focus, move/resize, and lifecycle-track** external GUI windows on both paths — and we need to surface the shape of a `CompositorControl` abstraction that fits both.

**Approach:** Throwaway two-phase Rust spike in `spikes/compositor-ipc/`, mirroring the `spikes/libghostty-ffi/` layout. Phase 1 (MVP) runs on the host KWin session using `ext-foreign-toplevel-list-v1` for enumeration and `org.kde.KWin.Scripting` (over D-Bus) for control. Phase 2 runs in a nested Hyprland (preferred) or Sway session using `wlr-foreign-toplevel-management-v1` + `hyprctl` / `swaymsg` for control. Deliverable is two watchable probe binaries plus a `FINDINGS.md` with a GO / NO-GO / PIVOT verdict and a proposed trait.

## Boundaries & Constraints

**Always:**
- Throwaway quality: code lives under `spikes/`, is not published, is not reused verbatim in v0.2.
- Each capability (spawn, identify, focus, move/resize, lifecycle) must be exercised end-to-end in a binary a human can *watch* succeed on screen — not just a unit test.
- Each phase produces a `FINDINGS.md` section with what worked, what didn't, gotchas, timings, and resulting abstraction-shape observations.
- Phase 1 must complete and yield findings before Phase 2 begins (so the trait proposal from KWin informs wlroots validation, not the other way around).

**Ask First:**
- Changing the `CompositorControl` trait signature after Phase 1 findings to accommodate Phase 2 — flag the reconciliation before hacking both backends.
- Installing system packages beyond: `hyprland` (or `sway`), `foot`, and Rust crates listed in the tasks.
- Any capability that cannot be done cleanly (e.g. KWin scripts requiring persistent registration, or wlr toplevel handle unable to move/resize) — halt and discuss before inventing a workaround.
- Skipping Phase 2 entirely — the MVP is KWin, but the multi-compositor verdict is a deliverable.

**Never:**
- Do not render via libghostty here — that's `libghostty-ffi` spike territory; this one is purely about external window control.
- Do not build a general compositor-abstraction *crate* — a trait plus two backends is the goal; polishing an API is v0.2 work.
- Do not target GNOME / Mutter or X11 — Wayland-only, wlroots + KWin only.
- Do not run `sudo` or modify system config outside the user's home.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Spawn + identify | Host KDE session; run `kwin-probe` | `konsole` opens; probe logs its toplevel handle (app_id + PID) within 2 s | Retry enumeration up to 2 s; fail loud with KWin / D-Bus diagnostics |
| Focus on demand | Two spawned windows | Target window gains keyboard focus within 200 ms of request | Log which handle + method failed; continue to next capability |
| Move / resize | Send geometry (100,100,800,600) then (400,200,640,480) | Window visibly transitions to each geometry within 100 ms | Dump the KWin script source + last ~50 lines of `journalctl --user -u plasma-kwin_wayland` |
| Lifecycle tracking | User closes the spawned window externally | Probe receives a `Closed` event within 1 s and exits 0 | Timeout at 5 s → fail with "lifecycle not observed" |
| Nested wlroots startup | Run `nested-session.sh` on host KDE | Nested Hyprland (or Sway) window appears; `WAYLAND_DISPLAY` exported for child shell | Fail fast if nested compositor exits <2 s or display var missing |
| Wlroots full probe | `wlroots-probe` run inside nested session | All five capabilities succeed against a spawned `foot` terminal | Same verbosity contract as Phase 1 |

</frozen-after-approval>

## Code Map

- `spikes/compositor-ipc/Cargo.toml` -- Cargo workspace root
- `spikes/compositor-ipc/compositor/` -- shared crate defining `CompositorControl` trait + types
- `spikes/compositor-ipc/phase1-kwin/` -- KWin backend + probe binary
- `spikes/compositor-ipc/phase1-kwin/scripts/*.js` -- KWin JS loaded over D-Bus
- `spikes/compositor-ipc/phase2-wlroots/` -- wlroots backend + probe binary
- `spikes/compositor-ipc/phase2-wlroots/nested-session.sh` -- harness to launch nested compositor
- `spikes/compositor-ipc/FINDINGS.md` -- verdicts + trait proposal
- `spikes/libghostty-ffi/` -- existing layout precedent (do not touch)

## Tasks & Acceptance

**Execution:**

*Scaffolding*
- [x] `spikes/compositor-ipc/Cargo.toml` -- create Cargo workspace with members `compositor`, `phase1-kwin`, `phase2-wlroots` -- mirror `libghostty-ffi` layout (phase2-wlroots member deferred; not yet scaffolded)
- [x] `spikes/compositor-ipc/compositor/src/lib.rs` -- define `trait CompositorControl` (methods: `enumerate`, `focus`, `set_geometry`, `subscribe_lifecycle`) plus `ToplevelHandle`, `Geometry`, `LifecycleEvent` -- v0 draft landed; amendments proposed in FINDINGS.md

*Phase 1 (KWin, MVP-critical)*
- [x] `spikes/compositor-ipc/phase1-kwin/Cargo.toml` -- deps: `zbus = "5"`, `wayland-client = "0.31"`, `wayland-protocols = { version = "0.32", features = ["client", "staging"] }`, `tokio = { version = "1", features = ["full"] }`, `anyhow`, `tracing`, `tracing-subscriber`
- [x] `spikes/compositor-ipc/phase1-kwin/src/toplevel_list.rs` -- implemented but KWin does NOT advertise `ext-foreign-toplevel-list-v1`; graceful fallback to KWin scripting (see FINDINGS.md)
- [x] `spikes/compositor-ipc/phase1-kwin/scripts/{enumerate,focus,move_resize}.js.tmpl` -- three templates; `Qt.rect` unavailable in plain JS scripts, used plain object coercion to QRectF
- [x] `spikes/compositor-ipc/phase1-kwin/src/kwin_script.rs` -- D-Bus bridge with reply-callback service (`org.lmux.Probe1`); correlation-id-keyed dispatch to `loadScript` + `run()`
- [x] `spikes/compositor-ipc/phase1-kwin/src/main.rs` -- spawn `konsole --separate --hold`, correlate by PID, focus, move/resize twice, child-wait lifecycle; all five capabilities exercised end-to-end

*Phase 2 (wlroots, multi-support) — DEFERRED this session*
- [ ] `spikes/compositor-ipc/phase2-wlroots/Cargo.toml`
- [ ] `spikes/compositor-ipc/phase2-wlroots/nested-session.sh`
- [ ] `spikes/compositor-ipc/phase2-wlroots/src/foreign_toplevel.rs`
- [ ] `spikes/compositor-ipc/phase2-wlroots/src/hyprctl_ipc.rs` (or `swaymsg_ipc.rs`)
- [ ] `spikes/compositor-ipc/phase2-wlroots/src/main.rs`

*Deliverable*
- [x] `spikes/compositor-ipc/FINDINGS.md` -- Phase 1 section complete: GO verdict, what worked / didn't, timings, abstraction amendments, unknowns. Phase 2 section stubbed as "Deferred."

**Acceptance Criteria:**
- Given a host KDE Plasma 6 Wayland session, when `cargo run -p phase1-kwin --bin kwin-probe` runs, then a `konsole` window appears, visibly moves/resizes twice, and the probe exits 0 after the user closes it; logs show one line per capability with a timing measurement.
- Given `nested-session.sh` has launched a nested Hyprland or Sway, when `cargo run -p phase2-wlroots --bin wlroots-probe` runs inside it, then the same five-capability sequence completes against a `foot` window, and logs include a timing measurement per capability.
- Given both probes pass, when the user opens `FINDINGS.md`, then they find (a) a GO / NO-GO / PIVOT verdict for Path A, (b) a concrete trait proposal suitable for the v0.2 PRD, and (c) an "unknowns / deferred" list (e.g. GNOME, multi-monitor, HiDPI).
- Given Phase 1 yields NO-GO, when Phase 2 begins, then it runs anyway — a wlroots-only lmux is a valid PIVOT — unless the user explicitly stops.

## Spec Change Log

- 2026-04-21 — Phase 2 (wlroots) resumed in a new session. Status flipped `phase1-complete` → `in-progress`; baseline set to `f67c37d` (Phase 1 GO commit).
- 2026-04-21 — Phase 2 (wlroots) deferred by user mid-session ("la oss teste for wayland nå og drite i de andre"). Ask-First trigger for skipping Phase 2 was satisfied by explicit user direction. Phase 2 scaffolding untouched; rerun before v0.2 PRD lock.
- 2026-04-21 — Phase 1 status flipped to `phase1-complete`; GO verdict recorded in `FINDINGS.md`.

## Design Notes

KWin intentionally does not implement `wlr-foreign-toplevel-management-v1`. The correct KDE primitives are `ext-foreign-toplevel-list-v1` (enumeration only — no focus/move/resize) plus KWin's D-Bus scripting API. This forces the trait to separate **identify** (Wayland protocol) from **control** (D-Bus for KWin, Wayland extension for wlroots) — this separation is the main design signal the spike must produce.

Identifier correlation is asymmetric: `ext-foreign-toplevel-list-v1` exposes `app_id` + `title` but *not* PID, while a KWin script can read `client.pid`. Phase 1 correlates spawned child by PID via a script query. Phase 2 correlates by app_id + first-seen-after-spawn heuristic. The trait should expose identifiers as opaque `ToplevelHandle` tokens; the correlation strategy is a backend concern.

KWin scripts return results via D-Bus signals on `/Scripting/Script<N>`, not synchronous return values. Use `zbus::Proxy::receive_signal` with a request/response correlation ID embedded in the script invocation.

Nested Hyprland is preferred over nested Sway because `hyprctl`'s socket IPC is more ergonomic than i3 IPC for the dispatchers we need; Sway is an acceptable fallback if Hyprland doesn't install cleanly on Arch.

## Verification

**Commands:**
- `cargo build --workspace` (in `spikes/compositor-ipc/`) -- expected: clean build on `rustc 1.93`
- `cargo run -p phase1-kwin --bin kwin-probe` (host KDE session) -- expected: konsole appears, moves twice, closes cleanly; probe exits 0
- `bash spikes/compositor-ipc/phase2-wlroots/nested-session.sh` -- expected: nested compositor window visible on KDE desktop
- `cargo run -p phase2-wlroots --bin wlroots-probe` (inside nested session) -- expected: foot window runs the same dance; probe exits 0

**Manual checks:**
- `FINDINGS.md` contains a GO / NO-GO / PIVOT verdict, a concrete trait block, and an "unknowns / deferred" list.
- Phase 1 findings are written *before* Phase 2 starts (git log shows the intermediate commit).
