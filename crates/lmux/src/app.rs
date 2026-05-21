use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, CssProvider, EventControllerKey, Orientation, PropagationPhase,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};

const APP_CSS: &str = "
frame.pane {
    border: 1px solid transparent;
    padding: 0;
}
frame.pane--focused {
    border: 1px solid #3b82f6;
    box-shadow: 0 0 0 1px alpha(#3b82f6, 0.35) inset;
}
frame.pane--anchor {
    border: 2px dashed #3b82f6;
}
frame.pane--anchor.pane--anchor-active {
    border: 2px solid #3b82f6;
}
frame.pane--anchor.pane--focused {
    border: 2px solid #3b82f6;
    box-shadow: 0 0 0 1px alpha(#3b82f6, 0.35) inset;
}
.lmux-sidebar__row--active {
    background-color: alpha(#3b82f6, 0.18);
    border-radius: 6px;
}
.lmux-sidebar__active-dot {
    color: #3b82f6;
    font-size: 10pt;
    padding-right: 4px;
}
.lmux-window-picker__row {
    border-radius: 6px;
}
.lmux-window-picker__row:hover {
    background-color: alpha(#3b82f6, 0.12);
}
.lmux-window-picker__row--attached-active {
    background-color: alpha(#10b981, 0.10);
    border-left: 4px solid #10b981;
}
.lmux-window-picker__row--attached-other {
    background-color: alpha(#f59e0b, 0.10);
    border-left: 4px solid #f59e0b;
}
.lmux-window-picker__preview {
    background-color: #1f2937;
    border: 1px solid alpha(#94a3b8, 0.45);
    border-radius: 6px;
}
.lmux-window-picker__preview--missing {
    background-color: #334155;
}
.lmux-window-picker__preview-text {
    color: #f8fafc;
    font-size: 10pt;
    font-weight: 700;
}
.lmux-window-picker__meta {
    color: #64748b;
    font-size: 9pt;
}
.lmux-window-picker__attached-active {
    color: #047857;
    font-weight: 700;
}
.lmux-window-picker__attached-other {
    color: #92400e;
    font-weight: 700;
}
/* Rearrange mode: dashed amber border on every pane signals that drag-to-
   reflow is active. The selector intentionally lives on the root container
   so toggling a single class lights up every nested Frame at once. */
.lmux--rearrange frame.pane {
    border: 2px dashed #f59e0b;
}
.lmux--rearrange frame.pane--focused {
    border: 2px solid #f59e0b;
}
";

use std::sync::Arc;

use lmux_control::{AppEvent, Response as CtrlResponse, PROTOCOL_VERSION};
use lmux_notify::Notifier;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::layout::{Dir, Layout, PaneId};
use crate::pane::{Pane, ShortcutPrefixCell, TerminalContextAction};
use crate::sidebar;
use crate::state::{self, AppState, SharedAppState};

/// Grace window before SIGKILLing any PTY children that ignored SIGTERM.
/// Matches FR34 / NFR8: full shutdown (SIGTERM → SIGKILL → app.quit) must
/// fit inside 2 s for a typical workload (≤6 panes).
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// How long a Ctrl+B prefix stays armed before auto-disarming. Matches tmux's
/// default `escape-time` + `repeat-time` feel. 1 s is long enough to not feel
/// racy but short enough that a stale prefix doesn't swallow real typing.
const PREFIX_TIMEOUT: Duration = Duration::from_millis(1000);

pub fn activate(app: &Application) {
    install_css();

    let snapshot = load_snapshot();
    let restored = snapshot.and_then(|s| build_restored(&s));

    let root = gtk4::Box::new(Orientation::Vertical, 0);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let (app_state, cell_w, cell_h) = match restored {
        Some(r) => {
            let (cw, ch) = r.first_cell_size.unwrap_or_else(first_cell_size_fallback);
            for pane in r.panes.values() {
                // Parent will be set by rebuild inside new_from_snapshot.
                let _ = pane;
            }
            let st = AppState::new_from_snapshot(
                root.clone(),
                r.panes,
                r.layout,
                r.focused,
                r.anchors,
                r.next_id,
            );
            tracing::info!(
                panes = st.pane_count(),
                "session restored from last-session.json"
            );
            (st, cw, ch)
        }
        None => {
            let first = match Pane::new(1, None) {
                Some(p) => p,
                None => {
                    tracing::error!("initial pane allocation failed; quitting");
                    app.quit();
                    return;
                }
            };
            let (cw, ch) = first.cell_size();
            let first_id = first.id();
            root.append(first.widget());
            let mut st = AppState::new(root.clone(), first);
            // Auto-anchor the first terminal on fresh startup so anchor-gated
            // features (sidebar, launcher, bus routing) have a target from
            // the start — otherwise the cockpit boots into an "orphan pane"
            // state where nothing knows where satellites should attach.
            st.add_anchor(first_id);
            (st, cw, ch)
        }
    };

    let default_w = (cell_w * 100).max(400);
    let default_h = (cell_h * 30).max(300);

    let app_state = {
        // Self-healing anchor invariant: any cockpit session must have at
        // least one anchor. If a legacy / corrupt snapshot restored with
        // none, tag the first terminal leaf so anchor-gated features
        // (sidebar, launcher, bus routing) have a valid target.
        let mut st = app_state;
        if st.anchor_count() == 0 {
            if let Some(seed) = st.first_terminal_leaf() {
                st.add_anchor(seed);
            }
        }
        st
    };

    let state: SharedAppState = Rc::new(RefCell::new(app_state));
    let shortcut_prefix: ShortcutPrefixCell = Rc::new(RefCell::new(load_keymap_prefix()));
    let focus_cb = make_focus_cb(&state);
    let reparent_cb = make_reparent_cb(&state);
    let terminal_action_cb = make_terminal_action_cb(&state);
    {
        let mut s = state.borrow_mut();
        s.attach_controllers_for_all(
            focus_cb.clone(),
            terminal_action_cb,
            shortcut_prefix.clone(),
        );
        s.attach_rearrange_for_all(reparent_cb);
    }

    // Wrap the pane tree in the anchor sidebar + install its refresh hook.
    let sidebar_cfg = sidebar::load_config();
    let root_with_sidebar = sidebar::install(sidebar_cfg, root, state.clone());
    root_with_sidebar.set_vexpand(true);
    install_application_menubar(app);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("lmux")
        .default_width(default_w)
        .default_height(default_h)
        .child(&root_with_sidebar)
        .build();
    window.set_size_request(400, 300);

    install_window_menu_actions(app, &window, &state, shortcut_prefix.clone());
    install_window_shortcuts(app, &window, &state, shortcut_prefix.clone());
    install_close_request(app, &window, &state);
    install_control_socket(&window, &state);
    install_notifier(&window, &state);
    install_bus_server(&state);
    install_config_watch(&window, &state, shortcut_prefix);
    #[cfg(target_os = "linux")]
    install_wayland_host(&state);
    crate::launcher::warm_cache();

    // Grab focus once the window is mapped so keystrokes reach the initial
    // pane immediately without the user having to click.
    let state_focus = state.clone();
    window.connect_map(move |_| {
        let focused = state_focus.borrow().focused();
        state_focus.borrow_mut().set_focused(focused);
    });

    unsafe {
        window.set_data::<SharedAppState>("lmux-state", state);
    }

    window.present();
    tracing::info!("window presented");
}

fn make_reparent_cb(state: &SharedAppState) -> crate::pane::ReparentCallback {
    let weak = Rc::downgrade(state);
    Rc::new(move |source, target, edge| {
        if let Some(s) = weak.upgrade() {
            if let Ok(mut b) = s.try_borrow_mut() {
                b.reparent_pane(source, target, edge);
            } else {
                tracing::debug!(source, target, "reparent skipped (state busy)");
            }
        }
    })
}

fn make_focus_cb(state: &SharedAppState) -> Rc<dyn Fn(PaneId)> {
    let weak = Rc::downgrade(state);
    Rc::new(move |id: PaneId| {
        if let Some(s) = weak.upgrade() {
            // Focus events can arrive synchronously from GTK during other
            // state mutations (e.g. `apply_config`'s font swap triggers
            // resize, which can re-pick the pointer and fire `enter` while
            // `AppState` is still borrowed). Drop the focus event rather
            // than panic — the next click or hover will resync focus.
            if let Ok(mut b) = s.try_borrow_mut() {
                b.set_focused(id);
            } else {
                tracing::debug!(pane = id, "focus cb skipped (state busy)");
            }
        }
    })
}

fn make_terminal_action_cb(state: &SharedAppState) -> crate::pane::TerminalActionCallback {
    let weak = Rc::downgrade(state);
    Rc::new(move |pane_id, action| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Ok(mut state) = state.try_borrow_mut() else {
            tracing::debug!(
                pane = pane_id,
                ?action,
                "terminal action skipped (state busy)"
            );
            return;
        };
        state.set_focused(pane_id);
        match action {
            TerminalContextAction::SplitRight => state.split_focused(Dir::Vertical),
            TerminalContextAction::SplitDown => state.split_focused(Dir::Horizontal),
            TerminalContextAction::ClosePane => state.close_focused(),
            TerminalContextAction::NewAnchor => state.create_new_anchor(),
            TerminalContextAction::NextPane => state.cycle_focus(true),
            TerminalContextAction::PreviousPane => state.cycle_focus(false),
            TerminalContextAction::ToggleRearrange => {
                state.toggle_rearrange_mode();
            }
        }
    })
}

fn install_application_menubar(app: &Application) {
    let root = gio::Menu::new();

    let window_menu = gio::Menu::new();
    window_menu.append(Some("Settings"), Some("app.settings"));
    window_menu.append(Some("Request Permissions"), Some("app.request-permissions"));
    window_menu.append(Some("Quit"), Some("app.quit"));
    root.append_submenu(Some("Window"), &window_menu);

    app.set_menubar(Some(&root));
}

fn install_window_menu_actions(
    app: &Application,
    window: &ApplicationWindow,
    state: &SharedAppState,
    shortcut_prefix: ShortcutPrefixCell,
) {
    let settings = gio::SimpleAction::new("settings", None);
    let window_for_settings = window.clone();
    let state_for_settings = state.clone();
    let prefix_for_settings = shortcut_prefix;
    settings.connect_activate(move |_, _| {
        sidebar::open_settings_dialog(
            &window_for_settings,
            &state_for_settings,
            prefix_for_settings.clone(),
        );
    });
    app.add_action(&settings);

    let request_permissions = gio::SimpleAction::new("request-permissions", None);
    request_permissions.connect_activate(move |_, _| {
        request_platform_permissions();
    });
    app.add_action(&request_permissions);

    let quit = gio::SimpleAction::new("quit", None);
    let app_for_quit = app.clone();
    let state_for_quit = state.clone();
    quit.connect_activate(move |_, _| run_shutdown(&app_for_quit, &state_for_quit));
    app.add_action(&quit);
}

#[cfg(target_os = "macos")]
fn request_platform_permissions() {
    let state = lmux_compositor::MacWindowCompositor::accessibility_permission_state(true);
    tracing::info!(?state, "macOS Accessibility permission requested");
}

#[cfg(not(target_os = "macos"))]
fn request_platform_permissions() {
    tracing::info!("no platform permissions are required for this build");
}

/// tmux-style prefix key handler. Ctrl+B arms the prefix; the next
/// keystroke is consumed as a command. Uses the Capture phase so we see the
/// prefix before the focused pane's key controller, preventing the shell
/// from receiving Ctrl+B. Rationale: Super-based shortcuts clashed too
/// heavily with KDE's global window-management bindings (Super+Q, Super+[],
/// Super+W), so v0.1 uses a prefix-namespaced keymap instead.
///
/// Commands after prefix:
///   `|` / `\`    — split vertical
///   `-`          — split horizontal
///   `x`          — close focused pane
///   `a`          — toggle anchor tag on focused pane
///   `q`          — coordinated shutdown
///   `o` / `]`    — cycle focus forward
///   `p` / `[`    — cycle focus backward
///   `s`          — session switcher
///   `l`          — GUI-program launcher
///
/// Any other non-modifier key disarms silently (tmux behavior). Modifier-only
/// presses (Shift/Ctrl/Alt/Super) keep the prefix armed so that e.g.
/// Shift+`\` → `|` still reaches the command dispatcher.
fn install_window_shortcuts(
    app: &Application,
    window: &ApplicationWindow,
    state: &SharedAppState,
    shortcut_prefix: ShortcutPrefixCell,
) {
    let armed: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let key = EventControllerKey::new();
    key.set_propagation_phase(PropagationPhase::Capture);
    let state_cb = state.clone();
    let app_cb = app.clone();
    let armed_cb = armed.clone();
    let window_cb = window.clone();
    key.connect_key_pressed(move |_ctrl, keyval, _code, modifier| {
        // When a Wayland satellite (browser/IDE) has focus, let every
        // keystroke pass through untouched — including Ctrl+B, which the
        // user's browser uses for the bookmarks bar and JetBrains uses
        // for "Go to Declaration". Otherwise the cockpit's tmux-style
        // prefix would swallow the chord and the follow-up key (e.g. `x`)
        // would run `close_focused`, tearing down the anchor the user was
        // actively working in. To invoke cockpit commands, the user
        // focuses a terminal pane first (click or sidebar).
        if state_cb.borrow().focused_is_satellite() {
            if armed_cb.get() {
                disarm_prefix(&armed_cb, &window_cb);
            }
            return glib::Propagation::Proceed;
        }
        if !armed_cb.get() {
            if is_configured_prefix(&shortcut_prefix.borrow(), keyval, modifier) {
                arm_prefix(&armed_cb, &window_cb);
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }

        // Armed: consume this keystroke as a command (or stay armed on a
        // modifier-only press). Always stop propagation so the shell never
        // receives a command key.
        use gdk::Key;
        let is_modifier_only = matches!(
            keyval,
            Key::Shift_L
                | Key::Shift_R
                | Key::Control_L
                | Key::Control_R
                | Key::Alt_L
                | Key::Alt_R
                | Key::Super_L
                | Key::Super_R
                | Key::Meta_L
                | Key::Meta_R
        );
        if is_modifier_only {
            return glib::Propagation::Stop;
        }
        disarm_prefix(&armed_cb, &window_cb);

        let as_char = keyval.to_unicode();
        match keyval {
            // `|` and `\` are US-layout ergonomics; `+` is the unshifted
            // symmetric sibling of `-` on Nordic layouts (Story 50 feedback).
            Key::bar | Key::backslash | Key::plus => {
                state_cb.borrow_mut().split_focused(Dir::Vertical);
            }
            Key::minus => {
                state_cb.borrow_mut().split_focused(Dir::Horizontal);
            }
            Key::bracketright => {
                state_cb.borrow_mut().cycle_focus(true);
            }
            Key::bracketleft => {
                state_cb.borrow_mut().cycle_focus(false);
            }
            _ => match as_char.map(|c| c.to_ascii_lowercase()) {
                Some('x') => {
                    state_cb.borrow_mut().close_focused();
                }
                Some('a') => {
                    state_cb.borrow_mut().cycle_active_anchor();
                }
                Some('s') => {
                    crate::switcher::open(&window_cb, &state_cb);
                }
                Some('l') => {
                    #[cfg(target_os = "macos")]
                    {
                        tracing::debug!(
                            "launcher is disabled on macOS; attach an already-open window instead"
                        );
                    }
                    #[cfg(not(target_os = "macos"))]
                    crate::launcher::open(&window_cb, &state_cb);
                }
                Some('m') => {
                    state_cb.borrow().toggle_rearrange_mode();
                }
                Some('q') => {
                    run_shutdown(&app_cb, &state_cb);
                }
                Some('o') | Some('n') => {
                    state_cb.borrow_mut().cycle_focus(true);
                }
                Some('p') => {
                    state_cb.borrow_mut().cycle_focus(false);
                }
                _ => {
                    tracing::debug!(?keyval, "prefix disarmed on unrecognized key");
                }
            },
        }
        glib::Propagation::Stop
    });
    window.add_controller(key);
}

fn arm_prefix(armed: &Rc<Cell<bool>>, window: &ApplicationWindow) {
    armed.set(true);
    window.set_title(Some("lmux [◆]"));
    let armed_to = armed.clone();
    let window_to = window.clone();
    glib::timeout_add_local_once(PREFIX_TIMEOUT, move || {
        if armed_to.get() {
            armed_to.set(false);
            window_to.set_title(Some("lmux"));
        }
    });
}

fn disarm_prefix(armed: &Rc<Cell<bool>>, window: &ApplicationWindow) {
    armed.set(false);
    window.set_title(Some("lmux"));
}

fn load_keymap_prefix() -> String {
    lmux_config::config_path()
        .and_then(|path| lmux_config::load(&path).ok())
        .map(|cfg| cfg.keymap.prefix)
        .filter(|prefix| !prefix.trim().is_empty())
        .unwrap_or_else(|| "ctrl+b".to_string())
}

fn is_configured_prefix(prefix: &str, keyval: gdk::Key, modifier: gdk::ModifierType) -> bool {
    let Some(binding) = parse_prefix_binding(prefix) else {
        return is_default_prefix(keyval, modifier);
    };
    if binding.ctrl != modifier.contains(gdk::ModifierType::CONTROL_MASK) {
        return false;
    }
    if binding.shift != modifier.contains(gdk::ModifierType::SHIFT_MASK) {
        return false;
    }
    if binding.alt != modifier.contains(gdk::ModifierType::ALT_MASK) {
        return false;
    }
    if binding.command
        && !modifier.intersects(gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK)
    {
        return false;
    }
    if !binding.command
        && modifier.intersects(gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK)
    {
        return false;
    }
    keyval
        .to_unicode()
        .map(|ch| ch.eq_ignore_ascii_case(&binding.key))
        .unwrap_or(false)
}

fn is_default_prefix(keyval: gdk::Key, modifier: gdk::ModifierType) -> bool {
    modifier.contains(gdk::ModifierType::CONTROL_MASK)
        && keyval
            .to_unicode()
            .map(|c| c.eq_ignore_ascii_case(&'b'))
            .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrefixBinding {
    ctrl: bool,
    shift: bool,
    alt: bool,
    command: bool,
    key: char,
}

pub(crate) fn is_valid_prefix_binding(prefix: &str) -> bool {
    parse_prefix_binding(prefix).is_some()
}

fn parse_prefix_binding(prefix: &str) -> Option<PrefixBinding> {
    let mut binding = PrefixBinding {
        ctrl: false,
        shift: false,
        alt: false,
        command: false,
        key: '\0',
    };
    for part in prefix
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
    {
        match part.as_str() {
            "ctrl" | "control" => binding.ctrl = true,
            "shift" => binding.shift = true,
            "alt" | "option" => binding.alt = true,
            "cmd" | "command" | "super" | "meta" => binding.command = true,
            key if key.chars().count() == 1 => binding.key = key.chars().next()?,
            _ => return None,
        }
    }
    (binding.key != '\0').then_some(binding)
}

/// Spawn the control-socket server and wire a UI-thread consumer to turn
/// `AppEvent`s into `AppState` mutations. The `ServerHandle` is stashed on
/// the window so the socket file is unlinked on drop.
/// Spawn the lmux-bus server on a background thread so external clients
/// (`lmux-cli`, KWin script, v0.3 plugins) can hit read-only endpoints
/// like `session.list` and `status.get`. Write endpoints still
/// surface `not_implemented` until Epic 3 step 2 wires cross-thread
/// dispatch into `AppState`.
/// Watch `~/.config/lmux/config.toml` and log each debounced reload.
/// Full propagation (rebuild sidebar, re-apply font, etc.) is a v0.3
/// follow-up — this version satisfies Epic 10's hot-reload surface by
/// proving the observer loop runs end-to-end.
fn install_config_watch(
    window: &ApplicationWindow,
    state: &SharedAppState,
    shortcut_prefix: ShortcutPrefixCell,
) {
    let Some(path) = lmux_config::config_path() else {
        tracing::warn!("lmux-config: watcher disabled (no config path)");
        return;
    };
    if !path.exists() {
        tracing::debug!(path = %path.display(), "lmux-config: skipping watcher (file missing)");
        return;
    }
    // Apply the current on-disk config once at startup so settings like
    // `focus_mode` that don't trigger re-attach take effect before the
    // first reload event.
    match lmux_config::load(&path) {
        Ok(cfg) => {
            *shortcut_prefix.borrow_mut() = cfg.keymap.prefix.clone();
            state.borrow().apply_config(&cfg);
        }
        Err(err) => tracing::warn!(error = %err, "lmux-config: initial load failed"),
    }
    // The watcher calls back on its own thread. Send freshly-parsed configs
    // across an async_channel and apply them on the GTK main loop so pane
    // mutations stay single-threaded.
    let (cfg_tx, cfg_rx) = async_channel::unbounded::<lmux_config::Config>();
    let handle = match lmux_config::watch::spawn(&path, move |res| match res {
        Ok(cfg) => {
            tracing::info!(
                font_size = cfg.general.font_size,
                autodetect_rules = cfg.autodetect.len(),
                "lmux-config: reloaded"
            );
            if cfg_tx.send_blocking(cfg).is_err() {
                tracing::debug!("lmux-config: GTK consumer dropped, ignoring reload");
            }
        }
        Err(err) => tracing::warn!(error = %err, "lmux-config: reload failed"),
    }) {
        Ok(h) => h,
        Err(err) => {
            tracing::warn!(error = %err, "lmux-config: watcher failed to start");
            return;
        }
    };
    // Park the handle on the window so it lives for the process lifetime.
    unsafe {
        window.set_data::<lmux_config::watch::WatchHandle>("lmux-config-watch", handle);
    }

    let state_apply = state.clone();
    let shortcut_prefix_apply = shortcut_prefix;
    glib::MainContext::default().spawn_local(async move {
        while let Ok(cfg) = cfg_rx.recv().await {
            *shortcut_prefix_apply.borrow_mut() = cfg.keymap.prefix.clone();
            state_apply.borrow().apply_config(&cfg);
        }
    });
}

fn install_bus_server(state: &SharedAppState) {
    let Some(state_home) = lmux_session::state_home() else {
        tracing::warn!("lmux-bus: disabled (XDG_STATE_HOME / HOME not set)");
        return;
    };
    // Channel from the tokio bus thread into the GTK main loop. Write
    // kinds post (req, oneshot) onto this; the dispatcher drains it and
    // applies mutations on the GTK thread, then replies via oneshot.
    let (write_tx, write_rx) = async_channel::unbounded::<crate::bus::DeferredRequest>();
    let anchor_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(
        state.borrow().anchor_count(),
    ));
    let compositor = build_compositor();
    let satellite_spawn_ok = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let satellite_spawn_fail = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let satellite_counters = crate::bus::SatelliteCounters {
        spawn_ok: satellite_spawn_ok.clone(),
        spawn_fail: satellite_spawn_fail.clone(),
    };
    // Bridge: AppState → compositor (for anchor-driven satellite hide/show).
    let bridge_tx = crate::compositor_bridge::spawn(compositor.clone());
    state.borrow_mut().set_compositor_tx(bridge_tx);
    let ctx = crate::bus::BusContext {
        store_root: state_home.clone(),
        cockpit_version: env!("CARGO_PKG_VERSION").to_string(),
        write_tx: Some(write_tx),
        anchor_count: anchor_count.clone(),
        compositor,
        satellite_spawn_ok,
        satellite_spawn_fail,
    };
    let _ = crate::bus::start(ctx);
    let state_for_dispatch = state.clone();
    glib::MainContext::default().spawn_local(crate::bus::run_dispatcher(
        write_rx,
        state_home,
        state_for_dispatch,
        satellite_counters,
    ));

    // Keep the atomic in sync with the real count. Can't read `AppState`
    // from the callback body (another listener may already hold the
    // borrow), so we fetch via a weak ref to the RefCell.
    let state_weak = Rc::downgrade(state);
    state
        .borrow_mut()
        .add_anchors_changed_callback(Rc::new(move || {
            if let Some(state) = state_weak.upgrade() {
                if let Ok(st) = state.try_borrow() {
                    anchor_count.store(st.anchor_count(), std::sync::atomic::Ordering::Relaxed);
                }
            }
        }));
}

/// Pick the compositor backend for this cockpit instance.
fn build_compositor() -> std::sync::Arc<dyn lmux_compositor::CompositorControl> {
    #[cfg(target_os = "macos")]
    {
        tracing::info!("compositor: macOS detected, using MacWindowCompositor");
        return std::sync::Arc::new(lmux_compositor::MacWindowCompositor::detect_or_prompt());
    }
    #[cfg(target_os = "linux")]
    {
        if !is_kde_session() {
            tracing::info!("compositor: no KDE session detected, using Noop");
            return std::sync::Arc::new(lmux_compositor::NoopCompositor::default());
        }
        match locate_kwin_script() {
            Some(path) => {
                tracing::info!(script = %path.display(), "compositor: KwinCompositor active");
                std::sync::Arc::new(lmux_compositor::KwinCompositor::new(
                    path.to_string_lossy().into_owned(),
                ))
            }
            None => {
                tracing::warn!("compositor: KDE detected but lmux-dock.js not found, using Noop");
                std::sync::Arc::new(lmux_compositor::NoopCompositor::default())
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        tracing::info!("compositor: unsupported target, using Noop");
        std::sync::Arc::new(lmux_compositor::NoopCompositor::default())
    }
}

#[cfg(target_os = "linux")]
fn is_kde_session() -> bool {
    if std::env::var_os("KDE_SESSION_VERSION").is_some() {
        return true;
    }
    std::env::var_os("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_string_lossy().to_ascii_lowercase().contains("kde"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn locate_kwin_script() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(override_path) = std::env::var_os("LMUX_KWIN_SCRIPT") {
        candidates.push(PathBuf::from(override_path));
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        candidates.push(PathBuf::from(data_home).join("lmux/kwin/lmux-dock.js"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/share/lmux/kwin/lmux-dock.js"));
    }
    candidates.push(PathBuf::from("/usr/share/lmux/kwin/lmux-dock.js"));
    candidates.push(PathBuf::from("/usr/local/share/lmux/kwin/lmux-dock.js"));
    // Dev layout: project-root/share/lmux/kwin/lmux-dock.js, discovered
    // relative to CARGO_MANIFEST_DIR of the `lmux` crate.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    candidates.push(PathBuf::from(manifest_dir).join("../../share/lmux/kwin/lmux-dock.js"));
    candidates.into_iter().find(|p| p.exists())
}

fn install_control_socket(window: &ApplicationWindow, state: &SharedAppState) {
    let (event_tx, event_rx) = async_channel::unbounded::<AppEvent>();
    let handle = match lmux_control::spawn_server(move |ev| {
        let _ = event_tx.send_blocking(ev);
    }) {
        Ok(h) => h,
        Err(err) => {
            tracing::warn!(error = %err, "control socket server failed to start");
            return;
        }
    };
    unsafe {
        window.set_data::<lmux_control::ServerHandle>("lmux-control-server", handle);
    }

    let state = state.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(ev) = event_rx.recv().await {
            match ev {
                AppEvent::MarkAnchor { source_pid, reply } => {
                    let resolved = {
                        let mut s = state.borrow_mut();
                        match s.resolve_owning_pane(source_pid) {
                            Some(id) => {
                                s.add_anchor(id);
                                Some(id)
                            }
                            None => None,
                        }
                    };
                    let resp = match resolved {
                        Some(id) => CtrlResponse::Ok {
                            v: PROTOCOL_VERSION,
                            pane_id: Some(id),
                        },
                        None => CtrlResponse::Error {
                            v: PROTOCOL_VERSION,
                            message: format!("no pane owns pid {source_pid}"),
                        },
                    };
                    let _ = reply.send(resp).await;
                }
            }
        }
    });
}

/// Spawn the notifier + wire bell events. A bell on pane P triggers a
/// freedesktop notification via zbus (Epic 6 Story 6.2). Clicking the
/// notification raises the window (via ActionInvoked → UI channel).
fn install_notifier(window: &ApplicationWindow, state: &SharedAppState) {
    let (click_tx, click_rx) = async_channel::unbounded::<()>();
    let on_click: lmux_notify::ClickCallback = Arc::new(move || {
        let _ = click_tx.send_blocking(());
    });
    let notifier = Notifier::spawn(on_click);

    // UI-thread consumer: raise the window whenever the notifier reports a
    // click. `present` covers both map-if-hidden and raise-to-top.
    let window_click = window.clone();
    glib::MainContext::default().spawn_local(async move {
        while click_rx.recv().await.is_ok() {
            window_click.present();
        }
    });

    // Bell channel: pane workers push pane ids; the UI consumer here builds
    // the toast body and fires the notifier. Done async so GTK main stays
    // responsive while zbus round-trips.
    let (bell_tx, bell_rx) = async_channel::unbounded::<PaneId>();
    let bell_cb: Rc<dyn Fn(PaneId)> = Rc::new(move |id| {
        let _ = bell_tx.send_blocking(id);
    });
    state.borrow_mut().set_bell_callback(bell_cb);

    let state_bell = state.clone();
    let notifier = Rc::new(notifier);

    // First-run onboarding: provision the default config + raise a
    // one-shot desktop notification pointing at the path. Marker file
    // under `$XDG_STATE_HOME/lmux/` keeps the toast from re-firing on
    // subsequent launches. NFR33 / Epic 11 Story 5.
    if let Some(toast) = onboarding_payload() {
        let notifier_onboard = notifier.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Err(err) = notifier_onboard.notify(0, toast.title, toast.body).await {
                tracing::warn!(error = %err, "onboarding notification failed");
            }
        });
    }

    glib::MainContext::default().spawn_local(async move {
        while let Ok(id) = bell_rx.recv().await {
            let (label, replaces) = {
                let s = state_bell.borrow();
                (s.pane_label(id), s.replaces_id_for(id))
            };
            let title = "lmux".to_string();
            let body = label;
            let span = tracing::info_span!("bell_to_toast", pane_id = id);
            let _g = span.enter();
            match notifier.notify(replaces, title, body).await {
                Ok(new_id) => {
                    state_bell.borrow_mut().record_notif_id(id, new_id);
                }
                Err(err) => {
                    tracing::warn!(error = %err, "notification delivery failed");
                }
            }
        }
    });
}

struct OnboardingToast {
    title: String,
    body: String,
}

/// Provision the default config on first run and decide whether to
/// raise the one-shot welcome notification. Returns `Some` on first
/// launch (toast should fire) and `None` otherwise. Writes both the
/// default config (`load_or_provision`) and the onboarding marker file
/// so a subsequent launch is quiet.
fn onboarding_payload() -> Option<OnboardingToast> {
    let state_dir = lmux_session::state_home()?;
    let marker = state_dir.join("onboarded.v0.2");
    if marker.exists() {
        return None;
    }
    let config_path = lmux_config::config_path()?;
    let outcome = match lmux_config::load_or_provision(&config_path) {
        Ok((_, outcome)) => outcome,
        Err(err) => {
            tracing::warn!(error = %err, "onboarding: load_or_provision failed");
            return None;
        }
    };
    if let Err(err) = std::fs::create_dir_all(&state_dir) {
        tracing::warn!(error = %err, dir = %state_dir.display(), "onboarding: create state dir failed");
    }
    if let Err(err) = std::fs::write(&marker, b"v0.2") {
        tracing::warn!(error = %err, path = %marker.display(), "onboarding: write marker failed");
        // Fall through anyway — better to show the toast twice than to
        // silently drop the first-run signal.
    }
    let body = match outcome {
        lmux_config::ProvisionOutcome::Provisioned => {
            format!(
                "Default config written to {}. Edit it anytime; lmux watches for changes.",
                config_path.display()
            )
        }
        lmux_config::ProvisionOutcome::Loaded => {
            format!("Config lives at {}. Welcome!", config_path.display())
        }
    };
    Some(OnboardingToast {
        title: "lmux".into(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_configured_prefix() {
        assert_eq!(
            parse_prefix_binding("ctrl+b"),
            Some(PrefixBinding {
                ctrl: true,
                shift: false,
                alt: false,
                command: false,
                key: 'b',
            })
        );
        assert_eq!(
            parse_prefix_binding("cmd+shift+k"),
            Some(PrefixBinding {
                ctrl: false,
                shift: true,
                alt: false,
                command: true,
                key: 'k',
            })
        );
    }

    #[test]
    fn rejects_invalid_prefix() {
        assert_eq!(parse_prefix_binding("ctrl+"), None);
        assert_eq!(parse_prefix_binding("ctrl+space"), None);
    }
}

/// Intercept the window's close button → run the same shutdown flow as
/// Super+Q so PTY children always get a cooperative SIGTERM before the
/// process exits. Returning `Propagation::Stop` keeps the window open until
/// `app.quit()` fires from the grace timer.
fn install_close_request(app: &Application, window: &ApplicationWindow, state: &SharedAppState) {
    let app = app.clone();
    let state = state.clone();
    window.connect_close_request(move |_| {
        run_shutdown(&app, &state);
        glib::Propagation::Stop
    });
}

/// Coordinated shutdown (Story 7.1 + 7.2). Idempotent — a second invocation
/// while shutdown is already in flight is a no-op. On the first call:
///   1. Transition state to `ShuttingDown` and drain every pane (SIGTERM is
///      sent inside `drain_panes_for_shutdown`).
///   2. After the grace window, SIGKILL anything still alive, drop the Pane
///      handles (which releases PTY masters and reaps children on drop),
///      and call `app.quit()`.
fn run_shutdown(app: &Application, state: &SharedAppState) {
    // Snapshot BEFORE begin_shutdown — the drain step SIGTERMs every child,
    // so `/proc/<pid>/cwd` may disappear mid-snapshot if we did it after.
    // Guard against double-snapshot by peeking the phase first via a
    // borrow: begin_shutdown is idempotent and returns None on the second
    // call, at which point we also skip the save.
    save_snapshot(state);

    let drained = match state.borrow_mut().begin_shutdown() {
        Some(d) => d,
        None => {
            tracing::debug!("shutdown already in progress — ignoring");
            return;
        }
    };
    tracing::info!(
        panes = drained.len(),
        "shutdown: SIGTERM sent; 500 ms grace"
    );
    let panes = Rc::new(RefCell::new(Some(drained)));
    let panes_clone = panes.clone();
    let app = app.clone();
    glib::timeout_add_local_once(SHUTDOWN_GRACE, move || {
        if let Some(panes) = panes_clone.borrow_mut().take() {
            let mut survivors = 0u32;
            for pane in &panes {
                if !pane.has_exited() {
                    pane.kill();
                    survivors += 1;
                }
            }
            drop(panes);
            tracing::info!(
                sigkilled = survivors,
                "shutdown: draining complete; quitting"
            );
        }
        app.quit();
    });
}

/// Result of a successful restore pass — everything `activate()` needs to
/// construct a pre-populated `AppState` without first creating a default pane.
struct Restored {
    panes: HashMap<PaneId, Pane>,
    layout: Layout,
    focused: PaneId,
    anchors: std::collections::BTreeSet<PaneId>,
    next_id: PaneId,
    first_cell_size: Option<(i32, i32)>,
}

fn load_snapshot() -> Option<lmux_state::SessionSnapshot> {
    let Some(path) = lmux_state::session_path() else {
        tracing::warn!("no XDG_DATA_HOME / HOME — skipping session restore");
        return None;
    };
    match lmux_state::load(&path) {
        lmux_state::LoadOutcome::Ok(s) => Some(s),
        lmux_state::LoadOutcome::Missing => {
            tracing::info!("no prior session found; starting fresh");
            None
        }
        lmux_state::LoadOutcome::Corrupt { error, renamed_to } => {
            tracing::warn!(
                error,
                renamed_to = ?renamed_to,
                "prior session file corrupt; starting fresh"
            );
            None
        }
    }
}

/// Walk the snapshot layout and spawn one `Pane` per leaf. If a recorded
/// CWD doesn't exist any more (FR29 / NFR9 edge case), fall back to `$HOME`
/// for that pane only and log a warning. Returns `None` if we couldn't
/// materialise *any* pane — callers then fall through to a fresh session.
fn build_restored(snap: &lmux_state::SessionSnapshot) -> Option<Restored> {
    let layout = state::layout_from_snapshot(&snap.layout);
    let leaves = layout.leaves();
    if leaves.is_empty() {
        return None;
    }
    let mut panes = HashMap::with_capacity(leaves.len());
    let mut first_cell_size = None;
    for id in &leaves {
        let cwd_opt = snap.cwds.get(id).map(PathBuf::from);
        // Leaves without a recorded cwd are satellite panes from a previous
        // run (or otherwise unrecoverable). Skip them outright — spawning a
        // fallback terminal here would silently replace the satellite with
        // an empty shell pane ("white box") that clutters the layout.
        if cwd_opt.is_none() {
            tracing::info!(
                pane_id = id,
                "restore: no recorded cwd; dropping ghost leaf"
            );
            continue;
        }
        let cwd = resolve_restore_cwd(*id, cwd_opt.as_deref());
        let Some(pane) = Pane::new(*id, cwd.as_deref()) else {
            tracing::warn!(pane_id = id, "restore: pane spawn failed; skipping");
            continue;
        };
        if first_cell_size.is_none() {
            first_cell_size = Some(pane.cell_size());
        }
        panes.insert(*id, pane);
    }
    if panes.is_empty() {
        return None;
    }
    // If some leaves failed to spawn, prune them from the layout so the
    // tree stays consistent. `remove_leaf` on the root is a no-op, but that
    // case only fires when every leaf spawned successfully.
    let mut layout = layout;
    for id in &leaves {
        if !panes.contains_key(id) {
            layout.remove_leaf(*id);
        }
    }
    // Filter recorded anchors to those whose panes actually survived the
    // restore. Uses `anchors()` so v=1 legacy snapshots fall back to the
    // singleton automatically.
    let anchors: std::collections::BTreeSet<PaneId> = snap
        .anchors()
        .into_iter()
        .filter(|id| panes.contains_key(id))
        .collect();
    // Choose the first surviving leaf as focused. Prefer an anchored pane
    // when one exists so the user's "primary" pane gets focus on restore.
    let focused = anchors
        .iter()
        .copied()
        .next()
        .or_else(|| layout.leaves().first().copied())?;
    let next_id = layout.leaves().iter().copied().max().unwrap_or(0) + 1;
    Some(Restored {
        panes,
        layout,
        focused,
        anchors,
        next_id,
        first_cell_size,
    })
}

fn resolve_restore_cwd(pane_id: PaneId, recorded: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = recorded {
        if p.is_dir() {
            return Some(p.to_path_buf());
        }
        tracing::warn!(
            pane_id,
            recorded = %p.display(),
            "pane: recorded cwd missing; falling back to $HOME"
        );
    }
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// If the restore path couldn't measure a cell (no leaves survived and we
/// already fell through to the fresh-start branch) we need *some* sane
/// cell size for the initial window geometry. 8×16 is a safe floor — the
/// real values are computed once a pane's pango layout runs.
fn first_cell_size_fallback() -> (i32, i32) {
    (8, 16)
}

/// Persist the current session to `$XDG_DATA_HOME/lmux/last-session.json`.
/// Called from `run_shutdown()` before any SIGTERM so `/proc/<pid>/cwd` is
/// still readable (Story 8.2). Errors are logged and swallowed — shutdown
/// must never block on disk.
fn save_snapshot(state: &SharedAppState) {
    let Some(path) = lmux_state::session_path() else {
        tracing::warn!("no XDG_DATA_HOME / HOME — skipping session save");
        return;
    };
    let snap = state.borrow().snapshot();
    match lmux_state::save(&path, &snap) {
        Ok(()) => tracing::info!(path = %path.display(), "session snapshot written"),
        Err(err) => tracing::error!(error = %err, path = %path.display(), "session save failed"),
    }
}

/// Start the nested Wayland compositor (ADR-0018) and install a
/// host-event dispatcher that runs on the GTK main loop. All satellite
/// lifecycle — create, frame, title, close — flows through this.
///
/// If the compositor fails to start (e.g. no `XDG_RUNTIME_DIR` in CI,
/// or the socket name is already taken), we log + continue: the cockpit
/// still runs as a pure-terminal multiplexer, just without GUI
/// satellites.
#[cfg(target_os = "linux")]
fn install_wayland_host(state: &SharedAppState) {
    let (handle, cmd_tx, evt_rx) = match lmux_wayland_host::start() {
        Ok(triple) => triple,
        Err(err) => {
            tracing::warn!(error = %err, "wayland host start failed; satellites disabled");
            return;
        }
    };

    state.borrow_mut().install_wayland_host(handle, cmd_tx);

    // Drain the event channel on the GTK main thread. Holding a weak
    // reference to AppState so the dispatcher goes away cleanly when the
    // last strong Rc is dropped (cockpit shutdown).
    let weak = Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move {
        while let Ok(event) = evt_rx.recv().await {
            let Some(st) = weak.upgrade() else {
                break;
            };
            st.borrow_mut().handle_host_event(event);
        }
        tracing::info!("wayland host event dispatcher exiting");
    });
}

fn install_css() {
    let Some(display) = gdk::Display::default() else {
        tracing::warn!("no default GDK display; skipping CSS install");
        return;
    };
    let provider = CssProvider::new();
    provider.load_from_string(APP_CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
