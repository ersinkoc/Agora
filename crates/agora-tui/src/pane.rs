//! The terminal pane (ADR-0063): an agent's screen, as `agora-vt` keeps it, drawn into a ratatui [`Buffer`].
//!
//! The agent's own escape sequences never reach the user's terminal: `agora-vt` has already interpreted them, and this widget only
//! writes cells (a character and a style). Control characters that a program left in a cell are drawn as spaces.

use agora_vt::{Color as VtColor, Style as VtStyle, Terminal};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// How many colours the user's terminal can show (ADR-0063 point 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// 24-bit colour and the 256-colour palette, as the agent asked.
    #[default]
    Full,
    /// The 16 classic colours: 256-colour and 24-bit colours are drawn as the nearest of them.
    Ansi16,
    /// No colour at all (`NO_COLOR`): attributes (bold, underline, inverse ...) stay.
    Mono,
}

impl ColorMode {
    /// The mode the environment asks for: `NO_COLOR` set to anything turns colour off (https://no-color.org).
    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>) -> Self {
        match (no_color, colorterm) {
            (Some(_), _) => Self::Mono,
            (None, Some(c)) if c.contains("truecolor") || c.contains("24bit") => Self::Full,
            _ => Self::Full,
        }
    }
}

/// A rectangular part of the agent's screen the user selected, in screen rows and columns, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub from: (u16, u16),
    pub to: (u16, u16),
}

impl Selection {
    /// Whether the cell at `row`, `col` is selected. The selection runs from `from` to `to` in reading order (whole lines between).
    pub fn contains(&self, row: u16, col: u16) -> bool {
        let (a, b) = if self.from <= self.to {
            (self.from, self.to)
        } else {
            (self.to, self.from)
        };
        (row, col) >= a && (row, col) <= b
    }
}

/// An agent's screen drawn into an area. Rows of the screen that do not fit are cut so that the cursor's row stays in view.
pub struct TerminalPane<'a> {
    term: &'a dyn Terminal,
    colors: ColorMode,
    selection: Option<Selection>,
}

impl<'a> TerminalPane<'a> {
    pub fn new(term: &'a dyn Terminal) -> Self {
        Self {
            term,
            colors: ColorMode::Full,
            selection: None,
        }
    }

    #[must_use]
    pub fn colors(mut self, colors: ColorMode) -> Self {
        self.colors = colors;
        self
    }

    #[must_use]
    pub fn selection(mut self, selection: Option<Selection>) -> Self {
        self.selection = selection;
        self
    }

    /// The first screen row drawn in an area `height` rows tall: 0 when the screen fits, else what keeps the cursor's row visible.
    pub fn first_row(term: &dyn Terminal, height: u16) -> u16 {
        let rows = term.size().rows;
        if rows <= height {
            return 0;
        }
        let below = term.cursor().row.saturating_add(1);
        below.saturating_sub(height).min(rows - height)
    }

    /// Where the agent's cursor is inside `area`, when it is visible and on a drawn cell (for `Frame::set_cursor_position`).
    pub fn cursor_position(term: &dyn Terminal, area: Rect) -> Option<Position> {
        let c = term.cursor();
        let first = Self::first_row(term, area.height);
        if !c.visible || c.row < first || c.row - first >= area.height || c.col >= area.width {
            return None;
        }
        Some(Position::new(area.x + c.col, area.y + (c.row - first)))
    }
}

