//! Best-effort X11/EWMH native-window backend.
//!
//! This intentionally uses common command-line tools instead of a Rust X11
//! crate so the initial attach path does not add a new dependency. Listing is
//! read-only via `xprop`; operations use `xdotool` when available and fail
//! closed when the target window is missing or the tools are unavailable.

use async_trait::async_trait;
use std::ffi::OsStr;
use uuid::Uuid;

use crate::{
    CompositorControl, CompositorError, Health, Rect, SatelliteWindowId, WindowAppIdentity,
    WindowCandidate, WindowCandidateBackend, WindowControlCapabilities, WindowId,
};

#[derive(Debug, Default)]
pub struct X11Compositor;

impl X11Compositor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CompositorControl for X11Compositor {
    async fn ensure_script_loaded(&self) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn health(&self) -> Health {
        if std::env::var_os("DISPLAY").is_some() {
            Health::Online
        } else {
            Health::Offline {
                reason: "DISPLAY is not set".into(),
            }
        }
    }

    fn window_control_capabilities(&self) -> WindowControlCapabilities {
        x11_capabilities(
            std::env::var_os("DISPLAY").is_some(),
            std::env::var_os("PATH").as_deref(),
        )
    }

    async fn spawn_satellite(
        &self,
        argv: &[String],
        cwd: Option<&str>,
    ) -> Result<Uuid, CompositorError> {
        crate::spawn::spawn_tagged(argv, cwd)
    }

    async fn set_geometry(&self, _window: &WindowId, _rect: Rect) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn detach(&self, _window: &WindowId) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn attach(&self, _window: &WindowId) -> Result<(), CompositorError> {
        Ok(())
    }

    async fn list_windows(&self) -> Result<Vec<WindowCandidate>, CompositorError> {
        tokio::task::spawn_blocking(list_windows_blocking)
            .await
            .map_err(|err| CompositorError::Domain(format!("x11 list task failed: {err}")))?
    }

    async fn attach_window(
        &self,
        candidate: &WindowCandidate,
    ) -> Result<SatelliteWindowId, CompositorError> {
        if candidate.backend != WindowCandidateBackend::X11 {
            return Err(CompositorError::Domain(format!(
                "X11 backend cannot attach {:?} windows",
                candidate.backend
            )));
        }
        if !command_exists("xdotool") {
            return Err(CompositorError::Unsupported(
                "xdotool is not installed; X11 exact window control is unavailable".into(),
            ));
        }
        parse_x11_backend_window_id(&candidate.backend_window_id)?;
        let backend_window_id = candidate.backend_window_id.clone();
        let current = tokio::task::spawn_blocking(move || {
            list_windows_blocking()?
                .into_iter()
                .find(|window| window.backend_window_id == backend_window_id)
                .ok_or_else(|| {
                    CompositorError::Domain(format!(
                        "X11 window is not attachable or no longer exists: {backend_window_id}"
                    ))
                })
        })
        .await
        .map_err(|err| CompositorError::Domain(format!("x11 attach validation failed: {err}")))??;
        Ok(SatelliteWindowId::for_attached(&current))
    }

    async fn set_window_visible(
        &self,
        window: &SatelliteWindowId,
        visible: bool,
    ) -> Result<(), CompositorError> {
        let raw_id = parse_x11_backend_window_id(&window.backend_window_id)?;
        let command = if visible {
            vec!["windowmap", raw_id.as_str()]
        } else {
            vec!["windowminimize", raw_id.as_str()]
        };
        run_xdotool(&command).await?;
        if visible {
            run_xdotool(&["windowraise", raw_id.as_str()]).await?;
        }
        Ok(())
    }

    async fn raise_window(&self, window: &SatelliteWindowId) -> Result<(), CompositorError> {
        let raw_id = parse_x11_backend_window_id(&window.backend_window_id)?;
        run_xdotool(&["windowraise", raw_id.as_str()]).await?;
        run_xdotool(&["windowactivate", raw_id.as_str()]).await
    }
}

fn list_windows_blocking() -> Result<Vec<WindowCandidate>, CompositorError> {
    let root = run_command("xprop", &["-root", "_NET_CLIENT_LIST"])?;
    let mut out = Vec::new();
    for raw_id in parse_client_list(&root) {
        let props = match run_command(
            "xprop",
            &[
                "-id",
                raw_id.as_str(),
                "_NET_WM_NAME",
                "WM_NAME",
                "_NET_WM_PID",
                "WM_CLASS",
                "_NET_WM_DESKTOP",
            ],
        ) {
            Ok(props) => props,
            Err(err) => {
                tracing::debug!(window = %raw_id, error = %err, "x11: skipping unreadable window");
                continue;
            }
        };
        out.push(candidate_from_xprop(&raw_id, &props));
    }
    Ok(out)
}

