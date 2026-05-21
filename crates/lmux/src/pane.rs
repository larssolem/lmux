use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::Path;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DragSource, DrawingArea, DropTarget, EventControllerMotion,
    EventControllerScroll, EventControllerScrollFlags, Frame, GestureClick, GestureDrag, Label,
    Orientation, Popover,
};
use lmux_config::FocusMode;
use lmux_libghostty::{
    CellView, CursorPos, Frame as LgFrame, RenderVisitor, Terminal, ViewportPoint,
};
use lmux_pty::{self, Pane as PtyPane};

use crate::keymap::{self, KeyAction, TerminalShortcut};
use crate::layout::PaneId;
use crate::render::{CairoRenderer, Selection};

const WHEEL_ROWS_PER_TICK: i32 = 3;

const INIT_COLS: u16 = 100;
const INIT_ROWS: u16 = 30;
const SCROLLBACK: usize = 10_000;
const READER_CHAN_CAPACITY: usize = 64;
#[cfg(target_os = "macos")]
const DEFAULT_FONT_FAMILY: &str = "JetBrains Mono";
#[cfg(target_os = "linux")]
const DEFAULT_FONT_FAMILY: &str = "monospace";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const DEFAULT_FONT_FAMILY: &str = "monospace";
#[cfg(target_os = "macos")]
const DEFAULT_FONT_SIZE_PT: i32 = 13;
#[cfg(target_os = "linux")]
const DEFAULT_FONT_SIZE_PT: i32 = 13;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const DEFAULT_FONT_SIZE_PT: i32 = 13;

// Minimum pane geometry — keeps `gtk::Paned` from shrinking a pane so far
// that it becomes unusable or hidden (FR17, Story 3.4).
const MIN_COLS: i32 = 20;
const MIN_ROWS: i32 = 3;

pub type FocusCallback = Rc<dyn Fn(PaneId)>;
pub type BellCallback = Rc<dyn Fn(PaneId)>;
pub type TerminalActionCallback = Rc<dyn Fn(PaneId, TerminalContextAction)>;
pub type ShortcutPrefixCell = Rc<RefCell<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalContextAction {
    SplitRight,
    SplitDown,
    ClosePane,
    NewAnchor,
    NextPane,
    PreviousPane,
    ToggleRearrange,
}

/// Shared focus-mode cell. Mutated when the cockpit reloads config; every
/// pane's hover handler checks this on each pointer enter to decide whether
/// to grab focus. Cheap to clone — it's an `Rc<Cell<_>>`.
pub type FocusModeCell = Rc<Cell<FocusMode>>;

/// Shared rearrange-mode flag. Toggled by `Ctrl+B m` / sidebar button;
/// every pane's `DragSource` and `DropTarget` consult it to know whether
/// to actually start a drag / accept a drop. Cheap to clone — it's an
/// `Rc<Cell<bool>>`.
pub type RearrangeModeCell = Rc<Cell<bool>>;

/// Callback invoked when the user drops `source` onto `target` in
/// rearrange mode. The cockpit re-parents the pane in the layout tree
/// and rebuilds the widget hierarchy.
pub type ReparentCallback = Rc<dyn Fn(PaneId, PaneId, crate::layout::Edge)>;

/// Debounce window between consecutive bell events from the same pane
/// (Story 6.1). A shell that emits a burst like `printf '\a\a\a\a'` produces
/// at most one `AppEvent::BellReceived` per window.
const BELL_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Stateful BEL scanner. Tracks whether we're inside an OSC sequence so the
/// ST form `ESC \` or the legacy BEL terminator don't register as user-
/// visible bells. Without this, emitting OSC 0 (set window title) via
/// common shell prompts would spam notifications.
#[derive(Default)]
struct BellScanner {
    in_osc: bool,
    prev_esc: bool,
}

impl BellScanner {
    fn scan(&mut self, bytes: &[u8]) -> u32 {
        let mut count = 0;
        for &b in bytes {
            if self.in_osc {
                match b {
                    0x07 => {
                        self.in_osc = false;
                        self.prev_esc = false;
                    }
                    0x1b => self.prev_esc = true,
                    b'\\' if self.prev_esc => {
                        self.in_osc = false;
                        self.prev_esc = false;
                    }
                    _ => self.prev_esc = false,
                }
            } else if self.prev_esc {
                if b == b']' {
                    self.in_osc = true;
                }
                self.prev_esc = false;
            } else {
                match b {
                    0x1b => self.prev_esc = true,
                    0x07 => count += 1,
                    _ => {}
                }
            }
        }
        count
    }
}

pub struct TerminalPane {
    id: PaneId,
    frame: Frame,
    drawing_area: DrawingArea,
    inner: Rc<RefCell<Inner>>,
    bell_cb: Rc<RefCell<Option<BellCallback>>>,
    /// CWD passed at spawn time. Persisted as the snapshot's per-pane CWD
    /// fallback when `/proc/<pid>/cwd` can't be read at shutdown (Story 8.2).
    spawn_cwd: Option<std::path::PathBuf>,
}

struct Inner {
    term: Terminal,
    pty: PtyPane,
    cell_w: f64,
    cell_h: f64,
    cols: u16,
    rows: u16,
    font_desc: pango::FontDescription,
    selection: Option<(ViewportPoint, ViewportPoint)>,
    drag_anchor: Option<ViewportPoint>,
    exit_code: Option<i32>,
}

