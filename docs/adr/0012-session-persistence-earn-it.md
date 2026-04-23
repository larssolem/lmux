# ADR-0012: Session state persistence — earn it or drop

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.1 (layout save/restore), v0.3 (session-level persistence)

## Context

The brainstorm included a "session state as append-only log" idea (syncable, replayable, fallback-readable) and explicitly marked it **lukewarm** with a "Phase-3 earn-it-or-drop" disposition. Persistence designs are a classic scope sink: once an append-only log exists, it attracts features (sync, replay, audit) that balloon faster than the core product.

Meanwhile, lmux's v0.1 commitment includes "save/restore layout on quit." That is a real, small requirement — not a persistence system. The question: how much persistence does each milestone actually earn?

## Decision

Persistence is shipped **only as each milestone earns it**, starting minimal and growing only when a concrete need appears.

- **v0.1:** on clean quit, write a single JSON snapshot (`serde_json`) to `$XDG_DATA_HOME/lmux/last-session.json` containing (a) pane layout, (b) each pane's working directory, (c) anchor tags. On next start, restore if the file exists. That's it.
- **v0.2:** extend the same snapshot with multi-session (each session a separate file under `$XDG_DATA_HOME/lmux/sessions/<id>.json`). Snapshot is atomic-replace on clean quit only.
- **v0.3:** **no change** unless a v0.2 dogfood pain point explicitly surfaces a need (e.g. "I lost 3 sessions to an OOM crash last week" → warrants incremental write; "I want to replay what Claude did yesterday" → warrants transcript, not an event log).

**Explicitly rejected for v0.3:**

- Append-only event log.
- Sync / remote session mirroring.
- Replay / time-travel UI.
- Crash-robust incremental persistence (unless observed pain demands it).

## Alternatives considered

- **Append-only log from v0.1.** Rejected: speculative scope; violates Display-Don't-Duplicate's spirit (don't build infrastructure before the need). "Earn it or drop" is the explicit brainstorm verdict.
- **No persistence at all; restore-from-shell-history.** Rejected: even MVP needs layout restore to avoid feeling hostile on day one.
- **SQLite-backed store.** Rejected for v0.3: overkill; JSON snapshot + atomic replace handles all the v0.2 load we see.

## Consequences

- **+** Zero scope creep; every persistence feature has to earn its weight against a real pain.
- **+** JSON snapshot is trivially debuggable and editable by the user.
- **+** Crash-robustness can be added per-feature when observed (not theorised).
- **−** Crash during a long session loses that session's layout. Acceptable for v0.1/v0.2 — documented behaviour.
- **−** Users who *want* session replay will not get it. Acceptable: Display-Don't-Duplicate — transcripts live in the shell / the agent, not in lmux.

## Follow-up

- During v0.2 dogfood, log every time the author wishes persistence had been better (OOM, crash, context drop). Each occurrence is one data point toward "earning" a richer design.
