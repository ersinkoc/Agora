//! Conformance tests every [`Terminal`] backend must pass. Call [`run_all`] from a backend's tests.

use crate::{Size, Terminal};

fn size(cols: u16, rows: u16) -> Size {
    Size { cols, rows }
}

/// Run every conformance test against terminals built by `make`.
pub fn run_all<T: Terminal>(make: impl Fn(Size) -> T) {
    text_and_newlines(&make);
    cursor_movement(&make);
    clear_screen(&make);
    resize_updates_size_and_keeps_in_range_cells(&make);
    bottom_lines_never_include_scrollback(&make);
    bottom_non_empty_lines_skip_blank_rows(&make);
    alt_screen_is_separate_and_restores(&make);
    title_from_osc(&make);
    wide_characters_use_two_cells(&make);
    modes_toggle(&make);
    styles_are_kept_per_cell(&make);
    styling_sequences_print_nothing(&make);
    malformed_input_does_not_panic(&make);
}

fn text_and_newlines<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 5));
    t.feed(b"hello\r\nworld");
    assert_eq!(t.line(0), "hello");
    assert_eq!(t.line(1), "world");
    assert_eq!(t.cell(0, 0).map(|c| c.ch), Some('h'));
    assert_eq!(t.cell(0, 19).map(|c| c.ch), Some(' '));
    assert_eq!(t.cell(5, 0), None, "row out of range");
    assert_eq!(t.cell(0, 20), None, "column out of range");
}

fn cursor_movement<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 5));
    t.feed(b"abc");
    assert_eq!((t.cursor().row, t.cursor().col), (0, 3));
    t.feed(b"\x1b[3;7H"); // row 3, column 7 (1-based)
    assert_eq!((t.cursor().row, t.cursor().col), (2, 6));
    t.feed(b"\x1b[?25l");
    assert!(!t.cursor().visible);
    t.feed(b"\x1b[?25h");
    assert!(t.cursor().visible);
}

fn clear_screen<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(10, 3));
    t.feed(b"one\r\ntwo");
    t.feed(b"\x1b[2J\x1b[H");
    assert!(t.bottom_lines(3).iter().all(String::is_empty));
    assert_eq!((t.cursor().row, t.cursor().col), (0, 0));
}

fn resize_updates_size_and_keeps_in_range_cells<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 5));
    t.feed(b"abc");
    t.resize(size(40, 10));
    assert_eq!(t.size(), size(40, 10));
    assert!(t.cell(9, 39).is_some());
    assert_eq!(t.cell(10, 0), None);
    t.resize(size(10, 3));
    assert_eq!(t.size(), size(10, 3));
    assert!(t.cell(3, 0).is_none());
    assert_eq!(t.line(0).chars().next(), Some('a'));
}

fn bottom_lines_never_include_scrollback<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 5));
    for i in 1..=100 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    let bottom = t.bottom_lines(5);
    assert_eq!(bottom.len(), 5);
    // 100 lines were printed; only the last ones are still on the live screen.
    assert!(
        bottom
            .iter()
            .all(|l| l.is_empty() || l.starts_with("line 9") || l == "line 100"),
        "{bottom:?}"
    );
    assert!(!bottom.iter().any(|l| l == "line 1" || l == "line 50"));
    // Asking for more rows than the screen has returns the screen, not history.
    assert_eq!(t.bottom_lines(50).len(), 5);
    assert_eq!(t.bottom_lines(2), bottom[3..].to_vec());
}

fn bottom_non_empty_lines_skip_blank_rows<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 8));
    t.feed(b"first\r\n\r\nsecond\r\n\r\n");
    assert_eq!(
        t.bottom_non_empty_lines(5),
        vec!["first".to_string(), "second".to_string()]
    );
    assert_eq!(t.bottom_non_empty_lines(1), vec!["second".to_string()]);
}

