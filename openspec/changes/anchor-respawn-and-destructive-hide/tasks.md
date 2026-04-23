## 1. Anchor output tap

- [ ] 1.1 Add `ScrollbackRing` (`VecDeque<String>`) with `push(line)` that evicts while `len > 10_000` or `byte_size > 1 MiB`
- [ ] 1.2 Unit tests for ring eviction: line-cap, byte-cap, mixed (spammy 10-byte lines vs a few huge lines)
- [ ] 1.3 Add `AnchorOutputTap` hooked into the PTY reader that line-splits bytes into the anchor's ring while enabled
- [ ] 1.4 Tap enable flag lives on the anchor, not globally; flag flips to true at tag time iff the destructive-hide opt-in is set

## 2. Spawn snapshot at tag time

- [ ] 2.1 Extend `AnchorMeta` with `SpawnSnapshot { argv, cwd, env, pgid_mode }` captured at tag time
- [ ] 2.2 Unit test: snapshot is stable across pause/resume/soft-hide/reattach cycles
- [ ] 2.3 Round-trip snapshot through serde for session persistence

## 3. Bus + CLI surface

- [ ] 3.1 Add `anchor.respawn { uuid }` to the frozen kind schema (`crates/lmux-bus/src/kinds.rs`)
- [ ] 3.2 Extend `anchor.hide` with optional `flavor: "soft" | "destructive"` (default `soft`)
- [ ] 3.3 Add round-trip + deny-list tests for the new kind shape
- [ ] 3.4 Wire `lmux anchor respawn <uuid>` into `lmux-cli`
- [ ] 3.5 Wire `lmux anchor hide --destructive <uuid>` into `lmux-cli`

## 4. Destructive-hide path

- [ ] 4.1 Implement `destructive_hide(uuid)` in `AnchorRegistry`: flush tap, `SIGTERM` + 500 ms grace + `SIGKILL`, reap, transition state
- [ ] 4.2 Replace pane widget with placeholder showing ring tail when destructively hidden
- [ ] 4.3 Unit test: process exits within 700 ms, ring contents preserved

## 5. Reattach-with-replay

- [ ] 5.1 Implement `destructive_reattach(uuid)`: fork via the same spawn path used for new panes, using the tagged snapshot
- [ ] 5.2 Replay ring tail in batches of 512 lines with a GTK yield between batches
- [ ] 5.3 Unit test: replay ordering preserved; UI stays responsive during replay (measured span)
- [ ] 5.4 Surface `error.cwd_missing` when the captured cwd no longer exists; anchor stays dead

## 6. Respawn path

- [ ] 6.1 Implement `respawn(uuid)` in `AnchorRegistry` using the spawn snapshot; reject if not dead with `error.anchor_not_dead`
- [ ] 6.2 Respawn preserves UUID, metadata, sort-key
- [ ] 6.3 Unit + bus-layer test for the reject path and the happy path

## 7. Sidebar wiring

- [ ] 7.1 Add "Respawn" menu item on dead anchor rows; disabled on live/paused/hidden
- [ ] 7.2 Add "Hide (destructive)" menu item, gated on the config opt-in
- [ ] 7.3 Toasts: ring overflow, respawn success/failure, replay done

## 8. Config toggle

- [ ] 8.1 Add `[anchors] destructive_hide_allowed = false` default to the shipped config + schema
- [ ] 8.2 Hot-reload: flipping the flag on enables taps for newly-tagged anchors; existing anchors remain unchanged
- [ ] 8.3 Document the trade-off (memory per line vs reclaiming process) in the default config's comments

## 9. Observability

- [ ] 9.1 `respawn` and `destructive_hide` each open a `tracing` span with outcome + duration
- [ ] 9.2 Ring-overflow count exposed via `status.get.result` under a new `anchors.ring_evictions` counter
- [ ] 9.3 `lmux status` prints the eviction count when non-zero