impl Widget for TerminalPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let size = self.term.size();
        let first = Self::first_row(self.term, area.height);
        for y in 0..area.height {
            let row = first + y;
            for x in 0..area.width {
                let pos = Position::new(area.x + x, area.y + y);
                let Some(out) = buf.cell_mut(pos) else {
                    continue;
                };
                out.reset();
                if row >= size.rows || x >= size.cols {
                    continue;
                }
                let Some(cell) = self.term.cell(row, x) else {
                    continue;
                };
                let mut style = to_style(cell.style, self.colors);
                if self.selection.is_some_and(|s| s.contains(row, x)) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                out.set_style(style);
                if cell.spacer {
                    // The right half of a wide character: ratatui skips it when it draws the left half.
                    out.set_symbol(" ");
                    continue;
                }
                let wide = self.term.cell(row, x + 1).is_some_and(|n| n.spacer);
                // A wide character whose right half falls outside the area is drawn as a space, never cut in half.
                let ch = if (wide && x + 1 >= area.width) || cell.ch.is_control() {
                    ' '
                } else {
                    cell.ch
                };
                let mut b = [0u8; 4];
                out.set_symbol(ch.encode_utf8(&mut b));
            }
        }
    }
}

/// The ratatui style for a cell's SGR attributes.
pub fn to_style(s: VtStyle, colors: ColorMode) -> Style {
    let mut m = Modifier::empty();
    for (on, flag) in [
        (s.bold, Modifier::BOLD),
        (s.dim, Modifier::DIM),
        (s.italic, Modifier::ITALIC),
        (s.underline, Modifier::UNDERLINED),
        (s.inverse, Modifier::REVERSED),
        (s.strike, Modifier::CROSSED_OUT),
        (s.hidden, Modifier::HIDDEN),
    ] {
        if on {
            m |= flag;
        }
    }
    let style = Style::default().add_modifier(m);
    if colors == ColorMode::Mono {
        return style;
    }
    style.fg(to_color(s.fg, colors)).bg(to_color(s.bg, colors))
}

/// A cell's colour in the given mode; [`VtColor::Default`] is the user's own terminal colour.
pub fn to_color(c: VtColor, colors: ColorMode) -> Color {
    match (c, colors) {
        (VtColor::Default, _) | (_, ColorMode::Mono) => Color::Reset,
        (VtColor::Indexed(i), ColorMode::Full) => Color::Indexed(i),
        (VtColor::Rgb(r, g, b), ColorMode::Full) => Color::Rgb(r, g, b),
        (VtColor::Indexed(i), ColorMode::Ansi16) if i < 16 => Color::Indexed(i),
        (VtColor::Indexed(i), ColorMode::Ansi16) => {
            let (r, g, b) = xterm_rgb(i);
            Color::Indexed(nearest16(r, g, b))
        }
        (VtColor::Rgb(r, g, b), ColorMode::Ansi16) => Color::Indexed(nearest16(r, g, b)),
    }
}

/// The RGB value xterm gives a 256-colour palette index (16-231 the 6x6x6 cube, 232-255 the grey ramp, below 16 the classic set).
fn xterm_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => ANSI16[usize::from(i)],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    }
}