fn run_command(command: &str, args: &[&str]) -> Result<String, CompositorError> {
    let output = std::process::Command::new(command)
        .args(args)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                CompositorError::Unsupported(format!("{command} is not installed"))
            } else {
                CompositorError::Io(err)
            }
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CompositorError::Domain(format!(
            "{command} failed{}",
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn command_exists(command: &str) -> bool {
    std::env::var_os("PATH")
        .as_deref()
        .is_some_and(|paths| command_exists_in_path(command, paths))
}

fn command_exists_in_path(command: &str, paths: &OsStr) -> bool {
    std::env::split_paths(paths).any(|path| {
        let candidate = path.join(command);
        candidate.is_file()
    })
}

fn x11_capabilities(display_present: bool, path: Option<&OsStr>) -> WindowControlCapabilities {
    let can_list =
        display_present && path.is_some_and(|paths| command_exists_in_path("xprop", paths));
    let can_control =
        can_list && path.is_some_and(|paths| command_exists_in_path("xdotool", paths));
    WindowControlCapabilities {
        list_windows: can_list,
        attach_window: can_control,
        set_visible: can_control,
        raise_window: can_control,
    }
}

async fn run_xdotool(args: &[&str]) -> Result<(), CompositorError> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        run_command("xdotool", &argv).map(|_| ())
    })
    .await
    .map_err(|err| CompositorError::Domain(format!("xdotool task failed: {err}")))?
}

fn parse_client_list(output: &str) -> Vec<String> {
    let Some((_, values)) = output.split_once('#') else {
        return Vec::new();
    };
    values
        .split(',')
        .filter_map(|part| normalize_x11_window_id(part.trim()))
        .collect()
}

fn candidate_from_xprop(raw_id: &str, props: &str) -> WindowCandidate {
    let wm_class = parse_wm_class(props);
    WindowCandidate {
        backend: WindowCandidateBackend::X11,
        backend_window_id: format!("x11:{raw_id}"),
        pid: parse_cardinal(props, "_NET_WM_PID"),
        app_identity: wm_class.map(WindowAppIdentity::WmClass),
        title: parse_string_property(props, "_NET_WM_NAME")
            .or_else(|| parse_string_property(props, "WM_NAME")),
        workspace: parse_cardinal(props, "_NET_WM_DESKTOP").map(|value| value.to_string()),
        output: None,
    }
}

fn parse_x11_backend_window_id(value: &str) -> Result<String, CompositorError> {
    value
        .strip_prefix("x11:")
        .and_then(normalize_x11_window_id)
        .ok_or_else(|| CompositorError::Domain(format!("invalid X11 backend window id: {value}")))
}

fn normalize_x11_window_id(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches(',');
    let hex = trimmed.strip_prefix("0x")?;
    if hex.is_empty() || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", hex.to_ascii_lowercase()))
}

fn parse_cardinal(props: &str, name: &str) -> Option<u32> {
    props.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        key.trim_start().starts_with(name).then_some(())?;
        value.trim().parse::<u32>().ok()
    })
}

fn parse_string_property(props: &str, name: &str) -> Option<String> {
    props.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        key.trim_start().starts_with(name).then_some(())?;
        parse_quoted_strings(value).into_iter().next()
    })
}

fn parse_wm_class(props: &str) -> Option<String> {
    props.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        key.trim_start().starts_with("WM_CLASS").then_some(())?;
        let values = parse_quoted_strings(value);
        values
            .get(1)
            .or_else(|| values.first())
            .filter(|value| !value.is_empty())
            .cloned()
    })
}

fn parse_quoted_strings(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' if in_quote => {
                out.push(current.clone());
                current.clear();
                in_quote = false;
            }
            '"' => in_quote = true,
            _ if in_quote => current.push(ch),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parses_client_list() {
        let output = "_NET_CLIENT_LIST(WINDOW): window id # 0x03a00007, 0x0400001a, 0xBAD";

        assert_eq!(
            parse_client_list(output),
            vec!["0x03a00007", "0x0400001a", "0xbad"]
        );
    }

    #[test]
    fn converts_xprop_to_window_candidate() {
        let props = r#"
_NET_WM_NAME(UTF8_STRING) = "Firefox"
WM_NAME(STRING) = "Fallback"
_NET_WM_PID(CARDINAL) = 1284
WM_CLASS(STRING) = "Navigator", "firefox"
_NET_WM_DESKTOP(CARDINAL) = 2
"#;

        let candidate = candidate_from_xprop("0x03a00007", props);

        assert_eq!(candidate.backend, WindowCandidateBackend::X11);
        assert_eq!(candidate.backend_window_id, "x11:0x03a00007");
        assert_eq!(candidate.pid, Some(1284));
        assert_eq!(
            candidate.app_identity,
            Some(WindowAppIdentity::WmClass("firefox".into()))
        );
        assert_eq!(candidate.title.as_deref(), Some("Firefox"));
        assert_eq!(candidate.workspace.as_deref(), Some("2"));
    }

    #[test]
    fn validates_exact_x11_backend_window_id() {
        assert_eq!(
            parse_x11_backend_window_id("x11:0x03A00007").unwrap(),
            "0x03a00007"
        );
        assert!(parse_x11_backend_window_id("pid:1284").is_err());
        assert!(parse_x11_backend_window_id("x11:not-hex").is_err());
    }

    #[test]
    fn parse_wm_class_prefers_class_over_instance() {
        let props = r#"WM_CLASS(STRING) = "navigator", "Firefox""#;
        assert_eq!(parse_wm_class(props).as_deref(), Some("Firefox"));
    }

    #[test]
    fn x11_capabilities_require_display_and_tools() {
        let caps = x11_capabilities(true, Some(OsStr::new("")));

        assert_eq!(caps, WindowControlCapabilities::default());
    }
}
