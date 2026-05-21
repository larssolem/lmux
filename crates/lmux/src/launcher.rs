//! GUI-program launcher (Epic 9 UX): a spotlight-style popover that scans
//! installed GUI apps and spawns the chosen entry as a satellite via
//! [`lmux_compositor::spawn::spawn_tagged`].
//!
//! Platform scanners stay in `launcher/linux.rs` and `launcher/macos.rs` so
//! Linux `.desktop` handling and macOS `.app` handling do not bleed together.
//!
//! Triggered by `Ctrl+B l` or the launcher button in the sidebar header.
//! Keyboard-only: type to filter, Up/Down to navigate, Enter to pick,
//! Esc to dismiss.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::cell::RefCell;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use gtk4::prelude::*;
#[cfg(not(target_os = "macos"))]
use gtk4::{gdk, EventControllerKey, ScrolledWindow, SelectionMode};
use gtk4::{
    glib, ApplicationWindow, Box as GtkBox, Entry, Label, ListBox, ListBoxRow, Orientation, Window,
};

use crate::state::SharedAppState;

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

/// One resolved launchable GUI entry from the current platform.
#[derive(Debug, Clone)]
pub struct LaunchEntry {
    pub name: String,
    pub exec: String,
    pub comment: Option<String>,
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchCacheStatus {
    Empty,
    Refreshing,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
struct LaunchCacheSnapshot {
    entries: Vec<LaunchEntry>,
    status: LaunchCacheStatus,
    generation: u64,
    last_error: Option<String>,
}

#[derive(Debug)]
struct LaunchEntryCache {
    entries: Vec<LaunchEntry>,
    status: LaunchCacheStatus,
    generation: u64,
    last_refresh: Option<SystemTime>,
    last_error: Option<String>,
}

impl Default for LaunchEntryCache {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            status: LaunchCacheStatus::Empty,
            generation: 0,
            last_refresh: None,
            last_error: None,
        }
    }
}

static LAUNCH_CACHE: OnceLock<Mutex<LaunchEntryCache>> = OnceLock::new();
#[cfg(test)]
static TEST_SCAN_CALLS: AtomicUsize = AtomicUsize::new(0);

fn launch_cache() -> &'static Mutex<LaunchEntryCache> {
    LAUNCH_CACHE.get_or_init(|| Mutex::new(LaunchEntryCache::default()))
}

fn cache_snapshot() -> LaunchCacheSnapshot {
    match launch_cache().lock() {
        Ok(cache) => LaunchCacheSnapshot {
            entries: cache.entries.clone(),
            status: cache.status,
            generation: cache.generation,
            last_error: cache.last_error.clone(),
        },
        Err(err) => {
            tracing::warn!(error = %err, "launcher.cache: poisoned lock");
            LaunchCacheSnapshot {
                entries: Vec::new(),
                status: LaunchCacheStatus::Failed,
                generation: 0,
                last_error: Some("launcher cache lock poisoned".into()),
            }
        }
    }
}

/// Start a background refresh of launchable applications. This function is
/// intentionally cheap enough to call from UI paths; actual platform scanning
/// happens on a worker thread.
pub fn warm_cache() {
    let should_spawn = match launch_cache().lock() {
        Ok(mut cache) => {
            if cache.status == LaunchCacheStatus::Refreshing {
                false
            } else {
                cache.status = LaunchCacheStatus::Refreshing;
                cache.generation = cache.generation.saturating_add(1);
                cache.last_error = None;
                true
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "launcher.cache: warm skipped");
            false
        }
    };
    if !should_spawn {
        return;
    }

    let spawn_started = Instant::now();
    match std::thread::Builder::new()
        .name("lmux-launcher-scan".into())
        .spawn(move || {
            let started = Instant::now();
            let mut entries = scan_launch_entries();
            entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            let count = entries.len();
            match launch_cache().lock() {
                Ok(mut cache) => {
                    cache.entries = entries;
                    cache.status = LaunchCacheStatus::Ready;
                    cache.generation = cache.generation.saturating_add(1);
                    cache.last_refresh = Some(SystemTime::now());
                    cache.last_error = None;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "launcher.cache: failed to store scan results");
                }
            }
            tracing::info!(
                operation = "launcher.scan",
                duration_ms = elapsed_ms(started),
                entries = count,
                ui_path = false,
                "launcher scan completed"
            );
        }) {
        Ok(_) => tracing::debug!(
            operation = "launcher.cache.warm",
            duration_ms = elapsed_ms(spawn_started),
            "launcher cache refresh started"
        ),
        Err(err) => {
            if let Ok(mut cache) = launch_cache().lock() {
                cache.status = LaunchCacheStatus::Failed;
                cache.generation = cache.generation.saturating_add(1);
                cache.last_error = Some(err.to_string());
            }
            tracing::warn!(error = %err, "launcher.cache: worker spawn failed");
        }
    }
}

