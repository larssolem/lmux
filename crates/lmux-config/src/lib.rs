//! lmux user-config TOML schema + loader.
//!
//! Epic 10 (partial): load + validate + first-run provisioning. Hot-reload
//! via inotify lives in the same crate but is not wired in v0.2-alpha.
//!
//! # Layout
//!
//! `$XDG_CONFIG_HOME/lmux/config.toml` (fallback `$HOME/.config/lmux/config.toml`).
//! Missing file → default config is returned AND written on first run so the
//! user has something to edit.
//!
//! ```toml
//! [general]
//! font_family = "JetBrains Mono"
//! font_size = 11 # macOS default: 13
//!
//! [keymap]
//! prefix = "ctrl+b"
//!
//! [[autodetect]]
//! name = "rust-build"
//! match = { command_contains = ["cargo build", "cargo test"] }
//! hide_on_session_close = true
//! ```

#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod watch;

/// Top-level config. All fields default to sensible values so a missing or
/// empty config file produces a usable cockpit (NFR12).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub keymap: Keymap,
    #[serde(default)]
    pub sidebar: Sidebar,
    #[serde(default)]
    pub autodetect: Vec<AutodetectRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct General {
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    /// Path to the lmux-dock KWin script. When unset, resolved from the
    /// installation prefix at runtime.
    #[serde(default)]
    pub compositor_script: Option<String>,
    #[serde(default)]
    pub focus_mode: FocusMode,
}

impl Default for General {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: default_font_size(),
            compositor_script: None,
            focus_mode: FocusMode::default(),
        }
    }
}

/// How pointer input transfers keyboard focus to a pane.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    /// Focus only moves on click (default, matches most tiling setups).
    #[default]
    Click,
    /// Focus follows the mouse — entering a pane grabs focus.
    Hover,
}

fn default_font_family() -> String {
    "JetBrains Mono".into()
}

#[cfg(test)]
fn default_font_family_name() -> &'static str {
    "JetBrains Mono"
}

fn default_font_size() -> u32 {
    #[cfg(target_os = "macos")]
    {
        13
    }
    #[cfg(target_os = "linux")]
    {
        11
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        11
    }
}

/// Keymap — v0.2 scopes this to the prefix key per the Norwegian-keyboard
/// feedback memory. The full binding table ships as defaults and is not
/// yet user-overridable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Keymap {
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            prefix: default_prefix(),
        }
    }
}

fn default_prefix() -> String {
    "ctrl+b".into()
}

/// Anchor sidebar presentation. Always-on per the v0.2 product direction;
/// user can collapse it but it is never dismissed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sidebar {
    #[serde(default)]
    pub position: SidebarPosition,
    #[serde(default = "default_sidebar_width")]
    pub width: u32,
    #[serde(default = "default_sidebar_collapsed_width")]
    pub collapsed_width: u32,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default = "default_sidebar_preview_enabled")]
    pub preview_enabled: bool,
    #[serde(default = "default_sidebar_preview_refresh_ms")]
    pub preview_refresh_ms: u32,
    #[serde(default)]
    pub default_sort: SidebarSort,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            position: SidebarPosition::default(),
            width: default_sidebar_width(),
            collapsed_width: default_sidebar_collapsed_width(),
            collapsed: false,
            preview_enabled: default_sidebar_preview_enabled(),
            preview_refresh_ms: default_sidebar_preview_refresh_ms(),
            default_sort: SidebarSort::default(),
        }
    }
}

fn default_sidebar_width() -> u32 {
    280
}
fn default_sidebar_collapsed_width() -> u32 {
    48
}
fn default_sidebar_preview_enabled() -> bool {
    true
}
fn default_sidebar_preview_refresh_ms() -> u32 {
    750
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarPosition {
    #[default]
    Left,
    Right,
}

/// How the sidebar orders anchors within a group by default. Users can
/// still drag to override manually; that just rewrites `sort_key` on the
/// affected anchors.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSort {
    /// Honour each anchor's `sort_key` (alpha ties break).
    #[default]
    Manual,
    /// Most-recently-active first.
    Recent,
    /// Alphabetical on display label.
    Alpha,
}

/// One anchor autodetect rule. Matches against the pane's running command
/// line and/or environment. Multiple rules are tried in order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutodetectRule {
    pub name: String,
    #[serde(rename = "match")]
    pub match_: MatchSpec,
    #[serde(default)]
    pub hide_on_session_close: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MatchSpec {
    /// Anchor the pane if its command line contains any of these
    /// substrings. Case-sensitive.
    #[serde(default)]
    pub command_contains: Vec<String>,
    /// Anchor if any listed env var is set on the pane.
    #[serde(default)]
    pub env_set: Vec<String>,
}

impl AutodetectRule {
    /// Returns true when this rule matches a pane whose command line is
    /// `command` and whose environment contains `env_keys`.
    pub fn matches(&self, command: &str, env_keys: &[&str]) -> bool {
        let cmd_hit = self
            .match_
            .command_contains
            .iter()
            .any(|needle| command.contains(needle));
        let env_hit = self
            .match_
            .env_set
            .iter()
            .any(|needed| env_keys.contains(&needed.as_str()));
        cmd_hit || env_hit
    }
}

/// Load errors. `InvalidToml` is separate from `Io` so the UI can surface
/// parse errors with line numbers without being mistaken for "file missing".
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid toml in {path}: {source}")]
    InvalidToml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("serialize defaults: {0}")]
    EncodeDefaults(#[from] toml::ser::Error),
    #[error("no config dir: $XDG_CONFIG_HOME / $HOME both unset")]
    NoConfigDir,
}

