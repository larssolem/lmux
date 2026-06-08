use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gtk4::gdk;
use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DragSource, DrawingArea, DropTarget, Entry,
    EventControllerMotion, EventControllerScroll, EventControllerScrollFlags, Frame, GestureClick,
    GestureDrag, Label, Orientation, Popover,
};
use lmux_config::FocusMode;
use lmux_libghostty::{
    CellView, CursorPos, Frame as LgFrame, RenderVisitor, Rgb, ScreenPoint, Terminal, ViewportPoint,
};
use lmux_pty::{self, Pane as PtyPane};

use crate::keymap::{self, KeyAction, TerminalShortcut};
use crate::layout::PaneId;
use crate::render::{CairoRenderer, SearchHighlight, Selection};

const WHEEL_ROWS_PER_TICK: i32 = 3;

const INIT_COLS: u16 = 100;
const INIT_ROWS: u16 = 30;
const SCROLLBACK: usize = 100_000;
const TRANSCRIPT_MAX_LINES: usize = 10_000;
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
pub type TerminalExitCallback = Rc<dyn Fn(PaneId)>;
pub type TerminalActionCallback = Rc<dyn Fn(PaneId, TerminalContextAction)>;
pub type ShortcutPrefixCell = Rc<RefCell<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalContextAction {
    SplitRight,
    SplitDown,
    NewTab,
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
    search_bar: GtkBox,
    search_entry: Entry,
    search_status: Label,
    inner: Rc<RefCell<Inner>>,
    transcript: Rc<RefCell<TranscriptBuffer>>,
    bell_cb: Rc<RefCell<Option<BellCallback>>>,
    exit_cb: Rc<RefCell<Option<TerminalExitCallback>>>,
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
    selection: Option<(ScreenPoint, ScreenPoint)>,
    search: Option<SearchState>,
    drag_anchor: Option<ScreenPoint>,
    drag_pointer: Option<(f64, f64)>,
    frozen_frame: Option<FrozenFrame>,
    exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SearchMatch {
    start: ScreenPoint,
    end: ScreenPoint,
}

#[derive(Clone, Debug)]
struct SearchState {
    query: String,
    matches: Vec<SearchMatch>,
    active: Option<SearchMatch>,
    scrollback_matches: Option<Vec<SearchMatch>>,
}

struct TranscriptBuffer {
    max_lines: usize,
    next_sequence: u64,
    dropped_before: u64,
    lines: VecDeque<lmux_bus::TranscriptLine>,
}

impl TranscriptBuffer {
    fn new(max_lines: usize) -> Self {
        Self {
            max_lines,
            next_sequence: 1,
            dropped_before: 1,
            lines: VecDeque::new(),
        }
    }

    fn append_bytes(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        for part in text.split_inclusive('\n') {
            let line = part.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                continue;
            }
            self.append_line(line.to_string(), now_unix_millis());
        }
    }

    fn append_line(&mut self, text: String, unix_millis: u64) {
        let line = lmux_bus::TranscriptLine {
            sequence: self.next_sequence,
            unix_millis,
            text,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.lines.push_back(line);
        while self.lines.len() > self.max_lines {
            if let Some(dropped) = self.lines.pop_front() {
                self.dropped_before = dropped.sequence.saturating_add(1);
            }
        }
    }

    fn tail(&self, pane_id: uuid::Uuid, max_lines: u32) -> lmux_bus::TranscriptRange {
        let keep = max_lines as usize;
        let start = self.lines.len().saturating_sub(keep);
        self.range_from_iter(pane_id, self.lines.iter().skip(start).cloned(), false)
    }

    fn capture_since(
        &self,
        pane_id: uuid::Uuid,
        since_sequence: Option<u64>,
        max_lines: Option<u32>,
    ) -> lmux_bus::TranscriptRange {
        let since = since_sequence.unwrap_or(0);
        let truncated = since > 0 && since < self.dropped_before;
        let iter = self
            .lines
            .iter()
            .filter(move |line| line.sequence > since)
            .cloned();
        let collected: Vec<_> = match max_lines {
            Some(max) => iter.take(max as usize).collect(),
            None => iter.collect(),
        };
        self.range_from_iter(pane_id, collected, truncated)
    }

    fn range_from_iter<I>(
        &self,
        pane_id: uuid::Uuid,
        iter: I,
        truncated: bool,
    ) -> lmux_bus::TranscriptRange
    where
        I: IntoIterator<Item = lmux_bus::TranscriptLine>,
    {
        let lines: Vec<_> = iter.into_iter().collect();
        let first_sequence = lines.first().map(|line| line.sequence).unwrap_or(0);
        let last_sequence = lines.last().map(|line| line.sequence).unwrap_or(0);
        lmux_bus::TranscriptRange {
            pane_id,
            first_sequence,
            last_sequence,
            truncated,
            lines,
        }
    }
}

fn now_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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
        frame.add_css_class("pane");
        let pane_body = GtkBox::new(Orientation::Vertical, 0);
        pane_body.set_hexpand(true);
        pane_body.set_vexpand(true);

        let search_bar = GtkBox::new(Orientation::Horizontal, 6);
        search_bar.add_css_class("lmux-terminal-search");
        search_bar.set_visible(false);
        search_bar.set_hexpand(true);

        let search_entry = Entry::new();
        search_entry.set_placeholder_text(Some("Search terminal..."));
        search_entry.set_hexpand(true);

        let search_status = Label::new(Some("Type to search"));
        search_status.set_xalign(0.0);
        search_status.set_width_chars(24);
        search_status.add_css_class("dim-label");

        let search_previous = Button::with_label("Previous");
        let search_next = Button::with_label("Next");
        let search_close = Button::with_label("Close");

        search_bar.append(&search_entry);
        search_bar.append(&search_status);
        search_bar.append(&search_previous);
        search_bar.append(&search_next);
        search_bar.append(&search_close);
        pane_body.append(&search_bar);
        pane_body.append(&drawing_area);
        frame.set_child(Some(&pane_body));
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
            search: None,
            drag_anchor: None,
            drag_pointer: None,
            frozen_frame: None,
            exit_code: None,
        }));
        let transcript = Rc::new(RefCell::new(TranscriptBuffer::new(TRANSCRIPT_MAX_LINES)));

        let bell_cb: Rc<RefCell<Option<BellCallback>>> = Rc::new(RefCell::new(None));
        let exit_cb: Rc<RefCell<Option<TerminalExitCallback>>> = Rc::new(RefCell::new(None));

        install_draw_func(&drawing_area, &inner);
        install_resize_handler(&drawing_area, &inner);
        install_search_bar(
            SearchBarParts {
                bar: &search_bar,
                entry: &search_entry,
                status: &search_status,
                previous: &search_previous,
                next: &search_next,
                close: &search_close,
            },
            &drawing_area,
            &inner,
        );
        start_pty_reader(
            &drawing_area,
            &inner,
            reader,
            id,
            transcript.clone(),
            bell_cb.clone(),
            exit_cb.clone(),
        );

        Some(Self {
            id,
            frame,
            drawing_area,
            search_bar,
            search_entry,
            search_status,
            inner,
            transcript,
            bell_cb,
            exit_cb,
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

    pub fn set_exit_callback(&self, cb: TerminalExitCallback) {
        *self.exit_cb.borrow_mut() = Some(cb);
    }

    pub fn transcript_tail(&self, pane_id: uuid::Uuid, lines: u32) -> lmux_bus::TranscriptRange {
        self.transcript.borrow().tail(pane_id, lines)
    }

    pub fn transcript_capture(
        &self,
        pane_id: uuid::Uuid,
        since_sequence: Option<u64>,
        max_lines: Option<u32>,
    ) -> lmux_bus::TranscriptRange {
        self.transcript
            .borrow()
            .capture_since(pane_id, since_sequence, max_lines)
    }

    pub fn send_input(&self, text: &str) -> std::io::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.pty.writer().write_all(text.as_bytes())?;
        inner.term.scroll_to_bottom();
        inner.selection = None;
        inner.search = None;
        drop(inner);
        self.drawing_area.queue_draw();
        Ok(())
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
        let search_bar = self.search_bar.clone();
        let search_entry = self.search_entry.clone();
        let search_status = self.search_status.clone();
        key.connect_key_pressed(move |_ctrl, keyval, _code, modifier| {
            if let Some(shortcut) = keymap::classify_terminal_shortcut(keyval, modifier) {
                match shortcut {
                    TerminalShortcut::Copy => copy_selection_to_clipboard(&inner),
                    TerminalShortcut::Paste if inner.borrow().exit_code.is_none() => {
                        request_paste_from_clipboard(&area, &inner)
                    }
                    TerminalShortcut::Paste => {}
                    TerminalShortcut::Find => {
                        show_search_bar(&search_bar, &search_entry, &search_status, &area, &inner)
                    }
                }
                return glib::Propagation::Stop;
            }
            if inner.borrow().exit_code.is_some() {
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
                    b.search = None;
                    b.frozen_frame = None;
                    drop(b);
                    search_bar.set_visible(false);
                    area.queue_draw();
                }
                KeyAction::ScrollRows(delta) => {
                    let mut b = inner.borrow_mut();
                    b.term.scroll_delta(delta as isize);
                    if b.selection.is_some() {
                        b.freeze_viewport();
                    }
                    drop(b);
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
                let mut b = inner.borrow_mut();
                b.term.scroll_delta(rows as isize);
                if let Some(anchor) = b.drag_anchor {
                    let viewport_row = if rows < 0 {
                        0
                    } else {
                        b.rows.saturating_sub(1)
                    };
                    let viewport_col = if rows < 0 {
                        0
                    } else {
                        b.cols.saturating_sub(1)
                    };
                    let viewport_point = ViewportPoint {
                        row: viewport_row,
                        col: viewport_col,
                    };
                    if let Some(end) = b.term.screen_point_from_viewport(viewport_point) {
                        b.selection = Some((anchor, end));
                    }
                }
                if b.selection.is_some() {
                    b.freeze_viewport();
                }
                drop(b);
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
            if let Some(anchor) = b.screen_point_from_px(x, y) {
                b.drag_anchor = Some(anchor);
                b.selection = Some((anchor, anchor));
                b.drag_pointer = Some((x, y));
                b.freeze_viewport();
            }
            drop(b);
            area.queue_draw();
        });
        let inner_scroll = self.inner.clone();
        let area_scroll = self.drawing_area.clone();
        drag.connect_drag_begin(move |_g, _x, _y| {
            let inner_scroll = inner_scroll.clone();
            let area_scroll = area_scroll.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                let mut b = inner_scroll.borrow_mut();
                let Some(anchor) = b.drag_anchor else {
                    return glib::ControlFlow::Break;
                };
                let Some((x, y)) = b.drag_pointer else {
                    return glib::ControlFlow::Continue;
                };
                if y < 0.0 {
                    b.term.scroll_delta(-(WHEEL_ROWS_PER_TICK as isize));
                } else if y >= f64::from(area_scroll.height().max(1)) {
                    b.term.scroll_delta(WHEEL_ROWS_PER_TICK as isize);
                } else {
                    return glib::ControlFlow::Continue;
                }
                b.freeze_viewport();
                if let Some(end) = b.screen_point_from_px(x, y) {
                    b.selection = Some((anchor, end));
                }
                drop(b);
                area_scroll.queue_draw();
                glib::ControlFlow::Continue
            });
        });
        let inner_up = self.inner.clone();
        let area_up = self.drawing_area.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some((sx, sy)) = g.start_point() else {
                return;
            };
            let x = sx + dx;
            let y = sy + dy;
            let mut b = inner_up.borrow_mut();
            let Some(anchor) = b.drag_anchor else {
                return;
            };
            b.drag_pointer = Some((x, y));
            if y < 0.0 {
                b.term.scroll_delta(-(WHEEL_ROWS_PER_TICK as isize));
                b.freeze_viewport();
            } else if y >= f64::from(area_up.height().max(1)) {
                b.term.scroll_delta(WHEEL_ROWS_PER_TICK as isize);
                b.freeze_viewport();
            }
            if let Some(end) = b.screen_point_from_px(x, y) {
                b.selection = Some((anchor, end));
            }
            drop(b);
            area_up.queue_draw();
        });
        let inner_end = self.inner.clone();
        let area_end = self.drawing_area.clone();
        drag.connect_drag_end(move |_g, _dx, _dy| {
            let selection = {
                let mut b = inner_end.borrow_mut();
                b.drag_anchor = None;
                b.drag_pointer = None;
                b.selection
            };
            if let Some((start, end)) = selection {
                if start == end {
                    let mut b = inner_end.borrow_mut();
                    b.selection = None;
                    b.frozen_frame = None;
                    area_end.queue_draw();
                    return;
                }
                let text = inner_end.borrow().term.selection_text_screen(start, end);
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
        let search_bar = self.search_bar.clone();
        let search_entry = self.search_entry.clone();
        let search_status = self.search_status.clone();
        click.connect_pressed(move |_g, _n, x, y| {
            area.grab_focus();
            on_focus(id);
            open_terminal_context_menu(TerminalContextMenu {
                area: &area,
                inner: &inner,
                search: SearchUi {
                    bar: &search_bar,
                    entry: &search_entry,
                    status: &search_status,
                },
                pane_id: id,
                x,
                y,
                on_action: on_action.clone(),
                prefix: shortcut_prefix.borrow().clone(),
            });
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

    fn screen_point_from_px(&self, x: f64, y: f64) -> Option<ScreenPoint> {
        self.term
            .screen_point_from_viewport(self.point_from_px(x, y))
    }

    fn freeze_viewport(&mut self) {
        let Some(top_screen_row) = self.term.viewport_top_screen_row() else {
            self.frozen_frame = None;
            return;
        };
        let mut visitor = FrozenVisitor::new(top_screen_row);
        self.term.render(&mut visitor);
        self.frozen_frame = visitor.into_frame();
    }
}

fn copy_selection_to_clipboard(inner: &Rc<RefCell<Inner>>) {
    let text = {
        let b = inner.borrow();
        match b.selection {
            Some((s, e)) if s != e => b.term.selection_text_screen(s, e),
            _ => None,
        }
    };
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
    }
}

struct SearchUi<'a> {
    bar: &'a GtkBox,
    entry: &'a Entry,
    status: &'a Label,
}

struct SearchBarParts<'a> {
    bar: &'a GtkBox,
    entry: &'a Entry,
    status: &'a Label,
    previous: &'a Button,
    next: &'a Button,
    close: &'a Button,
}

fn install_search_bar(parts: SearchBarParts<'_>, area: &DrawingArea, inner: &Rc<RefCell<Inner>>) {
    let inner_changed = inner.clone();
    let area_changed = area.clone();
    let status_changed = parts.status.clone();
    parts.entry.connect_changed(move |entry| {
        inner_changed
            .borrow_mut()
            .set_search_query(entry.text().to_string());
        update_search_status(&status_changed, &inner_changed);
        area_changed.queue_draw();
    });

    let inner_previous = inner.clone();
    let area_previous = area.clone();
    let status_previous = parts.status.clone();
    parts.previous.connect_clicked(move |_| {
        advance_search(&inner_previous, &area_previous, false);
        update_search_status(&status_previous, &inner_previous);
    });

    let inner_next = inner.clone();
    let area_next = area.clone();
    let status_next = parts.status.clone();
    parts.next.connect_clicked(move |_| {
        advance_search(&inner_next, &area_next, true);
        update_search_status(&status_next, &inner_next);
    });

    let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    let inner_scroll = inner.clone();
    let area_scroll = area.clone();
    let status_scroll = parts.status.clone();
    scroll.connect_scroll(move |_, _dx, dy| {
        let rows = (dy * f64::from(WHEEL_ROWS_PER_TICK)).round() as i32;
        scroll_search_viewport(&inner_scroll, &area_scroll, &status_scroll, rows);
        glib::Propagation::Stop
    });
    parts.bar.add_controller(scroll);

    let key = gtk4::EventControllerKey::new();
    let inner_key = inner.clone();
    let area_key = area.clone();
    let status_key = parts.status.clone();
    let bar_key = parts.bar.clone();
    key.connect_key_pressed(move |_, keyval, _, modifier| match keyval {
        gdk::Key::Escape => {
            close_search_bar(&bar_key, &inner_key, &area_key);
            glib::Propagation::Stop
        }
        gdk::Key::Return | gdk::Key::KP_Enter => {
            let forward = !modifier.contains(gdk::ModifierType::SHIFT_MASK);
            advance_search(&inner_key, &area_key, forward);
            update_search_status(&status_key, &inner_key);
            glib::Propagation::Stop
        }
        _ => {
            let page_rows = inner_key.borrow().rows;
            match keymap::classify_key(keyval, modifier, page_rows) {
                KeyAction::ScrollRows(rows) => {
                    scroll_search_viewport(&inner_key, &area_key, &status_key, rows);
                    glib::Propagation::Stop
                }
                KeyAction::Write(_) => glib::Propagation::Proceed,
            }
        }
    });
    parts.entry.add_controller(key);

    let inner_close = inner.clone();
    let area_close = area.clone();
    let bar_close = parts.bar.clone();
    parts.close.connect_clicked(move |_| {
        close_search_bar(&bar_close, &inner_close, &area_close);
    });
}

fn show_search_bar(
    bar: &GtkBox,
    entry: &Entry,
    status: &Label,
    area: &DrawingArea,
    inner: &Rc<RefCell<Inner>>,
) {
    bar.set_visible(true);
    let query = entry.text().to_string();
    if !query.is_empty() {
        inner.borrow_mut().set_search_query(query);
    }
    update_search_status(status, inner);
    area.queue_draw();
    entry.grab_focus();
    entry.select_region(0, -1);
}

fn close_search_bar(bar: &GtkBox, inner: &Rc<RefCell<Inner>>, area: &DrawingArea) {
    bar.set_visible(false);
    inner.borrow_mut().search = None;
    area.queue_draw();
    area.grab_focus();
}

fn update_search_status(label: &Label, inner: &Rc<RefCell<Inner>>) {
    label.set_text(&inner.borrow().search_status());
}

fn scroll_search_viewport(
    inner: &Rc<RefCell<Inner>>,
    area: &DrawingArea,
    status: &Label,
    rows: i32,
) {
    if rows == 0 {
        return;
    }
    {
        let mut b = inner.borrow_mut();
        b.term.scroll_delta(rows as isize);
        b.refresh_visible_search_matches(false);
    }
    update_search_status(status, inner);
    area.queue_draw();
}

fn advance_search(inner: &Rc<RefCell<Inner>>, area: &DrawingArea, forward: bool) {
    {
        let mut b = inner.borrow_mut();
        if b.search.as_ref().is_none_or(|state| state.query.is_empty()) {
            return;
        }
        b.select_adjacent_scrollback_match(forward);
    }
    area.queue_draw();
}

impl Inner {
    fn set_search_query(&mut self, query: String) {
        if query.is_empty() {
            self.search = None;
            return;
        }
        self.search = Some(SearchState {
            query,
            matches: Vec::new(),
            active: None,
            scrollback_matches: None,
        });
        self.refresh_search_matches(true);
    }

    fn refresh_search_matches(&mut self, select_first: bool) {
        let Some(query) = self.search.as_ref().map(|state| state.query.clone()) else {
            return;
        };
        let previous_active = self.search.as_ref().and_then(|state| state.active);
        let scrollback_matches =
            scrollback_search_matches(&mut self.term, &query, self.cols, self.rows);
        let matches = visible_search_matches(&mut self.term, &query);
        let active = if select_first {
            matches.first().copied()
        } else {
            previous_active.filter(|active| {
                scrollback_matches
                    .as_ref()
                    .is_some_and(|matches| matches.contains(active))
            })
        };
        if let Some(state) = self.search.as_mut() {
            state.matches = matches;
            state.active = active;
            state.scrollback_matches = scrollback_matches;
        }
    }

    fn refresh_visible_search_matches(&mut self, select_first: bool) {
        let Some(query) = self.search.as_ref().map(|state| state.query.clone()) else {
            return;
        };
        let previous_active = self.search.as_ref().and_then(|state| state.active);
        let matches = visible_search_matches(&mut self.term, &query);
        let active = if select_first {
            matches.first().copied()
        } else {
            previous_active
        };
        if let Some(state) = self.search.as_mut() {
            state.matches = matches;
            state.active = active;
        }
    }

    fn select_adjacent_scrollback_match(&mut self, forward: bool) -> bool {
        self.refresh_search_matches(false);
        let Some(state) = self.search.as_ref() else {
            return false;
        };
        let active = state.active;
        let Some(scrollback_matches) = state.scrollback_matches.as_ref() else {
            return false;
        };
        if scrollback_matches.is_empty() {
            return false;
        }
        let found = match (forward, active) {
            (true, Some(active)) => scrollback_matches
                .iter()
                .find(|m| search_match_order_key(m) > search_match_order_key(&active))
                .copied(),
            (false, Some(active)) => scrollback_matches
                .iter()
                .rev()
                .find(|m| search_match_order_key(m) < search_match_order_key(&active))
                .copied(),
            (true, None) => self
                .term
                .viewport_top_screen_row()
                .and_then(|top| {
                    scrollback_matches
                        .iter()
                        .find(|m| m.start.row >= top)
                        .copied()
                })
                .or_else(|| scrollback_matches.first().copied()),
            (false, None) => self
                .term
                .viewport_top_screen_row()
                .and_then(|top| {
                    let bottom = top.saturating_add(u32::from(self.rows.saturating_sub(1)));
                    scrollback_matches
                        .iter()
                        .rev()
                        .find(|m| m.start.row <= bottom)
                        .copied()
                })
                .or_else(|| scrollback_matches.last().copied()),
        };
        if let Some(found) = found {
            self.set_active_search_match(found);
            self.scroll_search_match_into_view(found);
            self.refresh_visible_search_matches(false);
            true
        } else {
            false
        }
    }

    fn scroll_search_match_into_view(&mut self, active: SearchMatch) {
        let Some(top) = self.term.viewport_top_screen_row() else {
            return;
        };
        let rows = u32::from(self.rows.max(1));
        let bottom = top.saturating_add(rows.saturating_sub(1));
        if active.start.row >= top && active.start.row <= bottom {
            return;
        }
        let target_top = active.start.row.saturating_sub(rows / 2);
        let delta = i64::from(target_top) - i64::from(top);
        let delta = delta.clamp(isize::MIN as i64, isize::MAX as i64) as isize;
        self.term.scroll_delta(delta);
    }

    fn set_active_search_match(&mut self, active: SearchMatch) {
        if let Some(state) = self.search.as_mut() {
            state.active = Some(active);
        }
    }

    fn search_highlights(&self, viewport_top_screen_row: u32) -> Vec<SearchHighlight> {
        self.search
            .as_ref()
            .map(|state| {
                state
                    .matches
                    .iter()
                    .map(|m| {
                        SearchHighlight::new(
                            m.start,
                            m.end,
                            state.active == Some(*m),
                            viewport_top_screen_row,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn search_status(&self) -> String {
        let Some(state) = self.search.as_ref() else {
            return "Type to search".to_string();
        };
        match state.scrollback_matches.as_ref().map(Vec::len) {
            Some(0) => "No matches found in scrollback".to_string(),
            Some(1) => "1 found in scrollback".to_string(),
            Some(total) => format!("{total} found in scrollback"),
            None => "Scrollback count unavailable".to_string(),
        }
    }
}

fn visible_search_matches(term: &mut Terminal, query: &str) -> Vec<SearchMatch> {
    let query_width = query.chars().count();
    if query.is_empty() || query_width == 0 {
        return Vec::new();
    }
    let Some(top) = term.viewport_top_screen_row() else {
        return Vec::new();
    };
    let mut visitor = ViewportTextVisitor::new(top);
    term.render(&mut visitor);
    visitor.matches(query)
}

fn scrollback_search_matches(
    term: &mut Terminal,
    query: &str,
    cols: u16,
    rows: u16,
) -> Option<Vec<SearchMatch>> {
    if query.is_empty() {
        return Some(Vec::new());
    }
    let original_top = term.viewport_top_screen_row()?;
    let scroll_extent = (SCROLLBACK as isize).saturating_add((rows as isize).max(1));

    term.scroll_delta(-scroll_extent);
    let top = term.viewport_top_screen_row();

    term.scroll_to_bottom();
    let bottom_top = term.viewport_top_screen_row();
    let bottom = term.screen_point_from_viewport(ViewportPoint {
        row: rows.saturating_sub(1),
        col: cols.saturating_sub(1),
    });
    let text = match (top, bottom) {
        (Some(top), Some(bottom)) => {
            term.selection_text_screen(ScreenPoint { row: top, col: 0 }, bottom)
        }
        _ => None,
    };

    let restore_from = bottom_top.unwrap_or(original_top);
    let restore_delta = i64::from(original_top) - i64::from(restore_from);
    let restore_delta = restore_delta.clamp(isize::MIN as i64, isize::MAX as i64) as isize;
    term.scroll_delta(restore_delta);

    match (top, text) {
        (Some(top), Some(text)) => Some(find_matches_in_text(top, &text, query)),
        _ => None,
    }
}

fn find_matches_in_text(first_row: u32, text: &str, query: &str) -> Vec<SearchMatch> {
    text.lines()
        .enumerate()
        .flat_map(|(row, line)| find_matches_in_line(first_row + row as u32, line, query))
        .collect()
}

#[cfg(test)]
fn count_matches_in_text(text: &str, query: &str) -> usize {
    find_matches_in_text(0, text, query).len()
}

fn search_match_order_key(m: &SearchMatch) -> (u32, u16) {
    (m.start.row, m.start.col)
}

struct ViewportTextVisitor {
    top_screen_row: u32,
    rows: Vec<String>,
}

impl ViewportTextVisitor {
    fn new(top_screen_row: u32) -> Self {
        Self {
            top_screen_row,
            rows: Vec::new(),
        }
    }

    fn matches(&self, query: &str) -> Vec<SearchMatch> {
        self.rows
            .iter()
            .enumerate()
            .flat_map(|(row, line)| {
                find_matches_in_line(self.top_screen_row + row as u32, line, query)
            })
            .collect()
    }
}

impl RenderVisitor for ViewportTextVisitor {
    fn begin(&mut self, frame: &LgFrame) {
        self.rows = vec![String::new(); frame.rows as usize];
    }

    fn cell(&mut self, cell: &CellView<'_>) {
        let Some(row) = self.rows.get_mut(cell.row as usize) else {
            return;
        };
        pad_to_column(row, cell.col as usize);
        row.push_str(cell.text);
    }

    fn cursor(&mut self, _cursor: &CursorPos) {}
    fn end(&mut self) {}
}

fn pad_to_column(row: &mut String, col: usize) {
    let len = row.chars().count();
    if len < col {
        row.extend(std::iter::repeat_n(' ', col - len));
    }
}

fn find_matches_in_line(row: u32, line: &str, query: &str) -> Vec<SearchMatch> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let haystack = line.to_lowercase();
    let query_width = query.chars().count().max(1);
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(relative) = haystack[offset..].find(&needle) {
        let start_byte = offset + relative;
        let start_col = line[..start_byte].chars().count();
        let end_col = start_col.saturating_add(query_width).saturating_sub(1);
        let start_col = start_col.min(u16::MAX as usize) as u16;
        let end_col = end_col.min(u16::MAX as usize) as u16;
        out.push(SearchMatch {
            start: ScreenPoint {
                row,
                col: start_col,
            },
            end: ScreenPoint { row, col: end_col },
        });
        offset = start_byte.saturating_add(needle.len().max(1));
        if offset >= haystack.len() {
            break;
        }
    }
    out
}

#[derive(Clone)]
struct FrozenFrame {
    top_screen_row: u32,
    frame: LgFrame,
    cells: Vec<FrozenCell>,
}

#[derive(Clone)]
struct FrozenCell {
    row: u16,
    col: u16,
    text: String,
    fg: Rgb,
    bg: Rgb,
    bg_is_default: bool,
}

struct FrozenVisitor {
    top_screen_row: u32,
    frame: Option<LgFrame>,
    cells: Vec<FrozenCell>,
}

impl FrozenVisitor {
    fn new(top_screen_row: u32) -> Self {
        Self {
            top_screen_row,
            frame: None,
            cells: Vec::new(),
        }
    }

    fn into_frame(self) -> Option<FrozenFrame> {
        self.frame.map(|frame| FrozenFrame {
            top_screen_row: self.top_screen_row,
            frame,
            cells: self.cells,
        })
    }
}

impl RenderVisitor for FrozenVisitor {
    fn begin(&mut self, frame: &LgFrame) {
        self.frame = Some(*frame);
        self.cells.clear();
    }

    fn cell(&mut self, cell: &CellView<'_>) {
        self.cells.push(FrozenCell {
            row: cell.row,
            col: cell.col,
            text: cell.text.to_string(),
            fg: cell.fg,
            bg: cell.bg,
            bg_is_default: cell.bg_is_default,
        });
    }

    fn cursor(&mut self, _cursor: &CursorPos) {}
    fn end(&mut self) {}
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
    b.selection = None;
    b.search = None;
    b.frozen_frame = None;
    drop(b);
    area.queue_draw();
}

struct TerminalContextMenu<'a> {
    area: &'a DrawingArea,
    inner: &'a Rc<RefCell<Inner>>,
    search: SearchUi<'a>,
    pane_id: PaneId,
    x: f64,
    y: f64,
    on_action: TerminalActionCallback,
    prefix: String,
}

fn open_terminal_context_menu(menu: TerminalContextMenu<'_>) {
    let popover = Popover::new();
    popover.set_has_arrow(true);
    popover.set_parent(menu.area);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(
        menu.x as i32,
        menu.y as i32,
        1,
        1,
    )));

    let body = GtkBox::new(Orientation::Vertical, 2);
    body.set_margin_top(6);
    body.set_margin_bottom(6);
    body.set_margin_start(6);
    body.set_margin_end(6);

    let inner_copy = menu.inner.clone();
    let popover_copy = popover.clone();
    body.append(&context_menu_button(
        "Copy",
        terminal_copy_shortcut(),
        move || {
            copy_selection_to_clipboard(&inner_copy);
            popover_copy.popdown();
        },
    ));

    let inner_paste = menu.inner.clone();
    let area_paste = menu.area.clone();
    let popover_paste = popover.clone();
    body.append(&context_menu_button(
        "Paste",
        terminal_paste_shortcut(),
        move || {
            request_paste_from_clipboard(&area_paste, &inner_paste);
            popover_paste.popdown();
        },
    ));

    let inner_find = menu.inner.clone();
    let area_find = menu.area.clone();
    let search_bar_find = menu.search.bar.clone();
    let search_entry_find = menu.search.entry.clone();
    let search_status_find = menu.search.status.clone();
    let popover_find = popover.clone();
    body.append(&context_menu_button(
        "Search",
        terminal_find_shortcut(),
        move || {
            popover_find.popdown();
            show_search_bar(
                &search_bar_find,
                &search_entry_find,
                &search_status_find,
                &area_find,
                &inner_find,
            );
        },
    ));

    append_separator(&body);

    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "Split right",
        &prefixed_shortcut(&menu.prefix, "|"),
        TerminalContextAction::SplitRight,
        menu.on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "Split down",
        &prefixed_shortcut(&menu.prefix, "-"),
        TerminalContextAction::SplitDown,
        menu.on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "New tab",
        &prefixed_shortcut(&menu.prefix, "t"),
        TerminalContextAction::NewTab,
        menu.on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "Close pane",
        &prefixed_shortcut(&menu.prefix, "x"),
        TerminalContextAction::ClosePane,
        menu.on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "New anchor",
        "",
        TerminalContextAction::NewAnchor,
        menu.on_action.clone(),
    );

    append_separator(&body);

    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "Next pane",
        &prefixed_shortcut(&menu.prefix, "o"),
        TerminalContextAction::NextPane,
        menu.on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "Previous pane",
        &prefixed_shortcut(&menu.prefix, "p"),
        TerminalContextAction::PreviousPane,
        menu.on_action.clone(),
    );
    append_action_button(
        &body,
        &popover,
        menu.pane_id,
        "Rearrange mode",
        &prefixed_shortcut(&menu.prefix, "m"),
        TerminalContextAction::ToggleRearrange,
        menu.on_action,
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

fn terminal_find_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+F"
    } else {
        "Ctrl+F"
    }
}

/// Save a clipboard image to a temp PNG and return the path. Tools
/// like `claude` cli accept the file path as an argument-like paste
/// and load the image themselves. Files land under
/// `$XDG_RUNTIME_DIR/lmux/pastes/` (or `/tmp/lmux-pastes-<uid>` if
/// the runtime dir is unset). The directory is kept private and filenames
/// use random UUIDs so clipboard contents are not exposed through predictable
/// paths.
fn write_clipboard_image(texture: &gdk::Texture) -> std::io::Result<std::path::PathBuf> {
    let dir = paste_dir();
    ensure_private_paste_dir(&dir)?;
    let filename = format!("paste-{}.png", uuid::Uuid::new_v4());
    let path = dir.join(filename);
    texture
        .save_to_png(&path)
        .map_err(|e| std::io::Error::other(format!("gdk_texture_save_to_png failed: {e}")))?;
    set_file_mode_0600(&path)?;
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

fn ensure_private_paste_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(dir)?;
    let meta = std::fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("paste path is not a directory: {}", dir.display()),
        ));
    }
    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "paste directory {} is owned by uid {}, expected {uid}",
                dir.display(),
                meta.uid()
            ),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        let mut perms = meta.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms)?;
    }
    Ok(())
}

