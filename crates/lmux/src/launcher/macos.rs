use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::LaunchEntry;

/// Scan launchable `.app` bundles from the standard macOS application dirs.
pub fn scan_launch_entries() -> Vec<LaunchEntry> {
    Vec::new()
}

#[allow(dead_code)]
fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Applications"));
    }
    dirs.push(PathBuf::from("/Applications"));
    dirs.push(PathBuf::from("/System/Applications"));
    dirs
}

fn scan_apps_in(dirs: &[PathBuf]) -> Vec<LaunchEntry> {
    let mut seen: HashMap<String, LaunchEntry> = HashMap::new();
    for dir in dirs {
        scan_app_dir(dir, 0, &mut seen);
    }
    seen.into_values().collect()
}

fn scan_app_dir(dir: &Path, depth: usize, seen: &mut HashMap<String, LaunchEntry>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if is_app_bundle(&path) {
            let Some(name) = app_name(&path) else {
                continue;
            };
            if seen.contains_key(&name) {
                continue;
            }
            seen.insert(
                name.clone(),
                LaunchEntry {
                    name,
                    exec: launch_command_for_app(&path),
                    comment: path.to_str().map(str::to_string),
                    bundle_id: app_bundle_id(&path),
                },
            );
            continue;
        }
        if depth < 1 && path.is_dir() {
            scan_app_dir(&path, depth + 1, seen);
        }
    }
}

fn launch_command_for_app(path: &Path) -> String {
    let bundle_id = app_bundle_id(path);
    match app_executable(path) {
        Some(exe) if prefers_direct_executable(bundle_id.as_deref(), &exe) => {
            shell_quote_path(&exe)
        }
        _ => format!("open -n {}", shell_quote_path(path)),
    }
}

fn prefers_direct_executable(bundle_id: Option<&str>, exe: &Path) -> bool {
    let base = exe.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    let bundle_id = bundle_id.unwrap_or_default();
    bundle_id.starts_with("com.jetbrains.")
        || matches!(
            bundle_id,
            "com.microsoft.VSCode"
                | "com.microsoft.VSCodeInsiders"
                | "com.visualstudio.code.oss"
                | "com.vscodium"
                | "com.google.Chrome"
                | "com.google.Chrome.beta"
                | "com.google.Chrome.canary"
                | "com.brave.Browser"
                | "com.microsoft.edgemac"
                | "com.vivaldi.Vivaldi"
                | "com.operasoftware.Opera"
        )
        || matches!(
            base,
            "Code"
                | "Code - Insiders"
                | "VSCodium"
                | "Google Chrome"
                | "Brave Browser"
                | "Microsoft Edge"
                | "Vivaldi"
                | "Opera"
                | "idea"
                | "pycharm"
                | "webstorm"
                | "goland"
                | "clion"
                | "datagrip"
        )
}

fn app_executable(path: &Path) -> Option<PathBuf> {
    let executable = plist_raw_value(path, "CFBundleExecutable")?;
    if executable.is_empty() {
        return None;
    }
    let candidate = path.join("Contents/MacOS").join(executable);
    candidate.is_file().then_some(candidate)
}

fn app_bundle_id(path: &Path) -> Option<String> {
    plist_raw_value(path, "CFBundleIdentifier")
}

fn plist_raw_value(path: &Path, key: &str) -> Option<String> {
    let info_plist = path.join("Contents/Info.plist");
    let output = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(&info_plist)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn is_app_bundle(path: &Path) -> bool {
    path.is_dir() && path.extension().and_then(|s| s.to_str()) == Some("app")
}

fn app_name(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    Some(
        file_name
            .strip_suffix(".app")
            .unwrap_or(file_name)
            .to_string(),
    )
}

fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn public_macos_launcher_entries_are_disabled() {
        assert!(scan_launch_entries().is_empty());
    }

    #[test]
    fn scan_macos_apps_falls_back_to_open_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("Applications");
        let nested = apps.join("Utilities");
        std::fs::create_dir_all(apps.join("Example App.app")).unwrap();
        std::fs::create_dir_all(nested.join("Nested.app")).unwrap();

        let entries = scan_apps_in(&[apps]);
        let mut names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["Example App".to_string(), "Nested".to_string()]);
        assert!(entries.iter().any(|e| {
            e.name == "Example App"
                && e.exec.starts_with("open -n ")
                && e.exec.contains("Example App.app")
        }));
    }

    #[test]
    fn scan_macos_apps_uses_open_new_for_generic_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("Applications");
        let bundle = apps.join("Example App.app");
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>no.jpro.ExampleApp</string>
<key>CFBundleExecutable</key><string>ExampleExec</string>
</dict></plist>
"#,
        )
        .unwrap();
        std::fs::write(macos.join("ExampleExec"), "#!/bin/sh\n").unwrap();

        let entries = scan_apps_in(&[apps]);
        let entry = entries
            .iter()
            .find(|entry| entry.name == "Example App")
            .unwrap();
        assert!(entry.exec.starts_with("open -n "));
        assert!(entry.exec.contains("Example App.app"));
        assert_eq!(entry.bundle_id.as_deref(), Some("no.jpro.ExampleApp"));
    }

    #[test]
    fn scan_macos_apps_uses_direct_executable_for_isolated_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("Applications");
        let bundle = apps.join("Visual Studio Code.app");
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.microsoft.VSCode</string>
<key>CFBundleExecutable</key><string>Code</string>
</dict></plist>
"#,
        )
        .unwrap();
        std::fs::write(macos.join("Code"), "#!/bin/sh\n").unwrap();

        let entries = scan_apps_in(&[apps]);
        let entry = entries
            .iter()
            .find(|entry| entry.name == "Visual Studio Code")
            .unwrap();
        assert!(entry.exec.contains("Contents/MacOS/Code"));
        assert!(!entry.exec.starts_with("open -n "));
        assert_eq!(entry.bundle_id.as_deref(), Some("com.microsoft.VSCode"));
    }

    #[test]
    fn scan_macos_apps_direct_launches_jetbrains_for_profile_control() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("Applications");
        let bundle = apps.join("IntelliJ IDEA.app");
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.jetbrains.intellij</string>
<key>CFBundleExecutable</key><string>idea</string>
</dict></plist>
"#,
        )
        .unwrap();
        std::fs::write(macos.join("idea"), "#!/bin/sh\n").unwrap();

        let entries = scan_apps_in(&[apps]);
        let entry = entries
            .iter()
            .find(|entry| entry.name == "IntelliJ IDEA")
            .unwrap();
        assert!(!entry.exec.starts_with("open -n "));
        assert!(entry.exec.contains("Contents/MacOS/idea"));
        assert_eq!(entry.bundle_id.as_deref(), Some("com.jetbrains.intellij"));
    }
}
