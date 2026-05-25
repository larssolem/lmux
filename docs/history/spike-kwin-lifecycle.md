---
title: 'Validation Spike 3: KWin Script Lifecycle Probe'
type: 'chore'
created: '2026-04-21'
status: 'proposed'
baseline_commit: 'f67c37d'
context:
  - '{project-root}/docs/adr/0011-kwin-script-lifecycle.md'
  - '{project-root}/spikes/compositor-ipc/FINDINGS.md'
  - '{project-root}/spikes/compositor-ipc/phase1-kwin/'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** ADR-0011 is **Proposed** because Phase 1 only exercised *ephemeral, per-operation* KWin scripts. v0.2 GUI satellite docking requires a **stream** of lifecycle events (`windowAdded`, `windowRemoved`) — the natural implementation is a long-lived KWin script registering workspace listeners and emitting via D-Bus signals. Before lmux commits to that primary path (or its polling-based fallback), four concrete unknowns must be settled: leak behaviour across unclean exit, survival across KWin / Plasma restart, cost under N concurrent sessions, and event-delivery reliability under spawn flux.

**Approach:** One-day throwaway extension of the existing `spikes/compositor-ipc/phase1-kwin/` workspace. Add a `lifecycle-probe` binary that registers a long-lived listener script (name-prefixed with its own PID), then exercises four named scenarios against a running host KDE Plasma 6 Wayland session. All results appended to the existing `spikes/compositor-ipc/FINDINGS.md` under a new "Lifecycle Prototype" section, with an `Accepted` / `Accepted-with-fallback` / `Pivot-to-polling` verdict that closes ADR-0011.

## Boundaries & Constraints

**Always:**

- Throwaway quality. Code lives under `spikes/compositor-ipc/phase1-kwin/`; is not merged into lmux product code.
- Each of the four named scenarios (Leak / Restart / Cost / Flux) must run end-to-end on host KDE and produce a watchable log line *per* expected event plus a pass/fail summary.
- PID-prefixed script naming is mandatory (`lmux-<pid>-lifecycle`) — cleanup strategy depends on it.
- Results appended to the existing `FINDINGS.md`; do not fork a new findings file.
- Keep runtime under ~15 minutes total for a full pass so it can be iterated.

**Ask First:**

- Extending the ADR-0011 decision matrix based on findings — if a novel outcome surfaces (e.g. scripts survive `kwin_wayland --replace` but stop firing silently), flag before rewriting ADR-0011.
- Installing any new crate beyond the ones already in `phase1-kwin/Cargo.toml` plus `sysinfo` for PID/RSS reads.
- Running more than 5 concurrent lmux session simulations during the Cost scenario — higher N risks desktop instability and is out of scope.

**Never:**

- Do not run in a nested compositor; scenario #2 (KWin restart) requires the host session.
- Do not integrate against the wlroots backend — that is Spike 2 Phase 2's job.
- Do not touch lmux product code or introduce a new compositor trait abstraction here; the only goal is a verdict.
- Do not `sudo` or modify system services. Everything happens under the user's session.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| **Leak across unclean exit** | Probe registers `lmux-<pid>-lifecycle`, parent shell sends `SIGKILL` | `qdbus6 org.kde.KWin /Scripting` still lists the script within 1 s of kill; restart probe → startup scan identifies the orphan by PID prefix (PID now dead) and unloads it within 2 s | On cleanup failure: log which handle + API call failed; dump full `loadedScripts` list |
| **Restart of KWin** | Long-lived script registered; user (or script) triggers `kwin_wayland --replace` while probe watches | Either (a) script survives and emits the next `windowAdded` event, logged within 2 s of a test konsole spawn; or (b) script disappears and probe observes a D-Bus `NameOwnerChanged` / signal-gap it can act on | If neither: fail with "silent stop" — treat as Pivot-to-polling trigger |
| **Cost under N=5 concurrent** | 5 probe instances each register their own listener, all idle for 5 min, then 1 min of flux (spawn/close 60 konsoles round-robin) | KWin `plasma-kwin_wayland` RSS delta ≤ 50 MB cumulative; KWin CPU share ≤ 10% averaged over the 5 min idle window; ≤ 25% during the flux minute | On threshold breach: record actual numbers, set verdict to `Pivot-to-polling`; capture 30 s perf trace |
| **Event delivery under flux** | Single long-lived script; burst 50 konsole spawns over 5 s; each spawn gets `hold` so probe can close them on observed-add | Probe receives exactly 50 `windowAdded` events (correlated by PID) and 50 `windowRemoved` events within 10 s of burst end; no drops, no duplicates | Any mismatch: log the delta; verdict downgrades to `Accepted-with-fallback` (hybrid: stream + reconciliation poll) |

