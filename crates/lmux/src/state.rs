//! Shared application state: the map of live panes, the layout tree, the
//! focused pane id, and the widget-tree builder that turns `Layout` into
//! nested `gtk::Paned` instances. Separated from `app` so keyboard/click
//! callbacks can share a single `Rc<RefCell<AppState>>`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Instant;

#[cfg(target_os = "linux")]
use async_channel::Sender;
use gtk4::prelude::*;
use gtk4::{
    Align, Button, Entry, GestureClick, Label, Orientation, Paned, Popover, PositionType, Stack,
    Widget,
};
use lmux_anchor::{Anchor, AnchorRegistry};
use lmux_compositor::{FocusPolicy, SatelliteWindowId, WindowBackend, WindowCandidate};
#[cfg(target_os = "macos")]
use lmux_macos_helper::WindowInfo as MacosWindowInfo;
#[cfg(target_os = "linux")]
use lmux_wayland_host::{HostCommand, HostEvent, HostHandle, SurfaceId};
use uuid::Uuid;

use crate::agent_control::{self, AgentPaneMetadata, GrantRecord};
use crate::layout::{Dir, Layout, PaneId};
use crate::pane::{
    BellCallback, FocusCallback, Pane, ShortcutPrefixCell, TerminalActionCallback,
    TerminalExitCallback,
};
#[cfg(target_os = "linux")]
use crate::satellite::SatelliteWidget;

/// CSS class applied to an anchor pane's Frame. Paired with the provider
/// loaded in `app::install_css`.
pub const ANCHOR_CSS_CLASS: &str = "pane--anchor";

/// CSS class carried by the currently-focused pane's Frame. Drives the
/// blue outline (styled in `APP_CSS`) so the user can tell at a glance
/// which pane their keystrokes will land in — especially useful for
/// satellites, since browser/IDE focus chrome isn't consistent across
/// apps.
pub const FOCUSED_CSS_CLASS: &str = "pane--focused";
/// Marker class added to the root container while rearrange mode is on.
/// CSS can hang dashed pane outlines / drop-zone overlays off this.
pub const REARRANGE_CSS_CLASS: &str = "lmux--rearrange";

/// CSS class for the currently active anchor — the one rendered on screen.
/// Other tagged anchors keep `pane--anchor` but lose `pane--anchor-active`.
pub const ANCHOR_ACTIVE_CSS_CLASS: &str = "pane--anchor-active";

fn default_window_backend() -> WindowBackend {
    #[cfg(target_os = "macos")]
    {
        WindowBackend::Macos
    }
    #[cfg(target_os = "linux")]
    {
        WindowBackend::Kwin
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        WindowBackend::Noop
    }
}