impl TerminalPane {
    /// Allocate a pane with a fresh PTY + terminal. Pass `cwd` to inherit the
    /// parent pane's working directory (Story 3.1). Returns `None` if the
    /// terminal or PTY allocation fails.
    pub fn new(id: PaneId, cwd: Option<&Path>) -> Option<Self> {
        let term = Terminal::new(INIT_COLS, INIT_ROWS, SCROLLBACK)?;

        let shell = lmux_pty::detect_shell();
        let (pty, reader) = match lmux_pty::spawn(lmux_pty::SpawnOpts {
            shell,
            cols: INIT_COLS,
            rows: INIT_ROWS,
            cwd,
        }) {
            Ok(pair) => pair,
            Err(err) => {
                tracing::error!(error = %err, "pty spawn failed");
                return None;
            }
        };

        let font_desc = font_description(DEFAULT_FONT_FAMILY, DEFAULT_FONT_SIZE_PT);
        let drawing_area = DrawingArea::builder().hexpand(true).vexpand(true).build();
        drawing_area.set_focusable(true);

        let (cell_w, cell_h) = measure_cell(&drawing_area, &font_desc);

        let frame = Frame::builder().hexpand(true).vexpand(true).build();
        frame.set_child(Some(&drawing_area));
        frame.add_css_class("pane");
        // Clamp minimum geometry so divider drags (Story 3.4) can't shrink a
        // pane below a usable size.
        frame.set_size_request(
            MIN_COLS * (cell_w as i32).max(1),
            MIN_ROWS * (cell_h as i32).max(1),
        );

        let inner = Rc::new(RefCell::new(Inner {
            term,
            pty,
            cell_w,
            cell_h,
            cols: INIT_COLS,
            rows: INIT_ROWS,
            font_desc: font_desc.clone(),
            selection: None,
            drag_anchor: None,
            exit_code: None,
        }));

        let bell_cb: Rc<RefCell<Option<BellCallback>>> = Rc::new(RefCell::new(None));

        install_draw_func(&drawing_area, &inner);
        install_resize_handler(&drawing_area, &inner);
        start_pty_reader(&drawing_area, &inner, reader, id, bell_cb.clone());

        Some(Self {
            id,
            frame,
            drawing_area,
            inner,
            bell_cb,
            spawn_cwd: cwd.map(std::path::Path::to_path_buf),
        })
    }

    /// Best-effort CWD for snapshot capture: prefer the live `/proc` read;
    /// fall back to the CWD we spawned with when `/proc` is unavailable
    /// (e.g., after the child has exited).
    pub fn snapshot_cwd(&self) -> Option<std::path::PathBuf> {
        if let Some(live) = self.cwd() {
            return Some(live);
        }
        self.spawn_cwd.clone()
    }

    pub fn set_bell_callback(&self, cb: BellCallback) {
        *self.bell_cb.borrow_mut() = Some(cb);
    }

    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn cell_size(&self) -> (i32, i32) {
        let b = self.inner.borrow();
        (b.cell_w as i32, b.cell_h as i32)
    }

    /// Swap the render font to `family` at `size_pt`. Re-measures the cell
    /// size so future resizes use the new metrics, updates the frame's
    /// minimum size so the divider limits track the new cell size, and
    /// queues a redraw. Called from the config hot-reload path (Epic 10).
    pub fn set_font(&self, family: &str, size_pt: i32) {
        let font = font_description(family, size_pt);
        let (cell_w, cell_h) = measure_cell(&self.drawing_area, &font);
        // Recompute the grid for the current allocation: changing the cell
        // size without resizing the term + PTY leaves them at the old cols
        // and rows, so the renderer draws cells at new pixel positions
        // while the cell stream still uses the old grid (visible as
        // "extra whitespace" or wrap mismatches).
        let w = self.drawing_area.width().max(0) as f64;
        let h = self.drawing_area.height().max(0) as f64;
        let cols = ((w / cell_w).floor() as u16).max(1);
        let rows = ((h / cell_h).floor() as u16).max(1);
        {
            let mut b = self.inner.borrow_mut();
            b.font_desc = font;
            b.cell_w = cell_w;
            b.cell_h = cell_h;
            if w >= cell_w && h >= cell_h {
                b.cols = cols;
                b.rows = rows;
                let cw_px = cell_w as u32;
                let ch_px = cell_h as u32;
                b.term.resize(cols, rows, cw_px, ch_px);
                if let Err(err) = b.pty.resize(cols, rows, cw_px as u16, ch_px as u16) {
                    tracing::warn!(error = %err, "pty resize on font change failed");
                }
            }
        }
        self.frame.set_size_request(
            MIN_COLS * (cell_w as i32).max(1),
            MIN_ROWS * (cell_h as i32).max(1),
        );
        self.drawing_area.queue_draw();
    }

    /// Low-resolution RGB thumbnail of the current viewport, one pixel per
    /// terminal cell. Empty cells become the frame's default background;
    /// cells with any text become the cell's foreground. Returns
    /// `(cols, rows, rgb_bytes)` where `rgb_bytes.len() == cols * rows * 3`,
    /// or `None` if the cell grid is degenerate.
    ///
    /// Used by the sidebar mini-preview (Epic 5). Cheap enough to call on
    /// a 200 ms timeout: the render pipeline is already allocated.
    pub fn snapshot_thumbnail(&self) -> Option<(u32, u32, Vec<u8>)> {
        let mut inner = self.inner.borrow_mut();
        let cols = inner.cols;
        let rows = inner.rows;
        if cols == 0 || rows == 0 {
            return None;
        }
        let mut v = ThumbnailVisitor::new(cols, rows);
        inner.term.render(&mut v);
        Some((cols as u32, rows as u32, v.into_rgb()))
    }

