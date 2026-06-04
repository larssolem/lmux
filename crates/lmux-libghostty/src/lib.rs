//! Safe Rust wrapper over libghostty-vt (ADR-0001 / NFR12).
//!
//! All `ghostty_*` C symbols live under `ffi` and are `pub(crate)`. Downstream
//! crates consume the safe types defined here — `Terminal`, `Rgb`,
//! `RenderVisitor`, etc. — and never touch the bindgen output.

pub(crate) mod ffi;

use std::ffi::c_void;
use std::ptr;

/// Version string reported by the linked-in libghostty-vt build.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 8-bit RGB triple. libghostty is sRGB internally; we just forward the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    fn from_ffi(c: ffi::GhosttyColorRgb) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

/// Frame-level properties emitted by [`Terminal::render`] before any cells.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cols: u16,
    pub rows: u16,
}

/// One cell in the visible grid. `text` may be empty (continuation of a
/// wide grapheme) or a short UTF-8 string (usually one scalar).
#[derive(Clone, Debug)]
pub struct CellView<'a> {
    pub row: u16,
    pub col: u16,
    pub text: &'a str,
    pub fg: Rgb,
    pub bg: Rgb,
    /// True when `bg` equals the frame's default background — the UI can skip
    /// the per-cell rectangle fill and rely on the window clear.
    pub bg_is_default: bool,
}

/// Terminal cursor position, in viewport coordinates.
#[derive(Clone, Copy, Debug)]
pub struct CursorPos {
    pub row: u16,
    pub col: u16,
    pub fg: Rgb,
}

/// A point in viewport coordinates. `row = 0` is the top row of the visible
/// viewport; `col = 0` is the leftmost column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportPoint {
    pub row: u16,
    pub col: u16,
}

/// A point in full-screen coordinates, including scrollback. `row = 0` is the
/// oldest retained row in the active screen buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPoint {
    pub row: u32,
    pub col: u16,
}