fn satellite_visibility_for_active(
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

fn owning_anchor_for_terminal_pane(
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

fn remove_satellite_request(
    windows_by_anchor: &mut HashMap<PaneId, Vec<SatelliteWindowId>>,
    request_id: Uuid,
) {
    for windows in windows_by_anchor.values_mut() {
        windows.retain(|existing| existing.request_id != Some(request_id));
    }
}

fn remove_satellite_backend_window(
    windows_by_anchor: &mut HashMap<PaneId, Vec<SatelliteWindowId>>,
    backend_window_id: &str,
) {
    for windows in windows_by_anchor.values_mut() {
        windows.retain(|existing| existing.backend_window_id != backend_window_id);
    }
}

fn insert_satellite_window_for_anchor(
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

fn noop_terminal_action_callback() -> TerminalActionCallback {
    Rc::new(|_, _| {})
}

#[cfg(target_os = "linux")]
fn agent_identity_from_child_env(pid: u32) -> Option<lmux_bus::AgentIdentity> {
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
fn agent_identity_from_child_env(_pid: u32) -> Option<lmux_bus::AgentIdentity> {
    None
}

#[cfg(target_os = "macos")]
fn macos_backend_window_id(window: &MacosWindowInfo) -> String {
    match window.window_id {
        Some(window_id) => format!("macos-window-id:{window_id}:index:{}", window.window_index),
        None => format!(
            "macos-window-pid:{}:index:{}",
            window.pid, window.window_index
        ),
    }
}

#[cfg(target_os = "macos")]
fn macos_satellite_for_window(
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use uuid::Uuid;

    #[test]
    fn satellite_visibility_follows_active_anchor() {
        let active_window =
            SatelliteWindowId::for_spawn(WindowBackend::Macos, Uuid::from_u128(1), 10);
        let inactive_window =
            SatelliteWindowId::for_spawn(WindowBackend::Macos, Uuid::from_u128(2), 20);
        let mut windows = HashMap::new();
        windows.insert(1, vec![active_window.clone()]);
        windows.insert(2, vec![inactive_window.clone()]);

        let (hide, show) = satellite_visibility_for_active(Some(1), &windows);

        assert_eq!(show, vec![active_window]);
        assert_eq!(hide, vec![inactive_window]);
    }

    #[test]
    fn satellite_request_replacement_removes_old_owner_records() {
        let request_id = Uuid::from_u128(42);
        let old_window = SatelliteWindowId::for_spawn(WindowBackend::Macos, request_id, 10);
        let other_window =
            SatelliteWindowId::for_spawn(WindowBackend::Macos, Uuid::from_u128(43), 20);
        let mut windows = HashMap::new();
        windows.insert(1, vec![old_window]);
        windows.insert(2, vec![other_window.clone()]);

        remove_satellite_request(&mut windows, request_id);

        assert!(windows.get(&1).is_some_and(Vec::is_empty));
        assert_eq!(windows.get(&2), Some(&vec![other_window]));
    }

    #[test]
    fn satellite_backend_window_removal_drops_only_matching_window() {
        let old_window = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(42)),
            pid: Some(10),
            backend_window_id: "macos-window-id:100:index:1".into(),
            bundle_id: None,
            title: None,
        };
        let keep_window = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(43)),
            pid: Some(10),
            backend_window_id: "macos-window-id:101:index:2".into(),
            bundle_id: None,
            title: None,
        };
        let mut windows = HashMap::new();
        windows.insert(1, vec![old_window, keep_window.clone()]);

        remove_satellite_backend_window(&mut windows, "macos-window-id:100:index:1");

        assert_eq!(windows.get(&1), Some(&vec![keep_window]));
    }

    #[test]
    fn satellite_backend_window_insert_moves_existing_window_between_anchors() {
        let window = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(42)),
            pid: Some(10),
            backend_window_id: "macos-window-id:100:index:1".into(),
            bundle_id: Some("com.example.App".into()),
            title: Some("Example".into()),
        };
        let mut windows = HashMap::new();
        windows.insert(1, vec![window.clone()]);

        insert_satellite_window_for_anchor(&mut windows, 2, window.clone());

        assert_eq!(windows.get(&1), Some(&Vec::new()));
        assert_eq!(windows.get(&2), Some(&vec![window]));
    }

    #[test]
    fn two_chrome_windows_on_different_anchors_switch_independently() {
        let chrome_a = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(1)),
            pid: Some(100),
            backend_window_id: "macos-window-id:1001:index:1".into(),
            bundle_id: Some("com.google.Chrome".into()),
            title: Some("Chrome A".into()),
        };
        let chrome_b = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(2)),
            pid: Some(100),
            backend_window_id: "macos-window-id:1002:index:2".into(),
            bundle_id: Some("com.google.Chrome".into()),
            title: Some("Chrome B".into()),
        };
        let mut windows = HashMap::new();
        windows.insert(1, vec![chrome_a.clone()]);
        windows.insert(2, vec![chrome_b.clone()]);

        let (hide, show) = satellite_visibility_for_active(Some(1), &windows);

        assert_eq!(show, vec![chrome_a]);
        assert_eq!(hide, vec![chrome_b]);
    }

    #[test]
    fn two_intellij_windows_on_different_anchors_switch_independently() {
        let idea_a = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(3)),
            pid: Some(200),
            backend_window_id: "macos-window-id:2001:index:1".into(),
            bundle_id: Some("com.jetbrains.intellij".into()),
            title: Some("Project A".into()),
        };
        let idea_b = SatelliteWindowId {
            backend: WindowBackend::Macos,
            request_id: Some(Uuid::from_u128(4)),
            pid: Some(200),
            backend_window_id: "macos-window-id:2002:index:2".into(),
            bundle_id: Some("com.jetbrains.intellij".into()),
            title: Some("Project B".into()),
        };
        let mut windows = HashMap::new();
        windows.insert(1, vec![idea_a.clone()]);
        windows.insert(2, vec![idea_b.clone()]);

        let (hide, show) = satellite_visibility_for_active(Some(2), &windows);

        assert_eq!(show, vec![idea_b]);
        assert_eq!(hide, vec![idea_a]);
    }

    #[test]
    fn set_active_anchor_hot_path_has_no_inline_macos_reconciliation() {
        let source = include_str!("state.rs");
        let start = source.find("pub fn set_active_anchor").unwrap();
        let tail = &source[start..];
        let end = tail.find("    /// Sidebar accessor").unwrap();
        let body = &tail[..end];

        assert!(!body.contains("reconcile_macos"));
        assert!(!body.contains("lmux_macos_helper"));
        assert!(!body.contains("list_windows("));
    }

    #[test]
    fn next_anchor_sort_key_appends_within_group() {
        let mut registry = AnchorRegistry::default();
        let mut first = Anchor::new_manual(Uuid::new_v4(), vec!["zsh".into()], String::new());
        first.sort_key = Some(0);
        let first_id = first.id;
        registry.insert(first);

        let mut second = Anchor::new_manual(Uuid::new_v4(), vec!["zsh".into()], String::new());
        second.sort_key = Some(1);
        let second_id = second.id;
        registry.insert(second);

        let mut grouped = Anchor::new_manual(Uuid::new_v4(), vec!["zsh".into()], String::new());
        grouped.group = Some("Work".into());
        grouped.sort_key = Some(9);
        let grouped_id = grouped.id;
        registry.insert(grouped);

        assert_eq!(
            next_anchor_sort_key_for([first_id, second_id, grouped_id], &registry, None),
            2
        );
        assert_eq!(
            next_anchor_sort_key_for([first_id, second_id, grouped_id], &registry, Some("Work")),
            10
        );
    }

    #[test]
    fn stale_tab_root_filters_otherwise_owned_anchor_leaf() {
        let layout = Layout::Split {
            dir: Dir::Vertical,
            a: Box::new(Layout::Leaf(1)),
            b: Box::new(Layout::Leaf(2)),
            ratio: 0.5,
        };
        let workspace = HashMap::from([(1, 1), (2, 2)]);

        let stale = prune_to_workspace(
            &layout,
            2,
            2,
            &workspace,
            |pane| {
                if pane == 2 {
                    1
                } else {
                    pane
                }
            },
        );
        assert!(stale.is_none());

        let fixed = prune_to_workspace(&layout, 2, 2, &workspace, |pane| pane);
        let fixed = match fixed {
            Some(layout) => layout,
            None => panic!("new anchor pane should remain visible"),
        };
        assert_eq!(fixed.leaves(), vec![2]);
    }

    #[test]
    fn terminal_tab_pane_reports_owning_anchor() {
        let anchors = BTreeSet::from([1]);
        let workspace = HashMap::new();
        let terminal_tabs = HashMap::from([(1, vec![1, 5])]);
        let tab_roots = HashMap::from([(1, 1), (5, 5)]);

        assert_eq!(
            owning_anchor_for_terminal_pane(5, &anchors, &workspace, &terminal_tabs, &tab_roots),
            Some(1)
        );
        assert_eq!(
            owning_anchor_for_terminal_pane(9, &anchors, &workspace, &terminal_tabs, &tab_roots),
            None
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    ShuttingDown,
}

pub struct PaneCreateRequest {
    pub target_anchor: Option<Uuid>,
    pub placement: lmux_bus::PanePlacement,
    pub activate: bool,
    pub title: Option<String>,
    pub argv: Vec<String>,
    pub agent: Option<lmux_bus::AgentIdentity>,
    pub purpose: Option<String>,
}

pub struct AppState {
    panes: HashMap<PaneId, Pane>,
    layout: Layout,
    focused: PaneId,
    next_id: PaneId,
    /// Set of pane ids currently tagged as anchors. Multi-anchor: an
    /// author can pin multiple panes (build watcher + dev server + logs)
    /// simultaneously. Satellites (Epic 9) are excluded — only terminal
    /// panes qualify.
    anchors: BTreeSet<PaneId>,
    /// Bridge from PaneId → anchor Uuid. Lets the sidebar resolve metadata
    /// (name/group/sort_key) for the panes in `anchors` without duplicating
    /// storage.
    pane_anchor_ids: HashMap<PaneId, Uuid>,
    /// Stable per-pane identity used by the bus (and any future IPC) to
    /// address a non-anchor pane. Assigned once at insert time and never
    /// reused. Drops when the pane is closed / replaced. NOT persisted
    /// across restarts — the identifier's scope is "this cockpit process".
    pane_uuids: HashMap<PaneId, Uuid>,
    /// Visible pane titles set through agent/user control surfaces.
    pane_titles: HashMap<PaneId, lmux_bus::PaneTitle>,
    /// Agent ownership metadata for panes created or managed by agents.
    pane_agents: HashMap<PaneId, AgentPaneMetadata>,
    /// Process-local grant state for agent access requests.
    grants: HashMap<Uuid, GrantRecord>,
    /// Anchor-local terminal tabs. Values are tab root pane ids; each tab can
    /// own a split subtree through `pane_terminal_tab_roots`.
    terminal_tabs_by_anchor: HashMap<PaneId, Vec<PaneId>>,
    active_terminal_tab_by_anchor: HashMap<PaneId, PaneId>,
    pane_terminal_tab_roots: HashMap<PaneId, PaneId>,
    /// Workspace membership: for each non-anchor pane, the id of the anchor
    /// it belongs to (its "satellite ownership"). Anchor panes map to
    /// themselves. Panes absent from this map are *unowned* — they exist
    /// before any anchor has been tagged and are visible only when no
    /// anchor is active. Each satellite belongs to exactly one anchor
    /// (enforced by [`AppState::add_anchor`] rejecting re-tag of a
    /// pane already owned by a different anchor).
    pane_workspace: HashMap<PaneId, PaneId>,
    /// Authoritative metadata store for tagged panes. One `Anchor` per
    /// entry in `anchors`; renamed/regrouped here, then the sidebar reads
    /// back via [`AppState::anchor_for_pane`].
    anchor_registry: AnchorRegistry,
    /// The single anchor that is currently "on screen". Only one anchor
    /// is rendered at a time — other tagged anchors (and any satellites
    /// they own) are hidden until the user switches to them via the
    /// sidebar or by tagging a new pane (which promotes it to active).
    /// `None` when no anchor is tagged.
    active_anchor: Option<PaneId>,
    /// Anchors that the user has explicitly hidden via `anchor.hide`. The
    /// pane + child process stay alive; only the widget is detached. When
    /// the user re-runs `anchor.reattach` the pane becomes visible again
    /// (subject to the usual workspace-membership filter).
    hidden_anchors: BTreeSet<PaneId>,
    root_container: gtk4::Box,
    workspace_stack: RefCell<Option<Stack>>,
    focus_cb: Option<FocusCallback>,
    /// Shared focus-mode cell cloned into every pane's hover handler. Live
    /// config reload mutates this via `apply_config`; no re-attach needed.
    focus_mode: crate::pane::FocusModeCell,
    /// Shared "rearrange mode" flag. When true, every pane's `DropTarget`
    /// accepts pane-id drops and rewires the layout tree on drop. The
    /// `Ctrl+B m` handler toggles this; sidebar reflects it via CSS.
    rearrange_mode: crate::pane::RearrangeModeCell,
    /// Re-parent callback wired into every pane's drop target so a drop
    /// in rearrange mode reaches `AppState::reparent_pane`. Cached so
    /// panes added later (splits, satellites) can be wired identically.
    reparent_cb: Option<crate::pane::ReparentCallback>,
    terminal_action_cb: Option<TerminalActionCallback>,
    shortcut_prefix: ShortcutPrefixCell,
    bell_cb: Option<BellCallback>,
    terminal_exit_cb: Option<TerminalExitCallback>,
    terminal_tab_cb: Option<TerminalTabActivatedCallback>,
    terminal_tab_rename_cb: Option<TerminalTabRenameCallback>,
    /// Notification ids per-pane, so the next bell from a pane replaces the
    /// previous toast rather than stacking. Story 6.2.
    last_notif_id: HashMap<PaneId, u32>,
    phase: Phase,
    /// Fires after any mutation to the anchor set (add/remove/close).
    /// The sidebar widget registers here to rebuild its row list; the bus
    /// status.get handler registers here to keep its atomic anchor count
    /// in sync.
    on_anchors_changed: Vec<AnchorsChangedCallback>,
    /// Fires when only the active anchor changes. Sidebar rows use this to
    /// update active styling without rebuilding the full anchor list.
    on_active_anchor_changed: Vec<ActiveAnchorChangedCallback>,
    /// Name of the session whose on-disk snapshot we're currently
    /// editing. `None` means "unnamed / legacy v0.1 single-session mode"
    /// — the switcher sets it to `Some(name)` on the first swap, and
    /// subsequent swaps save back to that name before loading the next.
    current_session: Option<String>,
    /// Which anchor owns each live GUI satellite.
    /// Populated by the launcher on successful spawn and drained by
    /// `set_active_anchor` so satellites share the lifecycle of their
    /// owning anchor (hide on switch-away, show on switch-back).
    satellite_windows_by_anchor: HashMap<PaneId, Vec<SatelliteWindowId>>,
    satellite_visibility_seq: u64,
    /// Sender to the compositor bridge thread. `None` in unit tests and
    /// on the snapshot-restore path before the cockpit wires one up.
    compositor_tx: Option<crate::compositor_bridge::CompositorSender>,
    /// Handle to the nested Wayland compositor thread (ADR-0018). Held so
    /// the compositor lives as long as AppState; dropping it requests
    /// shutdown. `None` when the compositor failed to start (e.g. CI
    /// without `XDG_RUNTIME_DIR`) — the cockpit still works, just without
    /// GUI satellites.
    #[cfg(target_os = "linux")]
    wayland_host: Option<HostHandle>,
    /// Command channel to the nested compositor. Cloned per satellite so
    /// GTK widgets can post `HostCommand::ResizeToplevel`, pointer/key
    /// events, etc. `None` when `wayland_host` is None.
    #[cfg(target_os = "linux")]
    host_cmd_tx: Option<Sender<HostCommand>>,
    /// Reverse lookup from nested-compositor surface id to the PaneId we
    /// allocated for it. Lets host events (FrameReady, Title/Close)
    /// find the right satellite in `self.panes` in O(1).
    #[cfg(target_os = "linux")]
    surface_to_pane: HashMap<SurfaceId, PaneId>,
    /// Reverse lookup from popup SurfaceId to the parent satellite's PaneId,
    /// so popup-targeted frames/repositions/closes route to the right
    /// `SatelliteWidget` overlay.
    #[cfg(target_os = "linux")]
    popup_to_pane: HashMap<SurfaceId, PaneId>,
    /// Socket name advertised by the nested compositor (`lmux-<pid>`).
    /// Set when `HostEvent::Ready` is dispatched so the launcher can set
    /// `WAYLAND_DISPLAY` on satellite children. `None` before the host
    /// signals ready (or when the host isn't running at all).
    #[cfg(target_os = "linux")]
    wayland_display_name: Option<String>,
}

pub type AnchorsChangedCallback = Rc<dyn Fn()>;
pub type ActiveAnchorChangedCallback = Rc<dyn Fn(Option<PaneId>)>;
pub type TerminalTabActivatedCallback = Rc<dyn Fn(PaneId, PaneId)>;
pub type TerminalTabRenameCallback = Rc<dyn Fn(PaneId, Option<String>)>;

pub type SharedAppState = Rc<RefCell<AppState>>;

#[derive(Clone)]
pub struct AnchorAgentActivity {
    pub agents: Vec<lmux_bus::AgentPaneStatus>,
    pub pending_grants: u32,
    pub active_grants: u32,
}

#[derive(Clone)]
pub struct AnchorGrantView {
    pub grant_id: Uuid,
    pub requester: lmux_bus::AgentIdentity,
    pub scope: lmux_bus::GrantScope,
    pub reason: Option<String>,
    pub pending: bool,
    pub active: bool,
}

impl AppState {
    pub fn new(root_container: gtk4::Box, first: Pane) -> Self {
        let focused = first.id();
        let mut panes = HashMap::new();
        panes.insert(first.id(), first);
        let mut pane_uuids = HashMap::new();
        pane_uuids.insert(focused, Uuid::new_v4());
        Self {
            panes,
            layout: Layout::Leaf(focused),
            focused,
            next_id: focused + 1,
            anchors: BTreeSet::new(),
            pane_anchor_ids: HashMap::new(),
            pane_uuids,
            pane_titles: HashMap::new(),
            pane_agents: HashMap::new(),
            grants: HashMap::new(),
            terminal_tabs_by_anchor: HashMap::new(),
            active_terminal_tab_by_anchor: HashMap::new(),
            pane_terminal_tab_roots: HashMap::new(),
            pane_workspace: HashMap::new(),
            anchor_registry: AnchorRegistry::default(),
            active_anchor: None,
            hidden_anchors: BTreeSet::new(),
            root_container,
            workspace_stack: RefCell::new(None),
            focus_cb: None,
            focus_mode: Rc::new(std::cell::Cell::new(lmux_config::FocusMode::default())),
            rearrange_mode: Rc::new(std::cell::Cell::new(false)),
            reparent_cb: None,
            terminal_action_cb: None,
            shortcut_prefix: Rc::new(RefCell::new("ctrl+b".to_string())),
            bell_cb: None,
            terminal_exit_cb: None,
            terminal_tab_cb: None,
            terminal_tab_rename_cb: None,
            last_notif_id: HashMap::new(),
            phase: Phase::Running,
            on_anchors_changed: Vec::new(),
            on_active_anchor_changed: Vec::new(),
            current_session: None,
            satellite_windows_by_anchor: HashMap::new(),
            satellite_visibility_seq: 0,
            compositor_tx: None,
            #[cfg(target_os = "linux")]
            wayland_host: None,
            #[cfg(target_os = "linux")]
            host_cmd_tx: None,
            #[cfg(target_os = "linux")]
            surface_to_pane: HashMap::new(),
            #[cfg(target_os = "linux")]
            popup_to_pane: HashMap::new(),
            #[cfg(target_os = "linux")]
            wayland_display_name: None,
        }
    }

    /// Constructor used by the snapshot-restore path (Story 8.3). The caller
    /// has already spawned every `Pane` at its recorded CWD and built the
    /// matching `Layout` tree; we just install the widget tree and tag every
    /// restored anchor.
    pub fn new_from_snapshot(
        root_container: gtk4::Box,
        panes: HashMap<PaneId, Pane>,
        layout: Layout,
        focused: PaneId,
        anchors: BTreeSet<PaneId>,
        next_id: PaneId,
    ) -> Self {
        let pane_uuids: HashMap<PaneId, Uuid> =
            panes.keys().map(|id| (*id, Uuid::new_v4())).collect();
        let mut st = Self {
            panes,
            layout,
            focused,
            next_id,
            anchors: BTreeSet::new(),
            pane_anchor_ids: HashMap::new(),
            pane_uuids,
            pane_titles: HashMap::new(),
            pane_agents: HashMap::new(),
            grants: HashMap::new(),
            terminal_tabs_by_anchor: HashMap::new(),
            active_terminal_tab_by_anchor: HashMap::new(),
            pane_terminal_tab_roots: HashMap::new(),
            pane_workspace: HashMap::new(),
            anchor_registry: AnchorRegistry::default(),
            active_anchor: None,
            hidden_anchors: BTreeSet::new(),
            root_container,
            workspace_stack: RefCell::new(None),
            focus_cb: None,
            focus_mode: Rc::new(std::cell::Cell::new(lmux_config::FocusMode::default())),
            rearrange_mode: Rc::new(std::cell::Cell::new(false)),
            reparent_cb: None,
            terminal_action_cb: None,
            shortcut_prefix: Rc::new(RefCell::new("ctrl+b".to_string())),
            bell_cb: None,
            terminal_exit_cb: None,
            terminal_tab_cb: None,
            terminal_tab_rename_cb: None,
            last_notif_id: HashMap::new(),
            phase: Phase::Running,
            on_anchors_changed: Vec::new(),
            on_active_anchor_changed: Vec::new(),
            current_session: None,
            satellite_windows_by_anchor: HashMap::new(),
            satellite_visibility_seq: 0,
            compositor_tx: None,
            #[cfg(target_os = "linux")]
            wayland_host: None,
            #[cfg(target_os = "linux")]
            host_cmd_tx: None,
            #[cfg(target_os = "linux")]
            surface_to_pane: HashMap::new(),
            #[cfg(target_os = "linux")]
            popup_to_pane: HashMap::new(),
            #[cfg(target_os = "linux")]
            wayland_display_name: None,
        };
        st.rebuild_widget_tree();
        // Restore anchors without the "absorb everything" semantics so
        // multi-anchor snapshots don't collapse every pane under the
        // first restored anchor. The primary anchor still owns all
        // non-anchor leaves (the snapshot doesn't yet track per-satellite
        // ownership), but each anchor self-owns its own slot.
        let first_anchor = anchors.iter().copied().next();
        for id in &anchors {
            st.pane_workspace.insert(*id, *id);
        }
        if let Some(primary) = first_anchor {
            let unowned: Vec<PaneId> = st
                .panes
                .keys()
                .copied()
                .filter(|id| !st.pane_workspace.contains_key(id))
                .collect();
            for id in unowned {
                st.pane_workspace.insert(id, primary);
            }
        }
        for id in anchors {
            st.restore_anchor(id);
        }
        if let Some(primary) = first_anchor {
            st.set_active_anchor(Some(primary));
        }
        st
    }

    /// Snapshot the current session — invoked before SIGTERM so `/proc/<pid>/
    /// cwd` is still readable for each pane (Story 8.2).
    pub fn snapshot(&self) -> lmux_state::SessionSnapshot {
        let mut cwds: std::collections::BTreeMap<u32, String> = std::collections::BTreeMap::new();
        for (id, pane) in &self.panes {
            if let Some(p) = pane.snapshot_cwd() {
                cwds.insert(*id, p.to_string_lossy().into_owned());
            }
        }
        let anchor_pane_ids: Vec<u32> = self.anchors.iter().copied().collect();
        // Populate the legacy singleton too so a v0.1 reader (or the
        // migration path in lmux-session) still gets something usable.
        let anchor_pane_id = anchor_pane_ids.first().copied();
        // Strip satellite leaves from the layout before serializing.
        // Satellites are live GUI client connections — they can't be
        // respawned on restore and leaving them in the tree produces
        // empty pane slots ("white boxes") that squish the real panes.
        let mut layout_for_snapshot = self.layout.clone();
        let satellite_ids: Vec<PaneId> = self
            .panes
            .iter()
            .filter_map(|(id, p)| p.is_satellite().then_some(*id))
            .collect();
        for id in satellite_ids {
            layout_for_snapshot.remove_leaf(id);
        }
        lmux_state::SessionSnapshot {
            v: lmux_state::SCHEMA_VERSION,
            created_at_unix_seconds: lmux_state::now_unix_seconds(),
            anchor_pane_id,
            anchor_pane_ids,
            layout: layout_to_snapshot(&layout_for_snapshot),
            cwds,
            pane_titles: self
                .pane_titles
                .iter()
                .filter(|(pane_id, _)| self.panes.contains_key(pane_id))
                .map(|(pane_id, title)| (*pane_id, pane_title_to_snapshot(title)))
                .collect(),
            terminal_tabs: self
                .terminal_tabs_by_anchor
                .iter()
                .filter(|(anchor, _)| self.panes.contains_key(anchor))
                .map(|(anchor, tabs)| lmux_state::TerminalTabStackSnapshot {
                    anchor_pane_id: *anchor,
                    tab_roots: tabs
                        .iter()
                        .copied()
                        .filter(|pane| self.panes.contains_key(pane))
                        .collect(),
                    active_tab: self.active_terminal_tab_by_anchor.get(anchor).copied(),
                })
                .filter(|stack| !stack.tab_roots.is_empty())
                .collect(),
            pane_terminal_tab_roots: self
                .pane_terminal_tab_roots
                .iter()
                .filter_map(|(pane, root)| {
                    (self.panes.contains_key(pane) && self.panes.contains_key(root))
                        .then_some((*pane, *root))
                })
                .collect(),
        }
    }

    /// Mark the state as shutting down and drain all panes. Returns `None` if
    /// shutdown is already in progress (idempotency for Story 7.1).
    pub fn begin_shutdown(&mut self) -> Option<Vec<Pane>> {
        if self.phase == Phase::ShuttingDown {
            return None;
        }
        self.phase = Phase::ShuttingDown;
        Some(self.drain_panes_for_shutdown())
    }

    pub fn set_bell_callback(&mut self, cb: BellCallback) {
        for pane in self.panes.values() {
            pane.set_bell_callback(cb.clone());
        }
        self.bell_cb = Some(cb);
    }

    pub fn set_terminal_exit_callback(&mut self, cb: TerminalExitCallback) {
        for pane in self.panes.values() {
            pane.set_exit_callback(cb.clone());
        }
        self.terminal_exit_cb = Some(cb);
    }

    pub fn set_terminal_tab_activated_callback(&mut self, cb: TerminalTabActivatedCallback) {
        self.terminal_tab_cb = Some(cb);
        self.rebuild_widget_tree();
    }

    pub fn set_terminal_tab_rename_callback(&mut self, cb: TerminalTabRenameCallback) {
        self.terminal_tab_rename_cb = Some(cb);
        self.rebuild_widget_tree();
    }

    /// Record the notification id returned by the notification daemon so
    /// subsequent bells from the same pane set `replaces_id` and produce a
    /// single replacing toast rather than stacking.
    pub fn record_notif_id(&mut self, pane_id: PaneId, id: u32) {
        self.last_notif_id.insert(pane_id, id);
    }

    pub fn replaces_id_for(&self, pane_id: PaneId) -> u32 {
        self.last_notif_id.get(&pane_id).copied().unwrap_or(0)
    }

    /// Human-readable label shown in the toast body. Anchor panes get the
    /// `[anchor]` prefix per Story 6.2.
    pub fn pane_label(&self, pane_id: PaneId) -> String {
        if self.anchors.contains(&pane_id) {
            format!("[anchor] pane {pane_id}: output ready")
        } else {
            format!("pane {pane_id}: output ready")
        }
    }

    /// Current anchor set (read-only view). Ordered ascending by pane id
    /// so sidebar rendering is deterministic. Consumed by the always-on
    /// sidebar widget (Epic 5) — `#[allow(dead_code)]` until that lands.
    #[allow(dead_code)]
    pub fn anchor_count(&self) -> u32 {
        self.anchors.len() as u32
    }

    fn satellite_window_count(&self) -> usize {
        self.satellite_windows_by_anchor
            .values()
            .map(Vec::len)
            .sum()
    }

    #[cfg(target_os = "macos")]
    pub fn macos_attached_anchor_for_window(
        &self,
        window: &MacosWindowInfo,
    ) -> Option<(PaneId, String)> {
        let backend_window_id = macos_backend_window_id(window);
        self.satellite_windows_by_anchor
            .iter()
            .find_map(|(anchor, windows)| {
                windows
                    .iter()
                    .any(|existing| existing.backend_window_id == backend_window_id)
                    .then(|| {
                        let label = self
                            .anchor_for_pane(*anchor)
                            .map(|anchor| anchor.display_label().to_string())
                            .unwrap_or_else(|| format!("pane {anchor}"));
                        (*anchor, label)
                    })
            })
    }

    pub fn attached_anchor_for_backend_window(
        &self,
        backend_window_id: &str,
    ) -> Option<(PaneId, String)> {
        self.satellite_windows_by_anchor
            .iter()
            .find_map(|(anchor, windows)| {
                windows
                    .iter()
                    .any(|existing| existing.backend_window_id == backend_window_id)
                    .then(|| {
                        let label = self
                            .anchor_for_pane(*anchor)
                            .map(|anchor| anchor.display_label().to_string())
                            .unwrap_or_else(|| format!("pane {anchor}"));
                        (*anchor, label)
                    })
            })
    }

    /// Reverse of `pane_anchor_ids`: given the UUID stored on the `Anchor`
    /// registry entry, return the pane that currently owns it. Used by the
    /// bus dispatcher to route `anchor.pause` / `anchor.resume` kinds that
    /// identify the target by UUID.
    pub fn pane_for_anchor_id(&self, id: Uuid) -> Option<PaneId> {
        self.pane_anchor_ids
            .iter()
            .find_map(|(pane, uuid)| if *uuid == id { Some(*pane) } else { None })
    }

    pub fn anchor_uuid_for_pane(&self, pane_id: PaneId) -> Option<Uuid> {
        self.pane_anchor_ids.get(&pane_id).copied()
    }

    /// Stable (for this process) UUID identity of a pane. Assigned at pane
    /// creation time so the bus can address a non-anchor pane for
    /// `anchor.tag`. Returns `None` for unknown ids.
    #[allow(dead_code)]
    pub fn pane_uuid(&self, pane_id: PaneId) -> Option<Uuid> {
        self.pane_uuids.get(&pane_id).copied()
    }

    /// Reverse of [`pane_uuid`]: resolve a pane UUID back to its PaneId.
    /// Returns `None` when no live pane carries this UUID.
    pub fn pane_for_uuid(&self, uuid: Uuid) -> Option<PaneId> {
        self.pane_uuids
            .iter()
            .find_map(|(pane, u)| if *u == uuid { Some(*pane) } else { None })
    }

    pub fn anchors(&self) -> &BTreeSet<PaneId> {
        &self.anchors
    }

    /// Enumerate every live pane: its stable UUID, the anchor UUID when
    /// owned by an anchor, and the pane's current cwd. Feeds the `pane.list`
    /// bus kind.
    pub fn pane_listing(&self) -> Vec<(Uuid, Option<Uuid>, Option<std::path::PathBuf>)> {
        self.panes
            .iter()
            .filter_map(|(pane_id, pane)| {
                let uuid = self.pane_uuids.get(pane_id).copied()?;
                let anchor = owning_anchor_for_terminal_pane(
                    *pane_id,
                    &self.anchors,
                    &self.pane_workspace,
                    &self.terminal_tabs_by_anchor,
                    &self.pane_terminal_tab_roots,
                )
                .and_then(|anchor| self.pane_anchor_ids.get(&anchor).copied());
                Some((uuid, anchor, pane.cwd()))
            })
            .collect()
    }

    pub fn anchor_listing(&self) -> Vec<lmux_bus::AnchorSummary> {
        self.anchors
            .iter()
            .filter_map(|pane_id| {
                let anchor_id = self.pane_anchor_ids.get(pane_id).copied()?;
                let pane_uuid = self.pane_uuids.get(pane_id).copied();
                let label = self
                    .anchor_for_pane(*pane_id)
                    .map(|anchor| anchor.display_label().to_string())
                    .unwrap_or_else(|| format!("terminal {pane_id}"));
                let agent_status = agent_control::agent_status_for_anchor(
                    *pane_id,
                    &self.pane_agents,
                    &self.pane_workspace,
                    &self.pane_uuids,
                    &self.pane_titles,
                );
                Some(lmux_bus::AnchorSummary {
                    anchor_id,
                    pane_id: pane_uuid,
                    label,
                    active: self.active_anchor == Some(*pane_id),
                    agent_status,
                    pending_grants: agent_control::pending_grants_for_anchor(
                        &self.grants,
                        anchor_id,
                    ),
                    active_grants: agent_control::active_grants_for_anchor(&self.grants, anchor_id),
                })
            })
            .collect()
    }

    pub fn restore_snapshot_metadata(&mut self, snap: &lmux_state::SessionSnapshot) {
        self.restore_persisted_metadata(
            &snap.pane_titles,
            &snap.terminal_tabs,
            &snap.pane_terminal_tab_roots,
        );
    }

    fn restore_session_metadata(&mut self, session: &lmux_session::Session) {
        self.restore_persisted_metadata(
            &session.pane_titles,
            &session.terminal_tabs,
            &session.pane_terminal_tab_roots,
        );
    }

    fn restore_persisted_metadata(
        &mut self,
        pane_titles: &std::collections::BTreeMap<u32, lmux_state::PaneTitleSnapshot>,
        terminal_tabs: &[lmux_state::TerminalTabStackSnapshot],
        pane_terminal_tab_roots: &std::collections::BTreeMap<u32, u32>,
    ) {
        self.pane_titles.clear();
        for (pane, title) in pane_titles {
            if self.panes.contains_key(pane) {
                self.pane_titles
                    .insert(*pane, pane_title_from_snapshot(title));
            }
        }

        self.terminal_tabs_by_anchor.clear();
        self.active_terminal_tab_by_anchor.clear();
        self.pane_terminal_tab_roots.clear();
        for stack in terminal_tabs {
            if !self.anchors.contains(&stack.anchor_pane_id) {
                continue;
            }
            let tabs: Vec<PaneId> = stack
                .tab_roots
                .iter()
                .copied()
                .filter(|pane| self.panes.contains_key(pane))
                .collect();
            if tabs.is_empty() {
                continue;
            }
            let active = stack
                .active_tab
                .filter(|pane| tabs.contains(pane))
                .unwrap_or(tabs[0]);
            self.terminal_tabs_by_anchor
                .insert(stack.anchor_pane_id, tabs);
            self.active_terminal_tab_by_anchor
                .insert(stack.anchor_pane_id, active);
        }
        let anchors: Vec<PaneId> = self.anchors.iter().copied().collect();
        for anchor in anchors {
            self.ensure_terminal_tab_stack(anchor);
        }
        for (pane, root) in pane_terminal_tab_roots {
            let Some(owner) = self.pane_workspace.get(pane).copied() else {
                continue;
            };
            let Some(root_owner) = self.pane_workspace.get(root).copied() else {
                continue;
            };
            let root_is_valid_tab = self
                .terminal_tabs_by_anchor
                .get(&owner)
                .is_some_and(|tabs| tabs.contains(root));
            if self.panes.contains_key(pane)
                && self.panes.contains_key(root)
                && owner == root_owner
                && root_is_valid_tab
            {
                self.pane_terminal_tab_roots.insert(*pane, *root);
            }
        }
        for anchor in self.anchors.iter().copied() {
            self.pane_terminal_tab_roots.insert(anchor, anchor);
        }
        self.close_orphan_terminal_panes();
        self.ensure_anchor_invariant();
        self.rebuild_widget_tree();
    }

    pub fn anchor_agent_activity(&self, pane_id: PaneId) -> AnchorAgentActivity {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return AnchorAgentActivity {
                agents: Vec::new(),
                pending_grants: 0,
                active_grants: 0,
            };
        };
        AnchorAgentActivity {
            agents: agent_control::agent_status_for_anchor(
                pane_id,
                &self.pane_agents,
                &self.pane_workspace,
                &self.pane_uuids,
                &self.pane_titles,
            ),
            pending_grants: agent_control::pending_grants_for_anchor(&self.grants, anchor_id),
            active_grants: agent_control::active_grants_for_anchor(&self.grants, anchor_id),
        }
    }

    pub fn anchor_grant_views(&self, pane_id: PaneId) -> Vec<AnchorGrantView> {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return Vec::new();
        };
        agent_control::grant_views_for_anchor(&self.grants, anchor_id)
            .into_iter()
            .map(|view| AnchorGrantView {
                grant_id: view.grant_id,
                requester: view.requester,
                scope: view.scope,
                reason: view.reason,
                pending: view.pending,
                active: view.active,
            })
            .collect()
    }

    fn ensure_terminal_tab_stack(&mut self, anchor: PaneId) {
        self.terminal_tabs_by_anchor
            .entry(anchor)
            .or_insert_with(|| vec![anchor]);
        self.active_terminal_tab_by_anchor
            .entry(anchor)
            .or_insert(anchor);
        self.pane_terminal_tab_roots.entry(anchor).or_insert(anchor);
    }

    fn active_terminal_tab(&self, anchor: PaneId) -> PaneId {
        self.active_terminal_tab_by_anchor
            .get(&anchor)
            .copied()
            .unwrap_or(anchor)
    }

    fn tab_root_for_pane(&self, pane_id: PaneId) -> PaneId {
        self.pane_terminal_tab_roots
            .get(&pane_id)
            .copied()
            .or_else(|| self.pane_workspace.get(&pane_id).copied())
            .unwrap_or(pane_id)
    }

    #[allow(dead_code)]
    pub fn activate_terminal_tab(&mut self, anchor: PaneId, tab_root: PaneId) -> bool {
        if !self
            .terminal_tabs_by_anchor
            .get(&anchor)
            .is_some_and(|tabs| tabs.contains(&tab_root))
        {
            return false;
        }
        self.active_terminal_tab_by_anchor.insert(anchor, tab_root);
        self.rebuild_widget_tree();
        true
    }

    fn remove_terminal_tab_metadata(&mut self, pane_id: PaneId) {
        self.pane_terminal_tab_roots.remove(&pane_id);
        for tabs in self.terminal_tabs_by_anchor.values_mut() {
            tabs.retain(|tab| *tab != pane_id);
        }
        self.terminal_tabs_by_anchor
            .retain(|anchor, tabs| !tabs.is_empty() && self.anchors.contains(anchor));
        self.active_terminal_tab_by_anchor
            .retain(|anchor, _| self.terminal_tabs_by_anchor.contains_key(anchor));
        for (anchor, active) in self.active_terminal_tab_by_anchor.iter_mut() {
            if *active == pane_id {
                *active = self
                    .terminal_tabs_by_anchor
                    .get(anchor)
                    .and_then(|tabs| tabs.first().copied())
                    .unwrap_or(*anchor);
            }
        }
    }

    fn close_orphan_terminal_panes(&mut self) {
        let orphans: Vec<PaneId> = self
            .panes
            .iter()
            .filter_map(|(pane_id, pane)| {
                if pane.is_satellite() || self.anchors.contains(pane_id) {
                    return None;
                }
                let owner_is_valid = self
                    .pane_workspace
                    .get(pane_id)
                    .is_some_and(|owner| self.anchors.contains(owner));
                (!owner_is_valid).then_some(*pane_id)
            })
            .collect();

        for pane_id in orphans {
            if !self.close_terminal_pane_without_focus(pane_id) {
                tracing::debug!(
                    pane_id,
                    "orphan terminal pane kept because it is the last layout leaf"
                );
            }
        }
    }

    fn close_terminal_pane_without_focus(&mut self, pane_id: PaneId) -> bool {
        if !self.layout.remove_leaf(pane_id) {
            return false;
        }
        self.pane_workspace.remove(&pane_id);
        self.pane_uuids.remove(&pane_id);
        self.pane_titles.remove(&pane_id);
        self.pane_agents.remove(&pane_id);
        self.remove_terminal_tab_metadata(pane_id);
        if let Some(pane) = self.panes.remove(&pane_id) {
            pane.terminate();
            schedule_force_kill(pane);
        }
        true
    }

    fn ensure_anchor_invariant(&mut self) {
        if let Some(active) = self.active_anchor {
            if self.anchors.contains(&active) {
                return;
            }
            self.active_anchor = None;
        }
        if let Some(next) = self.anchors.iter().copied().next() {
            self.set_active_anchor(Some(next));
            return;
        }
        let Some(seed) = self.first_terminal_leaf() else {
            return;
        };
        self.pane_workspace.remove(&seed);
        self.remove_terminal_tab_metadata(seed);
        if self.tag_anchor_core(seed) {
            self.ensure_terminal_tab_stack(seed);
            self.set_active_anchor(Some(seed));
            self.notify_anchors_changed();
        }
    }

    /// Convenience: `true` when `pane_id` is tagged as an anchor.
    #[allow(dead_code)]
    pub fn is_anchor(&self, pane_id: PaneId) -> bool {
        self.anchors.contains(&pane_id)
    }

    /// Low-res RGB thumbnail of `pane_id`'s current viewport, or `None` if
    /// the pane is gone or its grid is degenerate. Consumed by the sidebar
    /// mini-preview (Epic 5).
    pub fn pane_thumbnail(&self, pane_id: PaneId) -> Option<(u32, u32, Vec<u8>)> {
        self.panes
            .get(&pane_id)
            .and_then(|p| p.snapshot_thumbnail())
    }

    pub fn pane_transcript_tail(
        &mut self,
        pane_uuid: Uuid,
        lines: u32,
        agent: Option<lmux_bus::AgentIdentity>,
    ) -> Result<lmux_bus::TranscriptRange, lmux_bus::BusError> {
        let pane_id = self.pane_for_uuid(pane_uuid).ok_or_else(|| {
            lmux_bus::BusError::TranscriptUnavailable(format!(
                "pane.tail: unknown pane {pane_uuid}"
            ))
        })?;
        self.authorize_pane_access(
            pane_id,
            pane_uuid,
            lmux_bus::GrantScope::ReadOutput,
            agent.as_ref(),
            "pane.tail transcript read",
        )?;
        let pane = self.panes.get(&pane_id).ok_or_else(|| {
            lmux_bus::BusError::TranscriptUnavailable(format!(
                "pane.tail: unknown pane {pane_uuid}"
            ))
        })?;
        pane.transcript_tail(pane_uuid, lines)
    }

    pub fn pane_transcript_capture(
        &mut self,
        pane_uuid: Uuid,
        since_sequence: Option<u64>,
        max_lines: Option<u32>,
        agent: Option<lmux_bus::AgentIdentity>,
    ) -> Result<lmux_bus::TranscriptRange, lmux_bus::BusError> {
        let pane_id = self.pane_for_uuid(pane_uuid).ok_or_else(|| {
            lmux_bus::BusError::TranscriptUnavailable(format!(
                "pane.capture: unknown pane {pane_uuid}"
            ))
        })?;
        self.authorize_pane_access(
            pane_id,
            pane_uuid,
            lmux_bus::GrantScope::ReadOutput,
            agent.as_ref(),
            "pane.capture transcript read",
        )?;
        let pane = self.panes.get(&pane_id).ok_or_else(|| {
            lmux_bus::BusError::TranscriptUnavailable(format!(
                "pane.capture: unknown pane {pane_uuid}"
            ))
        })?;
        pane.transcript_capture(pane_uuid, since_sequence, max_lines)
    }

    pub fn send_input_to_pane(
        &mut self,
        pane_uuid: Uuid,
        text: &str,
        agent: Option<lmux_bus::AgentIdentity>,
    ) -> Result<(), lmux_bus::BusError> {
        let pane_id = self.pane_for_uuid(pane_uuid).ok_or_else(|| {
            lmux_bus::BusError::TranscriptUnavailable(format!(
                "pane.send_input: unknown pane {pane_uuid}"
            ))
        })?;
        self.authorize_pane_access(
            pane_id,
            pane_uuid,
            lmux_bus::GrantScope::SendInput,
            agent.as_ref(),
            "pane.send_input terminal input",
        )?;
        let pane = self.panes.get(&pane_id).ok_or_else(|| {
            lmux_bus::BusError::TranscriptUnavailable(format!(
                "pane.send_input: unknown pane {pane_uuid}"
            ))
        })?;
        pane.send_input(text)
    }

    pub fn rename_pane(
        &mut self,
        pane_uuid: Uuid,
        title: String,
        pin: bool,
        agent: Option<lmux_bus::AgentIdentity>,
    ) -> Result<(), lmux_bus::BusError> {
        let pane_id = self.pane_for_uuid(pane_uuid).ok_or_else(|| {
            lmux_bus::BusError::Domain(format!("pane.rename: unknown pane {pane_uuid}"))
        })?;
        self.authorize_pane_access(
            pane_id,
            pane_uuid,
            lmux_bus::GrantScope::Rename,
            agent.as_ref(),
            "pane.rename title change",
        )?;
        if let Some(existing) = self.pane_titles.get(&pane_id).cloned() {
            if existing.pinned {
                if let Some(agent) = agent.as_ref() {
                    let target_anchor = self
                        .pane_workspace
                        .get(&pane_id)
                        .copied()
                        .or_else(|| self.anchors.contains(&pane_id).then_some(pane_id))
                        .and_then(|anchor| self.pane_anchor_ids.get(&anchor).copied());
                    self.register_grant_request(lmux_bus::GrantRequest {
                        grant_id: Uuid::new_v4(),
                        requester: agent.clone(),
                        scope: lmux_bus::GrantScope::Rename,
                        source_anchor: None,
                        target_anchor,
                        target_pane: Some(pane_uuid),
                        target_window: None,
                        reason: Some(format!(
                            "rename user-pinned title from '{}' to '{}'",
                            existing.title, title
                        )),
                    });
                }
                return Err(lmux_bus::BusError::UserPinnedTitle(format!(
                    "pane.rename: pane {pane_uuid} title is user-pinned"
                )));
            }
        }
        let provenance = if agent.is_some() && !pin {
            lmux_bus::PaneTitleProvenance::Agent
        } else {
            lmux_bus::PaneTitleProvenance::User
        };
        self.pane_titles.insert(
            pane_id,
            lmux_bus::PaneTitle {
                title,
                provenance,
                pinned: pin || agent.is_none(),
            },
        );
        Ok(())
    }

    pub fn rename_pane_title_for_user(&mut self, pane_id: PaneId, title: Option<String>) -> bool {
        if !self.panes.contains_key(&pane_id) {
            return false;
        }
        match title
            .map(|title| title.trim().to_string())
            .filter(|title| !title.is_empty())
        {
            Some(title) => {
                self.pane_titles.insert(
                    pane_id,
                    lmux_bus::PaneTitle {
                        title,
                        provenance: lmux_bus::PaneTitleProvenance::User,
                        pinned: true,
                    },
                );
            }
            None => {
                self.pane_titles.remove(&pane_id);
            }
        }
        self.rebuild_widget_tree();
        true
    }

    pub fn create_pane_from_bus(
        &mut self,
        request: PaneCreateRequest,
    ) -> Result<lmux_bus::PaneNewResult, lmux_bus::BusError> {
        let PaneCreateRequest {
            target_anchor,
            placement,
            activate,
            title,
            argv,
            agent,
            purpose,
        } = request;
        let mut agent = agent;
        let target = match target_anchor {
            Some(anchor_uuid) => self.pane_for_anchor_id(anchor_uuid).ok_or_else(|| {
                lmux_bus::BusError::Domain(format!("pane.new: unknown anchor {anchor_uuid}"))
            })?,
            None => self.active_anchor.unwrap_or(self.focused),
        };
        if !self.panes.contains_key(&target) {
            return Err(lmux_bus::BusError::Domain(format!(
                "pane.new: target pane {target} is not live"
            )));
        }
        let dir = match placement {
            lmux_bus::PanePlacement::SplitRight | lmux_bus::PanePlacement::Tab => Dir::Vertical,
            lmux_bus::PanePlacement::SplitDown => Dir::Horizontal,
        };

        let cwd = self
            .panes
            .get(&target)
            .and_then(|p| p.cwd())
            .map(|p| p.to_path_buf());
        let new_id = self.next_id;
        let Some(new_pane) = Pane::new(new_id, cwd.as_deref()) else {
            return Err(lmux_bus::BusError::Domain(
                "pane.new: new pane allocation failed".into(),
            ));
        };
        if agent.is_none() {
            agent = new_pane.child_pid().and_then(agent_identity_from_child_env);
        }
        if let Some(cb) = &self.focus_cb {
            new_pane.attach_controllers(
                cb.clone(),
                self.focus_mode.clone(),
                self.terminal_action_cb
                    .clone()
                    .unwrap_or_else(noop_terminal_action_callback),
                self.shortcut_prefix.clone(),
            );
        }
        if let Some(cb) = &self.reparent_cb {
            new_pane.attach_rearrange_controllers(self.rearrange_mode.clone(), cb.clone());
        }
        if let Some(cb) = &self.bell_cb {
            new_pane.set_bell_callback(cb.clone());
        }
        if let Some(cb) = &self.terminal_exit_cb {
            new_pane.set_exit_callback(cb.clone());
        }

        let pane_uuid = Uuid::new_v4();
        self.panes.insert(new_id, new_pane);
        self.pane_uuids.insert(new_id, pane_uuid);
        let owner = self
            .pane_workspace
            .get(&target)
            .copied()
            .or_else(|| self.is_anchor(target).then_some(target));
        if let Some(owner) = owner {
            self.pane_workspace.insert(new_id, owner);
            self.ensure_terminal_tab_stack(owner);
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .unwrap_or_else(|| u32::MAX.saturating_sub(0));

        let replaced = self.layout.replace_leaf(target, |id| Layout::Split {
            dir,
            a: Box::new(Layout::Leaf(id)),
            b: Box::new(Layout::Leaf(new_id)),
            ratio: 0.5,
        });
        if !replaced {
            self.panes.remove(&new_id);
            self.pane_uuids.remove(&new_id);
            self.pane_titles.remove(&new_id);
            self.pane_agents.remove(&new_id);
            self.remove_terminal_tab_metadata(new_id);
            return Err(lmux_bus::BusError::Domain(format!(
                "pane.new: target pane {target} is not in layout"
            )));
        }

        if let Some(title) = title {
            let provenance = if agent.is_some() {
                lmux_bus::PaneTitleProvenance::Agent
            } else {
                lmux_bus::PaneTitleProvenance::User
            };
            self.pane_titles.insert(
                new_id,
                lmux_bus::PaneTitle {
                    title,
                    provenance,
                    pinned: agent.is_none(),
                },
            );
        }
        if let Some(agent) = agent.clone() {
            self.pane_agents
                .insert(new_id, AgentPaneMetadata { agent, purpose });
        }
        if let Some(owner) = owner {
            match placement {
                lmux_bus::PanePlacement::Tab => {
                    self.pane_terminal_tab_roots.insert(new_id, new_id);
                    self.terminal_tabs_by_anchor
                        .entry(owner)
                        .or_insert_with(|| vec![owner])
                        .push(new_id);
                    if activate {
                        self.active_terminal_tab_by_anchor.insert(owner, new_id);
                    }
                }
                lmux_bus::PanePlacement::SplitRight | lmux_bus::PanePlacement::SplitDown => {
                    let root = self.tab_root_for_pane(target);
                    self.pane_terminal_tab_roots.insert(new_id, root);
                }
            }
        }
        if !argv.is_empty() {
            let command = shell_command_line(&argv);
            if let Some(pane) = self.panes.get(&new_id) {
                pane.send_input(&format!("{command}\n"))?;
            }
        }
        if activate {
            self.focused = new_id;
            if let Some(pane) = self.panes.get(&new_id) {
                pane.grab_focus();
            }
            self.refresh_focus_css();
        }
        self.rebuild_widget_tree();
        Ok(lmux_bus::PaneNewResult {
            pane_id: pane_uuid,
            anchor_id: owner
                .and_then(|owner| self.pane_anchor_ids.get(&owner).copied())
                .unwrap_or_else(Uuid::nil),
            placement,
        })
    }

    pub fn register_grant_request(
        &mut self,
        request: lmux_bus::GrantRequest,
    ) -> lmux_bus::GrantRequest {
        let request = agent_control::register_grant_request(&mut self.grants, request);
        self.notify_anchors_changed();
        request
    }

    pub fn decide_grant(
        &mut self,
        grant_id: Uuid,
        decision: lmux_bus::GrantDecision,
    ) -> Result<(), lmux_bus::BusError> {
        let result = agent_control::decide_grant(&mut self.grants, grant_id, decision);
        if result.is_ok() {
            self.notify_anchors_changed();
        }
        result
    }

    fn authorize_pane_access(
        &mut self,
        pane_id: PaneId,
        pane_uuid: Uuid,
        scope: lmux_bus::GrantScope,
        agent: Option<&lmux_bus::AgentIdentity>,
        reason: &str,
    ) -> Result<(), lmux_bus::BusError> {
        let grant_count = self.grants.len();
        let result = agent_control::authorize_pane_access(
            &mut self.grants,
            &self.pane_agents,
            &self.anchors,
            &self.pane_workspace,
            &self.pane_anchor_ids,
            pane_id,
            pane_uuid,
            scope,
            agent,
            None,
            reason,
        );
        if self.grants.len() != grant_count {
            self.notify_anchors_changed();
        }
        result
    }

    pub fn authorize_active_anchor_window_attach(
        &mut self,
        candidate: &WindowCandidate,
        agent: Option<lmux_bus::AgentIdentity>,
    ) -> Result<(), lmux_bus::BusError> {
        self.authorize_active_anchor_window_attach_with_reason(candidate, agent, "attach window")
    }

    pub fn authorize_active_anchor_window_attach_with_reason(
        &mut self,
        candidate: &WindowCandidate,
        agent: Option<lmux_bus::AgentIdentity>,
        reason_prefix: &str,
    ) -> Result<(), lmux_bus::BusError> {
        let Some(anchor) = self.active_anchor else {
            return Err(lmux_bus::BusError::Domain("no active anchor".into()));
        };
        let pane_uuid = self.pane_uuids.get(&anchor).copied().ok_or_else(|| {
            lmux_bus::BusError::Domain(format!("active anchor {anchor} has no pane UUID"))
        })?;
        let target_window = window_candidate_prompt_metadata(candidate);
        let grant_count = self.grants.len();
        let result = agent_control::authorize_pane_access(
            &mut self.grants,
            &self.pane_agents,
            &self.anchors,
            &self.pane_workspace,
            &self.pane_anchor_ids,
            anchor,
            pane_uuid,
            lmux_bus::GrantScope::AttachWindow,
            agent.as_ref(),
            Some(target_window.clone()),
            &format!("{reason_prefix}: {target_window}"),
        );
        if self.grants.len() != grant_count {
            self.notify_anchors_changed();
        }
        result
    }

    pub fn authorize_active_anchor_launch_attach(
        &mut self,
        argv: &[String],
        title_hint: Option<&str>,
        app_hint: Option<&str>,
        timeout_ms: Option<u64>,
        agent: Option<lmux_bus::AgentIdentity>,
    ) -> Result<(), lmux_bus::BusError> {
        let Some(anchor) = self.active_anchor else {
            return Err(lmux_bus::BusError::Domain("no active anchor".into()));
        };
        let pane_uuid = self.pane_uuids.get(&anchor).copied().ok_or_else(|| {
            lmux_bus::BusError::Domain(format!("active anchor {anchor} has no pane UUID"))
        })?;
        let target_window =
            launch_attach_prompt_metadata(argv, title_hint, app_hint, timeout_ms.unwrap_or(5_000));
        let grant_count = self.grants.len();
        let result = agent_control::authorize_pane_access(
            &mut self.grants,
            &self.pane_agents,
            &self.anchors,
            &self.pane_workspace,
            &self.pane_anchor_ids,
            anchor,
            pane_uuid,
            lmux_bus::GrantScope::AttachWindow,
            agent.as_ref(),
            Some(target_window.clone()),
            &format!("launch-and-attach: {target_window}"),
        );
        if self.grants.len() != grant_count {
            self.notify_anchors_changed();
        }
        result
    }

    fn session_anchor_refs(&self) -> Vec<lmux_session::AnchorRef> {
        self.anchors
            .iter()
            .copied()
            .map(|pane_id| {
                let (argv, cwd) = self.anchor_spawn_metadata(pane_id);
                lmux_session::AnchorRef {
                    pane_id,
                    argv,
                    cwd,
                    hide_on_session_close: self.hidden_anchors.contains(&pane_id),
                }
            })
            .collect()
    }

    /// Register a freshly-spawned GUI satellite with its owning anchor.
    /// The compositor bridge will be notified on subsequent anchor switches
    /// so the satellite hides when its owner is inactive and shows again
    /// when it becomes active.
    #[allow(dead_code)]
    pub fn register_satellite(&mut self, anchor: PaneId, pid: u32) {
        self.register_satellite_window(
            anchor,
            SatelliteWindowId::for_pid(default_window_backend(), pid),
        );
    }

    #[allow(dead_code)]
    pub fn register_satellite_spawn(
        &mut self,
        anchor: PaneId,
        request_id: uuid::Uuid,
        pid: u32,
        bundle_id: Option<String>,
    ) {
        #[cfg(target_os = "macos")]
        {
            let _ = (request_id, bundle_id);
            tracing::warn!(
                anchor,
                pid,
                "register_satellite_spawn ignored on macOS; use explicit window attach"
            );
            return;
        }
        #[cfg(not(target_os = "macos"))]
        self.register_satellite_window(
            anchor,
            SatelliteWindowId::for_spawn_with_bundle(
                default_window_backend(),
                request_id,
                pid,
                bundle_id,
            ),
        );
    }

    pub fn register_satellite_window(&mut self, anchor: PaneId, window: SatelliteWindowId) {
        if !self.anchors.contains(&anchor) {
            tracing::warn!(
                anchor,
                pid = ?window.pid,
                "register_satellite: pane is not a tagged anchor"
            );
            return;
        }
        insert_satellite_window_for_anchor(
            &mut self.satellite_windows_by_anchor,
            anchor,
            window.clone(),
        );
        tracing::info!(anchor, window = ?window, "registered satellite under anchor");
        self.broadcast_satellite_visibility();
    }

    pub fn attach_native_window_to_active_anchor(
        &mut self,
        candidate: &WindowCandidate,
        window: SatelliteWindowId,
    ) -> Result<(), String> {
        let anchor = self
            .active_anchor
            .ok_or_else(|| "no active anchor".to_string())?;
        if candidate.backend_window_id.trim().is_empty() {
            return Err("native window has no backend window id".into());
        }
        self.register_satellite_window(anchor, window.clone());
        tracing::info!(
            anchor,
            backend = ?candidate.backend,
            backend_window_id = %candidate.backend_window_id,
            window = ?window,
            "attached native window to active anchor"
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn attach_focused_macos_window_to_active_anchor(&mut self) -> Result<(), String> {
        let anchor = self
            .active_anchor
            .ok_or_else(|| "no active anchor".to_string())?;
        let focused = lmux_macos_helper::focused_window()
            .map_err(|err| format!("focused window helper failed: {err}"))?
            .ok_or_else(|| "no focused macOS window".to_string())?;
        if focused.window_id.is_none() {
            return Err(format!(
                "focused macOS window for pid {} has no stable window id",
                focused.pid
            ));
        }

        let request_id = Uuid::new_v4();
        let window = macos_satellite_for_window(request_id, focused.bundle_id.clone(), &focused);
        self.register_satellite_window(anchor, window.clone());
        tracing::info!(
            anchor,
            request_id = %request_id,
            pid = focused.pid,
            window = ?window,
            "attached focused macOS window to active anchor"
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn attach_macos_window_to_active_anchor(
        &mut self,
        window: MacosWindowInfo,
    ) -> Result<(), String> {
        let anchor = self
            .active_anchor
            .ok_or_else(|| "no active anchor".to_string())?;
        if window.window_id.is_none() && window.window_index == 0 {
            return Err(format!(
                "macOS window for pid {} has no stable window id or window index",
                window.pid
            ));
        }

        let request_id = Uuid::new_v4();
        let satellite = macos_satellite_for_window(request_id, window.bundle_id.clone(), &window);
        self.register_satellite_window(anchor, satellite.clone());
        tracing::info!(
            anchor,
            request_id = %request_id,
            pid = window.pid,
            window = ?satellite,
            "attached selected macOS window to active anchor"
        );
        Ok(())
    }

    /// Wire up the bridge so anchor-switch side effects reach KWin.
    pub fn set_compositor_tx(&mut self, tx: crate::compositor_bridge::CompositorSender) {
        self.compositor_tx = Some(tx);
    }

    /// Emit a visibility command for each known satellite based on
    /// `active_anchor`: satellites under the active anchor become visible,
    /// satellites under every other anchor get hidden.
    fn broadcast_satellite_visibility(&mut self) {
        let Some(tx) = self.compositor_tx.as_ref().cloned() else {
            return;
        };
        self.satellite_visibility_seq = self.satellite_visibility_seq.saturating_add(1);
        let sequence = self.satellite_visibility_seq;
        let (hide, show) =
            satellite_visibility_for_active(self.active_anchor, &self.satellite_windows_by_anchor);
        tracing::info!(
            operation = "anchor.satellite_visibility",
            sequence,
            active_anchor = ?self.active_anchor,
            hide = hide.len(),
            show = show.len(),
            hide_windows = ?hide,
            show_windows = ?show,
            "satellite visibility command queued"
        );
        let command = crate::compositor_bridge::CompositorCommand::ApplyWindowGroupSwitch {
            sequence,
            hide,
            show,
            focus_policy: FocusPolicy::Terminal,
        };
        if let Err(err) = tx.try_send(command.clone()) {
            tracing::debug!(error = %err, "satellite visibility command dropped");
        }
    }

    /// Install the nested-Wayland compositor handle + command channel
    /// (ADR-0018). Must be called before the host-event dispatcher is
    /// spawned. Dropped panes that are satellites will still receive
    /// `request_close` via this channel until AppState itself is dropped.
    #[cfg(target_os = "linux")]
    pub fn install_wayland_host(&mut self, handle: HostHandle, cmd_tx: Sender<HostCommand>) {
        self.wayland_host = Some(handle);
        self.host_cmd_tx = Some(cmd_tx);
    }

    /// Public clone of the host command sender, used by the launcher when
    /// it wants to address a satellite directly (e.g., force-close on
    /// anchor-pane removal). `None` if the host never started.
    #[allow(dead_code)]
    #[cfg(target_os = "linux")]
    pub fn host_cmd_tx(&self) -> Option<Sender<HostCommand>> {
        self.host_cmd_tx.clone()
    }

    /// The socket name (`lmux-<pid>`) advertised by the nested compositor,
    /// for use as `WAYLAND_DISPLAY` in satellite child env. `None` when
    /// the host never started or no Ready event has been dispatched yet.
    /// Populated by `handle_host_event` on `HostEvent::Ready`.
    #[allow(dead_code)]
    #[cfg(target_os = "linux")]
    pub fn wayland_display_name(&self) -> Option<&str> {
        self.wayland_display_name.as_deref()
    }

    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    pub fn wayland_display_name(&self) -> Option<&str> {
        None
    }

    /// Dispatch a single event from `lmux_wayland_host`. Runs on the GTK
    /// main thread — the caller is a `spawn_local` task draining the
    /// host's async-channel receiver. Creating satellites, pushing
    /// frames, and collapsing the layout on close all happen here.
    #[cfg(target_os = "linux")]
    pub fn handle_host_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::Ready { display_name } => {
                tracing::info!(display = %display_name, "wayland host ready");
                self.wayland_display_name = Some(display_name);
            }
            HostEvent::Stopped => {
                tracing::info!("wayland host stopped");
            }
            HostEvent::ToplevelCreated { id, title, app_id } => {
                self.on_toplevel_created(id, title, app_id);
            }
            HostEvent::ToplevelTitleChanged { id, title } => {
                if let Some(pane_id) = self.surface_to_pane.get(&id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.set_title(title);
                    }
                }
            }
            HostEvent::ToplevelAppIdChanged { id, app_id } => {
                if let Some(pane_id) = self.surface_to_pane.get(&id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.set_app_id(app_id);
                    }
                }
            }
            HostEvent::FrameReady {
                id,
                width,
                height,
                rgb,
            } => {
                if let Some(pane_id) = self.popup_to_pane.get(&id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.push_popup_frame(id, width, height, rgb);
                    }
                } else if let Some(pane_id) = self.surface_to_pane.get(&id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.push_frame(width, height, rgb);
                    }
                }
            }
            HostEvent::DmabufFrame(frame) => {
                if let Some(pane_id) = self.popup_to_pane.get(&frame.id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        let popup_id = frame.id;
                        s.push_popup_dmabuf_frame(popup_id, frame);
                    }
                } else if let Some(pane_id) = self.surface_to_pane.get(&frame.id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.push_dmabuf_frame(frame);
                    }
                }
            }
            HostEvent::ToplevelClosed { id } => {
                self.on_toplevel_closed(id);
            }
            HostEvent::PopupCreated {
                id,
                parent,
                x,
                y,
                width,
                height,
            } => {
                let Some(parent_pane) = self.surface_to_pane.get(&parent).copied() else {
                    tracing::warn!(?id, ?parent, "PopupCreated for unknown parent surface");
                    return;
                };
                if let Some(s) = self.panes.get(&parent_pane).and_then(|p| p.as_satellite()) {
                    s.attach_popup(id, x, y, width, height);
                    self.popup_to_pane.insert(id, parent_pane);
                }
            }
            HostEvent::PopupRepositioned {
                id,
                x,
                y,
                width,
                height,
                token: _,
            } => {
                if let Some(pane_id) = self.popup_to_pane.get(&id).copied() {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.reposition_popup(id, x, y, width, height);
                    }
                }
            }
            HostEvent::PopupClosed { id } => {
                if let Some(pane_id) = self.popup_to_pane.remove(&id) {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.detach_popup(id);
                    }
                }
            }
            HostEvent::ChildToplevelCreated {
                id,
                parent,
                title: _,
                app_id: _,
                width,
                height,
            } => {
                let Some(parent_pane) = self.surface_to_pane.get(&parent).copied() else {
                    tracing::warn!(
                        ?id,
                        ?parent,
                        "ChildToplevelCreated for unknown parent surface"
                    );
                    return;
                };
                // Center the child on the parent pane using the parent's
                // current allocation. Falls back to (0,0) if the parent
                // hasn't been sized yet.
                let (px, py) = self
                    .panes
                    .get(&parent_pane)
                    .map(|p| {
                        use gtk4::prelude::*;
                        let w = p.widget();
                        let pw = w.width().max(0);
                        let ph = w.height().max(0);
                        let cx = (pw - width as i32).max(0) / 2;
                        let cy = (ph - height as i32).max(0) / 2;
                        (cx, cy)
                    })
                    .unwrap_or((0, 0));
                if let Some(s) = self.panes.get(&parent_pane).and_then(|p| p.as_satellite()) {
                    s.attach_popup(id, px, py, width, height);
                    self.popup_to_pane.insert(id, parent_pane);
                }
            }
            HostEvent::ChildToplevelClosed { id } => {
                if let Some(pane_id) = self.popup_to_pane.remove(&id) {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.detach_popup(id);
                    }
                }
            }
            HostEvent::CursorShape { surface_id, name } => {
                // Resolve to the satellite the pointer is over (falling
                // through popup_to_pane so the cursor set by a menu goes
                // to the parent satellite's pane as well). Passing `None`
                // resets every satellite to its default cursor.
                let target = surface_id.and_then(|sid| {
                    self.surface_to_pane
                        .get(&sid)
                        .copied()
                        .or_else(|| self.popup_to_pane.get(&sid).copied())
                });
                match target {
                    Some(pane_id) => {
                        if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                            s.set_cursor_shape(&name);
                        }
                    }
                    None => {
                        for pane in self.panes.values() {
                            if let Some(s) = pane.as_satellite() {
                                s.set_cursor_shape("default");
                            }
                        }
                    }
                }
            }
            HostEvent::CursorBitmap {
                surface_id,
                width,
                height,
                rgba,
                hotspot_x,
                hotspot_y,
            } => {
                let target = surface_id.and_then(|sid| {
                    self.surface_to_pane
                        .get(&sid)
                        .copied()
                        .or_else(|| self.popup_to_pane.get(&sid).copied())
                });
                if let Some(pane_id) = target {
                    if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
                        s.set_cursor_bitmap(width, height, rgba, hotspot_x, hotspot_y);
                    }
                }
            }
        }
    }

    /// Allocate a fresh PaneId, build a `SatelliteWidget`, and splice it
    /// into the layout by splitting the currently-focused leaf vertically.
    /// The widget goes on the bottom/right per the same convention as
    /// `split_focused` — focus stays on the originating pane unless the
    /// caller decides otherwise.
    #[cfg(target_os = "linux")]
    fn on_toplevel_created(
        &mut self,
        surface_id: SurfaceId,
        title: Option<String>,
        app_id: Option<String>,
    ) {
        let Some(cmd_tx) = self.host_cmd_tx.clone() else {
            tracing::warn!(
                ?surface_id,
                "ToplevelCreated with no host command tx; dropping satellite"
            );
            return;
        };
        let pane_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let widget = SatelliteWidget::new(pane_id, surface_id, title, app_id, cmd_tx);
        let pane = Pane::from_satellite(widget);

        let target = self.focused;
        if self.panes.contains_key(&target) {
            // Splice into the layout alongside the focused pane.
            let replaced = self.layout.replace_leaf(target, |id| Layout::Split {
                dir: Dir::Vertical,
                a: Box::new(Layout::Leaf(id)),
                b: Box::new(Layout::Leaf(pane_id)),
                ratio: 0.5,
            });
            if !replaced {
                tracing::warn!(target, "ToplevelCreated could not locate focused leaf");
                return;
            }
        } else {
            // Degenerate case (no focused pane yet) — satellite becomes
            // the sole leaf. Shouldn't happen in normal flow but keeps
            // AppState in a consistent state if the host races.
            self.layout = Layout::Leaf(pane_id);
        }

        if let Some(&owner) = self.pane_workspace.get(&target) {
            self.pane_workspace.insert(pane_id, owner);
        }
        if let Some(cb) = &self.focus_cb {
            pane.attach_controllers(
                cb.clone(),
                self.focus_mode.clone(),
                self.terminal_action_cb
                    .clone()
                    .unwrap_or_else(noop_terminal_action_callback),
                self.shortcut_prefix.clone(),
            );
        }
        if let Some(cb) = &self.reparent_cb {
            pane.attach_rearrange_controllers(self.rearrange_mode.clone(), cb.clone());
        }
        self.panes.insert(pane_id, pane);
        self.pane_uuids.insert(pane_id, Uuid::new_v4());
        self.surface_to_pane.insert(surface_id, pane_id);
        self.rebuild_widget_tree();
        tracing::info!(pane_id, ?surface_id, "satellite pane created");
    }

    #[cfg(target_os = "linux")]
    fn on_toplevel_closed(&mut self, surface_id: SurfaceId) {
        let Some(pane_id) = self.surface_to_pane.remove(&surface_id) else {
            return;
        };
        // Mark the widget as closed for parity with `has_exited()`; the
        // close path below removes it right after, but other code (e.g.
        // a pending `grab_focus`) may race.
        if let Some(s) = self.panes.get(&pane_id).and_then(|p| p.as_satellite()) {
            s.mark_closed();
        }
        // Only collapse the layout when more than one leaf remains; the
        // cockpit's "close last pane" rule (ignore close-focused on the
        // last pane) also applies to satellite closes.
        if matches!(&self.layout, Layout::Leaf(id) if *id == pane_id) {
            tracing::info!(
                pane_id,
                "ignoring satellite close on last pane — cockpit keeps running"
            );
            return;
        }
        if !self.layout.remove_leaf(pane_id) {
            tracing::warn!(pane_id, "satellite close: leaf not in layout");
        }
        self.panes.remove(&pane_id);
        self.pane_uuids.remove(&pane_id);
        self.pane_titles.remove(&pane_id);
        self.pane_agents.remove(&pane_id);
        self.remove_terminal_tab_metadata(pane_id);
        self.pane_workspace.remove(&pane_id);
        if self.focused == pane_id {
            // Prefer a surviving leaf that's in the active anchor's
            // workspace, so focus doesn't escape to a hidden anchor —
            // otherwise a satellite that shortly afterwards creates a
            // new toplevel (e.g. Rider replacing its splash with the
            // project window) would inherit the wrong anchor via
            // `on_toplevel_created`'s `pane_workspace` lookup.
            let leaves = self.layout.leaves();
            let in_active = self.active_anchor.and_then(|a| {
                leaves
                    .iter()
                    .copied()
                    .find(|id| self.pane_workspace.get(id) == Some(&a))
            });
            self.focused = in_active
                .or_else(|| leaves.first().copied())
                .unwrap_or(pane_id);
        }
        self.rebuild_widget_tree();
        if let Some(pane) = self.panes.get(&self.focused) {
            pane.grab_focus();
        }
        self.refresh_focus_css();
        tracing::info!(pane_id, ?surface_id, "satellite pane closed");
    }

    /// Drain every pane, sending SIGTERM to its child and returning the list
    /// of dropped panes (retained by the caller so they remain alive until
    /// `waitpid` resolves). Used by the clean-quit path (Story 7.2).
    pub fn drain_panes_for_shutdown(&mut self) -> Vec<Pane> {
        self.anchors.clear();
        self.pane_anchor_ids.clear();
        self.pane_uuids.clear();
        self.pane_titles.clear();
        self.pane_agents.clear();
        self.grants.clear();
        self.terminal_tabs_by_anchor.clear();
        self.active_terminal_tab_by_anchor.clear();
        self.pane_terminal_tab_roots.clear();
        self.pane_workspace.clear();
        self.hidden_anchors.clear();
        self.last_notif_id.clear();
        while let Some(child) = self.root_container.first_child() {
            self.root_container.remove(&child);
        }
        let mut out = Vec::new();
        for (_, pane) in self.panes.drain() {
            if !pane.has_exited() {
                pane.terminate();
            }
            out.push(pane);
        }
        out
    }

    /// Cycle which anchor is currently active (the one shown on screen),
    /// wrapping around the tagged set in pane-id order. Wired to Ctrl+B a.
    /// When no anchors are tagged yet, tag the focused pane as the first
    /// anchor — previously `a` toggled the tag, but untag is the popover's
    /// job now; the default `a` press should either create the first
    /// anchor or flip between existing anchors.
    pub fn cycle_active_anchor(&mut self) {
        if self.anchors.is_empty() {
            let target = self.focused;
            self.add_anchor(target);
            return;
        }
        let ordered: Vec<PaneId> = self.anchors.iter().copied().collect();
        let idx = self
            .active_anchor
            .and_then(|cur| ordered.iter().position(|id| *id == cur))
            .unwrap_or(usize::MAX);
        let next = ordered[(idx.wrapping_add(1)) % ordered.len()];
        if Some(next) != self.active_anchor {
            self.set_active_anchor(Some(next));
        }
    }

    /// Tag `target` as an anchor. Multi-anchor semantics: existing anchors
    /// are preserved. Idempotent when `target` is already tagged.
    ///
    /// A pane that is already a satellite of another anchor is rejected —
    /// satellites cannot be promoted to a new anchor (they belong to
    /// exactly one workspace). Unowned terminal panes are closed instead
    /// of being shown in a separate no-anchor view.
    pub fn add_anchor(&mut self, target: PaneId) {
        if !self.tag_anchor_core(target) {
            return;
        }
        self.close_orphan_terminal_panes();
        self.ensure_terminal_tab_stack(target);
        // A freshly-tagged anchor becomes the active one, displacing the
        // previously-active anchor (which stays tagged but hidden).
        self.set_active_anchor(Some(target));
        self.notify_anchors_changed();
    }

    /// Restore-path variant of [`add_anchor`] that does NOT absorb
    /// currently-unowned panes. Callers must pre-seed `pane_workspace`
    /// themselves before invoking this — otherwise satellites of the
    /// restored anchor won't be visible when the anchor activates. Used
    /// by `AppState::install_restored` and `switch_session` where the
    /// snapshot already encodes the desired workspace membership.
    pub fn restore_anchor(&mut self, target: PaneId) {
        if self.tag_anchor_core(target) {
            self.ensure_terminal_tab_stack(target);
        }
    }

    /// Core tagging primitive shared by [`add_anchor`] and
    /// [`restore_anchor`]. Returns `true` when the tag was applied,
    /// `false` when the pane is unknown, already owned by a different
    /// anchor, or already tagged (idempotent no-op).
    fn tag_anchor_core(&mut self, target: PaneId) -> bool {
        if !self.panes.contains_key(&target) {
            tracing::warn!(pane_id = target, "add_anchor: unknown pane");
            return false;
        }
        if let Some(&owner) = self.pane_workspace.get(&target) {
            if owner != target {
                tracing::warn!(
                    pane_id = target,
                    owner,
                    "add_anchor: refusing to promote a satellite to an anchor"
                );
                return false;
            }
        }
        if !self.anchors.insert(target) {
            return false;
        }
        let (argv, cwd) = self.anchor_spawn_metadata(target);
        let mut anchor = Anchor::new_manual(Uuid::new_v4(), argv, cwd);
        anchor.sort_key = Some(self.next_anchor_sort_key(anchor.group.as_deref()));
        let anchor_id = anchor.id;
        self.anchor_registry.insert(anchor);
        self.pane_anchor_ids.insert(target, anchor_id);
        self.pane_workspace.insert(target, target);
        self.terminal_tabs_by_anchor.insert(target, vec![target]);
        self.active_terminal_tab_by_anchor.insert(target, target);
        self.pane_terminal_tab_roots.insert(target, target);
        if let Some(pane) = self.panes.get(&target) {
            pane.widget().add_css_class(ANCHOR_CSS_CLASS);
        }
        tracing::info!(pane_id = target, anchor_id = %anchor_id, "anchor set");
        true
    }

    fn next_anchor_sort_key(&self, group: Option<&str>) -> i64 {
        next_anchor_sort_key_for(
            self.pane_anchor_ids.values().copied(),
            &self.anchor_registry,
            group,
        )
    }
}