    /// Return the widget that should be inserted into the GTK tree. We wrap
    /// the drawing area in a Frame so anchor borders (Epic 4) can style it
    /// without re-parenting the drawing surface.
    pub fn widget(&self) -> &Frame {
        &self.frame
    }

    pub fn grab_focus(&self) {
        self.drawing_area.grab_focus();
    }

    /// Returns the child's current CWD. Used when splitting so the new pane
    /// starts in the same directory as the one that spawned it.
    pub fn cwd(&self) -> Option<std::path::PathBuf> {
        self.inner.borrow().pty.cwd()
    }

    /// PID of the pane's PTY leader (usually the shell). Used by the control
    /// socket (Epic 5) to resolve `lmux-cli` invocations back to a pane.
    pub fn child_pid(&self) -> Option<u32> {
        self.inner.borrow().pty.child_pid()
    }

    pub fn has_exited(&self) -> bool {
        self.inner.borrow().exit_code.is_some()
    }

    /// Send SIGTERM to the child. Callers that want the full
    /// SIGTERM → 500 ms → SIGKILL cadence chain this with a timeout + `kill`.
    pub fn terminate(&self) {
        if let Err(err) = self.inner.borrow().pty.terminate() {
            tracing::warn!(error = %err, "pty SIGTERM failed");
        }
    }

    pub fn kill(&self) {
        let mut b = self.inner.borrow_mut();
        if let Err(err) = b.pty.kill() {
            tracing::warn!(error = %err, "pty SIGKILL failed");
        }
    }

    /// Attach input controllers. The focus callback fires on click, plus on
    /// pointer-enter when `focus_mode` is `Hover`. The mode cell is shared
    /// with the cockpit so a config reload mutates it without re-attaching.
    pub fn attach_controllers(
        &self,
        on_focus: FocusCallback,
        focus_mode: FocusModeCell,
        on_action: TerminalActionCallback,
        shortcut_prefix: ShortcutPrefixCell,
    ) {
        self.attach_key_controller();
        self.attach_scroll_controller();
        self.attach_drag_controller();
        self.attach_focus_click(on_focus.clone());
        self.attach_context_menu(on_focus.clone(), on_action, shortcut_prefix);
        self.attach_focus_hover(on_focus, focus_mode);
    }

    fn attach_key_controller(&self) {
        let key = gtk4::EventControllerKey::new();
        let inner = self.inner.clone();
        let area = self.drawing_area.clone();
        key.connect_key_pressed(move |_ctrl, keyval, _code, modifier| {
            if inner.borrow().exit_code.is_some() {
                return glib::Propagation::Stop;
            }
            if let Some(shortcut) = keymap::classify_terminal_shortcut(keyval, modifier) {
                match shortcut {
                    TerminalShortcut::Copy => copy_selection_to_clipboard(&inner),
                    TerminalShortcut::Paste => request_paste_from_clipboard(&area, &inner),
                }
                return glib::Propagation::Stop;
            }
            let page_rows = inner.borrow().rows;
            match keymap::classify_key(keyval, modifier, page_rows) {
                KeyAction::Write(bytes) if !bytes.is_empty() => {
                    let span = tracing::info_span!("input_to_paint");
                    let _g = span.enter();
                    let mut b = inner.borrow_mut();
                    if let Err(err) = b.pty.writer().write_all(&bytes) {
                        tracing::warn!(error = %err, "pty write failed");
                    }
                    b.term.scroll_to_bottom();
                    b.selection = None;
                    drop(b);
                    area.queue_draw();
                }
                KeyAction::ScrollRows(delta) => {
                    inner.borrow_mut().term.scroll_delta(delta as isize);
                    area.queue_draw();
                }
                KeyAction::Write(_) => {}
            }
            glib::Propagation::Stop
        });
        self.drawing_area.add_controller(key);
    }

    fn attach_scroll_controller(&self) {
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        let inner = self.inner.clone();
        let area = self.drawing_area.clone();
        scroll.connect_scroll(move |_ctrl, _dx, dy| {
            let rows = (dy * f64::from(WHEEL_ROWS_PER_TICK)).round() as i32;
            if rows != 0 {
                inner.borrow_mut().term.scroll_delta(rows as isize);
                area.queue_draw();
            }
            glib::Propagation::Stop
        });
        self.drawing_area.add_controller(scroll);
    }

