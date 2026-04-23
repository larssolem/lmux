//! Minimal fuzzy session switcher (Epic 4, v0.2 shell).
//!
//! Triggered by `Ctrl+B s`. Lists sessions from [`lmux_session::SessionStore`],
//! filters via simple case-insensitive substring match, and invokes
//! [`crate::state::AppState::switch_session`] on Enter so the layout
//! actually swaps.
//!
//! Keyboard-only: Up/Down to move, Enter to pick, Esc to dismiss.

use gtk4::prelude::*;
use gtk4::{
    gdk, glib, ApplicationWindow, Box as GtkBox, Entry, EventControllerKey, Label, ListBox,
    ListBoxRow, Orientation, Popover, PositionType, ScrolledWindow, SelectionMode,
};

use crate::state::SharedAppState;

/// Open the switcher rooted on `anchor` (typically the application window).
/// No-op if the session store is unavailable.
pub fn open(anchor: &ApplicationWindow, state: &SharedAppState) {
    let Some(state_home) = lmux_session::state_home() else {
        tracing::warn!("switcher: no XDG state dir, cannot list sessions");
        return;
    };
    let store = lmux_session::SessionStore::new(&state_home);
    let mut entries: Vec<String> = if store.root().exists() {
        store
            .list()
            .map(|v| v.into_iter().map(|e| e.name).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    entries.sort();

    let popover = Popover::new();
    popover.set_position(PositionType::Bottom);
    popover.set_has_arrow(false);
    popover.set_autohide(true);
    popover.set_parent(anchor);

    let body = GtkBox::new(Orientation::Vertical, 6);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);
    body.set_size_request(320, 280);

    let entry = Entry::new();
    entry.set_placeholder_text(Some("Switch session…"));
    body.append(&entry);

    let scroller = ScrolledWindow::new();
    scroller.set_vexpand(true);
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Browse);
    scroller.set_child(Some(&list));
    body.append(&scroller);

    let empty_label = Label::new(Some("(no sessions)"));
    empty_label.add_css_class("dim-label");

    rebuild_rows(&list, &empty_label, &entries, "");
    popover.set_child(Some(&body));

    let entry_list = list.clone();
    let entry_empty = empty_label.clone();
    let entry_entries = entries.clone();
    entry.connect_changed(move |e| {
        let query = e.text().to_string();
        rebuild_rows(&entry_list, &entry_empty, &entry_entries, &query);
    });

    let popover_activate = popover.clone();
    let list_activate = list.clone();
    let state_activate = state.clone();
    let store_root = state_home.clone();
    let commit = move || {
        if let Some(row) = list_activate.selected_row() {
            if let Some(label) = row_label(&row) {
                tracing::info!(session = %label, "switcher: selected");
                match state_activate
                    .borrow_mut()
                    .switch_session(label.clone(), &store_root)
                {
                    Ok(()) => tracing::info!(session = %label, "switcher: swapped"),
                    Err(err) => tracing::warn!(error = %err, "switcher: swap failed"),
                }
            }
        }
        popover_activate.popdown();
    };

    let commit_enter = commit.clone();
    entry.connect_activate(move |_| commit_enter());
    let commit_row = commit.clone();
    list.connect_row_activated(move |_, _| commit_row());

    // Arrow keys on the entry should move the list selection instead of the
    // cursor position.
    let list_nav = list.clone();
    let key_ctrl = EventControllerKey::new();
    key_ctrl.connect_key_pressed(move |_, keyval, _, _| match keyval {
        gdk::Key::Up => {
            move_selection(&list_nav, -1);
            glib::Propagation::Stop
        }
        gdk::Key::Down => {
            move_selection(&list_nav, 1);
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    entry.add_controller(key_ctrl);

    let popover_cleanup = popover.clone();
    popover.connect_closed(move |_| popover_cleanup.unparent());

    popover.popup();
    entry.grab_focus();
}

fn rebuild_rows(list: &ListBox, empty_label: &Label, entries: &[String], query: &str) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let needle = query.trim().to_lowercase();
    let filtered: Vec<&String> = entries
        .iter()
        .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
        .collect();
    if filtered.is_empty() {
        list.append(empty_label);
        return;
    }
    for name in filtered {
        let row = ListBoxRow::new();
        let label = Label::new(Some(name));
        label.set_xalign(0.0);
        label.set_margin_start(6);
        label.set_margin_end(6);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        row.set_child(Some(&label));
        list.append(&row);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
}

fn row_label(row: &ListBoxRow) -> Option<String> {
    let child = row.child()?;
    let label = child.downcast::<Label>().ok()?;
    Some(label.text().to_string())
}

fn move_selection(list: &ListBox, delta: i32) {
    let current = list.selected_row().map(|r| r.index()).unwrap_or(0);
    let mut next = current + delta;
    if next < 0 {
        next = 0;
    }
    if let Some(row) = list.row_at_index(next) {
        list.select_row(Some(&row));
        row.grab_focus();
    }
}