fn next_anchor_sort_key_for(
    anchor_ids: impl IntoIterator<Item = Uuid>,
    registry: &AnchorRegistry,
    group: Option<&str>,
) -> i64 {
    anchor_ids
        .into_iter()
        .filter_map(|anchor_id| registry.get(anchor_id))
        .filter(|anchor| anchor.group.as_deref() == group)
        .map(|anchor| anchor.sort_key.unwrap_or(0))
        .max()
        .map_or(0, |max| max.saturating_add(1))
}

impl AppState {
    /// Remove `target` from the anchor set and drop its registry entry.
    /// Idempotent. If `target` was the active anchor, promotes an
    /// arbitrary remaining anchor to active (or clears `active_anchor`
    /// when the set is empty).
    pub fn remove_anchor(&mut self, target: PaneId) {
        if !self.anchors.remove(&target) {
            return;
        }
        if let Some(anchor_id) = self.pane_anchor_ids.remove(&target) {
            let _ = self.anchor_registry.remove(anchor_id);
        }
        self.hidden_anchors.remove(&target);
        // Orphan every satellite that belonged to this anchor's workspace
        // (including the anchor pane itself). Orphans become unowned again;
        // if another anchor is then activated, they'll be hidden until the
        // user re-tags.
        self.pane_workspace.retain(|_, owner| *owner != target);
        if let Some(pane) = self.panes.get(&target) {
            pane.widget().remove_css_class(ANCHOR_CSS_CLASS);
            pane.widget().remove_css_class(ANCHOR_ACTIVE_CSS_CLASS);
            pane.widget().set_visible(true);
        }
        self.close_orphan_terminal_panes();
        if self.active_anchor == Some(target) {
            let next = self.anchors.iter().copied().next();
            self.set_active_anchor(next);
        }
        self.ensure_anchor_invariant();
        tracing::info!(pane_id = target, "anchor cleared");
        self.notify_anchors_changed();
    }