    fn attach_drag_controller(&self) {
        let drag = GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let inner = self.inner.clone();
        let area = self.drawing_area.clone();
        drag.connect_drag_begin(move |_g, x, y| {
            let mut b = inner.borrow_mut();
            let anchor = b.point_from_px(x, y);
            b.drag_anchor = Some(anchor);
            b.selection = Some((anchor, anchor));
            drop(b);
            area.queue_draw();
        });
        let inner_up = self.inner.clone();
        let area_up = self.drawing_area.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some((sx, sy)) = g.start_point() else {
                return;
            };
            let mut b = inner_up.borrow_mut();
            let Some(anchor) = b.drag_anchor else {
                return;
            };
            let end = b.point_from_px(sx + dx, sy + dy);
            b.selection = Some((anchor, end));
            drop(b);
            area_up.queue_draw();
        });
        let inner_end = self.inner.clone();
        let area_end = self.drawing_area.clone();
        drag.connect_drag_end(move |_g, _dx, _dy| {
            let selection = {
                let mut b = inner_end.borrow_mut();
                b.drag_anchor = None;
                b.selection
            };
            if let Some((start, end)) = selection {
                if start == end {
                    inner_end.borrow_mut().selection = None;
                    area_end.queue_draw();
                    return;
                }
                let text = inner_end.borrow().term.selection_text(start, end);
                if let Some(text) = text.filter(|t| !t.is_empty()) {
                    if let Some(display) = gdk::Display::default() {
                        display.clipboard().set_text(&text);
                        display.primary_clipboard().set_text(&text);
                    }
                }
            }
            area_end.queue_draw();
        });
        self.drawing_area.add_controller(drag);
    }

    fn attach_focus_click(&self, on_focus: FocusCallback) {
        let click = GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let id = self.id;
        let area = self.drawing_area.clone();
        click.connect_pressed(move |_g, _n, _x, _y| {
            area.grab_focus();
            on_focus(id);
        });
        self.drawing_area.add_controller(click);
    }

    fn attach_context_menu(
        &self,
        on_focus: FocusCallback,
        on_action: TerminalActionCallback,
        shortcut_prefix: ShortcutPrefixCell,
    ) {
        let click = GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let id = self.id;
        let area = self.drawing_area.clone();
        let inner = self.inner.clone();
        click.connect_pressed(move |_g, _n, x, y| {
            area.grab_focus();
            on_focus(id);
            open_terminal_context_menu(
                &area,
                &inner,
                id,
                x,
                y,
                on_action.clone(),
                shortcut_prefix.borrow().clone(),
            );
        });
        self.drawing_area.add_controller(click);
    }

    fn attach_focus_hover(&self, on_focus: FocusCallback, focus_mode: FocusModeCell) {
        let motion = EventControllerMotion::new();
        let id = self.id;
        let area = self.drawing_area.clone();
        motion.connect_enter(move |_c, _x, _y| {
            if focus_mode.get() == FocusMode::Hover {
                area.grab_focus();
                on_focus(id);
            }
        });
        self.drawing_area.add_controller(motion);
    }
}

fn font_description(family: &str, size_pt: i32) -> pango::FontDescription {
    let mut font = pango::FontDescription::new();
    font.set_family(family);
    font.set_size(size_pt.max(1) * pango::SCALE);
    font
}

/// Shared helper used by both `TerminalPane` and `SatelliteWidget` to
/// install the rearrange-mode DnD controllers on a pane's outer Frame.
pub(crate) fn attach_rearrange_to_frame(
    frame: &Frame,
    pane_id: PaneId,
    mode: RearrangeModeCell,
    on_reparent: ReparentCallback,
) {
    let drag = DragSource::new();
    drag.set_actions(gtk4::gdk::DragAction::MOVE);
    let drag_mode = mode.clone();
    drag.connect_prepare(move |_src, _x, _y| {
        if !drag_mode.get() {
            return None;
        }
        Some(gtk4::gdk::ContentProvider::for_value(&pane_id.to_value()))
    });
    frame.add_controller(drag);

    let drop = DropTarget::new(u32::static_type(), gtk4::gdk::DragAction::MOVE);
    let drop_mode = mode;
    let drop_frame = frame.clone();
    drop.connect_drop(move |_target, value, x, y| {
        if !drop_mode.get() {
            return false;
        }
        let Ok(src) = value.get::<u32>() else {
            return false;
        };
        if src == pane_id {
            return false;
        }
        let w = f64::from(drop_frame.width().max(1));
        let h = f64::from(drop_frame.height().max(1));
        let edge = crate::layout::Edge::from_xy(x, y, w, h);
        on_reparent(src, pane_id, edge);
        true
    });
    frame.add_controller(drop);
}

impl Inner {
    fn point_from_px(&self, x: f64, y: f64) -> ViewportPoint {
        let col = (x / self.cell_w).floor().max(0.0) as u32;
        let row = (y / self.cell_h).floor().max(0.0) as u32;
        let col = (col as u16).min(self.cols.saturating_sub(1));
        let row = (row as u16).min(self.rows.saturating_sub(1));
        ViewportPoint { row, col }
    }
}

fn copy_selection_to_clipboard(inner: &Rc<RefCell<Inner>>) {
    let text = {
        let b = inner.borrow();
        match b.selection {
            Some((s, e)) if s != e => b.term.selection_text(s, e),
            _ => None,
        }
    };
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
    }
}

fn request_paste_from_clipboard(area: &DrawingArea, inner: &Rc<RefCell<Inner>>) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let clipboard = display.clipboard();
    // Probe the clipboard's advertised formats: if an image MIME is on
    // offer we paste an image (write to tempfile, inject path);
    // otherwise fall back to plain text. Probing `formats()` avoids
    // racing two async reads.
    let has_image = clipboard
        .formats()
        .mime_types()
        .iter()
        .any(|m| m.starts_with("image/"));

    if has_image {
        let inner = inner.clone();
        let area = area.clone();
        clipboard.read_texture_async(None::<&gtk4::gio::Cancellable>, move |res| match res {
            Ok(Some(texture)) => match write_clipboard_image(&texture) {
                Ok(path) => paste_text(&inner, &area, &format!("{}", path.display())),
                Err(err) => tracing::warn!(error = %err, "image paste: write tempfile failed"),
            },
            Ok(None) => {
                // Clipboard advertised an image MIME but read returned
                // empty — quietly fall back to text paste.
                fallback_text_paste(&area, &inner);
            }
            Err(err) => {
                tracing::debug!(error = %err, "clipboard texture read failed; trying text");
                fallback_text_paste(&area, &inner);
            }
        });
        return;
    }

    fallback_text_paste(area, inner);
}