## Code Map

- `spikes/compositor-ipc/phase1-kwin/src/bin/lifecycle_probe.rs` — new probe binary (CLI subcommands: `leak`, `restart`, `cost`, `flux`, `all`)
- `spikes/compositor-ipc/phase1-kwin/scripts/lifecycle.js.tmpl` — new long-lived listener script template (emits `windowAdded` / `windowRemoved` with PID + app_id via D-Bus signal on `/Scripting/Script<N>`)
- `spikes/compositor-ipc/phase1-kwin/src/kwin_script.rs` — extended with helpers: `register_persistent()`, `list_loaded_scripts()`, `unload_by_name_prefix(&str)`
- `spikes/compositor-ipc/phase1-kwin/Cargo.toml` — add `sysinfo = "0.33"` (PID liveness + RSS reads)
- `spikes/compositor-ipc/FINDINGS.md` — new "Lifecycle Prototype" section appended after the Phase 1 verdict
- `spikes/compositor-ipc/phase1-kwin/src/main.rs` — unchanged (kwin-probe stays the Phase 1 demo)
- `docs/adr/0011-kwin-script-lifecycle.md` — status flips to Accepted / Accepted-with-fallback / reopened-for-redesign at the end

## Tasks & Acceptance

**Execution:**

*Scaffolding*

- [ ] `spikes/compositor-ipc/phase1-kwin/scripts/lifecycle.js.tmpl` — authoring the long-lived listener script; verify `workspace.windowAdded.connect(...)` and `.windowRemoved.connect(...)` emit via D-Bus signal with a correlation payload `{event, pid, app_id, window_id, ts_ms}`
- [ ] `spikes/compositor-ipc/phase1-kwin/src/kwin_script.rs` — add `register_persistent(script_name, template_path) -> ScriptHandle`, `list_loaded_scripts() -> Vec<LoadedScript>`, `unload_by_name_prefix(prefix)`; extend the reply-callback service to accept streaming (not only request/response) correlation
- [ ] `spikes/compositor-ipc/phase1-kwin/src/bin/lifecycle_probe.rs` — scaffold CLI (`clap`) with subcommands `leak`, `restart`, `cost`, `flux`, `all`; per-scenario structured output (pass/fail + timings + counts)
- [ ] `spikes/compositor-ipc/phase1-kwin/Cargo.toml` — add `sysinfo`

*Scenarios*

- [ ] `lifecycle_probe leak` — register `lmux-<pid>-lifecycle`; exec self via `nix::unistd::execv` or spawn a child-helper that SIGKILLs the registering process; re-run the binary in cleanup mode and verify orphan removal within 2 s. Log: probe-pid, orphan-detected-pid, dwell-time-before-cleanup
- [ ] `lifecycle_probe restart` — register; prompt the operator via stderr "run `kwin_wayland --replace` now and press Enter"; after re-prompt, spawn a test `konsole`; observe whether `windowAdded` still arrives. Log: pre-restart-events, post-restart-events, gap-duration
- [ ] `lifecycle_probe cost` — spawn N=5 child probe processes each registering their own script; sample KWin RSS + CPU via `sysinfo` every 5 s for 5 min idle; then orchestrate the 60-konsole round-robin flux; resample. Log: idle-rss-delta, idle-cpu-avg, flux-rss-peak, flux-cpu-peak
- [ ] `lifecycle_probe flux` — single script; burst 50 `konsole --separate --hold` spawns over 5 s; close each on observed-add; after 10 s measure `added_count`, `removed_count`, `duplicate_count`, `dropped_count`. Log: raw counts + p95 add-to-observe latency
- [ ] `lifecycle_probe all` — runs the four scenarios sequentially, writes a consolidated report and appends it to `FINDINGS.md`

*Deliverable*

- [ ] `spikes/compositor-ipc/FINDINGS.md` — new "Lifecycle Prototype" section with: verdict per scenario (pass / fail / degraded), raw numbers, decision-matrix row from ADR-0011 that applies, and a one-sentence recommended ADR-0011 resolution
- [ ] `docs/adr/0011-kwin-script-lifecycle.md` — status transition from `Proposed` to one of `Accepted` / `Accepted (fallback path)` / `Reopened — redesign required`; add a dated note under the Follow-up section referencing the FINDINGS commit