/// Open the launcher dialog over `anchor`. Implemented as a modal
/// transient-for Window rather than a Popover: Popovers parented to an
/// ApplicationWindow position their anchor rect at (0,0), which on some
/// KWin/Wayland setups lands the popup offscreen. A modal window renders
/// reliably regardless of surface.
#[cfg(target_os = "macos")]
pub fn open(anchor: &ApplicationWindow, state: &SharedAppState) {
    let _ = (anchor, state);
    tracing::debug!(
        operation = "launcher.open",
        "launcher is disabled on macOS; attach an already-open window instead"
    );
}

/// Open the launcher dialog over `anchor`. Implemented as a modal
/// transient-for Window rather than a Popover: Popovers parented to an
/// ApplicationWindow position their anchor rect at (0,0), which on some
/// KWin/Wayland setups lands the popup offscreen. A modal window renders
/// reliably regardless of surface.
#[cfg(not(target_os = "macos"))]
pub fn open(anchor: &ApplicationWindow, state: &SharedAppState) {
    let opened_at = Instant::now();
    let snapshot = launcher_open_snapshot();
    let entries = std::rc::Rc::new(RefCell::new(snapshot.entries));
    let status = std::rc::Rc::new(RefCell::new(snapshot.status));
    tracing::debug!(
        operation = "launcher.open",
        cache_status = ?snapshot.status,
        cache_generation = snapshot.generation,
        entries = entries.borrow().len(),
        "launcher open using cache snapshot"
    );

    let dialog = Window::builder()
        .transient_for(anchor)
        .modal(true)
        .title("Launch program")
        .default_width(440)
        .default_height(420)
        .decorated(true)
        .build();

    let body = GtkBox::new(Orientation::Vertical, 6);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);

    let entry = Entry::new();
    entry.set_placeholder_text(Some("Launch program…"));
    body.append(&entry);

    let scroller = ScrolledWindow::new();
    scroller.set_vexpand(true);
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Browse);
    scroller.set_child(Some(&list));
    body.append(&scroller);

    let empty_label = Label::new(Some(empty_state_text(
        snapshot.status,
        entries.borrow().is_empty(),
        "",
    )));
    empty_label.add_css_class("dim-label");

    rebuild_rows(&list, &empty_label, snapshot.status, &entries.borrow(), "");
    dialog.set_child(Some(&body));

    let entry_list = list.clone();
    let entry_empty = empty_label.clone();
    let entry_entries = entries.clone();
    let entry_status = status.clone();
    entry.connect_changed(move |e| {
        let query = e.text().to_string();
        rebuild_rows(
            &entry_list,
            &entry_empty,
            *entry_status.borrow(),
            &entry_entries.borrow(),
            &query,
        );
    });

    let dialog_activate = dialog.clone();
    let list_activate = list.clone();
    let entries_for_commit = entries.clone();
    let state_for_commit = state.clone();
    let commit = move || {
        if let Some(row) = list_activate.selected_row() {
            if let Some(origin) = row_origin_index(&row) {
                if let Some(de) = entries_for_commit.borrow().get(origin) {
                    spawn_entry(de, &state_for_commit);
                }
            }
        }
        dialog_activate.close();
    };

    let commit_enter = commit.clone();
    entry.connect_activate(move |_| commit_enter());
    let commit_row = commit.clone();
    list.connect_row_activated(move |_, _| commit_row());

    // Arrow keys on the entry should move the list selection; Esc closes.
    let list_nav = list.clone();
    let dialog_for_esc = dialog.clone();
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
        gdk::Key::Escape => {
            dialog_for_esc.close();
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    entry.add_controller(key_ctrl);

    dialog.present();
    entry.grab_focus();

    install_cache_refresh_poll(
        &dialog,
        &entry,
        &list,
        &empty_label,
        entries,
        status,
        snapshot.generation,
    );
    tracing::info!(
        operation = "launcher.open",
        duration_ms = elapsed_ms(opened_at),
        cache_status = ?snapshot.status,
        cache_generation = snapshot.generation,
        entries = cache_snapshot().entries.len(),
        ui_path = true,
        "launcher opened"
    );
}