fn set_file_mode_0600(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
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
        let frozen = b.frozen_frame.clone();
        let viewport_top = frozen
            .as_ref()
            .map(|frame| Some(frame.top_screen_row))
            .unwrap_or_else(|| b.term.viewport_top_screen_row());
        let selection = b
            .selection
            .and_then(|(s, e)| viewport_top.map(|top| Selection::new(s, e, top)));
        let search = viewport_top
            .map(|top| b.search_highlights(top))
            .unwrap_or_default();
        let exit = b.exit_code;
        let mut renderer =
            CairoRenderer::new(cr, &font, cell_w, cell_h, selection.as_ref(), &search, exit);
        if let Some(frozen) = frozen {
            renderer.begin(&frozen.frame);
            for cell in &frozen.cells {
                renderer.cell(&CellView {
                    row: cell.row,
                    col: cell.col,
                    text: &cell.text,
                    fg: cell.fg,
                    bg: cell.bg,
                    bg_is_default: cell.bg_is_default,
                });
            }
            renderer.end();
        } else {
            b.term.render(&mut renderer);
        }
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
    transcript: Rc<RefCell<TranscriptBuffer>>,
    bell_cb: Rc<RefCell<Option<BellCallback>>>,
    exit_cb: Rc<RefCell<Option<TerminalExitCallback>>>,
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
            transcript.borrow_mut().append_bytes(&bytes);
            let pty_output = {
                let mut inner = inner_chan.borrow_mut();
                inner.term.feed(&bytes);
                inner.term.take_pty_output()
            };
            if !pty_output.is_empty() {
                let write_result = inner_chan.borrow_mut().pty.writer().write_all(&pty_output);
                if let Err(err) = write_result {
                    tracing::warn!(error = %err, "pty response write failed");
                }
            }
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
                if let Some(cb) = exit_cb.borrow().clone() {
                    cb(id);
                }
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

    pub fn transcript_tail(
        &self,
        pane_id: uuid::Uuid,
        lines: u32,
    ) -> Result<lmux_bus::TranscriptRange, lmux_bus::BusError> {
        match self {
            Self::Terminal(t) => Ok(t.transcript_tail(pane_id, lines)),
            Self::Satellite(_) => Err(lmux_bus::BusError::TranscriptUnavailable(
                "pane is a GUI satellite; only PTY-backed terminal panes have transcript output"
                    .into(),
            )),
        }
    }

    pub fn transcript_capture(
        &self,
        pane_id: uuid::Uuid,
        since_sequence: Option<u64>,
        max_lines: Option<u32>,
    ) -> Result<lmux_bus::TranscriptRange, lmux_bus::BusError> {
        match self {
            Self::Terminal(t) => Ok(t.transcript_capture(pane_id, since_sequence, max_lines)),
            Self::Satellite(_) => Err(lmux_bus::BusError::TranscriptUnavailable(
                "pane is a GUI satellite; only PTY-backed terminal panes have transcript output"
                    .into(),
            )),
        }
    }

    pub fn send_input(&self, text: &str) -> Result<(), lmux_bus::BusError> {
        match self {
            Self::Terminal(t) => t.send_input(text).map_err(lmux_bus::BusError::Io),
            Self::Satellite(_) => Err(lmux_bus::BusError::TranscriptUnavailable(
                "pane is a GUI satellite; only PTY-backed terminal panes accept terminal input"
                    .into(),
            )),
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

    pub fn set_exit_callback(&self, cb: TerminalExitCallback) {
        if let Self::Terminal(t) = self {
            t.set_exit_callback(cb);
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
mod tests {
    use lmux_libghostty::ScreenPoint;

    use super::{
        count_matches_in_text, find_matches_in_line, find_matches_in_text, BellScanner,
        SearchMatch, TranscriptBuffer,
    };

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

    #[test]
    fn search_matcher_finds_case_insensitive_columns() {
        let matches = find_matches_in_line(7, "alpha Beta beta", "beta");

        assert_eq!(
            matches,
            vec![
                SearchMatch {
                    start: ScreenPoint { row: 7, col: 6 },
                    end: ScreenPoint { row: 7, col: 9 },
                },
                SearchMatch {
                    start: ScreenPoint { row: 7, col: 11 },
                    end: ScreenPoint { row: 7, col: 14 },
                },
            ]
        );
    }

    #[test]
    fn search_matcher_skips_empty_queries() {
        assert!(find_matches_in_line(1, "anything", "").is_empty());
    }

    #[test]
    fn search_count_reports_matches_across_scrollback_text() {
        let text = "alpha beta\nBETA gamma beta\nnothing";

        assert_eq!(count_matches_in_text(text, "beta"), 3);
        assert_eq!(count_matches_in_text(text, "missing"), 0);
    }

    #[test]
    fn search_index_preserves_scrollback_row_coordinates() {
        let matches = find_matches_in_text(40, "alpha\nbeta\nbeta", "beta");

        assert_eq!(
            matches
                .iter()
                .map(|m| (m.start.row, m.start.col))
                .collect::<Vec<_>>(),
            vec![(41, 0), (42, 0)]
        );
    }

    #[test]
    fn transcript_sequences_increase() {
        let pane_id = uuid::Uuid::new_v4();
        let mut t = TranscriptBuffer::new(10);
        t.append_line("one".into(), 1);
        t.append_line("two".into(), 2);

        let range = t.tail(pane_id, 10);

        assert_eq!(range.first_sequence, 1);
        assert_eq!(range.last_sequence, 2);
        assert_eq!(range.lines[0].text, "one");
        assert_eq!(range.lines[1].sequence, 2);
    }

    #[test]
    fn transcript_tail_limits_lines() {
        let pane_id = uuid::Uuid::new_v4();
        let mut t = TranscriptBuffer::new(10);
        for idx in 0..5 {
            t.append_line(format!("line {idx}"), idx);
        }

        let range = t.tail(pane_id, 2);

        assert_eq!(range.lines.len(), 2);
        assert_eq!(range.first_sequence, 4);
        assert_eq!(range.last_sequence, 5);
        assert_eq!(range.lines[0].text, "line 3");
    }

    #[test]
    fn transcript_capture_since_sequence() {
        let pane_id = uuid::Uuid::new_v4();
        let mut t = TranscriptBuffer::new(10);
        for idx in 0..5 {
            t.append_line(format!("line {idx}"), idx);
        }

        let range = t.capture_since(pane_id, Some(2), Some(2));

        assert_eq!(range.lines.len(), 2);
        assert_eq!(range.first_sequence, 3);
        assert_eq!(range.last_sequence, 4);
        assert!(!range.truncated);
    }

    #[test]
    fn transcript_reports_truncation() {
        let pane_id = uuid::Uuid::new_v4();
        let mut t = TranscriptBuffer::new(2);
        for idx in 0..5 {
            t.append_line(format!("line {idx}"), idx);
        }

        let range = t.capture_since(pane_id, Some(1), None);

        assert!(range.truncated);
        assert_eq!(range.first_sequence, 4);
        assert_eq!(range.last_sequence, 5);
        assert_eq!(range.lines.len(), 2);
    }
}