    /// Which anchor is currently visible on screen, if any. Exposed for the
    /// sidebar so it can highlight the active row.
    #[allow(dead_code)]
    pub fn active_anchor(&self) -> Option<PaneId> {
        self.active_anchor
    }

    /// Promote `target` to the active anchor — the one shown on screen.
    /// All other tagged anchors (and any satellites they own, Epic 9) get
    /// hidden. `None` clears the active slot. A pane passed in that is
    /// not currently tagged is silently ignored.
    pub fn set_active_anchor(&mut self, target: Option<PaneId>) {
        let started = Instant::now();
        if let Some(id) = target {
            if !self.anchors.contains(&id) {
                tracing::warn!(pane_id = id, "set_active_anchor: not a tagged anchor");
                return;
            }
        }
        if self.active_anchor == target {
            tracing::debug!(
                operation = "anchor.switch.local",
                duration_ms = elapsed_ms(started),
                target = ?target,
                active = ?self.active_anchor,
                anchors = self.anchors.len(),
                panes = self.panes.len(),
                satellite_windows = self.satellite_window_count(),
                changed = false,
                gtk_rebuild = false,
                "anchor switch skipped"
            );
            return;
        }
        self.active_anchor = target;
        // Apply visibility across every pane:
        //  * When an anchor is active, show only panes whose workspace ==
        //    active (the anchor pane + its satellites). Everything else
        //    (other anchors, their satellites, unowned panes) hides.
        //  * When no anchor is active, show nothing. Runtime code should
        //    quickly seed or activate an anchor; there is no standalone
        //    "unanchored shells" view.
        // CSS class is toggled only on anchor panes — satellites don't get
        // the active-anchor border.
        let active = self.active_anchor;
        for (pane_id, pane) in &self.panes {
            let w = pane.widget();
            let in_workspace = match active {
                None => false,
                Some(a) => self.pane_workspace.get(pane_id) == Some(&a),
            };
            let hidden = self.hidden_anchors.contains(pane_id);
            w.set_visible(in_workspace && !hidden);
            if self.anchors.contains(pane_id) {
                if Some(*pane_id) == active {
                    w.add_css_class(ANCHOR_ACTIVE_CSS_CLASS);
                } else {
                    w.remove_css_class(ANCHOR_ACTIVE_CSS_CLASS);
                }
            }
        }
        if let Some(id) = self.active_anchor {
            tracing::info!(pane_id = id, "anchor activated");
            self.focused = id;
        }
        // Rebuild the widget tree so the pruned-by-workspace view of
        // `self.layout` actually drops the inactive anchors' subtrees
        // out of GTK — visibility toggling alone leaves GtkPaned
        // allocating space for hidden children.
        let gtk_rebuild = if self.switch_workspace_view() {
            false
        } else {
            self.rebuild_widget_tree();
            true
        };
        if let Some(id) = self.active_anchor {
            if let Some(pane) = self.panes.get(&id) {
                pane.grab_focus();
            }
        }
        self.refresh_focus_css();
        self.broadcast_satellite_visibility();
        self.notify_active_anchor_changed();
        tracing::info!(
            operation = "anchor.switch.local",
            duration_ms = elapsed_ms(started),
            target = ?target,
            active = ?self.active_anchor,
            anchors = self.anchors.len(),
            panes = self.panes.len(),
            satellite_windows = self.satellite_window_count(),
            changed = true,
            gtk_rebuild,
            "anchor switched locally"
        );
    }