fn launcher_open_snapshot() -> LaunchCacheSnapshot {
    warm_cache();
    cache_snapshot()
}

fn install_cache_refresh_poll(
    dialog: &Window,
    entry: &Entry,
    list: &ListBox,
    empty_label: &Label,
    entries: std::rc::Rc<RefCell<Vec<LaunchEntry>>>,
    status: std::rc::Rc<RefCell<LaunchCacheStatus>>,
    initial_generation: u64,
) {
    let dialog_weak = dialog.downgrade();
    let entry = entry.clone();
    let list = list.clone();
    let empty_label = empty_label.clone();
    let mut seen_generation = initial_generation;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if dialog_weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let snapshot = cache_snapshot();
        if snapshot.generation != seen_generation {
            seen_generation = snapshot.generation;
            *status.borrow_mut() = snapshot.status;
            *entries.borrow_mut() = snapshot.entries;
            let query = entry.text().to_string();
            rebuild_rows(
                &list,
                &empty_label,
                snapshot.status,
                &entries.borrow(),
                &query,
            );
            tracing::debug!(
                operation = "launcher.cache.refresh_visible",
                cache_status = ?snapshot.status,
                cache_generation = snapshot.generation,
                entries = entries.borrow().len(),
                error = ?snapshot.last_error,
                "launcher rows refreshed from cache"
            );
        }
        if matches!(
            snapshot.status,
            LaunchCacheStatus::Ready | LaunchCacheStatus::Failed
        ) {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

/// Rebuild list rows from `entries` filtered by `query` (substring,
/// case-insensitive, matches against Name + Comment). Row `index()` maps
/// back into `entries` via the filtered-index table stored on each row as
/// a GObject data key.
fn rebuild_rows(
    list: &ListBox,
    empty_label: &Label,
    status: LaunchCacheStatus,
    entries: &[LaunchEntry],
    query: &str,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let needle = query.trim().to_lowercase();
    let filtered: Vec<(usize, &LaunchEntry)> = entries
        .iter()
        .enumerate()
        .filter(|(_, de)| {
            if needle.is_empty() {
                return true;
            }
            if de.name.to_lowercase().contains(&needle) {
                return true;
            }
            if let Some(c) = &de.comment {
                return c.to_lowercase().contains(&needle);
            }
            false
        })
        .collect();
    if filtered.is_empty() {
        empty_label.set_label(empty_state_text(status, entries.is_empty(), query));
        list.append(empty_label);
        return;
    }
    // Row indices after filtering no longer align with the full `entries`
    // slice, so we stamp each row with its original index and the commit
    // closure reads it back via `steal_data`.
    for (origin, de) in filtered {
        let row = ListBoxRow::new();
        let v = GtkBox::new(Orientation::Vertical, 0);
        v.set_margin_start(6);
        v.set_margin_end(6);
        v.set_margin_top(4);
        v.set_margin_bottom(4);
        let title = Label::new(Some(&de.name));
        title.set_xalign(0.0);
        v.append(&title);
        if let Some(c) = &de.comment {
            if !c.is_empty() {
                let sub = Label::new(Some(c));
                sub.set_xalign(0.0);
                sub.add_css_class("dim-label");
                v.append(&sub);
            }
        }
        row.set_child(Some(&v));
        unsafe {
            row.set_data::<usize>("lmux-origin-idx", origin);
        }
        list.append(&row);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
}

fn empty_state_text(status: LaunchCacheStatus, entries_empty: bool, query: &str) -> &'static str {
    if entries_empty && query.trim().is_empty() {
        match status {
            LaunchCacheStatus::Empty | LaunchCacheStatus::Refreshing => "(loading applications...)",
            LaunchCacheStatus::Failed => "(could not load applications)",
            LaunchCacheStatus::Ready => "(no launchable applications)",
        }
    } else {
        "(no matches)"
    }
}

fn row_origin_index(row: &ListBoxRow) -> Option<usize> {
    // SAFETY: we set this data with `set_data::<usize>` in `rebuild_rows`;
    // only `usize` is ever stored under this key.
    unsafe { row.data::<usize>("lmux-origin-idx").map(|nn| *nn.as_ref()) }
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

fn spawn_entry(de: &LaunchEntry, state: &SharedAppState) {
    let argv = match parse_exec(&de.exec) {
        Some(v) if !v.is_empty() => v,
        _ => {
            tracing::warn!(name = %de.name, exec = %de.exec, "launcher: unparseable Exec");
            return;
        }
    };
    let anchor_at_launch = state.borrow().active_anchor();

    // Pin the child onto the nested-compositor socket (ADR-0018). When
    // the cockpit's wayland host hasn't started (e.g. CI, or bind
    // failure) we fall back to inheriting WAYLAND_DISPLAY, which means
    // the satellite docks to the *outer* compositor like in v0.1.
    let nested_display = state.borrow().wayland_display_name().map(|s| s.to_string());

    let spawn_res =
        lmux_compositor::spawn::spawn_tagged_with_env(&argv, None, nested_display.as_deref());
    match spawn_res {
        Ok((id, pid)) => {
            tracing::info!(
                name = %de.name,
                request_id = %id,
                pid,
                nested = nested_display.is_some(),
                "launcher: satellite spawned"
            );
            // Tie the satellite's lifecycle to the currently-active anchor
            // so it hides on anchor switch-away and returns on switch-back,
            // mirroring terminal pane workspace membership.
            let mut s = state.borrow_mut();
            if let Some(anchor) = anchor_at_launch {
                s.register_satellite_spawn(anchor, id, pid, de.bundle_id.clone());
            } else {
                tracing::warn!(pid, "launcher: no active anchor — satellite is unmanaged");
            }
            drop(s);
        }
        Err(err) => tracing::warn!(name = %de.name, error = %err, "launcher: spawn failed"),
    }
}

/// Strip freedesktop Exec field codes (%f, %F, %u, %U, %i, %c, %k, ...) and
/// split the result into argv using a small shell-style tokenizer that
/// honors double-quoted strings and `\ ` escapes. Returns `None` if the
/// input is empty or unbalanced.
pub fn parse_exec(exec: &str) -> Option<Vec<String>> {
    let trimmed = exec.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
            }
            '\\' if in_quote => {
                if let Some(&next) = chars.peek() {
                    buf.push(next);
                    chars.next();
                }
            }
            '\\' => {
                if let Some(&next) = chars.peek() {
                    buf.push(next);
                    chars.next();
                }
            }
            '%' if !in_quote => {
                // Consume the field code; drop silently.
                chars.next();
            }
            c if c.is_whitespace() && !in_quote => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
            }
            _ => buf.push(c),
        }
    }
    if in_quote {
        return None;
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Scan launchable GUI entries for the current platform.
pub fn scan_launch_entries() -> Vec<LaunchEntry> {
    #[cfg(test)]
    TEST_SCAN_CALLS.fetch_add(1, Ordering::SeqCst);
    let started = Instant::now();
    let entries = scan_launch_entries_inner();
    tracing::debug!(
        operation = "launcher.scan",
        duration_ms = elapsed_ms(started),
        entries = entries.len(),
        ui_path = false,
        "launcher scan finished"
    );
    entries
}

fn scan_launch_entries_inner() -> Vec<LaunchEntry> {
    #[cfg(target_os = "macos")]
    {
        return macos::scan_launch_entries();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::scan_launch_entries();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parse_exec_strips_field_codes() {
        assert_eq!(
            parse_exec("firefox %u").unwrap(),
            vec!["firefox".to_string()]
        );
        assert_eq!(
            parse_exec("kate --new %F").unwrap(),
            vec!["kate".to_string(), "--new".to_string()]
        );
    }

    #[test]
    fn parse_exec_handles_quoted() {
        assert_eq!(
            parse_exec("\"/opt/my app/bin\" --flag %u").unwrap(),
            vec!["/opt/my app/bin".to_string(), "--flag".to_string()]
        );
    }

    #[test]
    fn parse_exec_empty_is_none() {
        assert!(parse_exec("").is_none());
        assert!(parse_exec("   ").is_none());
    }

    #[test]
    fn substring_filter_matches_name_prefix() {
        let entries = [
            LaunchEntry {
                name: "Chromium".into(),
                exec: "chromium".into(),
                comment: None,
                bundle_id: None,
            },
            LaunchEntry {
                name: "Firefox".into(),
                exec: "firefox".into(),
                comment: Some("Web browser".into()),
                bundle_id: None,
            },
            LaunchEntry {
                name: "Kate".into(),
                exec: "kate".into(),
                comment: None,
                bundle_id: None,
            },
        ];
        let matches: Vec<_> = entries
            .iter()
            .filter(|de| {
                let needle = "ch".to_lowercase();
                de.name.to_lowercase().contains(&needle)
                    || de
                        .comment
                        .as_ref()
                        .map(|c| c.to_lowercase().contains(&needle))
                        .unwrap_or(false)
            })
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(matches, vec!["Chromium".to_string()]);
    }

    #[test]
    fn substring_filter_also_matches_comment() {
        let entries = [LaunchEntry {
            name: "Firefox".into(),
            exec: "firefox".into(),
            comment: Some("Web browser".into()),
            bundle_id: None,
        }];
        let needle = "browser".to_lowercase();
        let matches = entries
            .iter()
            .filter(|de| {
                de.name.to_lowercase().contains(&needle)
                    || de
                        .comment
                        .as_ref()
                        .map(|c| c.to_lowercase().contains(&needle))
                        .unwrap_or(false)
            })
            .count();
        assert_eq!(matches, 1);
    }

    #[test]
    fn launcher_cache_snapshot_does_not_scan() {
        let before = TEST_SCAN_CALLS.load(Ordering::SeqCst);
        let _ = cache_snapshot();
        assert_eq!(TEST_SCAN_CALLS.load(Ordering::SeqCst), before);
    }

    #[test]
    fn launcher_open_snapshot_renders_loading_cache_without_scanning_inline() {
        {
            let mut cache = launch_cache().lock().unwrap();
            cache.entries.clear();
            cache.status = LaunchCacheStatus::Refreshing;
            cache.generation = 42;
            cache.last_error = None;
        }
        let before = TEST_SCAN_CALLS.load(Ordering::SeqCst);

        let snapshot = launcher_open_snapshot();

        assert_eq!(TEST_SCAN_CALLS.load(Ordering::SeqCst), before);
        assert_eq!(snapshot.status, LaunchCacheStatus::Refreshing);
        assert!(snapshot.entries.is_empty());
        assert_eq!(
            empty_state_text(snapshot.status, snapshot.entries.is_empty(), ""),
            "(loading applications...)"
        );
    }

    #[test]
    fn launcher_empty_state_reports_cache_failure_without_scanning() {
        {
            let mut cache = launch_cache().lock().unwrap();
            cache.entries.clear();
            cache.status = LaunchCacheStatus::Failed;
            cache.generation = 43;
            cache.last_error = Some("boom".into());
        }
        let before = TEST_SCAN_CALLS.load(Ordering::SeqCst);

        let snapshot = cache_snapshot();

        assert_eq!(TEST_SCAN_CALLS.load(Ordering::SeqCst), before);
        assert_eq!(snapshot.status, LaunchCacheStatus::Failed);
        assert_eq!(
            empty_state_text(snapshot.status, snapshot.entries.is_empty(), ""),
            "(could not load applications)"
        );
    }
}