/// Sink for render events. `render()` calls `begin()` once, `cell()` for every
/// non-empty cell in row-major order, optionally `cursor()`, then `end()`.
pub trait RenderVisitor {
    fn begin(&mut self, frame: &Frame);
    fn cell(&mut self, cell: &CellView<'_>);
    fn cursor(&mut self, cursor: &CursorPos);
    fn end(&mut self);
}

/// Opaque wrapper around a libghostty VT terminal + its per-terminal render
/// machinery (render state, row iterator, cell iterator — all reused across
/// frames so we allocate once at `new()`).
pub struct Terminal {
    term: ffi::GhosttyTerminal,
    rs: ffi::GhosttyRenderState,
    iter: ffi::GhosttyRenderStateRowIterator,
    cells: ffi::GhosttyRenderStateRowCells,
    pty_output: Box<PtyOutput>,
    pending_osc_query: Vec<u8>,
    cols: u16,
    rows: u16,
}

struct PtyOutput {
    bytes: Vec<u8>,
}

const DEFAULT_FG: ffi::GhosttyColorRgb = ffi::GhosttyColorRgb {
    r: 238,
    g: 238,
    b: 238,
};
const DEFAULT_BG: ffi::GhosttyColorRgb = ffi::GhosttyColorRgb { r: 0, g: 0, b: 0 };
const DEFAULT_CURSOR: ffi::GhosttyColorRgb = ffi::GhosttyColorRgb {
    r: 238,
    g: 238,
    b: 238,
};
const MAX_PENDING_OSC_QUERY: usize = 64;

unsafe extern "C" fn write_pty_callback(
    _terminal: ffi::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    // SAFETY: `userdata` points at `Terminal::pty_output`, a stable heap
    // allocation. Ghostty invokes this synchronously during `feed`, while the
    // caller holds `&mut self`, so there is no concurrent mutation.
    let output = unsafe { &mut *(userdata.cast::<PtyOutput>()) };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    output.bytes.extend_from_slice(bytes);
}

unsafe extern "C" fn color_scheme_callback(
    _terminal: ffi::GhosttyTerminal,
    _userdata: *mut c_void,
    out_scheme: *mut ffi::GhosttyColorScheme,
) -> bool {
    if out_scheme.is_null() {
        return false;
    }
    unsafe {
        *out_scheme = ffi::GhosttyColorScheme_GHOSTTY_COLOR_SCHEME_DARK;
    }
    true
}

impl Terminal {
    /// Allocate a new VT terminal and its render pipeline. Returns `None` if
    /// libghostty refuses any of the sub-allocations.
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Option<Self> {
        // SAFETY: All pointers start null; we only assign to them via the
        // `_new` FFI entry points. Each check after a call guards against
        // libghostty returning a non-success status.
        unsafe {
            let mut term: ffi::GhosttyTerminal = ptr::null_mut();
            let opts = ffi::GhosttyTerminalOptions {
                cols,
                rows,
                max_scrollback,
            };
            let r = ffi::ghostty_terminal_new(ptr::null(), &mut term, opts);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || term.is_null() {
                return None;
            }
            if !set_terminal_color(
                term,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND,
                &DEFAULT_FG,
            ) || !set_terminal_color(
                term,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND,
                &DEFAULT_BG,
            ) || !set_terminal_color(
                term,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_CURSOR,
                &DEFAULT_CURSOR,
            ) {
                ffi::ghostty_terminal_free(term);
                return None;
            }

            let mut rs: ffi::GhosttyRenderState = ptr::null_mut();
            let r = ffi::ghostty_render_state_new(ptr::null(), &mut rs);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || rs.is_null() {
                ffi::ghostty_terminal_free(term);
                return None;
            }

            let mut iter: ffi::GhosttyRenderStateRowIterator = ptr::null_mut();
            let r = ffi::ghostty_render_state_row_iterator_new(ptr::null(), &mut iter);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || iter.is_null() {
                ffi::ghostty_render_state_free(rs);
                ffi::ghostty_terminal_free(term);
                return None;
            }

            let mut cells: ffi::GhosttyRenderStateRowCells = ptr::null_mut();
            let r = ffi::ghostty_render_state_row_cells_new(ptr::null(), &mut cells);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || cells.is_null() {
                ffi::ghostty_render_state_row_iterator_free(iter);
                ffi::ghostty_render_state_free(rs);
                ffi::ghostty_terminal_free(term);
                return None;
            }

            let mut pty_output = Box::new(PtyOutput { bytes: Vec::new() });
            let userdata = pty_output.as_mut() as *mut PtyOutput as *const c_void;
            let r = ffi::ghostty_terminal_set(
                term,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata,
            );
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                ffi::ghostty_render_state_row_cells_free(cells);
                ffi::ghostty_render_state_row_iterator_free(iter);
                ffi::ghostty_render_state_free(rs);
                ffi::ghostty_terminal_free(term);
                return None;
            }
            let r = ffi::ghostty_terminal_set(
                term,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                write_pty_callback as *const () as *const c_void,
            );
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                ffi::ghostty_render_state_row_cells_free(cells);
                ffi::ghostty_render_state_row_iterator_free(iter);
                ffi::ghostty_render_state_free(rs);
                ffi::ghostty_terminal_free(term);
                return None;
            }
            let r = ffi::ghostty_terminal_set(
                term,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_SCHEME,
                color_scheme_callback as *const () as *const c_void,
            );
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                ffi::ghostty_render_state_row_cells_free(cells);
                ffi::ghostty_render_state_row_iterator_free(iter);
                ffi::ghostty_render_state_free(rs);
                ffi::ghostty_terminal_free(term);
                return None;
            }

