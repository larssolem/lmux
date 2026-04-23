//! Always-on anchor sidebar (Epic 5, v1 — static list, no preview).
//!
//! Layout:
//!
//! ```text
//!   ┌──────────┬─────────────────────────────┐
//!   │  ◂       │                             │
//!   │ Anchors  │                             │
//!   │  ▸ Build │      pane tree              │
//!   │    ▸ svr │                             │
//!   └──────────┴─────────────────────────────┘
//! ```
//!
//! The sidebar is a left (or right, per config) column of a horizontal
//! [`gtk4::Paned`]. The right child is the existing pane tree root. The
//! sidebar rebuilds itself every time [`AppState::add_anchor`] /
//! [`AppState::remove_anchor`] fires via the `set_anchors_changed_callback`
//! hook installed in [`install`].

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    gdk, glib, Align, Box as GtkBox, Button, DragSource, DropTarget, Entry, Label, ListBox,
    Orientation, Paned, Picture, Popover, PositionType, ScrolledWindow,
};

use lmux_config::{Sidebar as SidebarCfg, SidebarPosition};

use crate::layout::PaneId;
use crate::state::SharedAppState;

/// Install the sidebar around `pane_tree_root`. Returns the outer `Paned`
/// widget the caller should set as the window's child.
pub fn install(cfg: SidebarCfg, pane_tree_root: GtkBox, state: SharedAppState) -> gtk4::Widget {
    let sidebar_box = GtkBox::new(Orientation::Vertical, 4);
    sidebar_box.add_css_class("lmux-sidebar");
    sidebar_box.set_width_request(cfg.width as i32);

    // Header row: collapse toggle + label.
    let header = GtkBox::new(Orientation::Horizontal, 4);
    header.add_css_class("lmux-sidebar__header");
    let collapse_btn = Button::from_icon_name("pan-start-symbolic");
    collapse_btn.add_css_class("flat");
    header.append(&collapse_btn);
    let title = Label::new(Some("Anchors"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    // "+" button: spawn a fresh pane and tag it as a new anchor in its own
    // workspace. Needed because `add_anchor` refuses to promote satellites —
    // without this the user has no way to create a second anchor once every
    // pane is already owned.
    let new_anchor_btn = Button::from_icon_name("list-add-symbolic");
    new_anchor_btn.add_css_class("flat");
    new_anchor_btn.set_tooltip_text(Some("New anchor"));
    header.append(&new_anchor_btn);
    let state_for_new = state.clone();
    new_anchor_btn.connect_clicked(move |_| {
        state_for_new.borrow_mut().create_new_anchor();
    });
    // Launcher button: spotlight-style popover that scans installed
    // .desktop entries and spawns the chosen one as a satellite.
    let launcher_btn = Button::from_icon_name("system-search-symbolic");
    launcher_btn.add_css_class("flat");
    launcher_btn.set_tooltip_text(Some("Launch program (Ctrl+B l)"));
    header.append(&launcher_btn);
    let state_for_launcher = state.clone();
    launcher_btn.connect_clicked(move |btn| {
        if let Some(root) = btn.root() {
            if let Ok(win) = root.downcast::<gtk4::ApplicationWindow>() {
                crate::launcher::open(&win, &state_for_launcher);
            }
        }
    });
    sidebar_box.append(&header);

    // Anchor list inside a scroller.
    let list = ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("lmux-sidebar__list");
    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scroll.set_child(Some(&list));
    sidebar_box.append(&scroll);

    // Horizontal split: sidebar + pane tree. Order depends on config.
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_wide_handle(true);
    paned.set_resize_start_child(false);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    match cfg.position {
        SidebarPosition::Left => {
            paned.set_start_child(Some(&sidebar_box));
            paned.set_end_child(Some(&pane_tree_root));
            paned.set_position(cfg.width as i32);
        }
        SidebarPosition::Right => {
            paned.set_start_child(Some(&pane_tree_root));
            paned.set_end_child(Some(&sidebar_box));
            // Position set after allocation; handled below.
        }
    }

    // Collapsed state is a simple width swap — the widget stays visible but
    // renders at `collapsed_width` with its label hidden.
    let collapsed = Rc::new(RefCell::new(cfg.collapsed));
    apply_collapsed(&sidebar_box, &title, &cfg, *collapsed.borrow());

    let sb_for_btn = sidebar_box.clone();
    let title_for_btn = title.clone();
    let cfg_for_btn = cfg.clone();
    let collapsed_for_btn = collapsed.clone();
    collapse_btn.connect_clicked(move |_| {
        let mut c = collapsed_for_btn.borrow_mut();
        *c = !*c;
        apply_collapsed(&sb_for_btn, &title_for_btn, &cfg_for_btn, *c);
    });

    // Initial fill + install the refresh hook on AppState.
    let preview_cfg = PreviewCfg {
        enabled: cfg.preview_enabled,
        refresh_ms: cfg.preview_refresh_ms,
    };
    refresh_list(&list, &state, preview_cfg);
    let list_for_cb = list.clone();
    let state_for_cb = state.clone();
    state
        .borrow_mut()
        .set_anchors_changed_callback(Rc::new(move || {
            refresh_list(&list_for_cb, &state_for_cb, preview_cfg);
        }));

    paned.upcast()
}

fn apply_collapsed(sidebar: &GtkBox, title: &Label, cfg: &SidebarCfg, collapsed: bool) {
    if collapsed {
        sidebar.set_width_request(cfg.collapsed_width as i32);
        title.set_visible(false);
    } else {
        sidebar.set_width_request(cfg.width as i32);
        title.set_visible(true);
    }
}

#[derive(Clone, Copy)]
struct PreviewCfg {
    enabled: bool,
    refresh_ms: u32,
}

/// Wipe and repopulate the list from the current anchor registry + pane
/// set. v1 is flat + grouped — each group becomes a subheader followed by
/// its anchor rows. Within a group rows are ordered by `sort_key` (ASC),
/// then display label, so drag-to-reorder writes survive a refresh.
fn refresh_list(list: &ListBox, state: &SharedAppState, preview: PreviewCfg) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    let s = state.borrow();
    // Build (group, sort_key, pane_id, label) tuples so we can sort by the
    // registry's manual ordering ahead of label.
    let mut rows: Vec<(Option<String>, i64, PaneId, String)> = Vec::new();
    for pane_id in s.anchors().iter().copied() {
        let (group, sort_key, label) = match s.anchor_for_pane(pane_id) {
            Some(a) => (
                a.group.clone(),
                a.sort_key.unwrap_or(0),
                a.display_label().to_string(),
            ),
            None => (None, 0, format!("pane {pane_id}")),
        };
        rows.push((group, sort_key, pane_id, label));
    }
    rows.sort_by(|a, b| {
        let group_cmp = match (&a.0, &b.0) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        group_cmp
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.3.cmp(&b.3))
    });
    drop(s);

    if rows.is_empty() {
        let empty = Label::new(Some("(no anchors)\nCtrl+B a to tag"));
        empty.set_justify(gtk4::Justification::Center);
        empty.set_xalign(0.5);
        empty.add_css_class("dim-label");
        list.append(&empty);
        return;
    }

    // Per-group ordered pane ids, shared with the DnD handler on each row so
    // a drop can rewrite the group's sort_keys in one call.
    let mut group_order: std::collections::HashMap<Option<String>, Vec<PaneId>> =
        std::collections::HashMap::new();
    for (group, _, pane_id, _) in &rows {
        group_order.entry(group.clone()).or_default().push(*pane_id);
    }
    let group_order = Rc::new(group_order);

    let mut last_group: Option<Option<String>> = None;
    for (group, _sort_key, pane_id, label) in rows {
        if last_group.as_ref() != Some(&group) {
            let header_label = group.clone().unwrap_or_else(|| "Ungrouped".to_string());
            let header = Label::new(Some(&header_label));
            header.set_xalign(0.0);
            header.add_css_class("heading");
            header.add_css_class("dim-label");
            list.append(&header);
            last_group = Some(group.clone());
        }
        let (current_name, is_active) = {
            let s = state.borrow();
            (
                s.anchor_for_pane(pane_id)
                    .and_then(|a| a.name.clone())
                    .unwrap_or_default(),
                s.active_anchor() == Some(pane_id),
            )
        };
        let current_group = group.clone().unwrap_or_default();
        let row = build_row(
            pane_id,
            &label,
            current_name,
            current_group,
            group.clone(),
            group_order.clone(),
            is_active,
            state.clone(),
            preview,
        );
        list.append(&row);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_row(
    pane_id: PaneId,
    label: &str,
    current_name: String,
    current_group: String,
    group_key: Option<String>,
    group_order: Rc<std::collections::HashMap<Option<String>, Vec<PaneId>>>,
    is_active: bool,
    state: SharedAppState,
    preview: PreviewCfg,
) -> gtk4::Widget {
    let row = GtkBox::new(Orientation::Vertical, 2);
    row.add_css_class("lmux-sidebar__row");
    if is_active {
        row.add_css_class("lmux-sidebar__row--active");
    }
    row.set_margin_start(8);
    row.set_margin_end(4);
    row.set_margin_top(2);
    row.set_margin_bottom(2);

    let header_row = GtkBox::new(Orientation::Horizontal, 6);
    row.append(&header_row);

    // Drag source: publish this pane_id as a u32 so a same-group drop can
    // reorder. Any drop outside the sidebar is ignored by the target.
    let drag_source = DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE);
    drag_source
        .connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&pane_id.to_value())));
    row.add_controller(drag_source);

    // Drop target: accept a u32 pane_id, insert it before this row's pane_id
    // in the group's new order, then rewrite sort_keys for every pane in
    // that group. Cross-group drops keep the dragged pane in its original
    // group (v0.2: regroup is a popover action, not DnD).
    let drop_target = DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
    let drop_state = state.clone();
    let drop_group_key = group_key.clone();
    let drop_group_order = group_order.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        let Ok(src_pane) = value.get::<u32>() else {
            return false;
        };
        if src_pane == pane_id {
            return false;
        }
        let Some(order) = drop_group_order.get(&drop_group_key).cloned() else {
            return false;
        };
        if !order.contains(&src_pane) || !order.contains(&pane_id) {
            // Dragged row lives in a different group — ignore for v0.2.
            return false;
        }
        let mut new_order: Vec<PaneId> = order.into_iter().filter(|p| *p != src_pane).collect();
        let insert_at = new_order
            .iter()
            .position(|p| *p == pane_id)
            .unwrap_or(new_order.len());
        new_order.insert(insert_at, src_pane);
        drop_state.borrow_mut().reorder_anchors_in_group(&new_order);
        true
    });
    row.add_controller(drop_target);

    let id_badge = Label::new(Some(&format!("{pane_id}")));
    id_badge.add_css_class("dim-label");
    id_badge.set_width_chars(2);
    header_row.append(&id_badge);

    let title = Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    header_row.append(&title);

    if is_active {
        let dot = Label::new(Some("●"));
        dot.add_css_class("lmux-sidebar__active-dot");
        header_row.append(&dot);
    }

    if preview.enabled {
        attach_preview(&row, pane_id, &state, preview.refresh_ms);
    }

    // Left-click promotes this anchor to active — the displayed pane on
    // screen. Right-click opens the rename/group popover. GTK's
    // GestureClick fires per-button, so we install two.
    let activate_click = gtk4::GestureClick::new();
    activate_click.set_button(gtk4::gdk::BUTTON_PRIMARY);
    let state_activate = state.clone();
    activate_click.connect_released(move |_, _, _, _| {
        state_activate.borrow_mut().set_active_anchor(Some(pane_id));
    });
    row.add_controller(activate_click);

    let menu_click = gtk4::GestureClick::new();
    menu_click.set_button(gtk4::gdk::BUTTON_SECONDARY);
    let row_weak = row.downgrade();
    let state_menu = state.clone();
    let name_initial = current_name.clone();
    let group_initial = current_group.clone();
    menu_click.connect_pressed(move |_, _, _, _| {
        if let Some(row) = row_weak.upgrade() {
            show_row_popover(
                &row,
                pane_id,
                &name_initial,
                &group_initial,
                state_menu.clone(),
            );
        }
    });
    row.add_controller(menu_click);

    // Touch + trackpad long-press → same popover.
    let long_press = gtk4::GestureLongPress::new();
    let row_weak_lp = row.downgrade();
    let state_lp = state;
    let name_initial_lp = current_name;
    let group_initial_lp = current_group;
    long_press.connect_pressed(move |_, _, _| {
        if let Some(row) = row_weak_lp.upgrade() {
            show_row_popover(
                &row,
                pane_id,
                &name_initial_lp,
                &group_initial_lp,
                state_lp.clone(),
            );
        }
    });
    row.add_controller(long_press);

    row.upcast()
}