fn fallback_text_paste(area: &DrawingArea, inner: &Rc<RefCell<Inner>>) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let clipboard = display.clipboard();
    let inner = inner.clone();
    let area = area.clone();
    clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, move |res| match res {
        Ok(Some(text)) => {
            let text_str = text.to_string();
            if text_str.is_empty() {
                return;
            }
            paste_text(&inner, &area, &text_str);
        }
        Ok(None) => {}
        Err(err) => tracing::warn!(error = %err, "clipboard read failed"),
    });
}

fn paste_text(inner: &Rc<RefCell<Inner>>, area: &DrawingArea, text: &str) {
    let mut b = inner.borrow_mut();
    let bracketed = b.term.bracketed_paste_enabled();
    if bracketed {
        let _ = b.pty.writer().write_all(b"\x1b[200~");
    }
    if let Err(err) = b.pty.writer().write_all(text.as_bytes()) {
        tracing::warn!(error = %err, "paste write failed");
    }
    if bracketed {
        let _ = b.pty.writer().write_all(b"\x1b[201~");
    }
    b.term.scroll_to_bottom();
    drop(b);
    area.queue_draw();
}

fn open_terminal_context_menu(
    area: &DrawingArea,
    inner: &Rc<RefCell<Inner>>,
    pane_id: PaneId,
    x: f64,
    y: f64,
    on_action: TerminalActionCallback,
    prefix: String,
) {
    let popover = Popover::new();
    popover.set_has_arrow(true);
    popover.set_parent(area);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let body = GtkBox::new(Orientation::Vertical, 2);
    body.set_margin_top(6);
    body.set_margin_bottom(6);
    body.set_margin_start(6);
    body.set_margin_end(6);

    let inner_copy = inner.clone();
    let popover_copy = popover.clone();
    body.append(&context_menu_button(
        "Copy",
        terminal_copy_shortcut(),
        move || {
            copy_selection_to_clipboard(&inner_copy);
            popover_copy.popdown();
        },
    ));

    let inner_paste = inner.clone();
    let area_paste = area.clone();
    let popover_paste = popover.clone();
    body.append(&context_menu_button(
        "Paste",
        terminal_paste_shortcut(),
        move || {
            request_paste_from_clipboard(&area_paste, &inner_paste);
            popover_paste.popdown();
        },
    ));

    append_separator(&body);

    append_action_button(
        &body,
        &popover,
        pane_id,
        "Split right",
        &prefixed_shortcut(&prefix, "|"),
        TerminalContextAction::SplitRight,
        on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        pane_id,
        "Split down",
        &prefixed_shortcut(&prefix, "-"),
        TerminalContextAction::SplitDown,
        on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        pane_id,
        "Close pane",
        &prefixed_shortcut(&prefix, "x"),
        TerminalContextAction::ClosePane,
        on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        pane_id,
        "New anchor",
        "",
        TerminalContextAction::NewAnchor,
        on_action.clone(),
    );

    append_separator(&body);

    append_action_button(
        &body,
        &popover,
        pane_id,
        "Next pane",
        &prefixed_shortcut(&prefix, "o"),
        TerminalContextAction::NextPane,
        on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        pane_id,
        "Previous pane",
        &prefixed_shortcut(&prefix, "p"),
        TerminalContextAction::PreviousPane,
        on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        pane_id,
        "Rearrange mode",
        &prefixed_shortcut(&prefix, "m"),
        TerminalContextAction::ToggleRearrange,
        on_action,
    );

    popover.set_child(Some(&body));
    let popover_cleanup = popover.clone();
    popover.connect_closed(move |_| {
        popover_cleanup.unparent();
    });
    popover.popup();
}

fn append_action_button(
    body: &GtkBox,
    popover: &Popover,
    pane_id: PaneId,
    label: &str,
    shortcut: &str,
    action: TerminalContextAction,
    on_action: TerminalActionCallback,
) {
    let popover = popover.clone();
    body.append(&context_menu_button(label, shortcut, move || {
        popover.popdown();
        on_action(pane_id, action);
    }));
}

fn context_menu_button(label: &str, shortcut: &str, on_click: impl Fn() + 'static) -> Button {
    let row = GtkBox::new(Orientation::Horizontal, 16);
    row.set_hexpand(true);

    let title = Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&title);

    if !shortcut.is_empty() {
        let shortcut = Label::new(Some(shortcut));
        shortcut.set_xalign(1.0);
        shortcut.add_css_class("dim-label");
        shortcut.add_css_class("monospace");
        row.append(&shortcut);
    }

    let button = Button::new();
    button.add_css_class("flat");
    button.set_halign(Align::Fill);
    button.set_child(Some(&row));
    button.connect_clicked(move |_| on_click());
    button
}

fn append_separator(body: &GtkBox) {
    let separator = gtk4::Separator::new(Orientation::Horizontal);
    separator.set_margin_top(4);
    separator.set_margin_bottom(4);
    body.append(&separator);
}

fn prefixed_shortcut(prefix: &str, key: &str) -> String {
    format!("{} {}", prefix.trim(), key)
}

fn terminal_copy_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+C"
    } else {
        "Ctrl+Shift+C"
    }
}

fn terminal_paste_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+V"
    } else {
        "Ctrl+Shift+V"
    }
}

