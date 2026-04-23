## Context

Epic 6 of v0.2 shipped anchor pause/resume and soft-hide. Two follow-ons remain:
- The user cannot respawn a dead anchor in place.
- Soft-hide keeps the PTY and memory reserved; a destructive flavor that trades the running process for a capped scrollback ring is unimplemented.

Both depend on the anchor registry already owning the original spawn metadata (argv, cwd, env). The soft-hide path is preserved unchanged; destructive-hide and respawn are additive.

## Goals / Non-Goals

**Goals**
- Provide a bus-kind + CLI path for `anchor.respawn` symmetric to the other `anchor.*` kinds.
- Capture output from the anchor's child at the moment of destructive-hide in a bounded ring buffer.
- Replay the ring on reattach so the user does not lose recent context.
- Keep soft-hide behavior and semantics stable; destructive must be opt-in and clearly labeled.

**Non-goals**
- Persisting the ring across cockpit restarts. Anchor survival across cockpit shutdown remains the soft-hide contract; destructively-hidden anchors die with the cockpit.
- Guaranteed byte-perfect replay of pre-hide scrollback. A best-effort line-count or size-bounded capture is sufficient per FR23.
- Respawning satellites; this change is about PTY-backed anchors only.

## Decisions

### D1 — Destructive hide sources output from a live tap, not a libghostty export

The libghostty scrollback-export path does not yet exist and is not required for the ring if we tap the PTY read path. Decision: install an `AnchorOutputTap` on the pane's PTY reader that pushes line-split output into a `VecDeque<String>` owned by the anchor registry while the anchor is live and the tap is enabled. Destructive-hide enables the tap on the anchor at tag time (lazy — pay the per-line cost only for anchors that might be destructively hidden). A future libghostty export path can replace or augment the tap without changing the spec.

Alternatives considered:
- *Patch libghostty to expose a scrollback export API.* Higher engineering cost, ABI churn risk, and adds to the libghostty pin debt.
- *Read `/dev/pts/<n>` externally.* Racy with the existing PTY reader; deadlocks likely.

### D2 — Ring cap is `min(10_000 lines, 1 MiB)` evaluated at push time

Ring cap cannot be `max(...)` because unbounded memory under a spammy anchor is unacceptable. Decision: on each push evict from the front while either line count exceeds 10 000 or byte-size exceeds 1 MiB. Keeps FR23 honest.

### D3 — Respawn reuses the exact spawn surface from tag time

Decision: `AnchorMeta` gains an `original_spawn: SpawnSnapshot { argv, cwd, env, pgid_mode }` field recorded at tag time (not at hide time — the tagged argv is authoritative). `anchor.respawn` reuses the snapshot and calls the same PTY spawn path as any new pane, so the v0.1 shutdown contract and PTY-resize propagation apply unchanged.

### D4 — `hide` flavor is a field on the bus kind, not a new kind

The existing `anchor.hide` kind gains an optional `flavor: "soft" | "destructive"` field (default `soft`). A new kind would bloat the frozen schema and make backward compatibility with v0.2 clients awkward. Clients that omit the field get the existing soft semantics.

### D5 — Dead anchors retain their ring until respawn or dismissal

After a destructive-hide + child exit (or a respawn → exit cycle), the ring's tail is the "last output tail" surfaced in the sidebar. FR24–FR25's `last 200 lines` guarantee is satisfied by slicing the ring.

## Risks / Trade-offs

- *Risk: line tap doubles per-line work for anchors that will never be destructively hidden.* → Mitigation: enable the tap only when an anchor is tagged AND the user has toggled "allow destructive hide" in config (default off). Anchors that only use soft-hide never pay.
- *Risk: replaying 10 000 lines at reattach stalls the UI.* → Mitigation: the replay uses the existing libghostty write path in batches of ≤512 lines with a yield between batches so the GTK loop stays responsive.
- *Risk: ring captures ANSI escapes that become invalid when replayed out of order.* → Mitigation: the capture is the raw stream, not the rendered grid; replay is the same bytes in order so escape-sequence state is consistent.
- *Risk: a respawn without a valid cwd (directory deleted since tag time) silently spawns in `$HOME`.* → Mitigation: respawn fails loudly (`error.cwd_missing`) and the anchor stays in the dead state; the user can rename the cwd or dismiss and retag.

## Migration Plan

No data migration. Existing soft-hide remains the default. The bus kind gains an optional field; existing v0.2 clients that do not set it continue to work without change.

## Open Questions

- Should the config surface a per-anchor override `destructive_hide_allowed = true` instead of a global toggle? The global is simpler but gives up fine-grained control. Defer until user has dogfooded once.
- Do we want ring persistence across soft-reboot via the session file? Out of scope for this change; revisit if dogfooding surfaces lost-context complaints.