/// Install a low-res pane thumbnail under the row header. The `Picture`
/// samples [`AppState::pane_thumbnail`] every `refresh_ms`; when the pane
/// has gone away or the row has been rebuilt (weak upgrade fails) the timer
/// self-terminates. Costs one `Terminal::render` pass per visible row per
/// interval — cheap because the rendered cells are ints, not Cairo ops.
fn attach_preview(row: &GtkBox, pane_id: PaneId, state: &SharedAppState, refresh_ms: u32) {
    let picture = Picture::new();
    picture.add_css_class("lmux-sidebar__preview");
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_height_request(24);
    picture.set_margin_start(20);
    picture.set_margin_end(4);
    row.append(&picture);

    // Render once immediately so the row doesn't flash blank before the
    // first timer tick.
    if let Some((cols, rows, bytes)) = state.borrow().pane_thumbnail(pane_id) {
        picture.set_paintable(Some(&rgb_texture(cols, rows, bytes)));
    }

    let state_weak = Rc::downgrade(state);
    let picture_weak = picture.downgrade();
    glib::timeout_add_local(
        std::time::Duration::from_millis(refresh_ms.max(100) as u64),
        move || {
            let Some(picture) = picture_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(state) = state_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some((cols, rows, bytes)) = state.borrow().pane_thumbnail(pane_id) {
                picture.set_paintable(Some(&rgb_texture(cols, rows, bytes)));
            }
            glib::ControlFlow::Continue
        },
    );
}

