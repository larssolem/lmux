use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::LaunchEntry;

/// Scan every `.desktop` file found on the XDG application search path.
/// Duplicate file IDs are resolved by taking whichever path appears first,
/// mirroring the freedesktop override rule.
#[cfg_attr(test, allow(dead_code))]
pub fn scan_launch_entries() -> Vec<LaunchEntry> {
    scan_desktop_entries_in(&application_dirs())
}

#[cfg_attr(test, allow(dead_code))]
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

/// Same as [`scan_launch_entries`] but with explicit search directories.
/// Extracted so tests can point at tempdirs without mutating process env.
fn scan_desktop_entries_in(dirs: &[PathBuf]) -> Vec<LaunchEntry> {
    let mut seen: HashMap<String, LaunchEntry> = HashMap::new();
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
            if let Some(de) = parse_desktop_file(&path, &id) {
                seen.insert(id, de);
            }
        }
    }
    seen.into_values().collect()
}

/// Minimal parser for the `[Desktop Entry]` section. Skips entries where
/// `NoDisplay=true`, `Hidden=true`, `Type != Application`, or `Terminal=true`.
fn parse_desktop_file(path: &Path, desktop_id: &str) -> Option<LaunchEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut in_section = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut comment: Option<String> = None;
    let mut startup_wm_class: Option<String> = None;
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
        // Prefer plain keys over locale-suffixed variants like Name[nb_NO].
        match key.trim() {
            "Name" if name.is_none() => name = Some(value.trim().to_string()),
            "Exec" if exec.is_none() => exec = Some(value.trim().to_string()),
            "Comment" if comment.is_none() => comment = Some(value.trim().to_string()),
            "StartupWMClass" if startup_wm_class.is_none() => {
                startup_wm_class = Some(value.trim().to_string())
            }
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
    Some(LaunchEntry {
        name: name?,
        exec: exec?,
        comment,
        bundle_id: linux_app_identity(desktop_id, startup_wm_class.as_deref()),
    })
}

fn linux_app_identity(desktop_id: &str, startup_wm_class: Option<&str>) -> Option<String> {
    if let Some(startup_wm_class) = startup_wm_class {
        let trimmed = startup_wm_class.trim();
        if !trimmed.is_empty() {
            return Some(format!("startup-wm-class:{trimmed}"));
        }
    }
    desktop_id
        .strip_suffix(".desktop")
        .or(Some(desktop_id))
        .map(|id| format!("desktop-entry:{id}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

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
            "[Desktop Entry]\nType=Application\nName=Chrome\nExec=google-chrome %U\nStartupWMClass=google-chrome\n",
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
        let chrome = entries.iter().find(|entry| entry.name == "Chrome").unwrap();
        assert_eq!(
            chrome.bundle_id.as_deref(),
            Some("startup-wm-class:google-chrome")
        );
        let firefox = entries
            .iter()
            .find(|entry| entry.name == "Firefox")
            .unwrap();
        assert_eq!(firefox.bundle_id.as_deref(), Some("desktop-entry:firefox"));
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
}
