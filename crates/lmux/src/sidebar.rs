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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::pango::prelude::{FontFamilyExt, FontMapExt};
use gtk4::prelude::*;
use gtk4::{
    gdk, glib, Align, Box as GtkBox, Button, DragSource, DropDown, DropTarget, Entry,
    EventControllerMotion, Label, ListBox, Orientation, Paned, Picture, Popover, PositionType,
    ScrolledWindow, StringObject,
};

use lmux_config::{Sidebar as SidebarCfg, SidebarPosition};
#[cfg(target_os = "macos")]
use lmux_macos_helper::WindowInfo as MacosWindowInfo;
#[cfg(target_os = "macos")]
use lmux_macos_helper::WindowPreview as MacosWindowPreview;

use crate::layout::PaneId;
use crate::pane::ShortcutPrefixCell;
use crate::state::SharedAppState;

type ActiveRows = Rc<RefCell<HashMap<PaneId, ActiveRow>>>;

#[derive(Clone)]
struct ActiveRow {
    row: gtk4::Widget,
    dot: Label,
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct MacosWindowPickerItem {
    window: MacosWindowInfo,
    attached: Option<(PaneId, String)>,
    attached_here: bool,
}

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
    let mut expanded_only: Vec<gtk4::Widget> = vec![title.clone().upcast()];

    // "+" button: spawn a fresh pane and tag it as a new anchor in its own
    // workspace. Needed because `add_anchor` refuses to promote satellites —
    // without this the user has no way to create a second anchor once every
    // pane is already owned.
    let new_anchor_btn = Button::from_icon_name("list-add-symbolic");
    new_anchor_btn.add_css_class("flat");
    new_anchor_btn.set_tooltip_text(Some("New anchor"));
    header.append(&new_anchor_btn);
    expanded_only.push(new_anchor_btn.clone().upcast());
    let state_for_new = state.clone();
    new_anchor_btn.connect_clicked(move |_| {
        state_for_new.borrow_mut().create_new_anchor();
    });

    #[cfg(not(target_os = "macos"))]
    {
        // Launcher button: spotlight-style popover that scans installed GUI apps
        // for the current platform and spawns the chosen one as a satellite.
        let launcher_btn = Button::from_icon_name("system-search-symbolic");
        launcher_btn.add_css_class("flat");
        launcher_btn.set_tooltip_text(Some("Launch program (Ctrl+B l)"));
        header.append(&launcher_btn);
        expanded_only.push(launcher_btn.clone().upcast());
        let state_for_launcher = state.clone();
        launcher_btn.connect_clicked(move |btn| {
            if let Some(root) = btn.root() {
                if let Ok(win) = root.downcast::<gtk4::ApplicationWindow>() {
                    crate::launcher::open(&win, &state_for_launcher);
                }
            }
        });
    }
    #[cfg(target_os = "macos")]
    {
        let attach_btn = Button::from_icon_name("insert-link-symbolic");
        attach_btn.add_css_class("flat");
        attach_btn.set_tooltip_text(Some("Attach macOS window"));
        header.append(&attach_btn);
        expanded_only.push(attach_btn.clone().upcast());
        let state_for_attach = state.clone();
        attach_btn.connect_clicked(move |btn| {
            if let Some(root) = btn.root() {
                if let Ok(win) = root.downcast::<gtk4::ApplicationWindow>() {
                    open_macos_attach_picker(&win, &state_for_attach);
                }
            }
        });
    }
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
    expanded_only.push(scroll.clone().upcast());

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

    // Collapsed state is a compact rail. Hovering the rail temporarily expands
    // it so the anchor list remains quickly reachable without permanently
    // taking horizontal space from the workspace.
    let collapsed = Rc::new(Cell::new(cfg.collapsed));
    let hover_expanded = Rc::new(Cell::new(false));
    let expanded_only = Rc::new(expanded_only);
    apply_collapsed(
        &sidebar_box,
        &paned,
        &cfg,
        collapsed.get(),
        hover_expanded.get(),
        &expanded_only,
    );

