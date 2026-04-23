## Why

v0.2 treats multi-display as "works because GTK handles window placement, but we don't know anything about it". A session restored on a laptop with an external display plugged in opens on whichever monitor the compositor chose; when the external is unplugged, satellites dock into "the geometry they had", which may now be off-screen. Users with 3+ monitors (a common dev setup) see:

- Panes and satellites clustering on whichever display KWin decided was primary.
- Satellite docking going wrong on hotplug (a 4K display's rect doesn't fit on a 1080p replacement).
- `lmux open` always placing the new satellite on the cockpit's current display — not useful when the user wants a browser-satellite on display 2 and the cockpit on display 1.

This change makes displays a first-class domain object: sessions remember which display each pane and satellite lived on (by stable display id, not index), the compositor abstraction enumerates displays and their layouts, and a hotplug handler re-homes orphaned geometry with well-defined rules.

## What Changes

- `CompositorControl::displays() -> Vec<Display>` — display enumeration with stable `display_id` (EDID hash + connector name for persistence across unplug/replug), rect, scale factor, primary flag.
- `compositor.displays` event on the bus, emitted on hotplug add/remove/change.
- Per-pane and per-satellite `display_id` stored in session TOML; restored sessions place each pane/satellite on the matching display.
- Hotplug policy: when a display is removed, panes and satellites on that display migrate to the new primary; when a display is added, migrated panes stay where they are (no thrash) but new `lmux open --display <id>` placements target it.
- New CLI: `lmux display list` (JSON-friendly), `lmux display move <pane-id> <display-id>`, `lmux open --display <display-id> <cmd>`.
- Sidebar UI: per-pane "Move to display..." submenu; a display-picker drop-down on the launcher.
- Session migration: v0.2 sessions without display metadata fall back to "current primary", preserving their behaviour.

## Capabilities

### New Capabilities

(none — extends existing capabilities with display awareness)

### Modified Capabilities

- `compositor-control`: adds `Requirement: Display enumeration and hotplug events`; each backend (`NoopCompositor`, `KwinCompositor`, `HyprlandCompositor`) implements the new method.
- `sessions`: adds `Requirement: Per-pane display placement` and `Requirement: Display hotplug migration policy`; modifies `Requirement: Atomic TOML persistence` to include display metadata in the persisted shape.
- `satellites`: adds `Requirement: Display-aware satellite placement` so `lmux open --display` and restored-satellite placement respect the display choice.

## Impact

- Code: new `crates/lmux-compositor/src/display.rs` with `Display` and `DisplayId`; backend implementations in `crates/lmux-compositor/src/kwin.rs` (via KWin `Workspace.screens` bindings) and `crates/lmux-compositor-wlroots/src/hyprland/displays.rs` (via `hyprctl monitors -j`).
- Session schema: additive `display_id: Option<String>` on pane and satellite records; absent ⇒ v0.2 behaviour.
- Depends on `wlroots-backend-hyprland` for the Hyprland implementation of `displays()`, but the KWin path and the session changes ship independently.
- No breaking changes for single-display users.