            Some(Self {
                term,
                rs,
                iter,
                cells,
                pty_output,
                pending_osc_query: Vec::new(),
                cols,
                rows,
            })
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Write PTY bytes into the VT. Safe wrapper around `ghostty_terminal_vt_write`.
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.respond_to_terminal_queries(bytes);
        // SAFETY: `bytes` comes from a valid slice with length `bytes.len()`.
        unsafe {
            ffi::ghostty_terminal_vt_write(self.term, bytes.as_ptr(), bytes.len());
        }
    }

    /// Return response bytes generated by terminal queries since the last
    /// call. The PTY owner should write these back to the child process.
    pub fn take_pty_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pty_output.bytes)
    }

    /// Tell the VT it was resized to `(cols, rows)` cells of `(cell_w_px, cell_h_px)` pixels.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_w_px: u32, cell_h_px: u32) {
        self.cols = cols;
        self.rows = rows;
        // SAFETY: `self.term` is non-null while `self` is alive.
        unsafe {
            ffi::ghostty_terminal_resize(self.term, cols, rows, cell_w_px, cell_h_px);
        }
    }

    /// Scroll the viewport by `delta` rows. Negative = up (into scrollback),
    /// positive = down (toward active area).
    pub fn scroll_delta(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        let mut value: ffi::GhosttyTerminalScrollViewportValue = unsafe { std::mem::zeroed() };
        value.delta = delta;
        let behavior = ffi::GhosttyTerminalScrollViewport {
            tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_DELTA,
            value,
        };
        // SAFETY: `self.term` is non-null while `self` is alive; `behavior` is
        // the tagged union the C API expects.
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(self.term, behavior);
        }
    }

    /// Extract the plain-text content of the viewport region bounded inclusively
    /// by `start` and `end`. Returns `None` if either point is outside the
    /// viewport or the formatter fails. Ordering of `start`/`end` is normalised.
    pub fn selection_text(&self, start: ViewportPoint, end: ViewportPoint) -> Option<String> {
        let (s, e) = normalise_range(start, end);
        // SAFETY: All FFI calls below operate on `self.term` (valid while
        // `self` is alive) plus stack-allocated sized structs. On success we
        // take ownership of a buffer allocated by libghostty and free it via
        // `ghostty_free` before returning.
        unsafe {
            let s_point = make_viewport_point(s);
            let e_point = make_viewport_point(e);

            let mut start_ref: ffi::GhosttyGridRef = std::mem::zeroed();
            start_ref.size = std::mem::size_of::<ffi::GhosttyGridRef>();
            let r = ffi::ghostty_terminal_grid_ref(self.term, s_point, &mut start_ref);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return None;
            }

            let mut end_ref: ffi::GhosttyGridRef = std::mem::zeroed();
            end_ref.size = std::mem::size_of::<ffi::GhosttyGridRef>();
            let r = ffi::ghostty_terminal_grid_ref(self.term, e_point, &mut end_ref);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return None;
            }

            let mut sel: ffi::GhosttySelection = std::mem::zeroed();
            sel.size = std::mem::size_of::<ffi::GhosttySelection>();
            sel.start = start_ref;
            sel.end = end_ref;
            sel.rectangle = false;

            let mut opts: ffi::GhosttyFormatterTerminalOptions = std::mem::zeroed();
            opts.size = std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>();
            opts.emit = ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_PLAIN;
            opts.trim = true;
            opts.selection = &sel;

            let mut formatter: ffi::GhosttyFormatter = ptr::null_mut();
            let r =
                ffi::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.term, opts);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || formatter.is_null() {
                return None;
            }

            let mut out_ptr: *mut u8 = ptr::null_mut();
            let mut out_len: usize = 0;
            let r = ffi::ghostty_formatter_format_alloc(
                formatter,
                ptr::null(),
                &mut out_ptr,
                &mut out_len,
            );
            ffi::ghostty_formatter_free(formatter);

            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || out_ptr.is_null() {
                return None;
            }

            let slice = std::slice::from_raw_parts(out_ptr, out_len);
            let text = String::from_utf8_lossy(slice).into_owned();
            ffi::ghostty_free(ptr::null(), out_ptr, out_len);
            Some(text)
        }
    }

    /// Convert a visible viewport point to a full-screen point. Screen points
    /// remain meaningful as the viewport scrolls, which makes them suitable
    /// for selections that span beyond the currently visible rows.
    pub fn screen_point_from_viewport(&self, point: ViewportPoint) -> Option<ScreenPoint> {
        // SAFETY: All FFI calls use `self.term`, valid for the lifetime of
        // `self`, and stack-allocated output structs.
        unsafe {
            let mut grid_ref: ffi::GhosttyGridRef = std::mem::zeroed();
            grid_ref.size = std::mem::size_of::<ffi::GhosttyGridRef>();
            let r = ffi::ghostty_terminal_grid_ref(
                self.term,
                make_viewport_point(point),
                &mut grid_ref,
            );
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return None;
            }

            let mut coord: ffi::GhosttyPointCoordinate = std::mem::zeroed();
            let r = ffi::ghostty_terminal_point_from_grid_ref(
                self.term,
                &grid_ref,
                ffi::GhosttyPointTag_GHOSTTY_POINT_TAG_SCREEN,
                &mut coord,
            );
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return None;
            }

            Some(ScreenPoint {
                row: coord.y,
                col: coord.x,
            })
        }
    }

    /// Screen row corresponding to viewport row zero.
    pub fn viewport_top_screen_row(&self) -> Option<u32> {
        self.screen_point_from_viewport(ViewportPoint { row: 0, col: 0 })
            .map(|p| p.row)
    }

    /// Extract text from a range expressed in full-screen coordinates.
    pub fn selection_text_screen(&self, start: ScreenPoint, end: ScreenPoint) -> Option<String> {
        let (s, e) = normalise_screen_range(start, end);
        // SAFETY: Mirrors `selection_text`, but resolves `ScreenPoint` values
        // instead of viewport-local points before handing the range to the
        // Ghostty formatter.
        unsafe {
            let s_point = make_screen_point(s);
            let e_point = make_screen_point(e);

            let mut start_ref: ffi::GhosttyGridRef = std::mem::zeroed();
            start_ref.size = std::mem::size_of::<ffi::GhosttyGridRef>();
            let r = ffi::ghostty_terminal_grid_ref(self.term, s_point, &mut start_ref);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return None;
            }

            let mut end_ref: ffi::GhosttyGridRef = std::mem::zeroed();
            end_ref.size = std::mem::size_of::<ffi::GhosttyGridRef>();
            let r = ffi::ghostty_terminal_grid_ref(self.term, e_point, &mut end_ref);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return None;
            }

            let mut sel: ffi::GhosttySelection = std::mem::zeroed();
            sel.size = std::mem::size_of::<ffi::GhosttySelection>();
            sel.start = start_ref;
            sel.end = end_ref;
            sel.rectangle = false;

            let mut opts: ffi::GhosttyFormatterTerminalOptions = std::mem::zeroed();
            opts.size = std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>();
            opts.emit = ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_PLAIN;
            opts.trim = true;
            opts.selection = &sel;

            let mut formatter: ffi::GhosttyFormatter = ptr::null_mut();
            let r =
                ffi::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.term, opts);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || formatter.is_null() {
                return None;
            }

            let mut out_ptr: *mut u8 = ptr::null_mut();
            let mut out_len: usize = 0;
            let r = ffi::ghostty_formatter_format_alloc(
                formatter,
                ptr::null(),
                &mut out_ptr,
                &mut out_len,
            );
            ffi::ghostty_formatter_free(formatter);

            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS || out_ptr.is_null() {
                return None;
            }

            let slice = std::slice::from_raw_parts(out_ptr, out_len);
            let text = String::from_utf8_lossy(slice).into_owned();
            ffi::ghostty_free(ptr::null(), out_ptr, out_len);
            Some(text)
        }
    }

    /// Query whether the shell has enabled bracketed-paste mode (DEC 2004).
    pub fn bracketed_paste_enabled(&self) -> bool {
        // Mode encoding (modes.h): bits 0..14 = value, bit 15 = ANSI flag. DEC
        // 2004 → ansi=false → plain 2004u16.
        const BRACKETED_PASTE_MODE: u16 = 2004;
        let mut out = false;
        // SAFETY: `self.term` is non-null while `self` is alive.
        unsafe {
            let r = ffi::ghostty_terminal_mode_get(self.term, BRACKETED_PASTE_MODE, &mut out);
            if r != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                return false;
            }
        }
        out
    }

    /// Snap the viewport to the bottom (re-engage follow mode).
    pub fn scroll_to_bottom(&mut self) {
        let value: ffi::GhosttyTerminalScrollViewportValue = unsafe { std::mem::zeroed() };
        let behavior = ffi::GhosttyTerminalScrollViewport {
            tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
            value,
        };
        // SAFETY: `self.term` is non-null while `self` is alive.
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(self.term, behavior);
        }
    }

    /// Drive one frame of rendering, forwarding cells and cursor to `visitor`.
    pub fn render(&mut self, visitor: &mut dyn RenderVisitor) {
        // SAFETY: Every FFI call below receives handles owned by `self` and
        // pointers into locals we hold across the call. The rs/iter/cells
        // objects are all allocated in `new` and freed in `drop`.
        unsafe {
            ffi::ghostty_render_state_update(self.rs, self.term);

            let mut colors: ffi::GhosttyRenderStateColors = std::mem::zeroed();
            colors.size = std::mem::size_of::<ffi::GhosttyRenderStateColors>();
            ffi::ghostty_render_state_colors_get(self.rs, &mut colors);

            let default_bg = colors.background;
            let default_fg = colors.foreground;

            visitor.begin(&Frame {
                background: Rgb::from_ffi(default_bg),
                foreground: Rgb::from_ffi(default_fg),
                cols: self.cols,
                rows: self.rows,
            });

            ffi::ghostty_render_state_get(
                self.rs,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                ptr::addr_of_mut!(self.iter).cast::<c_void>(),
            );

            let mut row_idx: u16 = 0;
            while ffi::ghostty_render_state_row_iterator_next(self.iter) {
                ffi::ghostty_render_state_row_get(
                    self.iter,
                    ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    ptr::addr_of_mut!(self.cells).cast::<c_void>(),
                );

                let mut col_idx: u16 = 0;
                while ffi::ghostty_render_state_row_cells_next(self.cells) {
                    let mut len: u32 = 0;
                    ffi::ghostty_render_state_row_cells_get(
                        self.cells,
                        ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                        ptr::addr_of_mut!(len).cast::<c_void>(),
                    );
                    let mut bg_cell = default_bg;
                    let bg_result = ffi::ghostty_render_state_row_cells_get(
                        self.cells,
                        ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
                        ptr::addr_of_mut!(bg_cell).cast::<c_void>(),
                    );
                    let bg_is_default = bg_result != ffi::GhosttyResult_GHOSTTY_SUCCESS
                        || rgb_eq(bg_cell, default_bg);

                    let mut fg_cell = default_fg;
                    let fg_result = ffi::ghostty_render_state_row_cells_get(
                        self.cells,
                        ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
                        ptr::addr_of_mut!(fg_cell).cast::<c_void>(),
                    );
                    if fg_result != ffi::GhosttyResult_GHOSTTY_SUCCESS {
                        fg_cell = default_fg;
                    }

                    let text = if len == 0 {
                        if bg_is_default {
                            col_idx += 1;
                            continue;
                        }
                        String::new()
                    } else {
                        let mut buf = [0u32; 16];
                        let n = std::cmp::min(len as usize, buf.len());
                        ffi::ghostty_render_state_row_cells_get(
                            self.cells,
                            ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                            buf.as_mut_ptr().cast::<c_void>(),
                        );
                        buf[..n].iter().filter_map(|c| char::from_u32(*c)).collect()
                    };

                    visitor.cell(&CellView {
                        row: row_idx,
                        col: col_idx,
                        text: &text,
                        fg: Rgb::from_ffi(fg_cell),
                        bg: Rgb::from_ffi(bg_cell),
                        bg_is_default,
                    });
                    col_idx += 1;
                }
                row_idx += 1;
            }

            let mut visible = false;
            ffi::ghostty_render_state_get(
                self.rs,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE,
                ptr::addr_of_mut!(visible).cast::<c_void>(),
            );
            let mut in_vp = false;
            ffi::ghostty_render_state_get(
                self.rs,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE,
                ptr::addr_of_mut!(in_vp).cast::<c_void>(),
            );
            if visible && in_vp {
                let mut cx: u16 = 0;
                let mut cy: u16 = 0;
                ffi::ghostty_render_state_get(
                    self.rs,
                    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X,
                    ptr::addr_of_mut!(cx).cast::<c_void>(),
                );
                ffi::ghostty_render_state_get(
                    self.rs,
                    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y,
                    ptr::addr_of_mut!(cy).cast::<c_void>(),
                );
                visitor.cursor(&CursorPos {
                    row: cy,
                    col: cx,
                    fg: Rgb::from_ffi(default_fg),
                });
            }

            let clean = ffi::GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_FALSE;
            ffi::ghostty_render_state_set(
                self.rs,
                ffi::GhosttyRenderStateOption_GHOSTTY_RENDER_STATE_OPTION_DIRTY,
                ptr::addr_of!(clean).cast::<c_void>(),
            );

            visitor.end();
        }
    }
}

