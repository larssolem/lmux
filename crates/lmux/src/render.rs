use gtk4::cairo;
use gtk4::pango;
use lmux_libghostty::{CellView, CursorPos, Frame, RenderVisitor, Rgb, ViewportPoint};

/// A selection range, already normalised so `start <= end` in row-major order.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub start: ViewportPoint,
    pub end: ViewportPoint,
}

impl Selection {
    pub fn new(a: ViewportPoint, b: ViewportPoint) -> Self {
        let (start, end) = if (a.row, a.col) <= (b.row, b.col) {
            (a, b)
        } else {
            (b, a)
        };
        Self { start, end }
    }

    fn contains(&self, row: u16, col: u16) -> bool {
        let pos = (row, col);
        let s = (self.start.row, self.start.col);
        let e = (self.end.row, self.end.col);
        pos >= s && pos <= e
    }
}

pub struct CairoRenderer<'a> {
    cr: &'a cairo::Context,
    layout: pango::Layout,
    cell_w: f64,
    cell_h: f64,
    selection: Option<&'a Selection>,
    exit_code: Option<i32>,
    cols: u16,
    frame: Option<Frame>,
}

impl<'a> CairoRenderer<'a> {
    pub fn new(
        cr: &'a cairo::Context,
        font: &pango::FontDescription,
        cell_w: f64,
        cell_h: f64,
        selection: Option<&'a Selection>,
        exit_code: Option<i32>,
        cols: u16,
    ) -> Self {
        let pctx = pangocairo::functions::create_context(cr);
        // Match `measure_cell` — snap glyph advance to the pixel grid so
        // characters land on integer x positions.
        if let Ok(mut opts) = cairo::FontOptions::new() {
            opts.set_hint_metrics(cairo::HintMetrics::On);
            opts.set_hint_style(cairo::HintStyle::Slight);
            opts.set_antialias(cairo::Antialias::Subpixel);
            pangocairo::functions::context_set_font_options(&pctx, Some(&opts));
        }
        let layout = pango::Layout::new(&pctx);
        layout.set_font_description(Some(font));
        Self {
            cr,
            layout,
            cell_w,
            cell_h,
            selection,
            exit_code,
            cols,
            frame: None,
        }
    }
}

fn to_rgb(c: Rgb) -> (f64, f64, f64) {
    (
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
    )
}

impl RenderVisitor for CairoRenderer<'_> {
    fn begin(&mut self, frame: &Frame) {
        let (r, g, b) = to_rgb(frame.background);
        self.cr.set_source_rgb(r, g, b);
        let _ = self.cr.paint();
        self.frame = Some(*frame);
    }

    fn cell(&mut self, cell: &CellView<'_>) {
        let x = f64::from(cell.col) * self.cell_w;
        let y = f64::from(cell.row) * self.cell_h;
        let selected = self
            .selection
            .map(|s| s.contains(cell.row, cell.col))
            .unwrap_or(false);

        if selected {
            self.cr.set_source_rgba(0.27, 0.50, 0.78, 0.45);
            self.cr.rectangle(x, y, self.cell_w, self.cell_h);
            let _ = self.cr.fill();
        } else if !cell.bg_is_default {
            let (r, g, b) = to_rgb(cell.bg);
            self.cr.set_source_rgb(r, g, b);
            self.cr.rectangle(x, y, self.cell_w, self.cell_h);
            let _ = self.cr.fill();
        }

        let (r, g, b) = to_rgb(cell.fg);
        self.cr.set_source_rgb(r, g, b);
        self.layout.set_text(cell.text);
        self.cr.move_to(x, y);
        pangocairo::functions::show_layout(self.cr, &self.layout);
    }

    fn cursor(&mut self, cursor: &CursorPos) {
        let x = f64::from(cursor.col) * self.cell_w;
        let y = f64::from(cursor.row) * self.cell_h;
        let (r, g, b) = to_rgb(cursor.fg);
        self.cr.set_source_rgba(r, g, b, 0.5);
        self.cr.rectangle(x, y, self.cell_w, self.cell_h);
        let _ = self.cr.fill();
    }

    fn end(&mut self) {
        if let Some(code) = self.exit_code {
            self.draw_exit_banner(code);
        }
    }
}

impl CairoRenderer<'_> {
    fn draw_exit_banner(&self, code: i32) {
        let Some(frame) = self.frame else {
            return;
        };
        let rows = frame.rows;
        if rows == 0 {
            return;
        }
        let y = f64::from(rows - 1) * self.cell_h;
        let width = f64::from(self.cols.max(1)) * self.cell_w;
        self.cr.set_source_rgb(0.55, 0.08, 0.12);
        self.cr.rectangle(0.0, y, width, self.cell_h);
        let _ = self.cr.fill();
        self.cr.set_source_rgb(1.0, 1.0, 1.0);
        self.layout.set_text(&format!("exited: code {code}"));
        self.cr.move_to(self.cell_w * 0.5, y);
        pangocairo::functions::show_layout(self.cr, &self.layout);
    }
}