/// xterm's default values of the 16 classic colours, the targets of the 16-colour mode.
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn nearest16(r: u8, g: u8, b: u8) -> u8 {
    let d = |(x, y, z): (u8, u8, u8)| {
        let (dr, dg, db) = (
            i32::from(r) - i32::from(x),
            i32::from(g) - i32::from(y),
            i32::from(b) - i32::from(z),
        );
        dr * dr + dg * dg + db * db
    };
    // 16 entries: the index always fits in a u8.
    (0..16u8)
        .min_by_key(|&i| d(ANSI16[usize::from(i)]))
        .unwrap_or(7)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use agora_vt::{AlacrittyTerminal, Size};

    fn term(cols: u16, rows: u16, bytes: &[u8]) -> AlacrittyTerminal {
        let mut t = AlacrittyTerminal::new(Size { cols, rows });
        t.feed(bytes);
        t
    }

    fn draw(t: &dyn Terminal, w: u16, h: u16, colors: ColorMode) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        TerminalPane::new(t).colors(colors).render(area, &mut buf);
        buf
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn text_colours_and_attributes_reach_the_buffer() {
        let t = term(
            20,
            3,
            b"\x1b[1;31mred\x1b[0m \x1b[38;5;208mor\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m\r\n\x1b[3;4;7mx\x1b[9;2;8my\x1b[0m",
        );
        let buf = draw(&t, 20, 3, ColorMode::Full);
        assert_eq!(line(&buf, 0), "red or rgb");
        let red = &buf[(0, 0)];
        assert_eq!(red.fg, Color::Indexed(1));
        assert!(red.modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(4, 0)].fg, Color::Indexed(208));
        assert_eq!(buf[(7, 0)].fg, Color::Rgb(1, 2, 3));
        assert_eq!(
            buf[(3, 0)].fg,
            Color::Reset,
            "the default colour is the user's own"
        );
        let x = buf[(0, 1)].modifier;
        assert!(x.contains(Modifier::ITALIC | Modifier::UNDERLINED | Modifier::REVERSED));
        let y = buf[(1, 1)].modifier;
        assert!(y.contains(Modifier::CROSSED_OUT | Modifier::DIM | Modifier::HIDDEN));
    }

    #[test]
    fn the_colour_modes_degrade_but_keep_the_attributes() {
        let t = term(10, 1, b"\x1b[1;38;2;250;0;0;48;5;21mA");
        let full = draw(&t, 10, 1, ColorMode::Full);
        assert_eq!(full[(0, 0)].fg, Color::Rgb(250, 0, 0));
        let ansi = draw(&t, 10, 1, ColorMode::Ansi16);
        assert_eq!(ansi[(0, 0)].fg, Color::Indexed(9), "bright red");
        assert_eq!(
            ansi[(0, 0)].bg,
            Color::Indexed(4),
            "palette 21 (0,0,255) is nearest to blue (0,0,238)"
        );
        let mono = draw(&t, 10, 1, ColorMode::Mono);
        assert_eq!(
            (mono[(0, 0)].fg, mono[(0, 0)].bg),
            (Color::Reset, Color::Reset)
        );
        assert!(mono[(0, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(
            ColorMode::from_env(Some("1"), Some("truecolor")),
            ColorMode::Mono
        );
        assert_eq!(ColorMode::from_env(None, None), ColorMode::Full);
    }

    #[test]
    fn wide_characters_take_two_cells_and_are_never_cut_in_half() {
        let t = term(10, 1, "a\u{6f22}b".as_bytes());
        let buf = draw(&t, 10, 1, ColorMode::Full);
        assert_eq!(buf[(1, 0)].symbol(), "\u{6f22}");
        assert_eq!(buf[(3, 0)].symbol(), "b");
        assert_eq!(
            line(&buf, 0),
            "a\u{6f22} b",
            "the right half is a blank ratatui skips when it draws"
        );
        // A screen wider than the area: the wide character whose right half falls outside is drawn as a space.
        let wide = term(12, 1, "123456789\u{6f22}".as_bytes());
        let buf = draw(&wide, 10, 1, ColorMode::Full);
        assert_eq!(line(&buf, 0), "123456789");
        assert_eq!(buf[(9, 0)].symbol(), " ");
    }

    #[test]
    fn the_cursor_row_stays_in_view_and_its_position_is_reported() {
        let mut bytes = Vec::new();
        for i in 0..10 {
            bytes.extend_from_slice(format!("line {i}\r\n").as_bytes());
        }
        bytes.extend_from_slice(b"$ ls");
        let t = term(20, 12, &bytes);
        let buf = draw(&t, 20, 4, ColorMode::Full);
        assert_eq!(
            line(&buf, 3),
            "$ ls",
            "the bottom of the screen, where the cursor is"
        );
        assert_eq!(line(&buf, 0), "line 7");
        let area = Rect::new(5, 2, 20, 4);
        assert_eq!(
            TerminalPane::cursor_position(&t, area),
            Some(Position::new(9, 5))
        );
        let hidden = term(20, 2, b"\x1b[?25lx");
        assert_eq!(
            TerminalPane::cursor_position(&hidden, Rect::new(0, 0, 20, 2)),
            None
        );
    }

    #[test]
    fn escape_sequences_from_the_agent_never_reach_the_buffer_as_text() {
        // A title change, a hyperlink and a cursor move are interpreted by agora-vt; only their visible text is drawn.
        let t = term(
            30,
            2,
            b"\x1b]0;owned\x07\x1b]8;;https://x\x07link\x1b]8;;\x07\x1b[2;1Hnext\x07",
        );
        let buf = draw(&t, 30, 2, ColorMode::Full);
        assert_eq!(line(&buf, 0), "link");
        assert_eq!(line(&buf, 1), "next");
        for y in 0..2 {
            for x in 0..30 {
                assert!(
                    !buf[(x, y)].symbol().chars().any(char::is_control),
                    "a control character at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn a_selection_is_drawn_inverted_in_reading_order() {
        let t = term(6, 3, b"abcdef\r\nghijkl\r\nmnopqr");
        let area = Rect::new(0, 0, 6, 3);
        let mut buf = Buffer::empty(area);
        TerminalPane::new(&t)
            .selection(Some(Selection {
                from: (2, 1),
                to: (0, 4),
            }))
            .render(area, &mut buf);
        let inv = |x: u16, y: u16| buf[(x, y)].modifier.contains(Modifier::REVERSED);
        assert!(!inv(3, 0) && inv(4, 0) && inv(5, 0));
        assert!((0..6).all(|x| inv(x, 1)), "the whole line between");
        assert!(inv(0, 2) && inv(1, 2) && !inv(2, 2));
    }

    #[test]
    fn a_screen_smaller_than_the_area_leaves_the_rest_blank() {
        let t = term(3, 1, b"abc");
        let area = Rect::new(0, 0, 6, 3);
        let mut buf = Buffer::filled(area, ratatui::buffer::Cell::new("#"));
        TerminalPane::new(&t).render(area, &mut buf);
        assert_eq!(line(&buf, 0), "abc");
        assert_eq!(line(&buf, 1), "");
    }

    /// The inputs of `agora_vt::conformance` (text, cursor moves, clearing, the alternate screen, wide characters, styles, malformed
    /// input): for each, every cell the pane draws is the cell `agora-vt` holds, with its style.
    #[test]
    fn the_pane_draws_exactly_what_agora_vt_holds() {
        let inputs: &[&[u8]] = &[
            b"hello\r\nworld",
            b"abc\x1b[3;7Hxy",
            b"one\r\ntwo\x1b[2J\x1b[Hthree",
            b"main\x1b[?1049halt screen\x1b[?1049l",
            "wide \u{6f22}\u{5b57} ok".as_bytes(),
            b"\x1b[1;31mred\x1b[0m \x1b[4;38;5;99mu\x1b[0m\x1b[48;2;9;8;7mbg\x1b[0m",
            b"\x1b[999;999H\x1b[;;;mX\x1b[38;2m\x1b]junk\x07\xff\xfeok",
            b"\ttab\x08\x08b\r\n\x1b[2Kcleared",
        ];
        for bytes in inputs {
            let t = term(16, 4, bytes);
            let buf = draw(&t, 16, 4, ColorMode::Full);
            for row in 0..4 {
                for col in 0..16 {
                    let cell = t.cell(row, col).unwrap();
                    let out = &buf[(col, row)];
                    let want = if cell.spacer || cell.ch.is_control() {
                        " ".to_string()
                    } else {
                        cell.ch.to_string()
                    };
                    let ctx = format!("{:?} at {row},{col}", String::from_utf8_lossy(bytes));
                    assert_eq!(out.symbol(), want, "{ctx}");
                    let style = to_style(cell.style, ColorMode::Full);
                    assert_eq!(
                        (out.fg, out.bg, out.modifier),
                        (style.fg.unwrap(), style.bg.unwrap(), style.add_modifier),
                        "{ctx}"
                    );
                }
            }
        }
    }
}