impl Terminal {
    fn respond_to_terminal_queries(&mut self, bytes: &[u8]) {
        let mut scan = std::mem::take(&mut self.pending_osc_query);
        scan.extend_from_slice(bytes);

        let mut index = 0;
        while index < scan.len() {
            if scan[index] != b'\x1b' {
                index += 1;
                continue;
            }

            if index + 1 >= scan.len() {
                self.store_pending_osc_query(&scan[index..]);
                return;
            }

            if scan[index + 1] != b']' {
                index += 1;
                continue;
            }

            let body_start = index + 2;
            let Some((body_end, terminator_len)) = osc_body_end(&scan[body_start..]) else {
                self.store_pending_osc_query(&scan[index..]);
                return;
            };

            let body = &scan[body_start..body_start + body_end];
            self.respond_to_osc_query(body);
            index = body_start + body_end + terminator_len;
        }

        if scan.ends_with(b"\x1b]") {
            self.store_pending_osc_query(b"\x1b]");
        } else if scan.ends_with(b"\x1b") {
            self.store_pending_osc_query(b"\x1b");
        }
    }

    fn respond_to_osc_query(&mut self, body: &[u8]) {
        match body {
            b"10;?" => push_osc_color_response(&mut self.pty_output.bytes, 10, DEFAULT_FG),
            b"11;?" => push_osc_color_response(&mut self.pty_output.bytes, 11, DEFAULT_BG),
            b"12;?" => push_osc_color_response(&mut self.pty_output.bytes, 12, DEFAULT_CURSOR),
            _ => {}
        }
    }