**Acceptance Criteria:**

- Given a host KDE Plasma 6 Wayland session, when `cargo run -p phase1-kwin --bin lifecycle_probe -- all` runs, then the binary exits 0 and writes a consolidated section to `FINDINGS.md` within 15 minutes, covering each of the four scenarios with pass/fail + raw numbers.
- Given scenario Leak, when the registering process is SIGKILL'd, then either the script is observed to persist and be cleaned up by a subsequent probe start (PASS), or the script does not persist at all (also PASS, with a note that leak-risk is moot).
- Given scenario Restart, when `kwin_wayland --replace` runs, then the probe records one of three determinate outcomes: survives-and-emits, disappears-cleanly, or silent-stop — no ambiguous case.
- Given scenario Cost at N=5, when the 5-minute idle window closes, then KWin RSS delta and averaged CPU share are recorded and compared against the matrix thresholds; the probe surfaces a boolean `within_budget`.
- Given scenario Flux, when the 50-konsole burst completes, then `added_count`, `removed_count`, and drops are counted exactly; the probe surfaces a boolean `zero_drops_zero_duplicates`.
- Given all four scenarios have returned, when FINDINGS.md is updated, then ADR-0011 is edited in the same commit so its status and Follow-up section reflect the verdict.

## Spec Change Log

- 2026-04-21 — Initial spec authored from ADR-0011 Follow-up section.

## Design Notes

**Why D-Bus signals from a long-lived script are not trivially reliable.** KWin scripts emit via `callDBus(...)`. The Phase 1 spike validated this for *request/response* (reply-correlation by a generated ID). For *streaming*, the same mechanism works but (a) has no built-in back-pressure, (b) a probe that's slow to drain its listener will cause KWin-side signal buffering whose bound is unspecified. The Flux scenario is designed exactly to surface that bound.

**Why N=5 for Cost.** The envelope of realistic parallel-workstreams for a single user is 3–10 per the brainstorm; N=5 is the midpoint and matches the brief's v0.2 success criterion (≥3 parallel agents). Going higher (N=10, N=20) stresses the desktop without adding signal to the ADR decision.

**Restart scenario uses an operator prompt, not a scripted `kwin_wayland --replace`.** Replacing KWin programmatically is possible but risks killing the running desktop session mid-experiment. Manual operator action is the safer and more reproducible path for a one-day spike.

**Correlation ID strategy for streaming.** The persistent script embeds a deterministic `session_id = uuid()` in every emitted envelope. The probe subscribes to signals on `/Scripting/Script<N>` filtered by `session_id` — prevents cross-talk if multiple probes (Cost scenario) run concurrently.

**Cleanup-on-startup.** Orphan detection enumerates all `lmux-*-lifecycle` scripts currently loaded, parses the PID from the name, checks `sysinfo::Process::by_pid(pid)` — if the PID is gone or owned by a different executable, the script is unloaded.

## Verification

**Commands:**

- `cargo build --release -p phase1-kwin --bin lifecycle_probe` (in `spikes/compositor-ipc/`) — expected: clean build on `rustc 1.93`
- `cargo run -p phase1-kwin --bin lifecycle_probe -- leak` — expected: PASS within 10 s
- `cargo run -p phase1-kwin --bin lifecycle_probe -- restart` — expected: operator-gated; one of three determinate outcomes printed
- `cargo run -p phase1-kwin --bin lifecycle_probe -- cost` — expected: completes within 7 min; prints `within_budget: true|false`
- `cargo run -p phase1-kwin --bin lifecycle_probe -- flux` — expected: completes within 30 s; prints `zero_drops_zero_duplicates: true|false`
- `cargo run -p phase1-kwin --bin lifecycle_probe -- all` — expected: exits 0; appends to `FINDINGS.md`

**Manual checks:**

- `FINDINGS.md` contains a new "Lifecycle Prototype" section under the Phase 1 verdict.
- ADR-0011 has been edited in the same commit as the FINDINGS update; status field is no longer `Proposed`.
- No lingering `lmux-*-lifecycle` scripts remain loaded after the probe exits cleanly: verify with `qdbus6 org.kde.KWin /Scripting loadedScripts`.
- Spike directory untouched outside `spikes/compositor-ipc/phase1-kwin/`.