    /// Sidebar accessor: resolve the `Anchor` metadata for a tagged pane.
    /// Returns `None` when the pane isn't tagged.
    #[allow(dead_code)]
    pub fn anchor_for_pane(&self, pane_id: PaneId) -> Option<&Anchor> {
        let id = self.pane_anchor_ids.get(&pane_id).copied()?;
        self.anchor_registry.get(id)
    }

    /// Sidebar accessor: full registry for grouped/sorted rendering.
    #[allow(dead_code)]
    pub fn anchor_registry(&self) -> &AnchorRegistry {
        &self.anchor_registry
    }

    /// Pause the backing process of a tagged anchor. Sends SIGSTOP to
    /// the PTY leader (and, via the negative pid trick, its foreground
    /// process group) so the process tree is frozen without being
    /// killed. Pairs the OS effect with the registry state transition so
    /// the sidebar + autosaved snapshot see `Paused`. No-op when the
    /// pane isn't tagged or the child already exited.
    pub fn pause_anchor(&mut self, pane_id: PaneId) -> Result<(), String> {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return Err(format!("pane {pane_id} is not an anchor"));
        };
        let Some(pid) = self.panes.get(&pane_id).and_then(|p| p.child_pid()) else {
            return Err(format!("pane {pane_id} has no live child"));
        };
        send_signal_to_group(pid, libc::SIGSTOP)?;
        self.anchor_registry
            .pause(anchor_id)
            .map_err(|e| format!("registry: {e}"))?;
        tracing::info!(pane_id, pid, "anchor paused");
        self.notify_anchors_changed();
        Ok(())
    }

    /// Continue a paused anchor's backing process. Inverse of
    /// [`pause_anchor`]. Sends SIGCONT; transitions the registry back
    /// to `Live`.
    pub fn resume_anchor(&mut self, pane_id: PaneId) -> Result<(), String> {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return Err(format!("pane {pane_id} is not an anchor"));
        };
        let Some(pid) = self.panes.get(&pane_id).and_then(|p| p.child_pid()) else {
            return Err(format!("pane {pane_id} has no live child"));
        };
        send_signal_to_group(pid, libc::SIGCONT)?;
        self.anchor_registry
            .resume(anchor_id)
            .map_err(|e| format!("registry: {e}"))?;
        tracing::info!(pane_id, pid, "anchor resumed");
        self.notify_anchors_changed();
        Ok(())
    }

    /// Hide the widget of a tagged anchor without killing the backing
    /// process. The PTY + libghostty Terminal stay alive (so scrollback
    /// accumulates normally); only the GTK widget is detached from the
    /// active workspace until [`reattach_anchor`]. Idempotent when the
    /// anchor is already hidden.
    pub fn hide_anchor(&mut self, pane_id: PaneId) -> Result<(), String> {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return Err(format!("pane {pane_id} is not an anchor"));
        };
        if !self.hidden_anchors.insert(pane_id) {
            return Ok(());
        }
        if let Some(pane) = self.panes.get(&pane_id) {
            pane.widget().set_visible(false);
        }
        self.anchor_registry
            .set_hidden(anchor_id, true)
            .map_err(|e| format!("registry: {e}"))?;
        tracing::info!(pane_id, "anchor hidden");
        self.notify_anchors_changed();
        Ok(())
    }

    /// Reverse of [`hide_anchor`]. Re-shows the widget (subject to the
    /// usual active-workspace filter) and flips the registry state back
    /// to Live. Idempotent when already attached.
    pub fn reattach_anchor(&mut self, pane_id: PaneId) -> Result<(), String> {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return Err(format!("pane {pane_id} is not an anchor"));
        };
        if !self.hidden_anchors.remove(&pane_id) {
            return Ok(());
        }
        self.anchor_registry
            .set_hidden(anchor_id, false)
            .map_err(|e| format!("registry: {e}"))?;
        // Defer widget visibility to set_active_anchor's filter so the
        // pane only shows up when its workspace is active.
        let in_active_workspace = match self.active_anchor {
            None => true,
            Some(a) => self.pane_workspace.get(&pane_id) == Some(&a),
        };
        if let Some(pane) = self.panes.get(&pane_id) {
            pane.widget().set_visible(in_active_workspace);
        }
        tracing::info!(pane_id, "anchor reattached");
        self.notify_anchors_changed();
        Ok(())
    }

    /// Convenience predicate for the sidebar (Paused vs Hidden is a view
    /// concern only, so this lives in state rather than the registry).
    #[allow(dead_code)]
    pub fn is_anchor_hidden(&self, pane_id: PaneId) -> bool {
        self.hidden_anchors.contains(&pane_id)
    }

    pub fn pane_in_active_workspace(&self, pane_id: PaneId) -> bool {
        match self.active_anchor {
            None => true,
            Some(active) => self.pane_workspace.get(&pane_id) == Some(&active),
        }
    }

    /// Rename a tagged pane's anchor. `None`/empty clears the override so
    /// the sidebar falls back to argv-derived labels. Fires the refresh
    /// hook so the UI re-renders.
    /// Apply a freshly-loaded [`lmux_config::Config`] to every live pane.
    /// Today that's just the font family + size; as more runtime-tunable
    /// config fields appear this method grows. Runs on the GTK main loop
    /// (called from the hot-reload dispatch).
    /// Name of the session whose snapshot is being edited in-place, or
    /// `None` when no switcher swap has happened yet (legacy single-session
    /// behavior).
    #[allow(dead_code)]
    pub fn current_session(&self) -> Option<&str> {
        self.current_session.as_deref()
    }

    /// Save the current pane tree to `<name>.toml` in `store_root` and
    /// tag it as the current session. Used by the first-run path when
    /// the user hasn't named the session yet but wants the switcher to
    /// start tracking it.
    #[allow(dead_code)]
    pub fn set_current_session(&mut self, name: Option<String>) {
        self.current_session = name;
    }

    /// Swap the live pane tree for the one recorded under `target_name`.
    /// Saves the current tree first (to `current_session` if set), then
    /// terminates every live pane and rebuilds from the target's
    /// on-disk snapshot. If the target has no snapshot yet, a fresh
    /// single-pane session is created at `$HOME`.
    ///
    /// This is destructive: running shells in the outgoing session get
    /// SIGTERM + scheduled SIGKILL, matching `drain_panes_for_shutdown`.
    /// The caller is responsible for confirming the user's intent.
    pub fn switch_session(
        &mut self,
        target_name: String,
        store_root: &std::path::Path,
    ) -> Result<(), String> {
        if self.current_session.as_deref() == Some(target_name.as_str()) {
            return Ok(());
        }
        let store = lmux_session::SessionStore::new(store_root);
        let now = lmux_session::now_unix_seconds();

        // Persist the outgoing session so its layout + cwds survive the
        // swap. Only save when we know the name; legacy (current_session
        // == None) callers are expected to drop their state on switch.
        if let Some(cur) = self.current_session.clone() {
            let snap = self.snapshot();
            let anchors = self.session_anchor_refs();
            let session = lmux_session::Session {
                name: cur.clone(),
                created_at_unix_seconds: snap.created_at_unix_seconds,
                last_opened_at_unix_seconds: now,
                layout: snap.layout,
                cwds: snap.cwds,
                anchors,
                pane_titles: snap.pane_titles,
                terminal_tabs: snap.terminal_tabs,
                pane_terminal_tab_roots: snap.pane_terminal_tab_roots,
            };
            if let Err(err) = store.save(&session) {
                tracing::warn!(error = %err, session = %cur, "switch_session: save outgoing failed");
            }
        }

        let loaded = store.load(&target_name).ok();

        // Tear down the current pane tree. After this call `self.panes`
        // is empty and the root container has no children.
        let dropped = self.drain_panes_for_shutdown();
        for pane in &dropped {
            if !pane.has_exited() {
                // drain_panes_for_shutdown already sent SIGTERM; schedule
                // the follow-up SIGKILL via the same timer the close
                // path uses. We can't reach schedule_force_kill here
                // without moving Pane, so rely on Pane's Drop to release
                // the PTY master; the child shell exits soon after.
            }
        }
        drop(dropped);
        self.last_notif_id.clear();
        self.active_anchor = None;
        self.anchor_registry = AnchorRegistry::default();
        self.pane_workspace.clear();
        self.pane_anchor_ids.clear();
        self.pane_uuids.clear();
        self.pane_titles.clear();
        self.pane_agents.clear();
        self.grants.clear();
        self.terminal_tabs_by_anchor.clear();
        self.active_terminal_tab_by_anchor.clear();
        self.pane_terminal_tab_roots.clear();
        self.hidden_anchors.clear();
        self.anchors = BTreeSet::new();
        self.phase = Phase::Running;

        // Rehydrate the pane set from the target snapshot or fall back
        // to a single shell in $HOME.
        let (mut panes, layout, focused, restored_anchors, next_id) = loaded
            .as_ref()
            .and_then(build_session_panes)
            .or_else(|| fresh_session_panes(1))
            .ok_or_else(|| "switch_session: could not spawn any pane".to_string())?;

        // Hook the new panes into the focus/bell callbacks before they
        // get attached to the widget tree — otherwise the first grab
        // would miss the focus callback.
        if let Some(cb) = self.focus_cb.clone() {
            for pane in panes.values() {
                pane.attach_controllers(
                    cb.clone(),
                    self.focus_mode.clone(),
                    self.terminal_action_cb
                        .clone()
                        .unwrap_or_else(noop_terminal_action_callback),
                    self.shortcut_prefix.clone(),
                );
            }
        }
        if let Some(cb) = self.reparent_cb.clone() {
            for pane in panes.values() {
                pane.attach_rearrange_controllers(self.rearrange_mode.clone(), cb.clone());
            }
        }
        if let Some(cb) = self.bell_cb.clone() {
            for pane in panes.values_mut() {
                pane.set_bell_callback(cb.clone());
            }
        }
        if let Some(cb) = self.terminal_exit_cb.clone() {
            for pane in panes.values_mut() {
                pane.set_exit_callback(cb.clone());
            }
        }

        self.pane_uuids = panes.keys().map(|id| (*id, Uuid::new_v4())).collect();
        self.pane_titles.clear();
        self.pane_agents.clear();
        self.panes = panes;
        self.layout = layout;
        self.focused = focused;
        self.next_id = next_id;
        self.rebuild_widget_tree();

        // Restore anchor tags on the new tree. Each anchor self-owns its
        // workspace slot; remaining non-anchor panes are assigned to the
        // first anchor as a fallback (the snapshot format doesn't yet
        // encode per-satellite ownership, so the primary anchor absorbs
        // the whole session). Using `restore_anchor` (not `add_anchor`)
        // prevents each successive tag from rejecting already-absorbed
        // panes as "already a satellite".
        let first_anchor = restored_anchors.iter().copied().next();
        for id in &restored_anchors {
            self.pane_workspace.insert(*id, *id);
        }
        if let Some(primary) = first_anchor {
            let unowned: Vec<PaneId> = self
                .panes
                .keys()
                .copied()
                .filter(|id| !self.pane_workspace.contains_key(id))
                .collect();
            for id in unowned {
                self.pane_workspace.insert(id, primary);
                self.pane_terminal_tab_roots.insert(id, primary);
            }
        }
        for id in restored_anchors {
            self.restore_anchor(id);
        }
        if let Some(session) = loaded.as_ref() {
            self.restore_session_metadata(session);
        }
        if let Some(primary) = first_anchor {
            self.set_active_anchor(Some(primary));
        }
        self.notify_anchors_changed();

        self.current_session = Some(target_name.clone());

        // Bump the target's index entry to "now" so the switcher lists
        // it at the top next time.
        let bump = lmux_session::Session {
            name: target_name.clone(),
            created_at_unix_seconds: now,
            last_opened_at_unix_seconds: now,
            layout: lmux_state::LayoutNode::Leaf {
                pane_id: self.focused,
            },
            cwds: Default::default(),
            anchors: Vec::new(),
            pane_titles: Default::default(),
            terminal_tabs: Vec::new(),
            pane_terminal_tab_roots: Default::default(),
        };
        // Full save runs on shutdown; here we only need the index
        // recency bump. Re-using `save` would overwrite the freshly
        // installed pane tree on disk with an empty placeholder, so we
        // skip it and let the next shutdown save the real thing.
        let _ = bump;

        self.notify_anchors_changed();
        if let Some(pane) = self.panes.get(&self.focused) {
            pane.grab_focus();
        }
        self.refresh_focus_css();
        Ok(())
    }

    pub fn apply_config(&self, cfg: &lmux_config::Config) {
        let family = cfg.general.font_family.as_str();
        let size = cfg.general.font_size as i32;
        for pane in self.panes.values() {
            pane.set_font(family, size);
        }
        self.focus_mode.set(cfg.general.focus_mode);
    }

    pub fn rename_anchor_for_pane(&mut self, pane_id: PaneId, name: Option<String>) {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return;
        };
        if self.anchor_registry.rename(anchor_id, name).is_ok() {
            self.notify_anchors_changed();
        }
    }

    /// Move a tagged pane into a different (or no) group.
    pub fn regroup_anchor_for_pane(&mut self, pane_id: PaneId, group: Option<String>) {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return;
        };
        if self.anchor_registry.set_group(anchor_id, group).is_ok() {
            self.notify_anchors_changed();
        }
    }

    /// Set a manual sort key. The sidebar orders rows within a group by
    /// ascending sort_key, then display label.
    #[allow(dead_code)]
    pub fn set_anchor_sort_key_for_pane(&mut self, pane_id: PaneId, key: Option<i64>) {
        let Some(anchor_id) = self.pane_anchor_ids.get(&pane_id).copied() else {
            return;
        };
        if self.anchor_registry.set_sort_key(anchor_id, key).is_ok() {
            self.notify_anchors_changed();
        }
    }

    /// Apply a new intra-group ordering by assigning sort_key = 0..N to the
    /// given pane ids in order. Panes not in this group (or not tagged) are
    /// skipped. Used by the sidebar drag-to-reorder handler.
    pub fn reorder_anchors_in_group(&mut self, ordered_pane_ids: &[PaneId]) {
        let mut mutated = false;
        for (idx, pane_id) in ordered_pane_ids.iter().enumerate() {
            let Some(anchor_id) = self.pane_anchor_ids.get(pane_id).copied() else {
                continue;
            };
            if self
                .anchor_registry
                .set_sort_key(anchor_id, Some(idx as i64))
                .is_ok()
            {
                mutated = true;
            }
        }
        if mutated {
            self.notify_anchors_changed();
        }
    }

    /// Install the sidebar refresh hook. Fired after any anchor mutation
    /// (add/remove/close) so the UI can re-render. Back-compat alias for
    /// the now-additive [`AppState::add_anchors_changed_callback`].
    pub fn set_anchors_changed_callback(&mut self, cb: AnchorsChangedCallback) {
        self.add_anchors_changed_callback(cb);
    }

    /// Register an additional listener for anchor mutations. Multiple
    /// observers are supported (sidebar + bus status atomic).
    pub fn add_anchors_changed_callback(&mut self, cb: AnchorsChangedCallback) {
        self.on_anchors_changed.push(cb);
    }

    pub fn add_active_anchor_changed_callback(&mut self, cb: ActiveAnchorChangedCallback) {
        self.on_active_anchor_changed.push(cb);
    }

    fn notify_anchors_changed(&self) {
        // Defer to the next idle tick so callers already inside a
        // `borrow_mut()` don't trigger a RefCell reentrancy panic when a
        // listener re-borrows `AppState`.
        for cb in self.on_anchors_changed.iter().cloned() {
            gtk4::glib::idle_add_local_once(move || cb());
        }
    }

    fn notify_active_anchor_changed(&self) {
        let active = self.active_anchor;
        for cb in self.on_active_anchor_changed.iter().cloned() {
            gtk4::glib::idle_add_local_once(move || cb(active));
        }
    }

    /// Derive argv + cwd to record on the `Anchor`. Panes in v0.1 always
    /// run the detected shell, so argv is just `[shell]`; cwd prefers the
    /// live `/proc` read and falls back to the spawn cwd.
    fn anchor_spawn_metadata(&self, pane_id: PaneId) -> (Vec<String>, String) {
        let shell = lmux_pty::detect_shell();
        let cwd = self
            .panes
            .get(&pane_id)
            .and_then(|p| p.snapshot_cwd())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_default());
        (vec![shell], cwd)
    }

    /// Resolve which pane owns `source_pid` by walking the `/proc` ppid
    /// chain upward until we hit one of our tracked PTY leader PIDs.
    pub fn resolve_owning_pane(&self, source_pid: u32) -> Option<PaneId> {
        let leaders: Vec<(PaneId, u32)> = self
            .panes
            .iter()
            .filter_map(|(id, p)| p.child_pid().map(|pid| (*id, pid)))
            .collect();
        let mut cur = source_pid;
        for _ in 0..64 {
            if let Some((id, _)) = leaders.iter().find(|(_, p)| *p == cur) {
                return Some(*id);
            }
            match read_ppid(cur) {
                None | Some(0) | Some(1) => return None,
                Some(ppid) => cur = ppid,
            }
        }
        None
    }

    pub fn attach_controllers_for_all(
        &mut self,
        cb: FocusCallback,
        terminal_action_cb: TerminalActionCallback,
        shortcut_prefix: ShortcutPrefixCell,
    ) {
        self.terminal_action_cb = Some(terminal_action_cb.clone());
        self.shortcut_prefix = shortcut_prefix.clone();
        for pane in self.panes.values() {
            pane.attach_controllers(
                cb.clone(),
                self.focus_mode.clone(),
                terminal_action_cb.clone(),
                shortcut_prefix.clone(),
            );
        }
        self.focus_cb = Some(cb);
    }

    /// Install the rearrange-mode DnD controllers on every existing pane
    /// and remember the callback so future panes (splits, satellites)
    /// inherit the same wiring.
    pub fn attach_rearrange_for_all(&mut self, cb: crate::pane::ReparentCallback) {
        for pane in self.panes.values() {
            pane.attach_rearrange_controllers(self.rearrange_mode.clone(), cb.clone());
        }
        self.reparent_cb = Some(cb);
    }

    /// Toggle rearrange mode and return the new state. Adds/removes a
    /// CSS class on the root container so styling can flag the cockpit
    /// as "in rearrange mode" (e.g., dashed pane borders).
    pub fn toggle_rearrange_mode(&self) -> bool {
        let next = !self.rearrange_mode.get();
        self.rearrange_mode.set(next);
        if next {
            self.root_container.add_css_class(REARRANGE_CSS_CLASS);
        } else {
            self.root_container.remove_css_class(REARRANGE_CSS_CLASS);
        }
        tracing::info!(active = next, "rearrange mode toggled");
        next
    }

    #[allow(dead_code)]
    pub fn rearrange_mode_active(&self) -> bool {
        self.rearrange_mode.get()
    }

    /// Move `source` next to `target` in the active workspace's layout
    /// according to `edge`, then rebuild the widget tree. No-op if
    /// either pane is missing or they belong to different workspaces
    /// (cross-anchor moves are out of scope for v0.2).
    pub fn reparent_pane(&mut self, source: PaneId, target: PaneId, edge: crate::layout::Edge) {
        if source == target {
            return;
        }
        // Cross-workspace moves would orphan a pane in the other
        // anchor's view — disallow until anchor-routing is taught to
        // re-tag (v0.3).
        let src_ws = self.pane_workspace.get(&source).copied();
        let dst_ws = self.pane_workspace.get(&target).copied();
        if src_ws != dst_ws {
            tracing::debug!(
                source,
                target,
                "rearrange skipped: panes live in different workspaces"
            );
            return;
        }
        if !self.layout.reparent(source, target, edge) {
            tracing::debug!(source, target, ?edge, "rearrange: layout rejected the move");
            return;
        }
        self.rebuild_widget_tree();
    }

    pub fn focused(&self) -> PaneId {
        self.focused
    }

    /// Is the currently-focused pane a Wayland satellite (e.g. a browser
    /// or IDE embedded via the nested compositor)?
    ///
    /// Used by the window-level key controller to let shortcuts like
    /// Ctrl+B (browser bookmarks bar, JetBrains "Go to Declaration")
    /// pass through to the embedded app instead of arming the cockpit's
    /// tmux-style command prefix.
    pub fn focused_is_satellite(&self) -> bool {
        self.panes
            .get(&self.focused)
            .map(|p| p.is_satellite())
            .unwrap_or(false)
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// Find the first terminal leaf in layout-traversal order. Used by the
    /// cockpit's boot self-heal to seed an anchor when a restored session
    /// has none — satellites are skipped because only terminals can serve
    /// as workspace roots.
    pub fn first_terminal_leaf(&self) -> Option<PaneId> {
        self.layout.leaves().into_iter().find(|id| {
            self.panes
                .get(id)
                .map(|p| !p.is_satellite())
                .unwrap_or(false)
        })
    }

    /// Spawn a fresh pane and tag it as a brand-new anchor in its own
    /// workspace. Used by the sidebar "+" button. The new pane is
    /// produced by splitting the focused pane (vertically by default);
    /// inheritance of the source's workspace is skipped so the new pane
    /// is a workspace root, not a satellite, before [`add_anchor`]
    /// promotes it.
    pub fn create_new_anchor(&mut self) {
        let new_id = self.next_id;
        self.split_focused(Dir::Vertical);
        if !self.panes.contains_key(&new_id) {
            // split_focused aborted (allocation failure). Nothing to tag.
            return;
        }
        // Strip inherited workspace so `add_anchor` doesn't reject the
        // pane as "already a satellite of another anchor".
        self.pane_workspace.remove(&new_id);
        self.remove_terminal_tab_metadata(new_id);
        self.focused = new_id;
        self.refresh_focus_css();
        if self.tag_anchor_core(new_id) {
            self.ensure_terminal_tab_stack(new_id);
            self.set_active_anchor(Some(new_id));
            self.notify_anchors_changed();
        }
    }

    pub fn set_focused(&mut self, id: PaneId) {
        if self.panes.contains_key(&id) {
            self.focused = id;
            if let Some(pane) = self.panes.get(&id) {
                pane.grab_focus();
            }
            self.refresh_focus_css();
        }
    }

    /// Sync the `pane--focused` CSS class across every pane so only the
    /// currently-focused one carries it. Cheap — panes are small in
    /// practice (≤6 leaves in realistic sessions).
    fn refresh_focus_css(&self) {
        for (id, pane) in &self.panes {
            let widget = pane.widget();
            if *id == self.focused {
                widget.add_css_class(FOCUSED_CSS_CLASS);
            } else {
                widget.remove_css_class(FOCUSED_CSS_CLASS);
            }
        }
    }

    /// Split the focused pane in `dir`. The original pane stays on the top/
    /// left; the new pane appears bottom/right and starts in the same CWD.
    /// Focus stays on the original pane per Story 3.1.
    pub fn split_focused(&mut self, dir: Dir) {
        let target = self.focused;
        let cwd = self
            .panes
            .get(&target)
            .and_then(|p| p.cwd())
            .map(|p| p.to_path_buf());
        let new_id = self.next_id;
        let Some(new_pane) = Pane::new(new_id, cwd.as_deref()) else {
            tracing::warn!("split aborted — new pane allocation failed");
            return;
        };
        if let Some(cb) = &self.focus_cb {
            new_pane.attach_controllers(
                cb.clone(),
                self.focus_mode.clone(),
                self.terminal_action_cb
                    .clone()
                    .unwrap_or_else(noop_terminal_action_callback),
                self.shortcut_prefix.clone(),
            );
        }
        if let Some(cb) = &self.reparent_cb {
            new_pane.attach_rearrange_controllers(self.rearrange_mode.clone(), cb.clone());
        }
        if let Some(cb) = &self.bell_cb {
            new_pane.set_bell_callback(cb.clone());
        }
        if let Some(cb) = &self.terminal_exit_cb {
            new_pane.set_exit_callback(cb.clone());
        }
        self.panes.insert(new_id, new_pane);
        self.pane_uuids.insert(new_id, Uuid::new_v4());
        // New pane inherits its source's workspace. If the source is
        // unowned (no anchor tagged yet) the new pane is unowned too;
        // both will be absorbed by the first anchor tag.
        if let Some(&owner) = self.pane_workspace.get(&target) {
            self.pane_workspace.insert(new_id, owner);
            let root = self.tab_root_for_pane(target);
            self.pane_terminal_tab_roots.insert(new_id, root);
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .unwrap_or_else(|| u32::MAX.saturating_sub(0));

        let replaced = self.layout.replace_leaf(target, |id| Layout::Split {
            dir,
            a: Box::new(Layout::Leaf(id)),
            b: Box::new(Layout::Leaf(new_id)),
            ratio: 0.5,
        });
        if !replaced {
            tracing::warn!(target, "split_focused could not locate target leaf");
            self.panes.remove(&new_id);
            self.pane_uuids.remove(&new_id);
            self.pane_titles.remove(&new_id);
            self.pane_agents.remove(&new_id);
            self.remove_terminal_tab_metadata(new_id);
            return;
        }
        self.rebuild_widget_tree();
    }

    /// Close the focused pane. No-op when only one pane remains (the window's
    /// last pane can't be closed without quitting lmux — documented in README).
    pub fn close_focused(&mut self) {
        let target = self.focused;
        if matches!(&self.layout, Layout::Leaf(id) if *id == target) {
            tracing::info!("ignoring close-focused on last pane");
            return;
        }
        if !self.layout.remove_leaf(target) {
            tracing::warn!(target, "close_focused could not locate leaf");
            return;
        }
        if self.anchors.remove(&target) {
            if let Some(anchor_id) = self.pane_anchor_ids.remove(&target) {
                let _ = self.anchor_registry.remove(anchor_id);
            }
            self.hidden_anchors.remove(&target);
            // Closing an anchor orphans its satellites; they become
            // unowned. Same rationale as `remove_anchor`.
            self.pane_workspace.retain(|_, owner| *owner != target);
            if self.active_anchor == Some(target) {
                let next = self.anchors.iter().copied().next();
                self.set_active_anchor(next);
            }
            tracing::info!(pane_id = target, "anchor cleared (pane closed)");
            self.notify_anchors_changed();
        } else {
            self.pane_workspace.remove(&target);
        }
        if let Some(pane) = self.panes.remove(&target) {
            self.pane_uuids.remove(&target);
            self.pane_titles.remove(&target);
            self.pane_agents.remove(&target);
            self.remove_terminal_tab_metadata(target);
            // Cooperative shutdown — Epic 7 Story 7.2 wires the 500 ms grace
            // before SIGKILL via `glib::timeout_add_local`.
            pane.terminate();
            schedule_force_kill(pane);
        }
        self.close_orphan_terminal_panes();
        self.ensure_anchor_invariant();
        // Focus moves to the next leaf after the closed one in in-order
        // traversal; fall back to the first leaf if `target` was the last.
        let leaves = self.layout.leaves();
        let next = leaves.first().copied().unwrap_or(target);
        self.focused = next;
        self.rebuild_widget_tree();
        if let Some(pane) = self.panes.get(&self.focused) {
            pane.grab_focus();
        }
        self.refresh_focus_css();
    }

    /// Remove a terminal pane whose child process exited on its own. Unlike
    /// `close_focused`, this must not send SIGTERM/SIGKILL; it only collapses
    /// the layout and drops metadata for panes that are no longer alive.
    pub fn close_exited_pane(&mut self, target: PaneId) {
        if matches!(&self.layout, Layout::Leaf(id) if *id == target) {
            tracing::info!(pane_id = target, "keeping exited last pane visible");
            return;
        }
        if !self
            .panes
            .get(&target)
            .is_some_and(|pane| pane.has_exited())
        {
            tracing::debug!(
                pane_id = target,
                "ignoring close_exited_pane for live or missing pane"
            );
            return;
        }
        if !self.layout.remove_leaf(target) {
            tracing::warn!(pane_id = target, "close_exited_pane could not locate leaf");
            return;
        }
        if self.anchors.remove(&target) {
            if let Some(anchor_id) = self.pane_anchor_ids.remove(&target) {
                let _ = self.anchor_registry.remove(anchor_id);
            }
            self.hidden_anchors.remove(&target);
            self.pane_workspace.retain(|_, owner| *owner != target);
            if self.active_anchor == Some(target) {
                let next = self.anchors.iter().copied().next();
                self.set_active_anchor(next);
            }
            tracing::info!(pane_id = target, "anchor cleared (pane exited)");
            self.notify_anchors_changed();
        } else {
            self.pane_workspace.remove(&target);
        }
        self.panes.remove(&target);
        self.pane_uuids.remove(&target);
        self.pane_titles.remove(&target);
        self.pane_agents.remove(&target);
        self.remove_terminal_tab_metadata(target);
        self.close_orphan_terminal_panes();
        self.ensure_anchor_invariant();

        if self.focused == target || !self.panes.contains_key(&self.focused) {
            if let Some(next) = self.layout.leaves().first().copied() {
                self.focused = next;
            }
        }
        self.rebuild_widget_tree();
        if let Some(pane) = self.panes.get(&self.focused) {
            pane.grab_focus();
        }
        self.refresh_focus_css();
        tracing::info!(pane_id = target, "exited terminal pane removed");
    }

    /// Cycle focus to the next/previous leaf in in-order traversal.
    pub fn cycle_focus(&mut self, forward: bool) {
        let leaves = self.layout.leaves();
        if leaves.len() <= 1 {
            return;
        }
        let idx = leaves
            .iter()
            .position(|id| *id == self.focused)
            .unwrap_or(0);
        let n = leaves.len();
        let next = if forward {
            leaves[(idx + 1) % n]
        } else {
            leaves[(idx + n - 1) % n]
        };
        self.focused = next;
        if let Some(pane) = self.panes.get(&next) {
            pane.grab_focus();
        }
        self.refresh_focus_css();
    }

    /// Rebuild the GTK widget tree from the layout. Every pane frame is
    /// unparented first so we can splice them freely into new `gtk::Paned`
    /// nodes without GTK complaining about already-having-a-parent.
    fn rebuild_widget_tree(&self) {
        let started = Instant::now();
        self.workspace_stack.borrow_mut().take();
        // Unparent any existing child of the root container.
        while let Some(child) = self.root_container.first_child() {
            self.root_container.remove(&child);
        }
        // Unparent every pane frame — some may still be attached to Paned
        // children from the previous tree.
        for pane in self.panes.values() {
            let w: &Widget = pane.widget().upcast_ref();
            if let Some(parent) = w.parent() {
                if let Some(paned) = parent.downcast_ref::<Paned>() {
                    if paned.start_child().as_ref() == Some(w) {
                        paned.set_start_child(None::<&Widget>);
                    } else if paned.end_child().as_ref() == Some(w) {
                        paned.set_end_child(None::<&Widget>);
                    }
                } else if let Some(b) = parent.downcast_ref::<gtk4::Box>() {
                    b.remove(w);
                } else {
                    w.unparent();
                }
            }
        }

        // Prune the layout to workspace pages. With multi-anchor sessions
        // the shared `self.layout` tree contains every anchor's subtree; if
        // we hand the whole thing to GTK, GtkPaned still allocates space for
        // hidden children. A GtkStack keeps each anchor's mounted workspace
        // around so later active-anchor switches only flip pages.
        let pruned = if self.anchors.is_empty() {
            match self.active_anchor {
                None => None,
                Some(active) => self.prune_to_workspace(&self.layout, active),
            }
        } else {
            None
        };
        tracing::debug!(
            active = ?self.active_anchor,
            panes = ?self.panes.keys().collect::<Vec<_>>(),
            workspace = ?self.pane_workspace,
            ?pruned,
            "rebuild_widget_tree"
        );
        if self.anchors.is_empty() {
            let widget = pruned.as_ref().and_then(|l| build_widget(l, &self.panes));
            if let Some(w) = widget {
                self.root_container.append(&w);
            }
        } else {
            let stack = Stack::new();
            stack.set_hexpand(true);
            stack.set_vexpand(true);
            for anchor in &self.anchors {
                let Some(pruned) = self.prune_to_workspace(&self.layout, *anchor) else {
                    continue;
                };
                let Some(widget) = build_widget(&pruned, &self.panes) else {
                    continue;
                };
                let widget = self.wrap_workspace_with_tabs(*anchor, widget);
                stack.add_named(&widget, Some(&workspace_page_name(*anchor)));
            }
            if let Some(active) = self
                .active_anchor
                .or_else(|| self.anchors.iter().copied().next())
            {
                stack.set_visible_child_name(&workspace_page_name(active));
            }
            self.root_container.append(&stack);
            *self.workspace_stack.borrow_mut() = Some(stack);
        }
        // Diagnostic: walk the actual GTK tree under root_container so we
        // can confirm only the pruned leaves are present.
        let mut walk: Vec<String> = Vec::new();
        walk_tree(self.root_container.upcast_ref::<Widget>(), 0, &mut walk);
        tracing::debug!(
            operation = "gtk.workspace.switch",
            duration_ms = elapsed_ms(started),
            active = ?self.active_anchor,
            panes = self.panes.len(),
            anchors = self.anchors.len(),
            satellite_windows = self.satellite_window_count(),
            mounted_widgets = walk.len(),
            tree = walk.join(" | ").as_str(),
            "post-rebuild tree"
        );
    }

    fn switch_workspace_view(&self) -> bool {
        let Some(active) = self.active_anchor else {
            return false;
        };
        let Some(stack) = self.workspace_stack.borrow().as_ref().cloned() else {
            return false;
        };
        let page = workspace_page_name(active);
        if stack.child_by_name(&page).is_none() {
            return false;
        }
        stack.set_visible_child_name(&page);
        true
    }

    fn prune_to_workspace(&self, layout: &Layout, active: PaneId) -> Option<Layout> {
        let active_tab = self.active_terminal_tab(active);
        prune_to_workspace(layout, active, active_tab, &self.pane_workspace, |pane| {
            if self
                .panes
                .get(&pane)
                .is_some_and(|pane| pane.is_satellite())
            {
                return active_tab;
            }
            self.tab_root_for_pane(pane)
        })
    }

    fn wrap_workspace_with_tabs(&self, anchor: PaneId, widget: Widget) -> Widget {
        let tabs = self
            .terminal_tabs_by_anchor
            .get(&anchor)
            .cloned()
            .unwrap_or_else(|| vec![anchor]);
        if tabs.len() <= 1 {
            return widget;
        }
        let outer = gtk4::Box::new(Orientation::Vertical, 0);
        let strip = gtk4::Box::new(Orientation::Horizontal, 4);
        strip.add_css_class("lmux-terminal-tabs");
        let active = self.active_terminal_tab(anchor);
        for tab in tabs {
            let title = self.pane_title_display(tab);
            let button = Button::new();
            button.add_css_class("lmux-terminal-tab");
            button.set_tooltip_text(Some(&format!("Switch to {title}")));
            let label = Label::new(Some(&title));
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_xalign(0.5);
            button.set_child(Some(&label));
            if tab == active {
                button.add_css_class("lmux-terminal-tab--active");
            }
            if let Some(cb) = self.terminal_tab_cb.clone() {
                button.connect_clicked(move |_| cb(anchor, tab));
            }
            if let Some(rename_cb) = self.terminal_tab_rename_cb.clone() {
                let click = GestureClick::new();
                click.set_button(gtk4::gdk::BUTTON_SECONDARY);
                let button_weak = button.downgrade();
                let current_title = self.pane_title_edit_text(tab);
                click.connect_pressed(move |_, _, _, _| {
                    if let Some(button) = button_weak.upgrade() {
                        show_terminal_tab_popover(&button, tab, &current_title, rename_cb.clone());
                    }
                });
                button.add_controller(click);
            }
            strip.append(&button);
        }
        outer.append(&strip);
        outer.append(&widget);
        outer.upcast()
    }

    fn pane_title_display(&self, pane_id: PaneId) -> String {
        let Some(title) = self.pane_titles.get(&pane_id) else {
            return format!("pane {pane_id}");
        };
        let suffix = match (title.provenance, title.pinned) {
            (_, true) => " [pinned]",
            (lmux_bus::PaneTitleProvenance::Agent, false) => " [agent]",
            (lmux_bus::PaneTitleProvenance::User, false) => " [user]",
            (lmux_bus::PaneTitleProvenance::Default, false) => "",
        };
        format!("{}{}", title.title, suffix)
    }

    fn pane_title_edit_text(&self, pane_id: PaneId) -> String {
        self.pane_titles
            .get(&pane_id)
            .map(|title| title.title.clone())
            .unwrap_or_default()
    }
}

fn show_terminal_tab_popover(
    anchor_widget: &Button,
    pane_id: PaneId,
    current_title: &str,
    rename_cb: TerminalTabRenameCallback,
) {
    let popover = Popover::new();
    popover.set_position(PositionType::Bottom);
    popover.set_has_arrow(true);
    popover.set_autohide(true);

    let body = gtk4::Box::new(Orientation::Vertical, 6);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);

    let heading = Label::new(Some(&format!("Shell tab · pane {pane_id}")));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    body.append(&heading);

    let label = Label::new(Some("Name"));
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    body.append(&label);

    let entry = Entry::new();
    entry.set_text(current_title);
    entry.set_placeholder_text(Some("(default)"));
    body.append(&entry);

    let button_row = gtk4::Box::new(Orientation::Horizontal, 6);
    button_row.set_halign(Align::End);
    let clear_btn = Button::with_label("Clear");
    let apply_btn = Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");
    button_row.append(&clear_btn);
    button_row.append(&apply_btn);
    body.append(&button_row);

    popover.set_child(Some(&body));
    popover.set_parent(anchor_widget);

    let apply_entry = entry.clone();
    let apply_popover = popover.clone();
    let apply_cb = rename_cb.clone();
    let do_apply = move || {
        let title = trim_to_option(&apply_entry.text());
        apply_cb(pane_id, title);
        apply_popover.popdown();
    };

    let do_apply_btn = do_apply.clone();
    apply_btn.connect_clicked(move |_| do_apply_btn());
    entry.connect_activate(move |_| do_apply());

    let clear_popover = popover.clone();
    clear_btn.connect_clicked(move |_| {
        rename_cb(pane_id, None);
        clear_popover.popdown();
    });

    let popover_cleanup = popover.clone();
    popover.connect_closed(move |_| {
        popover_cleanup.unparent();
    });

    popover.popup();
    entry.grab_focus();
}

