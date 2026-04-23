//! GUI-program launcher (Epic 9 UX): a spotlight-style popover that scans
//! installed `.desktop` files and spawns the chosen entry as a satellite
//! via [`lmux_compositor::spawn::spawn_tagged`].
//!
//! Triggered by `Ctrl+B l` or the launcher button in the sidebar header.
//! Keyboard-only: type to filter, Up/Down to navigate, Enter to pick,
//! Esc to dismiss.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use gtk4::prelude::*;
use gtk4::{
    gdk, glib, ApplicationWindow, Box as GtkBox, Entry, EventControllerKey, Label, ListBox,
    ListBoxRow, Orientation, ScrolledWindow, SelectionMode, Window,
};

use crate::state::SharedAppState;

/// One resolved `.desktop` entry.
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub comment: Option<String>,
}

/// Open the launcher dialog over `anchor`. Implemented as a modal
/// transient-for Window rather than a Popover: Popovers parented to an
/// ApplicationWindow position their anchor rect at (0,0), which on some
/// KWin/Wayland setups lands the popup offscreen. A modal window renders
/// reliably regardless of surface.
pub fn open(anchor: &ApplicationWindow, state: &SharedAppState) {
    let mut entries = scan_desktop_entries();
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    tracing::info!(count = entries.len(), "launcher: scanned desktop entries");

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

    let empty_label = Label::new(Some("(no matches)"));
    empty_label.add_css_class("dim-label");

    rebuild_rows(&list, &empty_label, &entries, "");
    dialog.set_child(Some(&body));

    let entry_list = list.clone();
    let entry_empty = empty_label.clone();
    let entry_entries = entries.clone();
    entry.connect_changed(move |e| {
        let query = e.text().to_string();
        rebuild_rows(&entry_list, &entry_empty, &entry_entries, &query);
    });

    let dialog_activate = dialog.clone();
    let list_activate = list.clone();
    let entries_for_commit = entries.clone();
    let state_for_commit = state.clone();
    let commit = move || {
        if let Some(row) = list_activate.selected_row() {
            if let Some(origin) = row_origin_index(&row) {
                if let Some(de) = entries_for_commit.get(origin) {
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
}

/// Rebuild list rows from `entries` filtered by `query` (substring,
/// case-insensitive, matches against Name + Comment). Row `index()` maps
/// back into `entries` via the filtered-index table stored on each row as
/// a GObject data key.
fn rebuild_rows(list: &ListBox, empty_label: &Label, entries: &[DesktopEntry], query: &str) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let needle = query.trim().to_lowercase();
    let filtered: Vec<(usize, &DesktopEntry)> = entries
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

fn spawn_entry(de: &DesktopEntry, state: &SharedAppState) {
    let argv = match parse_exec(&de.exec) {
        Some(v) if !v.is_empty() => v,
        _ => {
            tracing::warn!(name = %de.name, exec = %de.exec, "launcher: unparseable Exec");
            return;
        }
    };
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
            if let Some(anchor) = s.active_anchor() {
                s.register_satellite(anchor, pid);
            } else {
                tracing::warn!(pid, "launcher: no active anchor — satellite is unmanaged");
            }
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

/// Scan every `.desktop` file found on the XDG application search path.
/// Duplicate file IDs (e.g. system entry overridden by a user one) are
/// resolved by taking whichever path appears first — mirroring the
/// freedesktop override rule: earlier dirs on `$XDG_DATA_DIRS` shadow
/// later ones, and `$XDG_DATA_HOME` shadows everything.
pub fn scan_desktop_entries() -> Vec<DesktopEntry> {
    scan_desktop_entries_in(&application_dirs())
}

/// Same as [`scan_desktop_entries`] but with an explicit list of search
/// directories. Extracted so tests can point at a tempdir without
/// mutating process-wide environment variables.
pub fn scan_desktop_entries_in(dirs: &[PathBuf]) -> Vec<DesktopEntry> {
    let mut seen: HashMap<String, DesktopEntry> = HashMap::new();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let id = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if seen.contains_key(&id) {
                continue;
            }
            if let Some(de) = parse_desktop_file(&path) {
                seen.insert(id, de);
            }
        }
    }
    seen.into_values().collect()
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for d in data_dirs.split(':') {
        if d.is_empty() {
            continue;
        }
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

/// Minimal parser for the `[Desktop Entry]` section. Skips entries where
/// `NoDisplay=true`, `Hidden=true`, `Type != Application`, or `Terminal=true`
/// (terminal apps want a terminal — not our problem to host them as a
/// satellite window).
fn parse_desktop_file(path: &Path) -> Option<DesktopEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut in_section = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut comment: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut no_display = false;
    let mut hidden = false;
    let mut terminal = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line == "[Desktop Entry]";
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Prefer the plain key over locale-suffixed variants (Name[nb_NO]=).
        // If we've already captured a plain value, skip locale variants.
        match key.trim() {
            "Name" if name.is_none() => name = Some(value.trim().to_string()),
            "Exec" if exec.is_none() => exec = Some(value.trim().to_string()),
            "Comment" if comment.is_none() => comment = Some(value.trim().to_string()),
            "Type" => kind = Some(value.trim().to_string()),
            "NoDisplay" => no_display = value.trim().eq_ignore_ascii_case("true"),
            "Hidden" => hidden = value.trim().eq_ignore_ascii_case("true"),
            "Terminal" => terminal = value.trim().eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    if no_display || hidden || terminal {
        return None;
    }
    if kind.as_deref() != Some("Application") {
        return None;
    }
    let name = name?;
    let exec = exec?;
    Some(DesktopEntry {
        name,
        exec,
        comment,
    })
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

    fn write_desktop(dir: &std::path::Path, name: &str, body: &str) {
        let path = dir.join(format!("{name}.desktop"));
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn scan_picks_up_application_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().to_path_buf();
        write_desktop(
            &apps,
            "chrome",
            "[Desktop Entry]\nType=Application\nName=Chrome\nExec=google-chrome %U\n",
        );
        write_desktop(
            &apps,
            "firefox",
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\nComment=Web browser\n",
        );
        let entries = scan_desktop_entries_in(&[apps]);
        assert_eq!(entries.len(), 2);
        let mut names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["Chrome".to_string(), "Firefox".to_string()]);
    }

    #[test]
    fn scan_skips_nodisplay_hidden_terminal_and_nonapp() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().to_path_buf();
        write_desktop(
            &apps,
            "hidden",
            "[Desktop Entry]\nType=Application\nName=Hidden\nExec=x\nHidden=true\n",
        );
        write_desktop(
            &apps,
            "nodisplay",
            "[Desktop Entry]\nType=Application\nName=ND\nExec=x\nNoDisplay=true\n",
        );
        write_desktop(
            &apps,
            "term",
            "[Desktop Entry]\nType=Application\nName=TermApp\nExec=x\nTerminal=true\n",
        );
        write_desktop(
            &apps,
            "link",
            "[Desktop Entry]\nType=Link\nName=Link\nURL=https://example.com\n",
        );
        write_desktop(
            &apps,
            "good",
            "[Desktop Entry]\nType=Application\nName=Good\nExec=ok\n",
        );
        let entries = scan_desktop_entries_in(&[apps]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Good");
    }

    #[test]
    fn scan_resolves_override_by_first_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        let sys = tmp.path().join("sys");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        write_desktop(
            &user,
            "chrome",
            "[Desktop Entry]\nType=Application\nName=Chrome (user override)\nExec=chrome\n",
        );
        write_desktop(
            &sys,
            "chrome",
            "[Desktop Entry]\nType=Application\nName=Chrome\nExec=chrome\n",
        );
        let entries = scan_desktop_entries_in(&[user, sys]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Chrome (user override)");
    }

    #[test]
    fn substring_filter_matches_name_prefix() {
        let entries = [
            DesktopEntry {
                name: "Chromium".into(),
                exec: "chromium".into(),
                comment: None,
            },
            DesktopEntry {
                name: "Firefox".into(),
                exec: "firefox".into(),
                comment: Some("Web browser".into()),
            },
            DesktopEntry {
                name: "Kate".into(),
                exec: "kate".into(),
                comment: None,
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
        let entries = [DesktopEntry {
            name: "Firefox".into(),
            exec: "firefox".into(),
            comment: Some("Web browser".into()),
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
}