/// Save a clipboard image to a temp PNG and return the path. Tools
/// like `claude` cli accept the file path as an argument-like paste
/// and load the image themselves. Files land under
/// `$XDG_RUNTIME_DIR/lmux/pastes/` (or `/tmp/lmux-pastes-<uid>` if
/// the runtime dir is unset). Filenames embed the cockpit pid + a
/// monotonic counter so concurrent panes don't collide.
fn write_clipboard_image(texture: &gdk::Texture) -> std::io::Result<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let dir = paste_dir();
    std::fs::create_dir_all(&dir)?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let filename = format!("paste-{pid}-{stamp}-{n}.png");
    let path = dir.join(filename);
    texture
        .save_to_png(&path)
        .map_err(|e| std::io::Error::other(format!("gdk_texture_save_to_png failed: {e}")))?;
    tracing::info!(path = %path.display(), "image paste: wrote tempfile");
    Ok(path)
}

fn paste_dir() -> std::path::PathBuf {
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        if !rt.is_empty() {
            return std::path::PathBuf::from(rt).join("lmux/pastes");
        }
    }
    let uid = unsafe { libc::getuid() };
    std::env::temp_dir().join(format!("lmux-pastes-{uid}"))
}

/// RenderVisitor that collapses each terminal cell to a single RGB pixel.
/// Empty cells carry the frame's default background; any cell with text
/// carries the cell's foreground. Feeds [`Pane::snapshot_thumbnail`].
struct ThumbnailVisitor {
    cols: u16,
    rows: u16,
    buf: Vec<u8>,
    default_bg: [u8; 3],
}

impl ThumbnailVisitor {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            buf: vec![0u8; (cols as usize) * (rows as usize) * 3],
            default_bg: [0, 0, 0],
        }
    }

    fn into_rgb(self) -> Vec<u8> {
        self.buf
    }

    fn write_pixel(&mut self, row: u16, col: u16, rgb: [u8; 3]) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let idx = ((row as usize) * (self.cols as usize) + (col as usize)) * 3;
        self.buf[idx] = rgb[0];
        self.buf[idx + 1] = rgb[1];
        self.buf[idx + 2] = rgb[2];
    }
}

impl RenderVisitor for ThumbnailVisitor {
    fn begin(&mut self, frame: &LgFrame) {
        self.default_bg = [frame.background.r, frame.background.g, frame.background.b];
        for chunk in self.buf.chunks_exact_mut(3) {
            chunk.copy_from_slice(&self.default_bg);
        }
    }

    fn cell(&mut self, cell: &CellView<'_>) {
        let rgb = if cell.text.trim().is_empty() {
            [cell.bg.r, cell.bg.g, cell.bg.b]
        } else {
            [cell.fg.r, cell.fg.g, cell.fg.b]
        };
        self.write_pixel(cell.row, cell.col, rgb);
    }

    fn cursor(&mut self, _cursor: &CursorPos) {}
    fn end(&mut self) {}
}

fn measure_cell(area: &DrawingArea, font: &pango::FontDescription) -> (f64, f64) {
    let pctx = area.create_pango_context();
    // Snap glyph metrics to the pixel grid — without HINT_METRICS_ON Pango
    // returns sub-pixel advances and characters get drawn at fractional x
    // positions, which produces blurry, slightly-mis-spaced text on a
    // monospace grid.
    if let Ok(mut opts) = gtk4::cairo::FontOptions::new() {
        opts.set_hint_metrics(gtk4::cairo::HintMetrics::On);
        #[cfg(target_os = "macos")]
        {
            opts.set_hint_style(gtk4::cairo::HintStyle::None);
            opts.set_antialias(gtk4::cairo::Antialias::Gray);
        }
        #[cfg(target_os = "linux")]
        {
            opts.set_hint_style(gtk4::cairo::HintStyle::Slight);
            opts.set_antialias(gtk4::cairo::Antialias::Subpixel);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            opts.set_hint_style(gtk4::cairo::HintStyle::Slight);
            opts.set_antialias(gtk4::cairo::Antialias::Subpixel);
        }
        pangocairo::functions::context_set_font_options(&pctx, Some(&opts));
    }
    // Use Pango font metrics for the cell advance width — the recommended
    // way to size a monospace grid. `pixel_size("M")` returns the glyph's
    // ink width, which is narrower than the cell advance.
    let metrics = pctx.metrics(Some(font), None);
    let scale = f64::from(pango::SCALE);
    let cell_w = (f64::from(metrics.approximate_digit_width()) / scale).round();
    let cell_h = (f64::from(metrics.height()) / scale).round();
    let cell_w = if cell_w >= 1.0 { cell_w } else { 1.0 };
    let cell_h = if cell_h >= 1.0 { cell_h } else { 1.0 };
    (cell_w, cell_h)
}

fn install_draw_func(area: &DrawingArea, inner: &Rc<RefCell<Inner>>) {
    let inner = inner.clone();
    area.set_draw_func(move |_area, cr, _w, _h| {
        let mut b = inner.borrow_mut();
        let cell_w = b.cell_w;
        let cell_h = b.cell_h;
        let font = b.font_desc.clone();
        let selection = b.selection.map(|(s, e)| Selection::new(s, e));
        let exit = b.exit_code;
        let cols = b.cols;
        let mut renderer =
            CairoRenderer::new(cr, &font, cell_w, cell_h, selection.as_ref(), exit, cols);
        b.term.render(&mut renderer);
    });
}