/// Outcome of [`load_or_provision`].
#[derive(Debug, PartialEq, Eq)]
pub enum ProvisionOutcome {
    /// Existing config loaded.
    Loaded,
    /// No config file existed; defaults were written to disk.
    Provisioned,
}

/// Resolve `$XDG_CONFIG_HOME/lmux/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("lmux").join("config.toml"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/lmux").join("config.toml"))
}

/// Parse TOML at `path`. Missing files return default [`Config`] without
/// writing anything; use [`load_or_provision`] when you want first-run
/// provisioning.
pub fn load(path: &Path) -> Result<Config, LoadError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(err) => return Err(LoadError::Io(err)),
    };
    let text = String::from_utf8_lossy(&bytes);
    toml::from_str::<Config>(&text).map_err(|source| LoadError::InvalidToml {
        path: path.display().to_string(),
        source,
    })
}

/// Persist a complete config. Parent directory is created if needed.
pub fn save(path: &Path, cfg: &Config) -> Result<(), LoadError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Load `path`, or if it doesn't exist, write defaults there and return
/// them. Parent directory is created if needed. FR59: first-run behaviour
/// must not surprise the user — we leave a file they can edit.
pub fn load_or_provision(path: &Path) -> Result<(Config, ProvisionOutcome), LoadError> {
    if path.exists() {
        return Ok((load(path)?, ProvisionOutcome::Loaded));
    }
    let cfg = Config::default();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    save(path, &cfg)?;
    tracing::info!(path = %path.display(), "wrote default lmux config");
    Ok((cfg, ProvisionOutcome::Provisioned))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn tempdir() -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }

    #[test]
    fn defaults_roundtrip_through_toml() {
        let cfg = Config::default();
        assert_eq!(cfg.general.font_family, default_font_family_name());
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn missing_file_returns_defaults_without_writing() {
        let dir = tempdir();
        let path = dir.join("config.toml");
        let cfg = load(&path).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(!path.exists(), "load must not write");
    }

    #[test]
    fn provision_writes_defaults_on_first_run() {
        let dir = tempdir();
        let path = dir.join("nested").join("config.toml");
        let (cfg, outcome) = load_or_provision(&path).unwrap();
        assert_eq!(outcome, ProvisionOutcome::Provisioned);
        assert_eq!(cfg, Config::default());
        assert!(path.exists(), "provision must write");

        // Second call is a plain load.
        let (_, outcome2) = load_or_provision(&path).unwrap();
        assert_eq!(outcome2, ProvisionOutcome::Loaded);
    }

    #[test]
    fn save_persists_updated_font_settings() {
        let dir = tempdir();
        let path = dir.join("config.toml");
        let mut cfg = Config::default();
        cfg.general.font_family = "Fira Code".into();
        cfg.general.font_size = 16;

        save(&path, &cfg).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.general.font_family, "Fira Code");
        assert_eq!(loaded.general.font_size, 16);
    }

    #[test]
    fn invalid_toml_surfaces_parse_error() {
        let dir = tempdir();
        let path = dir.join("config.toml");
        std::fs::write(&path, b"this is not = valid toml = at all").unwrap();
        match load(&path) {
            Err(LoadError::InvalidToml { .. }) => {}
            other => panic!("expected InvalidToml, got {other:?}"),
        }
    }

    #[test]
    fn autodetect_rule_matches_by_command() {
        let rule = AutodetectRule {
            name: "cargo".into(),
            match_: MatchSpec {
                command_contains: vec!["cargo build".into()],
                env_set: vec![],
            },
            hide_on_session_close: true,
        };
        assert!(rule.matches("cargo build --release", &[]));
        assert!(!rule.matches("npm run build", &[]));
    }

    #[test]
    fn autodetect_rule_matches_by_env() {
        let rule = AutodetectRule {
            name: "devserver".into(),
            match_: MatchSpec {
                command_contains: vec![],
                env_set: vec!["LMUX_ANCHOR".into()],
            },
            hide_on_session_close: false,
        };
        assert!(rule.matches("node server.js", &["LMUX_ANCHOR"]));
        assert!(!rule.matches("node server.js", &["PATH"]));
    }

    #[test]
    fn sidebar_defaults_are_sensible() {
        let s = Sidebar::default();
        assert_eq!(s.position, SidebarPosition::Left);
        assert_eq!(s.default_sort, SidebarSort::Manual);
        assert!(s.preview_enabled);
        assert!(!s.collapsed);
        assert!(s.width > s.collapsed_width);
    }

    #[test]
    fn sidebar_overrides_parse() {
        let toml_src = r#"
[sidebar]
position = "right"
width = 320
collapsed_width = 56
collapsed = true
preview_enabled = false
preview_refresh_ms = 1500
default_sort = "alpha"
"#;
        let cfg: Config = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.sidebar.position, SidebarPosition::Right);
        assert_eq!(cfg.sidebar.width, 320);
        assert_eq!(cfg.sidebar.collapsed_width, 56);
        assert!(cfg.sidebar.collapsed);
        assert!(!cfg.sidebar.preview_enabled);
        assert_eq!(cfg.sidebar.preview_refresh_ms, 1500);
        assert_eq!(cfg.sidebar.default_sort, SidebarSort::Alpha);
    }

    #[test]
    fn example_config_parses() {
        let toml_src = r#"
[general]
font_family = "Fira Code"
font_size = 12

[keymap]
prefix = "ctrl+shift+space"

[[autodetect]]
name = "rust-build"
match = { command_contains = ["cargo build", "cargo test"] }
hide_on_session_close = true
"#;
        let cfg: Config = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.general.font_family, "Fira Code");
        assert_eq!(cfg.keymap.prefix, "ctrl+shift+space");
        assert_eq!(cfg.autodetect.len(), 1);
        assert_eq!(cfg.autodetect[0].name, "rust-build");
    }
}
