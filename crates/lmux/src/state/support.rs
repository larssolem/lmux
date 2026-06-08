use std::collections::{BTreeSet, HashMap};

use lmux_compositor::SatelliteWindowId;
#[cfg(target_os = "macos")]
use lmux_compositor::WindowBackend;
#[cfg(target_os = "macos")]
use lmux_macos_helper::WindowInfo as MacosWindowInfo;
use uuid::Uuid;

use crate::layout::PaneId;

pub(super) fn satellite_visibility_for_active(
    active: Option<PaneId>,
    windows_by_anchor: &HashMap<PaneId, Vec<SatelliteWindowId>>,
) -> (Vec<SatelliteWindowId>, Vec<SatelliteWindowId>) {
    let mut hide = Vec::new();
    let mut show = Vec::new();
    for (anchor, windows) in windows_by_anchor {
        if Some(*anchor) == active {
            show.extend(windows.iter().cloned());
        } else {
            hide.extend(windows.iter().cloned());
        }
    }
    (hide, show)
}

pub(super) fn owning_anchor_for_terminal_pane(
    pane_id: PaneId,
    anchors: &BTreeSet<PaneId>,
    pane_workspace: &HashMap<PaneId, PaneId>,
    terminal_tabs_by_anchor: &HashMap<PaneId, Vec<PaneId>>,
    pane_terminal_tab_roots: &HashMap<PaneId, PaneId>,
) -> Option<PaneId> {
    if anchors.contains(&pane_id) {
        return Some(pane_id);
    }
    if let Some(owner) = pane_workspace
        .get(&pane_id)
        .copied()
        .filter(|owner| anchors.contains(owner))
    {
        return Some(owner);
    }

    let tab_root = pane_terminal_tab_roots
        .get(&pane_id)
        .copied()
        .unwrap_or(pane_id);
    terminal_tabs_by_anchor.iter().find_map(|(anchor, tabs)| {
        if anchors.contains(anchor) && tabs.contains(&tab_root) {
            Some(*anchor)
        } else {
            None
        }
    })
}

pub(super) fn remove_satellite_request(
    windows_by_anchor: &mut HashMap<PaneId, Vec<SatelliteWindowId>>,
    request_id: Uuid,
) {
    for windows in windows_by_anchor.values_mut() {
        windows.retain(|existing| existing.request_id != Some(request_id));
    }
}

pub(super) fn remove_satellite_backend_window(
    windows_by_anchor: &mut HashMap<PaneId, Vec<SatelliteWindowId>>,
    backend_window_id: &str,
) {
    for windows in windows_by_anchor.values_mut() {
        windows.retain(|existing| existing.backend_window_id != backend_window_id);
    }
}

pub(super) fn insert_satellite_window_for_anchor(
    windows_by_anchor: &mut HashMap<PaneId, Vec<SatelliteWindowId>>,
    anchor: PaneId,
    window: SatelliteWindowId,
) {
    if let Some(request_id) = window.request_id {
        remove_satellite_request(windows_by_anchor, request_id);
    }
    remove_satellite_backend_window(windows_by_anchor, &window.backend_window_id);
    windows_by_anchor.entry(anchor).or_default().push(window);
}

#[cfg(target_os = "linux")]
pub(super) fn agent_identity_from_child_env(pid: u32) -> Option<lmux_bus::AgentIdentity> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let mut id = None;
    let mut name = None;
    for entry in bytes.split(|b| *b == 0) {
        if let Some(value) = entry.strip_prefix(b"LMUX_AGENT_ID=") {
            let value = String::from_utf8_lossy(value).trim().to_string();
            if !value.is_empty() {
                id = Some(value);
            }
        } else if let Some(value) = entry.strip_prefix(b"LMUX_AGENT_NAME=") {
            let value = String::from_utf8_lossy(value).trim().to_string();
            if !value.is_empty() {
                name = Some(value);
            }
        }
    }
    id.map(|id| lmux_bus::AgentIdentity { id, name })
}

#[cfg(not(target_os = "linux"))]
pub(super) fn agent_identity_from_child_env(_pid: u32) -> Option<lmux_bus::AgentIdentity> {
    None
}

#[cfg(target_os = "macos")]
pub(super) fn macos_backend_window_id(window: &MacosWindowInfo) -> String {
    match window.window_id {
        Some(window_id) => format!("macos-window-id:{window_id}:index:{}", window.window_index),
        None => format!(
            "macos-window-pid:{}:index:{}",
            window.pid, window.window_index
        ),
    }
}

#[cfg(target_os = "macos")]
pub(super) fn macos_satellite_for_window(
    request_id: Uuid,
    bundle_id: Option<String>,
    window: &MacosWindowInfo,
) -> SatelliteWindowId {
    SatelliteWindowId {
        backend: WindowBackend::Macos,
        request_id: Some(request_id),
        pid: Some(window.pid),
        backend_window_id: macos_backend_window_id(window),
        bundle_id: window.bundle_id.clone().or(bundle_id),
        title: window.title.clone(),
    }
}
