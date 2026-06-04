#![allow(clippy::expect_used)]

use lmux_libghostty::{CellView, CursorPos, Frame, RenderVisitor, Rgb, Terminal};

#[derive(Default)]
struct Capture {
    frame: Option<Frame>,
    cells: Vec<(String, Rgb, Rgb, bool)>,
    cursor: Option<CursorPos>,
}

impl RenderVisitor for Capture {
    fn begin(&mut self, frame: &Frame) {
        self.frame = Some(*frame);
    }

    fn cell(&mut self, cell: &CellView<'_>) {
        self.cells
            .push((cell.text.to_string(), cell.fg, cell.bg, cell.bg_is_default));
    }

    fn cursor(&mut self, cursor: &CursorPos) {
        self.cursor = Some(*cursor);
    }

    fn end(&mut self) {}
}

fn render_bytes(bytes: &[u8]) -> Capture {
    let mut term = Terminal::new(20, 5, 100).expect("terminal allocates");
    term.feed(bytes);
    let mut capture = Capture::default();
    term.render(&mut capture);
    capture
}

fn cell(capture: &Capture, text: &str) -> (Rgb, Rgb, bool) {
    capture
        .cells
        .iter()
        .find(|(t, _, _, _)| t == text)
        .map(|(_, fg, bg, bg_default)| (*fg, *bg, *bg_default))
        .unwrap_or_else(|| panic!("missing cell {text:?}: {:?}", capture.cells))
}

#[test]
fn render_resolves_basic_ansi_foreground_colors() {
    let capture = render_bytes(b"\x1b[31mR\x1b[32mG\x1b[34mB\x1b[0mD");

    let frame = capture.frame.expect("frame");
    let (red, _, _) = cell(&capture, "R");
    let (green, _, _) = cell(&capture, "G");
    let (blue, _, _) = cell(&capture, "B");
    let (default, _, _) = cell(&capture, "D");

    assert_ne!(red, frame.foreground);
    assert_ne!(green, frame.foreground);
    assert_ne!(blue, frame.foreground);
    assert_eq!(default, frame.foreground);
    assert_ne!(red, green);
    assert_ne!(red, blue);
    assert_ne!(green, blue);
}

#[test]
fn render_resolves_256_color_and_truecolor_foregrounds() {
    let capture = render_bytes(b"\x1b[38;5;208mP\x1b[38;2;12;34;56mT");

    let (palette, _, _) = cell(&capture, "P");
    let (truecolor, _, _) = cell(&capture, "T");

    assert_ne!(palette, truecolor);
    assert_eq!(
        truecolor,
        Rgb {
            r: 12,
            g: 34,
            b: 56
        }
    );
}

#[test]
fn render_resolves_background_and_inverse_colors() {
    let capture = render_bytes(b"\x1b[38;2;1;2;3;48;2;4;5;6mN\x1b[7mI");

    let frame = capture.frame.expect("frame");
    let (normal_fg, normal_bg, normal_bg_default) = cell(&capture, "N");
    let (inverse_fg, inverse_bg, inverse_bg_default) = cell(&capture, "I");

    assert_eq!(normal_fg, Rgb { r: 1, g: 2, b: 3 });
    assert_eq!(normal_bg, Rgb { r: 4, g: 5, b: 6 });
    assert!(!normal_bg_default);

    assert_ne!(inverse_fg, frame.foreground);
    assert_ne!(inverse_bg, frame.background);
    assert!(!inverse_bg_default);
}

#[test]
fn render_includes_background_only_cells() {
    let capture = render_bytes(b"\x1b[48;2;9;8;7m\x1b[K");

    assert!(
        capture
            .cells
            .iter()
            .any(|(text, _, bg, bg_default)| text.is_empty()
                && *bg == Rgb { r: 9, g: 8, b: 7 }
                && !*bg_default),
        "missing background-only cell: {:?}",
        capture.cells
    );
}

#[test]
fn terminal_queries_emit_pty_responses() {
    let mut term = Terminal::new(20, 5, 100).expect("terminal allocates");

    term.feed(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[?996n\x1b[6n");
    let output = term.take_pty_output();
    let output_text = String::from_utf8_lossy(&output);

    assert!(
        output_text.contains("]10;"),
        "missing OSC 10 response: {output:?}"
    );
    assert!(
        output_text.contains("]11;"),
        "missing OSC 11 response: {output:?}"
    );
    assert!(
        output_text.contains("]10;rgb:eeee/eeee/eeee"),
        "foreground response should include the configured default color: {output:?}"
    );
    assert!(
        output_text.contains("]11;rgb:0000/0000/0000"),
        "background response should include the configured default color: {output:?}"
    );
    assert!(
        output_text.contains("[?997;1n"),
        "missing dark color-scheme response: {output:?}"
    );
    assert!(
        output_text.contains("[1;1R"),
        "missing cursor-position response: {output:?}"
    );
}

#[test]
fn split_terminal_color_queries_are_answered() {
    let mut term = Terminal::new(20, 5, 100).expect("terminal allocates");

    term.feed(b"\x1b]10");
    assert!(term.take_pty_output().is_empty());

    term.feed(b";?\x1b\\");
    let output = term.take_pty_output();
    let output_text = String::from_utf8_lossy(&output);

    assert!(
        output_text.contains("]10;rgb:eeee/eeee/eeee"),
        "missing split OSC 10 response: {output:?}"
    );
}