    fn store_pending_osc_query(&mut self, bytes: &[u8]) {
        self.pending_osc_query.clear();
        if bytes.len() <= MAX_PENDING_OSC_QUERY {
            self.pending_osc_query.extend_from_slice(bytes);
        }
    }
}

fn osc_body_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\x07' => return Some((index, 1)),
            b'\x1b' if index + 1 < bytes.len() && bytes[index + 1] == b'\\' => {
                return Some((index, 2));
            }
            _ => index += 1,
        }
    }
    None
}

fn push_osc_color_response(output: &mut Vec<u8>, code: u8, color: ffi::GhosttyColorRgb) {
    output.extend_from_slice(
        format!(
            "\x1b]{code};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
            color.r, color.r, color.g, color.g, color.b, color.b
        )
        .as_bytes(),
    );
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: Each pointer is non-null on successful `new` and nulled only
        // here. Free order is reverse of allocation.
        unsafe {
            if !self.cells.is_null() {
                ffi::ghostty_render_state_row_cells_free(self.cells);
                self.cells = ptr::null_mut();
            }
            if !self.iter.is_null() {
                ffi::ghostty_render_state_row_iterator_free(self.iter);
                self.iter = ptr::null_mut();
            }
            if !self.rs.is_null() {
                ffi::ghostty_render_state_free(self.rs);
                self.rs = ptr::null_mut();
            }
            if !self.term.is_null() {
                ffi::ghostty_terminal_free(self.term);
                self.term = ptr::null_mut();
            }
        }
    }
}

