# ADR-0019: Rearrange mode — drag-and-drop on the split tree

- Status: Accepted
- Date: 2026-04-23
- Deciders: Lars
- Blocks: v0.2 (UX polish for pane reflow)
- Depends on: ADR-0001 (GTK4 + libghostty rendering), ADR-0018 (nested compositor — terminals and GUI satellites are equal pane citizens)

## Context

Pane splits in v0.1 are immutable once created: you can split a pane, kill a pane, or close one — but you cannot reflow an existing layout. Achieving e.g. "IntelliJ on the left half, terminal + browser stacked on the right" requires destroying and re-creating panes in the right order, which is hostile to a cockpit whose whole pitch is "your tools, arranged how you want them, persistently."

Two design axes had to be picked:

1. **Always-on DnD vs. modal**: tmux-style "everything is keyboard, modes are explicit" is the project's idiom. Always-on drag handles add visual chrome and risk accidental drags inside a terminal where mouse selection matters.
2. **What the user drops onto**: a separate "drop zone" overlay (clean visuals, more state) vs. dropping on the target pane itself with edge-detection (no extra widget, but the drop-edge must be inferred from cursor coordinates).

The chosen split-tree representation (`Layout::Split { dir, a, b, ratio }`, ADR-0001 implementation) already supports surgical reparenting: remove a leaf, splice a new `Split` in place of the target leaf. The hard part is the GTK / DnD plumbing and the UX, not the data structure.

## Decision

Add a **modal "rearrange mode"** toggled by `Ctrl+B m` (and a future sidebar button). While active:

- Every pane frame becomes a GTK4 `DragSource` that emits the source pane's `PaneId` as a `u32` `ContentProvider`.
- Every pane frame becomes a `DropTarget` accepting the same `u32` type.
- On drop, the cursor's widget-local `(x, y)` is mapped to the closest of four edges via `Edge::from_xy(x, y, w, h)` (Top / Right / Bottom / Left), then `Layout::reparent(source, target, edge)` does the surgery: remove `source`; replace `target`'s leaf with a fresh `Split` whose direction and child order are dictated by the edge.
- A CSS class `lmux--rearrange` is toggled on the root window so panes get a dashed amber border (focused pane gets a solid amber border) — the only visual cue, no separate overlay layer.

Outside rearrange mode, the `DragSource` start handler returns `None`, so accidental drags inside a terminal pane never fire and mouse selection in libghostty is unaffected.

### Layout operation contract

```rust
impl Layout {
    /// Move `source` next to `target` along `edge`. No-op (returns false)
    /// when source == target, either is missing, or source is the root.
    /// Atomic: any internal failure restores the pre-call tree.
    pub fn reparent(&mut self, source: PaneId, target: PaneId, edge: Edge) -> bool;
}
```

Edge → split mapping:

| Edge   | Split direction     | New child order (a, b)    |
| ------ | ------------------- | ------------------------- |
| Top    | `Dir::Horizontal`   | (source, target)          |
| Bottom | `Dir::Horizontal`   | (target, source)          |
| Left   | `Dir::Vertical`     | (source, target)          |
| Right  | `Dir::Vertical`     | (target, source)          |

(`Dir::Horizontal` = stacked top/bottom; `Dir::Vertical` = side-by-side. Convention inherited from existing layout code.)

### Cross-workspace guard

`State::reparent_pane` rejects drops where source and target belong to different anchor workspaces. Cross-workspace moves would silently re-home a pane and confuse the workspace bar; if we want that operation later, it gets its own keybind.

## Alternatives considered

- **Always-on drag handles in pane title bars.** Rejected: adds permanent chrome to every pane and conflates "click to focus" with "click to drag." Modal toggle keeps the default UI clean.
- **Drop-zone overlay (separate transparent zones along each edge of every pane).** Rejected: doubles the widget count and competes with libghostty for input events. Edge-from-cursor on the existing frame is cheaper and good enough; the dashed border tells the user "this pane is a target."
- **Free-form geometry (drag a pane to arbitrary coordinates).** Rejected: lmux is a tiling cockpit, not a floating WM. Free placement breaks the split-tree invariant that drives persistence (ADR-0012) and workspace semantics.
- **Use the existing focus keybinds + a new "swap with focused" action.** Rejected: covers the simple "swap two panes" case but cannot express "take this pane and make it the right half of that other pane" without a multi-step ritual.
- **GTK4 `Notebook`-style tear-off + dock.** Rejected: notebook tabs are a different UX (stacked, not tiled) and don't compose with the split tree.

## Consequences

- **+** A single modal keybind unlocks arbitrary split-tree rearrangement without touching any other UX surface.
- **+** Layout operation is unit-testable (`Layout::reparent` has no GTK dependency); the GTK side is just "translate a drop into one call."
- **+** Treats GUI satellites and terminals identically once Epic 9 lands — both live in the same `Pane` enum and the same split tree (ADR-0018), so the same drop logic works for IntelliJ-as-pane.
- **−** Drop-edge ambiguity near the center of a pane: `Edge::from_xy` currently picks "whichever margin is smallest," which can flip between adjacent edges with sub-pixel cursor jitter. Acceptable for v0.2; if user feedback shows it confusing, add a deadband or visualize the chosen edge before commit.
- **−** No multi-pane drag (pick up a whole subtree). The current operation moves one leaf at a time. A "promote subtree" op would need a new selection model — out of scope until asked for.
- **−** Cross-workspace guard means moving a pane to another workspace still requires a separate operation (planned but not designed).

## Follow-up

- Sidebar button to toggle rearrange mode (mirror of the keybind).
- Visual ghost preview of where the pane will land before drop is committed.
- Persist split ratio when reparenting (currently resets to 0.5; reasonable default for a fresh split, but the user may have spent effort tuning the source pane's ratio).
- Decide whether `Edge::from_xy` should adopt a deadband or a "snap to nearest of 5 zones" model (4 edges + center = swap).