fn rgb_texture(cols: u32, rows: u32, bytes: Vec<u8>) -> gdk::MemoryTexture {
    let stride = (cols as usize) * 3;
    let glib_bytes = glib::Bytes::from_owned(bytes);
    gdk::MemoryTexture::new(
        cols as i32,
        rows as i32,
        gdk::MemoryFormat::R8g8b8,
        &glib_bytes,
        stride,
    )
}

fn show_row_popover(
    anchor_widget: &GtkBox,
    pane_id: PaneId,
    current_name: &str,
    current_group: &str,
    state: SharedAppState,
) {
    let popover = Popover::new();
    popover.set_position(PositionType::Right);
    popover.set_has_arrow(true);
    popover.set_autohide(true);

    let body = GtkBox::new(Orientation::Vertical, 6);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);

    let heading = Label::new(Some(&format!("Anchor · pane {pane_id}")));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    body.append(&heading);

    // Surface the UUID so users can copy it into `lmux-cli anchor pause/resume`.
    if let Some(uuid) = state.borrow().anchor_uuid_for_pane(pane_id) {
        let uuid_label = Label::new(Some(&uuid.to_string()));
        uuid_label.set_xalign(0.0);
        uuid_label.add_css_class("dim-label");
        uuid_label.add_css_class("monospace");
        uuid_label.set_selectable(true);
        body.append(&uuid_label);
    }

    let name_label = Label::new(Some("Name"));
    name_label.set_xalign(0.0);
    name_label.add_css_class("dim-label");
    body.append(&name_label);
    let name_entry = Entry::new();
    name_entry.set_text(current_name);
    name_entry.set_placeholder_text(Some("(argv default)"));
    body.append(&name_entry);

    let group_label = Label::new(Some("Group"));
    group_label.set_xalign(0.0);
    group_label.add_css_class("dim-label");
    body.append(&group_label);
    let group_entry = Entry::new();
    group_entry.set_text(current_group);
    group_entry.set_placeholder_text(Some("(ungrouped)"));
    body.append(&group_entry);

    let is_paused = {
        let s = state.borrow();
        s.anchor_for_pane(pane_id)
            .map(|a| matches!(a.state, lmux_anchor::AnchorState::Paused))
            .unwrap_or(false)
    };

    let btn_row = GtkBox::new(Orientation::Horizontal, 6);
    btn_row.set_halign(Align::End);
    let untag_btn = Button::with_label("Untag");
    untag_btn.add_css_class("destructive-action");
    let pause_btn = Button::with_label(if is_paused { "Resume" } else { "Pause" });
    let apply_btn = Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");
    btn_row.append(&untag_btn);
    btn_row.append(&pause_btn);
    btn_row.append(&apply_btn);
    body.append(&btn_row);

    popover.set_child(Some(&body));
    popover.set_parent(anchor_widget);

    let apply_state = state.clone();
    let name_entry_apply = name_entry.clone();
    let group_entry_apply = group_entry.clone();
    let popover_apply = popover.clone();
    let do_apply = move || {
        let name = trim_to_option(&name_entry_apply.text());
        let group = trim_to_option(&group_entry_apply.text());
        let mut s = apply_state.borrow_mut();
        s.rename_anchor_for_pane(pane_id, name);
        s.regroup_anchor_for_pane(pane_id, group);
        drop(s);
        popover_apply.popdown();
    };

    let do_apply_btn = do_apply.clone();
    apply_btn.connect_clicked(move |_| do_apply_btn());

    let do_apply_enter = do_apply.clone();
    name_entry.connect_activate(move |_| do_apply_enter());
    let do_apply_enter2 = do_apply;
    group_entry.connect_activate(move |_| do_apply_enter2());

    let pause_state = state.clone();
    let popover_pause = popover.clone();
    pause_btn.connect_clicked(move |_| {
        let res = {
            let mut s = pause_state.borrow_mut();
            if is_paused {
                s.resume_anchor(pane_id)
            } else {
                s.pause_anchor(pane_id)
            }
        };
        if let Err(err) = res {
            tracing::warn!(error = %err, "anchor pause/resume failed");
        }
        popover_pause.popdown();
    });

    let untag_state = state;
    let popover_untag = popover.clone();
    untag_btn.connect_clicked(move |_| {
        untag_state.borrow_mut().remove_anchor(pane_id);
        popover_untag.popdown();
    });

    // Clean up the popover once dismissed so it doesn't stay parented.
    let popover_cleanup = popover.clone();
    popover.connect_closed(move |_| {
        popover_cleanup.unparent();
    });

    popover.popup();
    name_entry.grab_focus();
}

fn trim_to_option(s: &gtk4::glib::GString) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Dispatcher hook used from `app::activate` when the GTK config reports no
/// sidebar (future: user disables it). Currently never taken — config
/// always produces a `Sidebar` — kept for the eventual opt-out.
#[allow(dead_code)]
pub fn no_sidebar(pane_tree_root: GtkBox) -> gtk4::Widget {
    pane_tree_root.upcast()
}

/// Pull the sidebar config off disk. Falls back to defaults on any error
/// so the cockpit never fails to start because of a malformed TOML.
pub fn load_config() -> SidebarCfg {
    let Some(path) = lmux_config::config_path() else {
        return SidebarCfg::default();
    };
    match lmux_config::load(&path) {
        Ok(cfg) => cfg.sidebar,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "sidebar: config load failed, using defaults");
            SidebarCfg::default()
        }
    }
}

// Silence the unused-import warning when glib isn't referenced directly.
#[allow(dead_code)]
fn _keep_glib_linked() {
    let _ = glib::MainContext::default;
}