fn alt_screen_is_separate_and_restores<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 4));
    t.feed(b"main screen");
    t.feed(b"\x1b[?1049h");
    assert!(t.modes().alt_screen);
    assert_eq!(t.line(0), "", "alternate screen starts blank");
    t.feed(b"\x1b[Hfull screen app"); // the cursor keeps its column across the switch, so home first
    assert_eq!(t.line(0), "full screen app");
    t.feed(b"\x1b[?1049l");
    assert!(!t.modes().alt_screen);
    assert_eq!(t.line(0), "main screen");
}

fn title_from_osc<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 4));
    assert_eq!(t.title(), None);
    t.feed(b"\x1b]0;my title\x07");
    assert_eq!(t.title().as_deref(), Some("my title"));
    t.feed(b"\x1b]2;other\x1b\\");
    assert_eq!(t.title().as_deref(), Some("other"));
}

fn wide_characters_use_two_cells<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(10, 2));
    t.feed("日本a".as_bytes());
    assert_eq!(t.cell(0, 0).map(|c| (c.ch, c.spacer)), Some(('日', false)));
    assert_eq!(t.cell(0, 1).map(|c| c.spacer), Some(true));
    assert_eq!(t.cell(0, 2).map(|c| c.ch), Some('本'));
    assert_eq!(t.cell(0, 4).map(|c| c.ch), Some('a'));
    assert_eq!(t.line(0), "日本a");
    assert_eq!(t.cursor().col, 5);
}

fn modes_toggle<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(10, 2));
    let m = t.modes();
    assert!(!m.bracketed_paste && !m.application_cursor);
    t.feed(b"\x1b[?2004h\x1b[?1h");
    let m = t.modes();
    assert!(m.bracketed_paste && m.application_cursor);
    t.feed(b"\x1b[?2004l\x1b[?1l");
    let m = t.modes();
    assert!(!m.bracketed_paste && !m.application_cursor);
}

fn styles_are_kept_per_cell<T: Terminal>(make: &impl Fn(Size) -> T) {
    use crate::{Color, Style};
    let mut t = make(size(20, 2));
    t.feed(
        b"\x1b[1;3;4;31mA\x1b[0m\x1b[38;5;208;48;2;10;20;30mB\x1b[0m\x1b[2;7;9mC\x1b[0mD\x1b[91mE",
    );
    let st = |c: u16| t.cell(0, c).map(|x| x.style).unwrap_or_default();
    assert_eq!(
        st(0),
        Style {
            fg: Color::Indexed(1),
            bold: true,
            italic: true,
            underline: true,
            ..Style::default()
        }
    );
    assert_eq!(
        st(1),
        Style {
            fg: Color::Indexed(208),
            bg: Color::Rgb(10, 20, 30),
            ..Style::default()
        }
    );
    assert_eq!(
        st(2),
        Style {
            dim: true,
            inverse: true,
            strike: true,
            ..Style::default()
        }
    );
    assert_eq!(st(3), Style::default());
    assert_eq!(st(4).fg, Color::Indexed(9), "bright red is palette 9");
}

fn styling_sequences_print_nothing<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(20, 2));
    t.feed(b"\x1b[1;31mred\x1b[0m plain\x1b[38;2;1;2;3m!");
    assert_eq!(t.line(0), "red plain!");
}

fn malformed_input_does_not_panic<T: Terminal>(make: &impl Fn(Size) -> T) {
    let mut t = make(size(10, 3));
    t.feed(&[
        0xff, 0xfe, 0x1b, 0x1b, b'[', 0x80, b']', 0x07, 0x9b, b'9', b'9', b'9', b'9', b'9', b'H',
    ]);
    t.feed(b"\x1b[99999;99999H\x1b[99999A\x1b[99999B\x1b[0;0r\x1b[?9999h");
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..200 {
        let bytes: Vec<u8> = (0..64)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        t.feed(&bytes);
    }
    // Still usable and in range afterwards.
    let c = t.cursor();
    assert!(c.row < t.size().rows && c.col < t.size().cols);
    t.resize(size(5, 2));
    t.feed(b"ok");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AlacrittyTerminal;

    #[test]
    fn alacritty_backend_passes_conformance() {
        run_all(AlacrittyTerminal::new);
    }
}
