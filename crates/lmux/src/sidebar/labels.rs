use lmux_bus::kinds::WindowCandidateBackend;
use lmux_compositor::{WindowAppIdentity, WindowCandidate};
#[cfg(target_os = "macos")]
use lmux_macos_helper::WindowInfo as MacosWindowInfo;

pub(super) fn window_title(window: &WindowCandidate) -> String {
    window
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("(untitled window)")
        .to_string()
}

pub(super) fn window_meta(window: &WindowCandidate) -> String {
    let mut parts = vec![
        window_backend_label(window),
        window.backend_window_id.clone(),
    ];
    if let Some(pid) = window.pid {
        parts.push(format!("pid {pid}"));
    }
    if let Some(app) = app_identity_label(window.app_identity.as_ref()) {
        parts.push(app);
    }
    if let Some(workspace) = &window.workspace {
        parts.push(format!("workspace {workspace}"));
    }
    if let Some(output) = &window.output {
        parts.push(output.clone());
    }
    parts.join(" · ")
}

pub(super) fn window_backend_label(window: &WindowCandidate) -> String {
    match window.backend {
        WindowCandidateBackend::Macos => "macOS",
        WindowCandidateBackend::Kwin => "KWin",
        WindowCandidateBackend::X11 => "X11",
        WindowCandidateBackend::Hyprland => "Hyprland",
        WindowCandidateBackend::Sway => "Sway",
        WindowCandidateBackend::Noop => "Noop",
        WindowCandidateBackend::Unsupported => "Unsupported",
    }
    .to_string()
}

pub(super) fn window_app_label(window: &WindowCandidate) -> String {
    app_identity_label(window.app_identity.as_ref()).unwrap_or_default()
}

pub(super) fn app_identity_label(identity: Option<&WindowAppIdentity>) -> Option<String> {
    identity.map(|identity| match identity {
        WindowAppIdentity::BundleId(value)
        | WindowAppIdentity::DesktopEntry(value)
        | WindowAppIdentity::WmClass(value)
        | WindowAppIdentity::AppId(value)
        | WindowAppIdentity::Other(value) => value.clone(),
    })
}

pub(super) fn window_initials(window: &WindowCandidate) -> String {
    let source =
        app_identity_label(window.app_identity.as_ref()).unwrap_or_else(|| window_title(window));
    let initials: String = source
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase();
    if initials.is_empty() {
        "W".into()
    } else {
        initials
    }
}

#[cfg(target_os = "macos")]
pub(super) fn macos_window_title(window: &MacosWindowInfo) -> String {
    window
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("(untitled window)")
        .to_string()
}

#[cfg(target_os = "macos")]
pub(super) fn macos_window_meta(window: &MacosWindowInfo) -> String {
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
pub(super) fn macos_window_initials(window: &MacosWindowInfo) -> String {
    let source = window
        .bundle_id
        .as_deref()
        .and_then(|bundle| bundle.rsplit('.').next())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| macos_window_title(window));
    let initials: String = source
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase();
    if initials.is_empty() {
        "W".into()
    } else {
        initials
    }
}