fn trim_to_option(s: &gtk4::glib::GString) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn workspace_page_name(anchor: PaneId) -> String {
    format!("anchor-{anchor}")
}

fn elapsed_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

fn walk_tree(w: &Widget, depth: usize, out: &mut Vec<String>) {
    let ty = w.type_().name().to_string();
    let vis = if w.is_visible() { "V" } else { "H" };
    let (ww, wh) = (w.width(), w.height());
    out.push(format!("{}{}[{vis} {ww}x{wh}]", "  ".repeat(depth), ty));
    let mut child = w.first_child();
    while let Some(c) = child {
        walk_tree(&c, depth + 1, out);
        child = c.next_sibling();
    }
}

/// Walk `layout` and drop any leaf whose pane isn't owned by `active`.
/// Splits with one surviving child collapse to that child; splits with
/// none collapse to `None`. Unowned leaves (no entry in `workspace`) are
/// dropped — they belong to the "no anchor active" view, not to any
/// specific anchor.
fn prune_to_workspace(
    layout: &Layout,
    active: PaneId,
    active_tab: PaneId,
    workspace: &HashMap<PaneId, PaneId>,
    tab_root: impl Fn(PaneId) -> PaneId + Copy,
) -> Option<Layout> {
    match layout {
        Layout::Leaf(id) => (workspace.get(id) == Some(&active) && tab_root(*id) == active_tab)
            .then_some(Layout::Leaf(*id)),
        Layout::Split { dir, a, b, ratio } => {
            let pa = prune_to_workspace(a, active, active_tab, workspace, tab_root);
            let pb = prune_to_workspace(b, active, active_tab, workspace, tab_root);
            match (pa, pb) {
                (None, None) => None,
                (Some(only), None) | (None, Some(only)) => Some(only),
                (Some(a), Some(b)) => Some(Layout::Split {
                    dir: *dir,
                    a: Box::new(a),
                    b: Box::new(b),
                    ratio: *ratio,
                }),
            }
        }
    }
}