fn install_resize_handler(area: &DrawingArea, inner: &Rc<RefCell<Inner>>) {
    let inner = inner.clone();
    area.connect_resize(move |_area, w, h| {
        let mut b = inner.borrow_mut();
        if b.cell_w < 1.0 || b.cell_h < 1.0 {
            return;
        }
        let cols = ((f64::from(w) / b.cell_w).floor() as u16).max(1);
        let rows = ((f64::from(h) / b.cell_h).floor() as u16).max(1);
        if cols == b.cols && rows == b.rows {
            return;
        }
        b.cols = cols;
        b.rows = rows;
        let cw_px = b.cell_w as u32;
        let ch_px = b.cell_h as u32;
        b.term.resize(cols, rows, cw_px, ch_px);
        if let Err(err) = b.pty.resize(cols, rows, cw_px as u16, ch_px as u16) {
            tracing::warn!(error = %err, "pty resize failed");
        }
        tracing::debug!(cols, rows, "pane resized");
    });
}

fn start_pty_reader(
    area: &DrawingArea,
    inner: &Rc<RefCell<Inner>>,
    mut reader: Box<dyn Read + Send>,
    id: PaneId,
    bell_cb: Rc<RefCell<Option<BellCallback>>>,
) {
    let (tx, rx) = async_channel::bounded::<Vec<u8>>(READER_CHAN_CAPACITY);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    tracing::info!("pty reader hit EOF");
                    break;
                }
                Ok(n) => {
                    if tx.send_blocking(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "pty read error");
                    break;
                }
            }
        }
    });

    let inner_chan = inner.clone();
    let area_chan = area.clone();
    glib::MainContext::default().spawn_local(async move {
        let mut scanner = BellScanner::default();
        let mut last_bell: Option<std::time::Instant> = None;
        while let Ok(bytes) = rx.recv().await {
            let span = tracing::info_span!("pty_to_paint", len = bytes.len());
            let _g = span.enter();
            let bells = scanner.scan(&bytes);
            inner_chan.borrow_mut().term.feed(&bytes);
            area_chan.queue_draw();
            if bells > 0 {
                let now = std::time::Instant::now();
                let fire = match last_bell {
                    Some(t) => now.duration_since(t) >= BELL_DEBOUNCE,
                    None => true,
                };
                if fire {
                    last_bell = Some(now);
                    if let Some(cb) = bell_cb.borrow().clone() {
                        cb(id);
                    }
                }
            }
        }
        tracing::info!("pty reader channel closed");
    });

    let inner_exit = inner.clone();
    let area_exit = area.clone();
    glib::timeout_add_seconds_local(1, move || {
        let mut b = inner_exit.borrow_mut();
        if b.exit_code.is_some() {
            return glib::ControlFlow::Break;
        }
        match b.pty.try_wait() {
            Ok(Some(status)) => {
                // portable-pty 0.9 synthesizes code=1 whenever the child was
                // killed by signal (std::process::ExitStatus::code() is None
                // for signal-killed, and the crate does `.unwrap_or(1)`).
                // We want the log to distinguish "shell exited with status 1"
                // from "shell was terminated by SIGTERM" — the latter is the
                // expected path on Ctrl+B x / shutdown.
                let signal = status.signal().map(str::to_string);
                let code = status.exit_code() as i32;
                b.exit_code = Some(code);
                drop(b);
                match signal.as_deref() {
                    Some(sig) => tracing::info!(signal = sig, "pane child terminated"),
                    None => tracing::info!(code, "pane child exited"),
                }
                area_exit.queue_draw();
                glib::ControlFlow::Break
            }
            Ok(None) => glib::ControlFlow::Continue,
            Err(err) => {
                tracing::warn!(error = %err, "pty try_wait failed");
                glib::ControlFlow::Continue
            }
        }
    });
}

/// A single cell in the lmux pane tree — either a terminal attached to a
/// PTY (the v0.1 shape) or a GUI satellite rendering a nested-wayland
/// toplevel (v0.2, ADR-0018). The enum collapses both into one value so
/// `AppState::panes: HashMap<PaneId, Pane>` can hold either kind and the
/// layout/rebuild/focus code paths treat them uniformly.
pub enum Pane {
    Terminal(TerminalPane),
    #[allow(dead_code)]
    Satellite(crate::satellite::SatelliteWidget),
}

impl Pane {
    /// Spawn a fresh terminal pane. Legacy entry point — preserves the
    /// callsites from the v0.1 code where every pane was a terminal.
    pub fn new(id: PaneId, cwd: Option<&Path>) -> Option<Self> {
        TerminalPane::new(id, cwd).map(Self::Terminal)
    }

    /// Wrap a pre-built satellite widget as a pane. The satellite was
    /// already allocated by the host-event dispatcher with its PaneId
    /// baked in, so we just promote it here.
    #[cfg(target_os = "linux")]
    pub fn from_satellite(widget: crate::satellite::SatelliteWidget) -> Self {
        Self::Satellite(widget)
    }

    pub fn id(&self) -> PaneId {
        match self {
            Self::Terminal(t) => t.id(),
            Self::Satellite(s) => s.pane_id(),
        }
    }

    pub fn widget(&self) -> &Frame {
        match self {
            Self::Terminal(t) => t.widget(),
            Self::Satellite(s) => s.widget(),
        }
    }

    pub fn grab_focus(&self) {
        match self {
            Self::Terminal(t) => t.grab_focus(),
            Self::Satellite(s) => s.grab_focus(),
        }
    }

    pub fn has_exited(&self) -> bool {
        match self {
            Self::Terminal(t) => t.has_exited(),
            Self::Satellite(s) => s.has_exited(),
        }
    }

    /// Graceful close. Terminal panes SIGTERM their PTY child; satellites
    /// ask the nested compositor to fire `xdg_toplevel.close`.
    pub fn terminate(&self) {
        match self {
            Self::Terminal(t) => t.terminate(),
            Self::Satellite(s) => s.request_close(),
        }
    }