    let sb_for_btn = sidebar_box.clone();
    let paned_for_btn = paned.clone();
    let cfg_for_btn = cfg.clone();
    let collapsed_for_btn = collapsed.clone();
    let hover_for_btn = hover_expanded.clone();
    let expanded_only_for_btn = expanded_only.clone();
    collapse_btn.connect_clicked(move |_| {
        collapsed_for_btn.set(!collapsed_for_btn.get());
        hover_for_btn.set(false);
        apply_collapsed(
            &sb_for_btn,
            &paned_for_btn,
            &cfg_for_btn,
            collapsed_for_btn.get(),
            hover_for_btn.get(),
            &expanded_only_for_btn,
        );
    });

    let hover = EventControllerMotion::new();
    let sb_for_enter = sidebar_box.clone();
    let paned_for_enter = paned.clone();
    let cfg_for_enter = cfg.clone();
    let collapsed_for_enter = collapsed.clone();
    let hover_for_enter = hover_expanded.clone();
    let expanded_only_for_enter = expanded_only.clone();
    hover.connect_enter(move |_, _, _| {
        if collapsed_for_enter.get() {
            hover_for_enter.set(true);
            apply_collapsed(
                &sb_for_enter,
                &paned_for_enter,
                &cfg_for_enter,
                collapsed_for_enter.get(),
                hover_for_enter.get(),
                &expanded_only_for_enter,
            );
        }
    });
    let sb_for_leave = sidebar_box.clone();
    let paned_for_leave = paned.clone();
    let cfg_for_leave = cfg.clone();
    let collapsed_for_leave = collapsed.clone();
    let hover_for_leave = hover_expanded.clone();
    let expanded_only_for_leave = expanded_only.clone();
    hover.connect_leave(move |_| {
        hover_for_leave.set(false);
        apply_collapsed(
            &sb_for_leave,
            &paned_for_leave,
            &cfg_for_leave,
            collapsed_for_leave.get(),
            hover_for_leave.get(),
            &expanded_only_for_leave,
        );
    });
    sidebar_box.add_controller(hover);

    // Initial fill + install the refresh hook on AppState.
    let preview_cfg = PreviewCfg {
        enabled: cfg.preview_enabled,
        refresh_ms: cfg.preview_refresh_ms,
    };
    let active_rows: ActiveRows = Rc::new(RefCell::new(HashMap::new()));
    refresh_list(&list, &state, preview_cfg, &active_rows);
    let list_for_cb = list.clone();
    let state_for_cb = state.clone();
    let active_rows_for_cb = active_rows.clone();
    state
        .borrow_mut()
        .set_anchors_changed_callback(Rc::new(move || {
            refresh_list(
                &list_for_cb,
                &state_for_cb,
                preview_cfg,
                &active_rows_for_cb,
            );
        }));
    let active_rows_for_active = active_rows.clone();
    state
        .borrow_mut()
        .add_active_anchor_changed_callback(Rc::new(move |active| {
            update_active_rows(&active_rows_for_active, active);
        }));

    paned.upcast()
}

fn apply_collapsed(
    sidebar: &GtkBox,
    paned: &Paned,
    cfg: &SidebarCfg,
    collapsed: bool,
    hover_expanded: bool,
    expanded_only: &[gtk4::Widget],
) {
    let expanded = !collapsed || hover_expanded;
    let width = if expanded {
        cfg.width
    } else {
        cfg.collapsed_width
    }
    .max(32) as i32;
    sidebar.set_width_request(width);
    for widget in expanded_only {
        widget.set_visible(expanded);
    }
    match cfg.position {
        SidebarPosition::Left => paned.set_position(width),
        SidebarPosition::Right => {
            let paned_width = paned.width();
            if paned_width > width {
                paned.set_position(paned_width - width);
            }
        }
    };
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
fn refresh_list(
    list: &ListBox,
    state: &SharedAppState,
    preview: PreviewCfg,
    active_rows: &ActiveRows,
) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    active_rows.borrow_mut().clear();
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
    sort_sidebar_rows(&mut rows);
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
            active_rows.clone(),
        );
        list.append(&row);
    }
}

