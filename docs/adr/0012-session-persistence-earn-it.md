# ADR-0012: Session persistence - earned snapshot layers

- Status: Accepted
- Date: 2026-04-21
- Updated: 2026-05-24
- Deciders: Lars
- Blocks: startup restore, named sessions, session switcher

## Context

The original brainstorm included richer persistence ideas: append-only logs,
sync, replay, and crash-proof incremental state. Those remain out of scope until
real usage earns them.

The implementation now has two pragmatic snapshot layers:

- a legacy startup/shutdown snapshot at `$XDG_DATA_HOME/lmux/last-session.json`;
- named TOML sessions under `$XDG_STATE_HOME/lmux/sessions/`.

The legacy JSON file keeps first-run and clean shutdown restore simple. The
named TOML store powers `lmux-cli session ...` and the `Ctrl+B s` fuzzy
switcher. A migration path copies a v0.1 `last-session.json` into
`sessions/default.toml` when needed.

## Decision

lmux persists only state it can restore honestly:

- terminal pane layout;
- pane working directories;
- anchor pane ids and named-session anchor references;
- session recency index.

Live GUI satellite panes are stripped from saved terminal layout snapshots.
Native app windows are compositor state, not process state lmux can reliably
respawn. lmux may remember attached-window identity while the process is live,
but a saved session is not a promise to relaunch arbitrary GUI windows later.

Named sessions use TOML under `$XDG_STATE_HOME/lmux/sessions/` with
`sessions/index.toml` for recency. The legacy JSON snapshot remains under
`$XDG_DATA_HOME/lmux/last-session.json` for startup/shutdown compatibility.

Persistence remains snapshot-based. There is no append-only event log, no sync,
no replay UI, and no background session daemon.

## Alternatives considered

- **Append-only event log.** Rejected as speculative infrastructure.
- **Only the legacy JSON file.** Rejected once named session switching shipped.
- **SQLite store.** Still unnecessary for the current snapshot model.
- **Full native app restore.** Rejected for current behavior because app
  relaunch, per-window identity, unsaved documents, and OS permission prompts
  are outside lmux's reliable control.

## Consequences

- Users get stable terminal/session restore without a daemon.
- Session files are easy to inspect and repair.
- Switching named sessions tears down live terminal panes and recreates them
  from the target snapshot.
- GUI windows attached to an anchor are part of the live workspace experience,
  but not guaranteed durable session contents.
- Crash-robust incremental persistence can still be added later if dogfooding
  shows real data loss pain.

## Follow-up

- Keep `openspec/specs/sessions/` as the current behavior contract.
- User docs must explain the difference between terminal/session restore and
  live native-window attachment.