/// When a pane is closed we SIGTERM first, then schedule a SIGKILL after
/// 500 ms in case the child ignored SIGTERM. Dropping the Pane at the end
/// releases the PTY master. Story 3.2 satisfies FR13; Epic 7 tightens the
/// shutdown budget further.
fn schedule_force_kill(pane: Pane) {
    let pane_rc = Rc::new(RefCell::new(Some(pane)));
    let pane_clone = pane_rc.clone();
    gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
        let mut slot = pane_clone.borrow_mut();
        if let Some(p) = slot.as_ref() {
            if !p.has_exited() {
                p.kill();
            }
        }
        *slot = None;
    });
}

/// Parse the ppid from `/proc/<pid>/stat`. The `comm` field can contain
/// arbitrary bytes including parens and whitespace, so we find the last
/// `)` and split the remainder.
fn read_ppid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let last = s.rfind(')')?;
    let rest = s[last + 1..].trim_start();
    let mut it = rest.split_whitespace();
    it.next()?; // state
    it.next()?.parse().ok()
}

/// Send `sig` to the whole process group led by `pid`. Most interactive
/// shells put their children in their own group, so targeting the group
/// (via negated pid) stops the shell *and* whatever it spawned. Falls
/// back to a plain `kill(pid, sig)` if the group variant fails — e.g.
/// when the child isn't a group leader.
fn send_signal_to_group(pid: u32, sig: libc::c_int) -> Result<(), String> {
    let pgid_target = -(pid as i32);
    // SAFETY: libc::kill is safe to call with any integer arguments; it
    // just returns -1/EINVAL for bad values.
    let rc = unsafe { libc::kill(pgid_target, sig) };
    if rc == 0 {
        return Ok(());
    }
    let rc2 = unsafe { libc::kill(pid as i32, sig) };
    if rc2 == 0 {
        return Ok(());
    }
    Err(format!(
        "kill({pid}, {sig}): {}",
        std::io::Error::last_os_error()
    ))
}