    /// Hard close. Terminal panes SIGKILL; satellites don't have a
    /// SIGKILL equivalent over xdg-shell — dropping the widget tears the
    /// wayland client connection, which is effectively the same thing.
    pub fn kill(&self) {
        match self {
            Self::Terminal(t) => t.kill(),
            Self::Satellite(s) => s.request_close(),
        }
    }

    // ---- Terminal-only accessors: satellites return None / no-op -------

    pub fn cwd(&self) -> Option<std::path::PathBuf> {
        match self {
            Self::Terminal(t) => t.cwd(),
            Self::Satellite(_) => None,
        }
    }

    pub fn child_pid(&self) -> Option<u32> {
        match self {
            Self::Terminal(t) => t.child_pid(),
            Self::Satellite(_) => None,
        }
    }

    pub fn snapshot_cwd(&self) -> Option<std::path::PathBuf> {
        match self {
            Self::Terminal(t) => t.snapshot_cwd(),
            Self::Satellite(_) => None,
        }
    }

    pub fn snapshot_thumbnail(&self) -> Option<(u32, u32, Vec<u8>)> {
        match self {
            Self::Terminal(t) => t.snapshot_thumbnail(),
            Self::Satellite(_) => None,
        }
    }

    pub fn cell_size(&self) -> (i32, i32) {
        match self {
            Self::Terminal(t) => t.cell_size(),
            // Satellites don't have a cell grid. Return (1,1) so callers
            // that scale MIN_COLS/MIN_ROWS by cell size don't divide by
            // zero; the sentinel is never used for real layout maths.
            Self::Satellite(_) => (1, 1),
        }
    }

    pub fn set_font(&self, family: &str, size_pt: i32) {
        if let Self::Terminal(t) = self {
            t.set_font(family, size_pt);
        }
    }

    pub fn set_bell_callback(&self, cb: BellCallback) {
        if let Self::Terminal(t) = self {
            t.set_bell_callback(cb);
        }
    }

    pub fn attach_controllers(
        &self,
        cb: FocusCallback,
        focus_mode: FocusModeCell,
        on_action: TerminalActionCallback,
        shortcut_prefix: ShortcutPrefixCell,
    ) {
        match self {
            Self::Terminal(t) => t.attach_controllers(cb, focus_mode, on_action, shortcut_prefix),
            // Satellites install pointer/keyboard/scroll controllers at
            // construction time, but the cockpit's focus callback isn't
            // known until the pane is spliced in — wire it now so
            // `AppState.focused` tracks satellites like it tracks
            // terminals.
            Self::Satellite(s) => s.attach_focus_callback(cb, focus_mode),
        }
    }

    /// Wire the rearrange-mode DnD controllers. Both pane types use the
    /// same Frame-level controller pair and share the helper, so the
    /// cockpit doesn't need to know which underlying widget it owns.
    pub fn attach_rearrange_controllers(
        &self,
        mode: RearrangeModeCell,
        on_reparent: ReparentCallback,
    ) {
        let id = self.id();
        attach_rearrange_to_frame(self.widget(), id, mode, on_reparent);
    }

    /// True iff this is a GUI satellite. Used by anchor-tag logic to
    /// skip satellites (only terminal panes qualify as anchors).
    #[allow(dead_code)]
    pub fn is_satellite(&self) -> bool {
        matches!(self, Self::Satellite(_))
    }

    #[cfg(target_os = "linux")]
    pub fn as_satellite(&self) -> Option<&crate::satellite::SatelliteWidget> {
        if let Self::Satellite(s) = self {
            Some(s)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod bell_scanner_tests {
    use super::BellScanner;

    #[test]
    fn counts_bare_bells() {
        let mut s = BellScanner::default();
        assert_eq!(s.scan(b"hi\x07there"), 1);
        assert_eq!(s.scan(b"\x07\x07"), 2);
    }

    #[test]
    fn bel_inside_osc_is_st_terminator_not_a_bell() {
        let mut s = BellScanner::default();
        // OSC 0 ; title <BEL> — classic set-window-title sequence. The BEL
        // terminates the OSC; it must not count as a user-visible bell.
        let seq = b"\x1b]0;my title\x07after";
        assert_eq!(s.scan(seq), 0);
        // After the OSC ends, a fresh BEL counts normally.
        assert_eq!(s.scan(b"\x07"), 1);
    }

    #[test]
    fn esc_backslash_st_terminates_osc() {
        let mut s = BellScanner::default();
        // OSC 0 ; title ST(ESC \)
        let seq = b"\x1b]0;title\x1b\\rest";
        assert_eq!(s.scan(seq), 0);
        assert_eq!(s.scan(b"ping\x07"), 1);
    }

    #[test]
    fn esc_without_osc_does_not_swallow_bel() {
        let mut s = BellScanner::default();
        // ESC [ 31 m is a CSI SGR — not an OSC. BEL outside OSC counts.
        let seq = b"\x1b[31mred\x07";
        assert_eq!(s.scan(seq), 1);
    }

    #[test]
    fn chunk_boundary_preserves_osc_state() {
        let mut s = BellScanner::default();
        // Split the OSC across two calls; the scanner must remember it's
        // still inside OSC and swallow the BEL in the second chunk.
        assert_eq!(s.scan(b"\x1b]0;par"), 0);
        assert_eq!(s.scan(b"tial\x07after\x07"), 1);
    }
}
