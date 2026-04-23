# ADR-0011: KWin script lifecycle strategy

- Status: **Accepted** (primary path validated; restart scenario deferred to v0.2 dogfood)
- Date: 2026-04-21 (proposed) · 2026-04-21 (accepted after lifecycle-probe run)
- Deciders: Lars
- Blocks: v0.2

## Context

Phase 1 of the compositor IPC spike (`spikes/compositor-ipc/FINDINGS.md`, GO verdict) uses a per-operation pattern: lmux loads a small KWin JavaScript script over D-Bus, calls `run()`, receives the reply via a D-Bus signal on `/Scripting/Script<N>`, and the script is unloaded. This is clean for discrete operations (enumerate, focus, move/resize).

For `LifecycleEvent::Closed` and a future `GeometryChanged`, lmux needs a **stream**, not a one-shot. The obvious design is a long-lived KWin script registering `workspace.windowAdded` / `workspace.windowRemoved` listeners, streaming matching events back via a D-Bus signal. This raised two open questions the original spike did **not** resolve:

1. Does a long-lived script survive `lmux` process exit? If it leaks, after N restarts the user has N ghost scripts installed in KWin.
2. Do KWin script registrations survive KWin restarts / Plasma session restarts cleanly, or silently stop firing?

A follow-up "lifecycle-probe" spike (`spikes/compositor-ipc/FINDINGS.md` → *Lifecycle Prototype* section) was written specifically to answer these before v0.2 committed to the primary path.

## Decision

**Adopt the primary path: per-session long-lived KWin script + PID-prefixed orphan cleanup.**

Validated by the lifecycle-probe spike (2026-04-21) on KDE Plasma 6 Wayland:

- **Leak scenario — PASS.** A long-lived script outlives SIGKILL on its registering process. Startup cleanup via PID-prefix naming (`lmux-<pid>-lifecycle`) plus rendezvous files under `$XDG_RUNTIME_DIR/lmux-lifecycle/<plugin>.json` reliably detects the orphan (`kill(pid, 0)` liveness probe) and calls `unloadScript(plugin_name)` within 1 ms — KWin confirms the script was still loaded.
- **Flux scenario — PASS.** Under a 50-spawn burst over 5 s, every `windowAdded` and `windowRemoved` event arrived at the probe: 50/50/50/50 with zero drops and zero duplicates. KWin's D-Bus reply channel does not buffer-overflow at this rate.
- **Cost scenario — PASS (`within_budget=true`).** 5 concurrent long-lived listeners added 8.1 MiB RSS to KWin over 5 min idle (budget 50 MiB), averaged 1.99 % CPU (budget 10 %), peaked 23.1 % during a 60-konsole flux minute (budget 25 %).

Implementation shape:

- **Plugin name:** `lmux-<pid>-lifecycle` (PID is `std::process::id()` at registration time).
- **Rendezvous file:** `$XDG_RUNTIME_DIR/lmux-lifecycle/<plugin>.json` containing `{plugin_name, session_id, pid, script_path, created_ts_ms}`. Written atomically on `register_persistent`, deleted on clean unload.
- **Startup cleanup:** `cleanup_orphans_with_prefix("lmux-")` enumerates rendezvous files, checks owner liveness via `nix::sys::signal::kill(pid, None)`, unloads dead owners via `unloadScript`, removes the stale rendezvous file + tempfile.
- **Runtime correlation:** each emitted event carries a `session_id` (UUIDv4 fixed at registration) as the D-Bus `Report` correlation field; the `ProbeServer` routes incoming reports to the matching `UnboundedSender<ReportPayload>`.
- **Shutdown path:** RAII-owned `PersistentScript::unload()` on clean exit. Best-effort on SIGTERM; next-boot cleanup handles the unclean case.

**Restart behaviour (scenario #2) is DEFERRED**, not validated. Running `kwin_wayland --replace` against a working Plasma 6 session risks disconnecting Wayland clients and potentially the session itself. The gain (discovering whether scripts survive compositor restart) does not outweigh the cost during active development. This is re-opened as a v0.2 dogfood observation item.

## Fallback trigger (kill-switch)

If the primary path exhibits pain during v0.2 dogfood — examples: silent-stop after a KWin restart, memory creep over days, events dropped under real workload — pivot to the polling fallback documented in the original "Proposed strategy" section (500 ms `workspace.windowList()` poll gated on spawn-pending + `child.wait()` for `Closed`). The v0.2 PRD should note this fallback exists so a regression doesn't block the release.

## Alternatives considered

- **Assume long-lived scripts are fine, ship them blind.** Rejected: documented fragility, no rollback if it breaks users. The lifecycle-probe spike was the concrete cost of avoiding this.
- **Always poll; never use long-lived scripts.** Rejected: steady-state CPU cost (measured negligible for the primary path) is a worse trade than the scripts' streaming fidelity.
- **Use `ext-foreign-toplevel-list-v1` events for lifecycle.** Rejected: KWin does not advertise it (Phase 1 finding). This blocks the "protocol-pure" route on KDE.

## Consequences

- **+** Primary path reuses the validated KWin scripting surface; no new machinery beyond rendezvous bookkeeping.
- **+** PID-prefixed script names + rendezvous files give deterministic cleanup after unclean exits. Leak-after-crash is a solved problem, not a hope.
- **+** Fallback exists as a documented kill-switch; v0.2 is never undecidable.
- **+** Cost headroom is 5-10× the measured envelope, so the design is not fragile against modest user-load increases.
- **−** Restart fidelity unvalidated — if `kwin_wayland --replace` silently stops our listener, we won't learn until someone tries it. Mitigation: document the scenario in v0.2 release notes; ask dogfood users to report compositor restart incidents.
- **−** Rendezvous files assume `$XDG_RUNTIME_DIR` is persistent within a session and wiped between logins. If a user manually clears it mid-session, orphan cleanup becomes best-effort (the next probe still runs, just with an empty rendezvous set).

## Follow-up

- **v0.2 PRD:** name this as the committed lifecycle strategy; reference the fallback trigger above.
- **Dogfood instrumentation:** log every compositor restart incident and every silent-stop observation against the registered script. Re-open this ADR if either recurs.
- **Restart scenario revisit:** before a public v0.3 launch, re-run the restart scenario in a dedicated throwaway Plasma session (no active work), so the ADR can move from "Accepted (restart deferred)" to "Accepted (fully validated)".
- **Long-running stability:** observe KWin RSS over multi-day v0.2 usage; alert if growth exceeds 10 MiB/day with the lmux listener active.

## History

- 2026-04-21 — Proposed, blocked on prototype (`spec-kwin-lifecycle-probe.md`).
- 2026-04-21 — lifecycle-probe ran on host KDE Plasma 6 Wayland; leak + flux + cost all PASS; restart deferred. Status → **Accepted**. See `spikes/compositor-ipc/FINDINGS.md` → *Lifecycle Prototype*.