fn normalise_range(a: ViewportPoint, b: ViewportPoint) -> (ViewportPoint, ViewportPoint) {
    if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn normalise_screen_range(a: ScreenPoint, b: ScreenPoint) -> (ScreenPoint, ScreenPoint) {
    if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn make_viewport_point(p: ViewportPoint) -> ffi::GhosttyPoint {
    let mut value: ffi::GhosttyPointValue = unsafe { std::mem::zeroed() };
    value.coordinate.x = p.col;
    value.coordinate.y = u32::from(p.row);
    ffi::GhosttyPoint {
        tag: ffi::GhosttyPointTag_GHOSTTY_POINT_TAG_VIEWPORT,
        value,
    }
}

fn make_screen_point(p: ScreenPoint) -> ffi::GhosttyPoint {
    let mut value: ffi::GhosttyPointValue = unsafe { std::mem::zeroed() };
    value.coordinate.x = p.col;
    value.coordinate.y = p.row;
    ffi::GhosttyPoint {
        tag: ffi::GhosttyPointTag_GHOSTTY_POINT_TAG_SCREEN,
        value,
    }
}

fn rgb_eq(a: ffi::GhosttyColorRgb, b: ffi::GhosttyColorRgb) -> bool {
    a.r == b.r && a.g == b.g && a.b == b.b
}

fn set_terminal_color(
    term: ffi::GhosttyTerminal,
    option: ffi::GhosttyTerminalOption,
    color: &ffi::GhosttyColorRgb,
) -> bool {
    let result = unsafe {
        ffi::ghostty_terminal_set(term, option, (color as *const ffi::GhosttyColorRgb).cast())
    };
    result == ffi::GhosttyResult_GHOSTTY_SUCCESS
}