/// Materialised pane set produced by [`build_session_panes`] /
/// [`fresh_session_panes`]: `(panes, layout, focused, anchors, next_id)`.
type RestoredPanes = (
    HashMap<PaneId, Pane>,
    Layout,
    PaneId,
    BTreeSet<PaneId>,
    PaneId,
);

/// Turn a loaded `Session` into a pane map + layout + starting focus,
/// mirroring `app::build_restored` but without Session / Snapshot mixing.
/// Returns `None` if no leaf pane could be spawned — callers fall back to
/// [`fresh_session_panes`].
fn build_session_panes(session: &lmux_session::Session) -> Option<RestoredPanes> {
    let mut layout = layout_from_snapshot(&session.layout);
    let leaves = layout.leaves();
    if leaves.is_empty() {
        return None;
    }
    let mut panes: HashMap<PaneId, Pane> = HashMap::with_capacity(leaves.len());
    for id in &leaves {
        let recorded = session
            .cwds
            .get(id)
            .map(|s| std::path::PathBuf::from(s.as_str()));
        let cwd = match recorded {
            Some(p) if p.is_dir() => Some(p),
            _ => std::env::var("HOME").ok().map(std::path::PathBuf::from),
        };
        if let Some(pane) = Pane::new(*id, cwd.as_deref()) {
            panes.insert(*id, pane);
        } else {
            tracing::warn!(pane_id = id, "switch_session: pane spawn failed; skipping");
        }
    }
    if panes.is_empty() {
        return None;
    }
    for id in &leaves {
        if !panes.contains_key(id) {
            layout.remove_leaf(*id);
        }
    }
    let surviving_leaves = layout.leaves();
    let next_id = surviving_leaves.iter().copied().max().unwrap_or(0) + 1;
    let anchors: BTreeSet<PaneId> = session
        .anchors
        .iter()
        .map(|a| a.pane_id)
        .filter(|id| panes.contains_key(id))
        .collect();
    let focused = anchors
        .iter()
        .copied()
        .next()
        .or_else(|| surviving_leaves.first().copied())?;
    Some((panes, layout, focused, anchors, next_id))
}

/// Spawn a single fresh pane at `$HOME` with id `first_id`. Used when
/// the switcher target has no on-disk snapshot yet. Returns `None` if
/// allocation fails — callers bubble this up to the UI.
fn fresh_session_panes(first_id: PaneId) -> Option<RestoredPanes> {
    let cwd = std::env::var("HOME").ok().map(std::path::PathBuf::from);
    let pane = Pane::new(first_id, cwd.as_deref())?;
    let mut panes = HashMap::new();
    panes.insert(first_id, pane);
    Some((
        panes,
        Layout::Leaf(first_id),
        first_id,
        BTreeSet::new(),
        first_id + 1,
    ))
}

/// Convert the in-app `Layout` to the serialisable `lmux_state::LayoutNode`.
pub fn layout_to_snapshot(l: &Layout) -> lmux_state::LayoutNode {
    match l {
        Layout::Leaf(id) => lmux_state::LayoutNode::Leaf { pane_id: *id },
        Layout::Split { dir, a, b, ratio } => lmux_state::LayoutNode::Split {
            dir: match dir {
                Dir::Horizontal => lmux_state::SplitDir::Horizontal,
                Dir::Vertical => lmux_state::SplitDir::Vertical,
            },
            a: Box::new(layout_to_snapshot(a)),
            b: Box::new(layout_to_snapshot(b)),
            ratio: *ratio,
        },
    }
}

/// Inverse of `layout_to_snapshot` — used during restore (Story 8.3).
pub fn layout_from_snapshot(n: &lmux_state::LayoutNode) -> Layout {
    match n {
        lmux_state::LayoutNode::Leaf { pane_id } => Layout::Leaf(*pane_id),
        lmux_state::LayoutNode::Split { dir, a, b, ratio } => Layout::Split {
            dir: match dir {
                lmux_state::SplitDir::Horizontal => Dir::Horizontal,
                lmux_state::SplitDir::Vertical => Dir::Vertical,
            },
            a: Box::new(layout_from_snapshot(a)),
            b: Box::new(layout_from_snapshot(b)),
            ratio: *ratio,
        },
    }
}

fn pane_title_to_snapshot(title: &lmux_bus::PaneTitle) -> lmux_state::PaneTitleSnapshot {
    lmux_state::PaneTitleSnapshot {
        title: title.title.clone(),
        provenance: match title.provenance {
            lmux_bus::PaneTitleProvenance::Default => {
                lmux_state::PaneTitleProvenanceSnapshot::Default
            }
            lmux_bus::PaneTitleProvenance::Agent => lmux_state::PaneTitleProvenanceSnapshot::Agent,
            lmux_bus::PaneTitleProvenance::User => lmux_state::PaneTitleProvenanceSnapshot::User,
        },
        pinned: title.pinned,
    }
}

fn pane_title_from_snapshot(title: &lmux_state::PaneTitleSnapshot) -> lmux_bus::PaneTitle {
    lmux_bus::PaneTitle {
        title: title.title.clone(),
        provenance: match title.provenance {
            lmux_state::PaneTitleProvenanceSnapshot::Default => {
                lmux_bus::PaneTitleProvenance::Default
            }
            lmux_state::PaneTitleProvenanceSnapshot::Agent => lmux_bus::PaneTitleProvenance::Agent,
            lmux_state::PaneTitleProvenanceSnapshot::User => lmux_bus::PaneTitleProvenance::User,
        },
        pinned: title.pinned,
    }
}

fn window_candidate_prompt_metadata(candidate: &WindowCandidate) -> String {
    let mut parts = vec![
        format!("backend={:?}", candidate.backend),
        format!("window_id={}", candidate.backend_window_id),
    ];
    if let Some(pid) = candidate.pid {
        parts.push(format!("pid={pid}"));
    }
    if let Some(app) = &candidate.app_identity {
        parts.push(format!("app={app:?}"));
    }
    if let Some(title) = &candidate.title {
        parts.push(format!("title={title}"));
    }
    if let Some(workspace) = &candidate.workspace {
        parts.push(format!("workspace={workspace}"));
    }
    if let Some(output) = &candidate.output {
        parts.push(format!("output={output}"));
    }
    parts.join(", ")
}

fn launch_attach_prompt_metadata(
    argv: &[String],
    title_hint: Option<&str>,
    app_hint: Option<&str>,
    timeout_ms: u64,
) -> String {
    let mut parts = vec![format!("argv={argv:?}"), format!("timeout_ms={timeout_ms}")];
    if let Some(title_hint) = title_hint {
        parts.push(format!("title_hint={title_hint}"));
    }
    if let Some(app_hint) = app_hint {
        parts.push(format!("app_hint={app_hint}"));
    }
    parts.join(", ")
}

fn shell_command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'=')
        })
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn build_widget(layout: &Layout, panes: &HashMap<PaneId, Pane>) -> Option<Widget> {
    match layout {
        Layout::Leaf(id) => panes.get(id).map(|p| p.widget().clone().upcast()),
        Layout::Split { dir, a, b, ratio } => {
            let orient = match dir {
                // Horizontal split line → children stacked top/bottom → GTK vertical orientation.
                Dir::Horizontal => Orientation::Vertical,
                Dir::Vertical => Orientation::Horizontal,
            };
            let paned = Paned::new(orient);
            paned.set_hexpand(true);
            paned.set_vexpand(true);
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            if let Some(child_a) = build_widget(a, panes) {
                paned.set_start_child(Some(&child_a));
            }
            if let Some(child_b) = build_widget(b, panes) {
                paned.set_end_child(Some(&child_b));
            }
            // Apply the ratio once the paned has a real allocation. We
            // retry every frame clock tick until width()/height() go
            // positive — `idle_add` and `connect_map` both race the first
            // layout pass (width == 0), and a one-shot `notify::position`
            // flag backfires because GTK flips `position` during its own
            // initialization, which would lock out our apply before it
            // runs. After a successful apply the tick handler breaks, so
            // subsequent user drags on the divider are preserved.
            let r = *ratio;
            paned.add_tick_callback(move |p, _clock| {
                let total = match p.orientation() {
                    Orientation::Horizontal => p.width(),
                    _ => p.height(),
                };
                if total <= 0 {
                    return gtk4::glib::ControlFlow::Continue;
                }
                p.set_position((f64::from(total) * r) as i32);
                gtk4::glib::ControlFlow::Break
            });
            Some(paned.upcast())
        }
    }
}