fn sort_sidebar_rows(rows: &mut [(Option<String>, i64, PaneId, String)]) {
    rows.sort_by(|a, b| {
        let group_cmp = match (&a.0, &b.0) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        group_cmp
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
}

fn update_active_rows(active_rows: &ActiveRows, active: Option<PaneId>) {
    for (pane_id, active_row) in active_rows.borrow().iter() {
        let is_active = Some(*pane_id) == active;
        if is_active {
            active_row.row.add_css_class("lmux-sidebar__row--active");
        } else {
            active_row.row.remove_css_class("lmux-sidebar__row--active");
        }
        active_row.dot.set_visible(is_active);
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
    active_rows: ActiveRows,
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

    let dot = Label::new(Some("●"));
    dot.add_css_class("lmux-sidebar__active-dot");
    dot.set_visible(is_active);
    header_row.append(&dot);
    active_rows.borrow_mut().insert(
        pane_id,
        ActiveRow {
            row: row.clone().upcast(),
            dot: dot.clone(),
        },
    );

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
    let initial = {
        let state = state.borrow();
        if !state.pane_in_active_workspace(pane_id) || state.is_anchor_hidden(pane_id) {
            None
        } else {
            state.pane_thumbnail(pane_id)
        }
    };
    if let Some((cols, rows, bytes)) = initial {
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
            let snapshot = {
                let state = state.borrow();
                if !state.pane_in_active_workspace(pane_id) || state.is_anchor_hidden(pane_id) {
                    None
                } else {
                    state.pane_thumbnail(pane_id)
                }
            };
            if let Some((cols, rows, bytes)) = snapshot {
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

#[cfg(target_os = "macos")]
fn open_macos_attach_picker(parent: &gtk4::ApplicationWindow, state: &SharedAppState) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Attach window")
        .default_width(640)
        .default_height(460)
        .build();

    let body = GtkBox::new(Orientation::Vertical, 8);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(10);
    body.set_margin_end(10);

    let heading = Label::new(Some("Attach window"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    body.append(&heading);

    let list = ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.set_activate_on_single_click(true);
    list.add_css_class("lmux-window-picker");

    match lmux_macos_helper::list_windows(None, None) {
        Ok(windows) if windows.is_empty() => {
            let empty = Label::new(Some("No windows found"));
            empty.set_xalign(0.0);
            empty.add_css_class("dim-label");
            list.append(&empty);
        }
        Ok(windows) => {
            let items = Rc::new(macos_window_picker_items(windows, state));
            for item in items.iter() {
                let row = gtk4::ListBoxRow::new();
                row.set_activatable(true);
                row.set_selectable(false);
                row.set_child(Some(&macos_window_picker_row(item)));
                list.append(&row);
            }

            let state_for_activate = state.clone();
            let items_for_activate = items.clone();
            let dialog_for_activate = dialog.clone();
            list.connect_row_activated(move |_, row| {
                let Ok(index) = usize::try_from(row.index()) else {
                    return;
                };
                let Some(item) = items_for_activate.get(index).cloned() else {
                    return;
                };
                if let Err(err) = state_for_activate
                    .borrow_mut()
                    .attach_macos_window_to_active_anchor(item.window)
                {
                    tracing::warn!(error = %err, "attach selected macOS window failed");
                    return;
                }
                dialog_for_activate.close();
            });
        }
        Err(err) => {
            tracing::warn!(error = %err, "macOS window picker failed to list windows");
            let error = Label::new(Some("Could not list windows"));
            error.set_xalign(0.0);
            error.add_css_class("dim-label");
            list.append(&error);
        }
    }

    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scroll.set_child(Some(&list));
    body.append(&scroll);

    let footer = GtkBox::new(Orientation::Horizontal, 6);
    footer.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    footer.append(&cancel);
    body.append(&footer);

    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| dialog_for_cancel.close());

    dialog.set_child(Some(&body));
    dialog.present();
}

#[cfg(target_os = "macos")]
fn macos_window_picker_items(
    windows: Vec<MacosWindowInfo>,
    state: &SharedAppState,
) -> Vec<MacosWindowPickerItem> {
    let state = state.borrow();
    let active_anchor = state.active_anchor();
    let mut items: Vec<_> = windows
        .into_iter()
        .map(|window| {
            let attached = state.macos_attached_anchor_for_window(&window);
            let attached_here = attached
                .as_ref()
                .map(|(anchor, _)| Some(*anchor) == active_anchor)
                .unwrap_or(false);
            MacosWindowPickerItem {
                window,
                attached,
                attached_here,
            }
        })
        .collect();
    items.sort_by(|a, b| {
        macos_window_picker_item_rank(a)
            .cmp(&macos_window_picker_item_rank(b))
            .then_with(|| a.window.pid.cmp(&b.window.pid))
            .then_with(|| a.window.window_index.cmp(&b.window.window_index))
            .then_with(|| macos_window_title(&a.window).cmp(&macos_window_title(&b.window)))
    });
    items
}

#[cfg(target_os = "macos")]
fn macos_window_picker_item_rank(item: &MacosWindowPickerItem) -> u8 {
    if item.attached_here {
        0
    } else if item.attached.is_some() {
        1
    } else {
        2
    }
}

#[cfg(target_os = "macos")]
fn macos_window_picker_row(item: &MacosWindowPickerItem) -> gtk4::Widget {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    row.add_css_class("lmux-window-picker__row");
    if item.attached_here {
        row.add_css_class("lmux-window-picker__row--attached-active");
    } else if item.attached.is_some() {
        row.add_css_class("lmux-window-picker__row--attached-other");
    }
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    row.set_margin_start(6);
    row.set_margin_end(6);

    row.append(&macos_window_preview_tile(&item.window));

    let text = GtkBox::new(Orientation::Vertical, 3);
    text.set_hexpand(true);
    text.set_valign(Align::Center);

    let title = Label::new(Some(&macos_window_title(&item.window)));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    text.append(&title);

    let meta = Label::new(Some(&macos_window_meta(&item.window)));
    meta.set_xalign(0.0);
    meta.add_css_class("lmux-window-picker__meta");
    meta.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    text.append(&meta);

    if let Some((_, label)) = &item.attached {
        let attached_text = if item.attached_here {
            "Active anchor".to_string()
        } else {
            format!("Other anchor: {label}")
        };
        let attached = Label::new(Some(&attached_text));
        attached.set_xalign(0.0);
        if item.attached_here {
            attached.add_css_class("lmux-window-picker__attached-active");
        } else {
            attached.add_css_class("lmux-window-picker__attached-other");
        }
        text.append(&attached);
    }

    row.append(&text);

    let attach_text = if item.attached_here {
        "Active"
    } else if item.attached.is_some() {
        "Move here"
    } else {
        "Attach"
    };
    let attach = Label::new(Some(attach_text));
    if item.attached_here {
        attach.add_css_class("lmux-window-picker__attached-active");
    } else if item.attached.is_some() {
        attach.add_css_class("lmux-window-picker__attached-other");
    } else {
        attach.add_css_class("dim-label");
    }
    attach.set_valign(Align::Center);
    row.append(&attach);

    row.upcast()
}

#[cfg(target_os = "macos")]
fn macos_window_preview_tile(window: &MacosWindowInfo) -> gtk4::Widget {
    let tile = GtkBox::new(Orientation::Vertical, 0);
    tile.add_css_class("lmux-window-picker__preview");
    tile.set_size_request(118, 66);
    tile.set_valign(Align::Center);

    match lmux_macos_helper::window_preview(window, 118, 66) {
        Ok(Some(preview)) => {
            let picture = Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Contain);
            picture.set_paintable(Some(&bgra_texture(preview)));
            tile.append(&picture);
        }
        Ok(None) => {
            macos_window_preview_fallback(&tile, window);
        }
        Err(err) => {
            tracing::debug!(error = %err, "macOS window preview capture failed");
            macos_window_preview_fallback(&tile, window);
        }
    }

    tile.upcast()
}

#[cfg(target_os = "macos")]
fn macos_window_preview_fallback(tile: &GtkBox, window: &MacosWindowInfo) {
    tile.add_css_class("lmux-window-picker__preview--missing");
    let text = format!("{}\nNo preview", macos_window_initials(window));
    let fallback = Label::new(Some(&text));
    fallback.add_css_class("lmux-window-picker__preview-text");
    fallback.set_halign(Align::Center);
    fallback.set_valign(Align::Center);
    fallback.set_justify(gtk4::Justification::Center);
    fallback.set_hexpand(true);
    fallback.set_vexpand(true);
    tile.append(&fallback);
}

#[cfg(target_os = "macos")]
fn bgra_texture(preview: MacosWindowPreview) -> gdk::MemoryTexture {
    let glib_bytes = glib::Bytes::from_owned(preview.bgra);
    gdk::MemoryTexture::new(
        preview.width as i32,
        preview.height as i32,
        gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &glib_bytes,
        preview.bytes_per_row,
    )
}

#[cfg(target_os = "macos")]
fn macos_window_title(window: &MacosWindowInfo) -> String {
    window
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("(untitled window)")
        .to_string()
}

#[cfg(target_os = "macos")]
fn macos_window_meta(window: &MacosWindowInfo) -> String {
    let app = window
        .bundle_id
        .as_deref()
        .and_then(|bundle| bundle.rsplit('.').next())
        .filter(|name| !name.is_empty())
        .unwrap_or("macOS");
    let id = window
        .window_id
        .map(|id| format!("id {id}"))
        .unwrap_or_else(|| "no id".to_string());
    format!(
        "{app} · pid {} · window {} · {id}",
        window.pid, window.window_index
    )
}

#[cfg(target_os = "macos")]
fn macos_window_initials(window: &MacosWindowInfo) -> String {
    let source = window
        .bundle_id
        .as_deref()
        .and_then(|bundle| bundle.rsplit('.').next())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| macos_window_title(window));
    let mut initials = String::new();
    for word in source.split(|c: char| !c.is_alphanumeric()) {
        if let Some(ch) = word.chars().next() {
            initials.extend(ch.to_uppercase());
        }
        if initials.chars().count() >= 2 {
            break;
        }
    }
    if initials.is_empty() {
        "W".to_string()
    } else {
        initials
    }
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

pub(crate) fn open_settings_dialog(
    parent: &gtk4::ApplicationWindow,
    state: &SharedAppState,
    shortcut_prefix: ShortcutPrefixCell,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Settings")
        .default_width(420)
        .default_height(420)
        .build();

    let body = GtkBox::new(Orientation::Vertical, 8);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let heading = Label::new(Some("Settings"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    body.append(&heading);

    match load_settings_config() {
        Ok((cfg, path)) => {
            append_settings_controls(&body, &dialog, state, cfg, path, shortcut_prefix)
        }
        Err(message) => append_settings_error(&body, &dialog, &message),
    }

    let scroll = ScrolledWindow::new();
    scroll.set_min_content_width(420);
    scroll.set_min_content_height(320);
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scroll.set_child(Some(&body));

    dialog.set_child(Some(&scroll));
    dialog.present();
}

fn load_settings_config() -> Result<(lmux_config::Config, PathBuf), String> {
    let path =
        lmux_config::config_path().ok_or_else(|| "Could not find a config path".to_string())?;
    let cfg = lmux_config::load(&path).map_err(|err| format!("Could not load config: {err}"))?;
    Ok((cfg, path))
}

fn append_settings_controls(
    body: &GtkBox,
    dialog: &gtk4::Window,
    state: &SharedAppState,
    cfg: lmux_config::Config,
    path: PathBuf,
    shortcut_prefix: ShortcutPrefixCell,
) {
    let font_label = Label::new(Some("Font family"));
    font_label.set_xalign(0.0);
    font_label.add_css_class("dim-label");
    body.append(&font_label);

    let font_combo = build_font_combo(body, &cfg.general.font_family);
    body.append(&font_combo);

    let size_label = Label::new(Some("Font size"));
    size_label.set_xalign(0.0);
    size_label.add_css_class("dim-label");
    body.append(&size_label);

    let size_spin = gtk4::SpinButton::with_range(6.0, 48.0, 1.0);
    size_spin.set_numeric(true);
    size_spin.set_digits(0);
    size_spin.set_value(cfg.general.font_size.clamp(6, 48) as f64);
    body.append(&size_spin);

    let keymap_heading = Label::new(Some("Keyboard shortcuts"));
    keymap_heading.set_xalign(0.0);
    keymap_heading.add_css_class("heading");
    keymap_heading.set_margin_top(8);
    body.append(&keymap_heading);

    let prefix_label = Label::new(Some("Prefix"));
    prefix_label.set_xalign(0.0);
    prefix_label.add_css_class("dim-label");
    body.append(&prefix_label);

    let prefix_entry = Entry::new();
    prefix_entry.set_text(&cfg.keymap.prefix);
    prefix_entry.set_placeholder_text(Some("ctrl+b"));
    body.append(&prefix_entry);

    let prefix_error = Label::new(None);
    prefix_error.set_xalign(0.0);
    prefix_error.set_wrap(true);
    prefix_error.add_css_class("error");
    prefix_error.set_visible(false);
    body.append(&prefix_error);

    append_shortcut_hint(body, "Split right", "|", &cfg.keymap.prefix);
    append_shortcut_hint(body, "Split down", "-", &cfg.keymap.prefix);
    append_shortcut_hint(body, "Close pane", "x", &cfg.keymap.prefix);
    append_shortcut_hint(body, "Next pane", "o", &cfg.keymap.prefix);
    append_shortcut_hint(body, "Previous pane", "p", &cfg.keymap.prefix);
    append_shortcut_hint(body, "Rearrange mode", "m", &cfg.keymap.prefix);

    let footer = GtkBox::new(Orientation::Horizontal, 6);
    footer.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let apply = Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    footer.append(&cancel);
    footer.append(&apply);
    body.append(&footer);

    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| dialog_for_cancel.close());

    let cfg_for_apply = cfg.clone();
    let path_for_apply = path.clone();
    let state_for_apply = state.clone();
    let dialog_for_apply = dialog.clone();
    let font_combo_apply = font_combo.clone();
    let size_spin_apply = size_spin.clone();
    let prefix_entry_apply = prefix_entry.clone();
    let prefix_error_apply = prefix_error.clone();
    let shortcut_prefix_apply = shortcut_prefix;
    apply.connect_clicked(move |_| {
        let applied = apply_settings_config(
            cfg_for_apply.clone(),
            path_for_apply.clone(),
            &state_for_apply,
            &font_combo_apply,
            &size_spin_apply,
            &prefix_entry_apply,
            &prefix_error_apply,
            &shortcut_prefix_apply,
        );
        if applied {
            dialog_for_apply.close();
        }
    });
}

fn apply_settings_config(
    mut cfg: lmux_config::Config,
    path: PathBuf,
    state: &SharedAppState,
    font_combo: &DropDown,
    size_spin: &gtk4::SpinButton,
    prefix_entry: &Entry,
    prefix_error: &Label,
    shortcut_prefix: &ShortcutPrefixCell,
) -> bool {
    if let Some(family) = selected_font_family(font_combo) {
        cfg.general.font_family = family;
    }
    cfg.general.font_size = size_spin.value_as_int().clamp(6, 48) as u32;
    let prefix = prefix_entry.text().trim().to_string();
    if !prefix.is_empty() {
        if !crate::app::is_valid_prefix_binding(&prefix) {
            prefix_error.set_text("Use a prefix like ctrl+b, ctrl+shift+k, alt+x, or cmd+k.");
            prefix_error.set_visible(true);
            prefix_entry.grab_focus();
            return false;
        }
        prefix_error.set_visible(false);
        cfg.keymap.prefix = prefix.clone();
        *shortcut_prefix.borrow_mut() = prefix;
    }
    state.borrow().apply_config(&cfg);
    if let Err(err) = lmux_config::save(&path, &cfg) {
        tracing::warn!(error = %err, path = %path.display(), "settings: config save failed");
    }
    true
}

fn append_shortcut_hint(body: &GtkBox, action: &str, key: &str, prefix: &str) {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    let label = Label::new(Some(action));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);

    let shortcut = Label::new(Some(&format!("{} {}", prefix.trim(), key)));
    shortcut.add_css_class("dim-label");
    shortcut.add_css_class("monospace");
    row.append(&shortcut);
    body.append(&row);
}

fn build_font_combo(source: &GtkBox, current_family: &str) -> DropDown {
    let mut families = system_font_families(source);
    if !families.iter().any(|family| family == current_family) {
        families.insert(0, current_family.to_string());
    }
    let family_refs: Vec<_> = families.iter().map(String::as_str).collect();
    let combo = DropDown::from_strings(&family_refs);
    combo.set_hexpand(true);
    combo.set_enable_search(true);
    combo.set_tooltip_text(Some("System font family"));
    if let Some(index) = families.iter().position(|family| family == current_family) {
        combo.set_selected(index as u32);
    }
    combo
}

fn system_font_families(source: &GtkBox) -> Vec<String> {
    let Some(font_map) = source.pango_context().font_map() else {
        return Vec::new();
    };
    let mut families: Vec<_> = font_map
        .list_families()
        .into_iter()
        .map(|family| family.name().to_string())
        .filter(|name| !name.trim().is_empty())
        .collect();
    families.sort_by_key(|family| family.to_ascii_lowercase());
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    families
}

fn selected_font_family(font_combo: &DropDown) -> Option<String> {
    font_combo
        .selected_item()
        .and_then(|item| item.downcast::<StringObject>().ok())
        .map(|item| item.string().trim().to_string())
        .filter(|family| !family.is_empty())
}

fn append_settings_error(body: &GtkBox, dialog: &gtk4::Window, message: &str) {
    let error = Label::new(Some(message));
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("dim-label");
    body.append(&error);

    let footer = GtkBox::new(Orientation::Horizontal, 6);
    footer.set_halign(Align::End);
    let close = Button::with_label("Close");
    footer.append(&close);
    body.append(&footer);

    let dialog_for_close = dialog.clone();
    close.connect_clicked(move |_| dialog_for_close.close());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_sort_tie_breaker_ignores_label() {
        let mut rows = vec![
            (None, 0, 2, "Alpha".to_string()),
            (None, 0, 1, "Zulu".to_string()),
        ];

        sort_sidebar_rows(&mut rows);

        let pane_ids: Vec<PaneId> = rows.into_iter().map(|(_, _, pane_id, _)| pane_id).collect();
        assert_eq!(pane_ids, vec![1, 2]);
    }

    #[test]
    fn sidebar_sort_places_newer_higher_sort_key_later() {
        let mut rows = vec![
            (None, 2, 3, "Newest".to_string()),
            (None, 0, 1, "First".to_string()),
            (None, 1, 2, "Second".to_string()),
        ];

        sort_sidebar_rows(&mut rows);

        let pane_ids: Vec<PaneId> = rows.into_iter().map(|(_, _, pane_id, _)| pane_id).collect();
        assert_eq!(pane_ids, vec![1, 2, 3]);
    }
}
