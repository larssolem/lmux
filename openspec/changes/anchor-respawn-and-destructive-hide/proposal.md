## Why

Epic 6 shipped anchor pause/resume and soft-hide (widget-visibility toggle), but two capabilities from the PRD remain stubbed:

- **`anchor.respawn`** — a dead anchor cannot be re-launched in place; the user has to manually recreate the pane and retype the command (FR26, Journey 2).
- **Destructive hide with replay** — soft-hide keeps the PTY running in-process, which is cheap in memory for 1–2 anchors but does not let the user reclaim resources from a long-hidden anchor (FR23 specifies a 10 000-line / 1 MiB ring). The destructive flavor kills the child on hide and replays captured scrollback on reattach by re-running the same argv from the original cwd.

Both capabilities are load-bearing for the "peace of mind" success criterion: close a session window confidently knowing the user can always bring everything back.

## What Changes

- `anchor.respawn` bus kind and `lmux anchor respawn <uuid>` CLI that re-forks the same argv + cwd + env in the existing pane slot.
- A `ScrollbackRing` per anchor (`VecDeque<String>`, capped at 10 000 lines or 1 MiB whichever is smaller) populated when the anchor transitions to destructive-hide; drained into the new terminal on reattach.
- Hide now takes an explicit flavor (`soft` default, `destructive` opt-in); the existing soft semantics are preserved unchanged.
- New sidebar action "Respawn" on dead-anchor rows; the context-menu action dispatches the same code path as the CLI.
- libghostty scrollback-export hook so the destructive-hide path can capture the PTY's currently-visible scrollback window at the moment of termination (in addition to the live output stream).

## Capabilities

### New Capabilities

(none — this extends the existing `anchors` capability)

### Modified Capabilities

- `anchors`: adds `Requirement: Respawn a dead anchor`, `Requirement: Destructive-hide with scrollback ring`, and a small modification to the existing soft-hide requirement to clarify that it is one of two hide flavors.

## Impact

- Code: `crates/lmux-anchor/src/registry.rs` (new `process.rs` module for the ring + destructive path), `crates/lmux/src/state.rs` (respawn entry point), `crates/lmux-bus` (new kind), `crates/lmux-cli` (new subcommand), `crates/lmux` sidebar context actions.
- Depends on a libghostty scrollback-format export path, which does not yet exist — design.md will address whether to ship a libghostty-side patch or approximate via an in-process line tap.
- No changes to PTY shutdown contract; respawn goes through the existing spawn path.
