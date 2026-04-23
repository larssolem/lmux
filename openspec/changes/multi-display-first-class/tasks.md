## 1. Display domain type

- [ ] 1.1 New `crates/lmux-compositor/src/display.rs` with `Display { id: DisplayId, connector: String, rect: Rect, scale: f32, primary: bool }`
- [ ] 1.2 `DisplayId` newtype over `String`, constructor takes EDID bytes + connector name, falls back to `unknown:<connector>`
- [ ] 1.3 `DisplayChange` enum variants: `Added(id)`, `Removed(id)`, `Reconfigured(id)`, `InitialEnumeration`
- [ ] 1.4 Serde round-trip tests; prefix-collision tests (two different EDIDs hashing to same prefix → fall back to full hash)

## 2. CompositorControl::displays() method

- [ ] 2.1 Extend the `CompositorControl` trait with `async fn displays(&self) -> Result<Vec<Display>>` and `fn subscribe_displays(&self) -> mpsc::Receiver<(Vec<Display>, DisplayChange)>`
- [ ] 2.2 `NoopCompositor`: one synthetic display
- [ ] 2.3 `KwinCompositor`: one-shot script that queries `Workspace.screens` and writes the result to a temp file; Rust side reads and parses
- [ ] 2.4 `KwinCompositor`: subscribe via a resident KWin script event hook (`screensChanged` → bus-side debounce → event publish)
- [ ] 2.5 `HyprlandCompositor` (behind `hyprland` feature): `hyprctl monitors -j` for enumeration; Hyprland socket2 event stream for hotplug (`monitoradded` / `monitorremoved`)
- [ ] 2.6 Unit tests against fake backends for each impl

## 3. Bus kind

- [ ] 3.1 Add `compositor.displays { displays: [Display], changed: DisplayChange }` to `crates/lmux-bus/src/kinds.rs`
- [ ] 3.2 Subscription glob `compositor.*` now matches this kind as well
- [ ] 3.3 `status.get.result` grows a `displays_count: u32` field (additive, optional in v0.2 clients)

## 4. Session schema additions

- [ ] 4.1 Add `display_id: Option<String>` and `rect_on_display: Option<Rect>` to `PaneRecord` and `SatelliteRecord`
- [ ] 4.2 TOML backward-compat: absent fields parse to `None`
- [ ] 4.3 On save, populate fields from the live compositor state
- [ ] 4.4 On load, if the named display is not present, record a `NeedsMigration` flag that the restore handler consults

## 5. Restore and migration

- [ ] 5.1 `SessionRestore::restore` consults the current display list; for each pane, place it on its recorded display if present
- [ ] 5.2 If recorded display missing → migration policy per D5 (remove → primary, clamp rect)
- [ ] 5.3 Integration test: save a session with panes on displays A and B → unplug B → restore → assert all panes on A, clamped
- [ ] 5.4 Config key `[display].on_removed = "migrate_to_primary" | "hide"` (default `migrate_to_primary`)
- [ ] 5.5 Hotplug event on a running cockpit triggers the same migration path for panes on the removed display

## 6. CLI subcommands

- [ ] 6.1 `lmux display list` prints `{id, connector, rect, scale, primary, short_id}`; `--json` for scripting
- [ ] 6.2 `lmux display move <pane-id> <display-id>` calls a new bus kind `pane.move_to_display`
- [ ] 6.3 `lmux open --display <id|short> <cmd>` resolves the display or errors with `error.display_not_found`
- [ ] 6.4 Integration test: all three subcommands round-trip through the bus

## 7. Sidebar UI

- [ ] 7.1 Per-pane "Move to display…" context submenu listing current displays with connector names
- [ ] 7.2 Launcher "Open on display…" dropdown; defaults to current display
- [ ] 7.3 Hotplug toast: "Display <connector> disconnected — moved N panes to <primary connector>"
- [ ] 7.4 "Display <connector> connected — use 'Move to display…' to place panes there" (one-time per connector per session)

## 8. Satellite display awareness

- [ ] 8.1 `spawn_satellite` accepts an optional `target_display_id`; placement respects it
- [ ] 8.2 Geometry-follow translates `rect_on_display` to absolute screen coords using the display's current rect and scale
- [ ] 8.3 Display reconfigure event triggers `set_geometry` on all docked satellites whose display changed
- [ ] 8.4 Integration test: satellite on display B → change display B's resolution → observe a single `set_geometry` call with the new clamped rect

## 9. Observability

- [ ] 9.1 Span `displays.enumeration { count }` on each `displays()` call
- [ ] 9.2 Span `displays.hotplug { changed }` on each event
- [ ] 9.3 Counter `displays.migrations_applied` (per removal event)
- [ ] 9.4 `lmux status` prints `displays.count` + lists connectors

## 10. Documentation

- [ ] 10.1 README section: "Multi-display support and the hotplug migration policy"
- [ ] 10.2 BUILD.md: note that the KWin backend needs the resident script updated for `screensChanged` (same file, extended section)
- [ ] 10.3 Add a short "what happens when I unplug my monitor?" FAQ entry
- [ ] 10.4 ADR follow-up (if needed) describing the display-id stability scheme
