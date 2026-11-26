//! The terminal pane (ADR-0063): an agent's screen, as `agora-vt` keeps it, drawn into a ratatui [`Buffer`].
//!
//! The agent's own escape sequences never reach the user's terminal: `agora-vt` has already interpreted them, and this widget only
//! writes cells (a character and a style). Control characters that a program left in a cell are drawn as spaces.

use agora_vt::{Cell, Color as VtColor, Style as VtStyle, Terminal};
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
    /// The 256-colour palette without 24-bit colour (macOS Terminal, `TERM=xterm-256color` with no `COLORTERM`): a 24-bit colour is
    /// drawn as the nearest palette entry instead of whatever the terminal makes of it.
    Ansi256,
    /// The 16 classic colours: 256-colour and 24-bit colours are drawn as the nearest of them.
    Ansi16,
    /// No colour at all (`NO_COLOR`): attributes (bold, underline, inverse ...) stay.
    Mono,
}

impl ColorMode {
    /// The mode the environment asks for:
    /// - `NO_COLOR` set to anything turns colour off (https://no-color.org), and so does `TERM=dumb`;
    /// - `COLORTERM=truecolor` / `24bit` is 24-bit colour;
    /// - a `TERM` with `256color` in it is the 256-colour palette, any other `TERM` the 16 colours;
    /// - no `TERM` at all is a Windows console: 24-bit colour in a modern host that says so (`host`: Windows Terminal's
    ///   `WT_SESSION`, or a `TERM_PROGRAM`), else the classic console (conhost), whose 256-colour palette is not the xterm one (a
    ///   grey came out orange): the 16 colours there.
    pub fn from_env(
        no_color: Option<&str>,
        colorterm: Option<&str>,
        term: Option<&str>,
        host: Option<&str>,
    ) -> Self {
        if no_color.is_some() || term == Some("dumb") {
            return Self::Mono;
        }
        if colorterm.is_some_and(|c| c.contains("truecolor") || c.contains("24bit")) {
            return Self::Full;
        }
        match term {
            None | Some("") if host.is_some_and(|h| !h.is_empty()) => Self::Full,
            None | Some("") => Self::Ansi16,
            Some(t) if t.contains("256color") || t.contains("direct") => Self::Ansi256,
            Some(_) => Self::Ansi16,
        }
    }
}

/// A part of the agent's terminal the user selected, in the pane's rows and columns as drawn (scrolled back too: a row may show the
/// scrollback), both ends included.
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
    /// A theme's colours for what the program leaves to the terminal (ADR-0113); `None` draws them as the user's terminal does.
    palette: Option<&'a crate::theme::map::PanePalette>,
    selection: Option<Selection>,
    /// Lines scrolled back from the live bottom (the wheel): the screen's rows above the area first, then the scrollback.
    scroll: usize,
}

impl<'a> TerminalPane<'a> {
    pub fn new(term: &'a dyn Terminal) -> Self {
        Self {
            term,
            colors: ColorMode::Full,
            palette: None,
            selection: None,
            scroll: 0,
        }
    }

    #[must_use]
    pub fn colors(mut self, colors: ColorMode) -> Self {
        self.colors = colors;
        self
    }

    /// Draws the program's default colours and its 16 colours in a theme's palette (precomputed: a lookup per cell).
    #[must_use]
    pub fn palette(mut self, palette: Option<&'a crate::theme::map::PanePalette>) -> Self {
        self.palette = palette;
        self
    }

    #[must_use]
    pub fn selection(mut self, selection: Option<Selection>) -> Self {
        self.selection = selection;
        self
    }

    /// Draws `lines` back from the live bottom (clamped to what there is: [`max_scroll`](Self::max_scroll)).
    #[must_use]
    pub fn scroll(mut self, lines: usize) -> Self {
        self.scroll = lines;
        self
    }

    /// How far back the area can be scrolled: the screen's rows above it, then every line of scrollback.
    pub fn max_scroll(term: &dyn Terminal, height: u16) -> usize {
        usize::from(Self::first_row(term, height)) + term.history_len()
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

    /// `style` of the pane's cell at row `y`, column `x`, inverted when it is selected: inside text the agent already shows inverse
    /// (SGR 7) a selection turns inversion off, so its edges stay visible.
    fn selected(&self, style: Style, y: u16, x: u16) -> Style {
        if !self.selection.is_some_and(|s| s.contains(y, x)) {
            return style;
        }
        if let Some(p) = self.palette {
            return crate::theme::map::selected(style, p);
        }
        if style.add_modifier.contains(Modifier::REVERSED) {
            style.remove_modifier(Modifier::REVERSED)
        } else {
            style.add_modifier(Modifier::REVERSED)
        }
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

/// The cell a pane `height` rows tall, scrolled `back` lines, draws at its row `y`, column `col`: a screen row, or a line of the
/// scrollback when scrolled back far enough. One mapping for drawing and copying, so what is copied is what is shown.
fn shown_cell(term: &dyn Terminal, height: u16, back: usize, y: u16, col: u16) -> Option<Cell> {
    let first = TerminalPane::first_row(term, height);
    let back = back.min(TerminalPane::max_scroll(term, height));
    let shown = i64::from(first) + i64::from(y) - i64::try_from(back).unwrap_or(i64::MAX);
    if shown < 0 {
        term.history_cell(usize::try_from(-shown).unwrap_or(usize::MAX), col)
    } else {
        term.cell(u16::try_from(shown).ok()?, col)
    }
}

/// The text of a selection made in a pane `height` rows tall and scrolled `back` lines (in the pane's rows and columns): the
/// characters drawn from its start to its end in reading order, scrollback included, each line without its trailing blanks, lines
/// joined with `\n`. The right half of a wide character adds nothing.
pub fn selected_text(term: &dyn Terminal, height: u16, back: usize, sel: Selection) -> String {
    let (a, b) = if sel.from <= sel.to {
        (sel.from, sel.to)
    } else {
        (sel.to, sel.from)
    };
    let cols = term.size().cols;
    let mut lines = Vec::new();
    for row in a.0..=b.0 {
        let start = if row == a.0 { a.1 } else { 0 };
        let end = if row == b.0 {
            b.1
        } else {
            cols.saturating_sub(1)
        };
        let mut line = String::new();
        for col in start..=end.min(cols.saturating_sub(1)) {
            match shown_cell(term, height, back, row, col) {
                Some(c) if c.spacer => {}
                Some(c) if !c.ch.is_control() => line.push(c.ch),
                _ => line.push(' '),
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

/// The web address under the pane's row `row`, column `col` (a pane `height` rows tall, scrolled `back` lines), as drawn: `None` unless
/// it is a plain `http` or `https` address ([`crate::termio::safe_link`]). A program's own hyperlinks (OSC 8) are not kept by the
/// emulator, and an address broken over two rows is not joined: only what is written on the one row.
pub fn link_at(
    term: &dyn Terminal,
    height: u16,
    back: usize,
    row: u16,
    col: u16,
) -> Option<String> {
    let cols = term.size().cols;
    // One char per column; a wide character's right half and anything that is not printable break an address.
    let chars: Vec<char> = (0..cols)
        .map(|c| match shown_cell(term, height, back, row, c) {
            Some(cell) if !cell.spacer && !cell.ch.is_control() => cell.ch,
            _ => ' ',
        })
        .collect();
    crate::termio::link_in_row(&chars, usize::from(col))
}

impl Widget for TerminalPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let size = self.term.size();
        let first = Self::first_row(self.term, area.height);
        let back = self.scroll.min(Self::max_scroll(self.term, area.height));
        for y in 0..area.height {
            // Scrolled back: the rows above `first` are earlier screen rows, then (negative) the scrollback, newest first.
            let shown = i64::from(first) + i64::from(y) - i64::try_from(back).unwrap_or(i64::MAX);
            if shown < 0 {
                let line = usize::try_from(-shown).unwrap_or(usize::MAX);
                for x in 0..area.width {
                    let Some(out) = buf.cell_mut(Position::new(area.x + x, area.y + y)) else {
                        continue;
                    };
                    out.reset();
                    if let Some(p) = self.palette {
                        out.set_style(Style::default().fg(p.fg).bg(p.bg));
                    }
                    let Some(cell) = self.term.history_cell(line, x) else {
                        continue;
                    };
                    out.set_style(self.selected(
                        themed_style(cell.style, self.colors, self.palette),
                        y,
                        x,
                    ));
                    let ch = if cell.spacer || cell.ch.is_control() {
                        ' '
                    } else {
                        cell.ch
                    };
                    let mut b = [0u8; 4];
                    out.set_symbol(ch.encode_utf8(&mut b));
                }
                continue;
            }
            let row = u16::try_from(shown).unwrap_or(u16::MAX);
            for x in 0..area.width {
                let pos = Position::new(area.x + x, area.y + y);
                let Some(out) = buf.cell_mut(pos) else {
                    continue;
                };
                out.reset();
                if let Some(p) = self.palette {
                    out.set_style(Style::default().fg(p.fg).bg(p.bg));
                }
                if row >= size.rows || x >= size.cols {
                    continue;
                }
                let Some(cell) = self.term.cell(row, x) else {
                    continue;
                };
                out.set_style(self.selected(
                    themed_style(cell.style, self.colors, self.palette),
                    y,
                    x,
                ));
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
    themed_style(s, colors, None)
}

/// [`to_style`] under a theme's palette: the program's default colours and its 16 colours from the palette (never in `Mono`).
pub fn themed_style(
    s: VtStyle,
    colors: ColorMode,
    palette: Option<&crate::theme::map::PanePalette>,
) -> Style {
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
    match palette {
        None => style.fg(to_color(s.fg, colors)).bg(to_color(s.bg, colors)),
        Some(p) => style
            .fg(palette_color(s.fg, colors, p, p.fg))
            .bg(palette_color(s.bg, colors, p, p.bg)),
    }
}

/// A cell's colour under a palette: its default (`default`, the palette's text or background) and the 16 colours from the palette,
/// the rest as without one.
fn palette_color(
    c: VtColor,
    colors: ColorMode,
    p: &crate::theme::map::PanePalette,
    default: Color,
) -> Color {
    match c {
        VtColor::Default => default,
        VtColor::Indexed(i) if i < 16 => p.ansi[usize::from(i)],
        c => to_color(c, colors),
    }
}

/// A cell's colour in the given mode; [`VtColor::Default`] is the user's own terminal colour.
pub fn to_color(c: VtColor, colors: ColorMode) -> Color {
    match (c, colors) {
        (VtColor::Default, _) | (_, ColorMode::Mono) => Color::Reset,
        (VtColor::Indexed(i), ColorMode::Full | ColorMode::Ansi256) => Color::Indexed(i),
        (VtColor::Rgb(r, g, b), ColorMode::Full) => Color::Rgb(r, g, b),
        (VtColor::Rgb(r, g, b), ColorMode::Ansi256) => Color::Indexed(nearest256(r, g, b)),
        (VtColor::Indexed(i), ColorMode::Ansi16) if i < 16 => Color::Indexed(i),
        (VtColor::Indexed(i), ColorMode::Ansi16) => {
            let (r, g, b) = xterm_rgb(i);
            Color::Indexed(nearest16(r, g, b))
        }
        (VtColor::Rgb(r, g, b), ColorMode::Ansi16) => Color::Indexed(nearest16(r, g, b)),
    }
}

/// The RGB value xterm gives a 256-colour palette index (16-231 the 6x6x6 cube, 232-255 the grey ramp, below 16 the classic set).
pub(crate) fn xterm_rgb(i: u8) -> (u8, u8, u8) {
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

/// The nearest entry of xterm's 256-colour palette above the first 16 (whose look the user's theme decides): the 6×6×6 cube or the
/// grey ramp.
pub(crate) fn nearest256(r: u8, g: u8, b: u8) -> u8 {
    let d = |(x, y, z): (u8, u8, u8)| {
        let (dr, dg, db) = (
            i32::from(r) - i32::from(x),
            i32::from(g) - i32::from(y),
            i32::from(b) - i32::from(z),
        );
        dr * dr + dg * dg + db * db
    };
    (16..=255u8).min_by_key(|&i| d(xterm_rgb(i))).unwrap_or(16)
}

pub(crate) fn nearest16(r: u8, g: u8, b: u8) -> u8 {
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
        // The environment decides the mode (the review: 16 colours were never chosen).
        for (no_color, colorterm, term, host, want) in [
            (
                Some("1"),
                Some("truecolor"),
                Some("xterm-256color"),
                None,
                ColorMode::Mono,
            ),
            (None, None, Some("dumb"), None, ColorMode::Mono),
            (
                None,
                Some("truecolor"),
                Some("xterm-256color"),
                None,
                ColorMode::Full,
            ),
            (None, Some("24bit"), Some("xterm"), None, ColorMode::Full),
            (None, None, Some("xterm-256color"), None, ColorMode::Ansi256),
            (None, None, Some("tmux-256color"), None, ColorMode::Ansi256),
            (None, None, Some("xterm"), None, ColorMode::Ansi16),
            (None, None, Some("linux"), None, ColorMode::Ansi16),
            // No TERM: Windows. Windows Terminal (WT_SESSION) takes 24-bit; the classic console (conhost) the 16 colours.
            (None, None, None, Some("1f2c-…"), ColorMode::Full),
            (None, None, None, None, ColorMode::Ansi16),
        ] {
            assert_eq!(
                ColorMode::from_env(no_color, colorterm, term, host),
                want,
                "{no_color:?} {colorterm:?} {term:?} {host:?}"
            );
        }
        // Without 24-bit colour, a 24-bit colour is the nearest palette entry (never one of the theme's 16).
        assert_eq!(
            to_color(VtColor::Rgb(255, 135, 0), ColorMode::Ansi256),
            Color::Indexed(208)
        );
        assert_eq!(
            to_color(VtColor::Rgb(8, 8, 8), ColorMode::Ansi256),
            Color::Indexed(232)
        );
        assert_eq!(
            to_color(VtColor::Indexed(3), ColorMode::Ansi256),
            Color::Indexed(3)
        );
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
    fn the_selected_text_is_the_screen_as_drawn_in_reading_order() {
        let t = term(
            20,
            3,
            b"hello world\r\nsecond   \r\n\xe6\x9d\xb1\xe4\xba\xac ok",
        );
        // From "world" to "sec": the rest of the first row, then the start of the second, trailing blanks dropped.
        let sel = Selection {
            from: (0, 6),
            to: (1, 2),
        };
        assert_eq!(selected_text(&t, 3, 0, sel), "world\nsec");
        // Backwards (dragged up) is the same text.
        let back = Selection {
            from: sel.to,
            to: sel.from,
        };
        assert_eq!(selected_text(&t, 3, 0, back), "world\nsec");
        // A wide character is copied once, not with its right half.
        let wide = Selection {
            from: (2, 0),
            to: (2, 6),
        };
        assert_eq!(selected_text(&t, 3, 0, wide), "東京 ok");
        // A pane lower than the screen shows its bottom rows: pane row 0 is screen row 1 here.
        let low = Selection {
            from: (0, 0),
            to: (0, 5),
        };
        assert_eq!(selected_text(&t, 2, 0, low), "second");
    }

    #[test]
    fn a_scrolled_pane_shows_the_lines_that_scrolled_away() {
        let mut lines = String::new();
        for i in 1..=30 {
            lines.push_str(&format!("line {i}\r\n"));
        }
        let t = term(20, 5, lines.as_bytes());
        let draw = |back: usize| {
            let area = Rect::new(0, 0, 20, 5);
            let mut buf = Buffer::empty(area);
            TerminalPane::new(&t).scroll(back).render(area, &mut buf);
            (0..5).map(|y| line(&buf, y)).collect::<Vec<_>>()
        };
        // The live bottom: lines 27..30 and the empty row.
        assert_eq!(draw(0)[0], "line 27");
        // Three back: the three lines that scrolled off last come first.
        assert_eq!(
            draw(3),
            ["line 24", "line 25", "line 26", "line 27", "line 28"]
        );
        // As far back as there is, and no further.
        assert_eq!(TerminalPane::max_scroll(&t, 5), 26);
        assert_eq!(draw(26)[0], "line 1");
        assert_eq!(draw(500)[0], "line 1");
    }

    #[test]
    fn scrolled_back_the_selection_is_on_the_lines_shown_and_copies_them() {
        // #538 review: an agent printed 30 lines and the pane is scrolled 10 back. Each row of the pane, the scrollback's too, copies
        // what it shows, and the highlight sits on the rows dragged over.
        let mut lines = String::new();
        for i in 1..=30 {
            lines.push_str(&format!("line {i}\r\n"));
        }
        let t = term(20, 5, lines.as_bytes());
        let area = Rect::new(0, 0, 20, 5);
        for back in [0, 3, 10, 26] {
            let mut buf = Buffer::empty(area);
            let sel = Selection {
                from: (1, 0),
                to: (1, 19),
            };
            TerminalPane::new(&t)
                .scroll(back)
                .selection(Some(sel))
                .render(area, &mut buf);
            for y in 0..5 {
                let row = Selection {
                    from: (y, 0),
                    to: (y, 19),
                };
                assert_eq!(
                    selected_text(&t, 5, back, row),
                    line(&buf, y),
                    "{back} back, row {y}"
                );
            }
            let inv = |x: u16, y: u16| buf[(x, y)].modifier.contains(Modifier::REVERSED);
            assert!(
                inv(0, 1) && inv(5, 1) && !inv(0, 0) && !inv(0, 2),
                "{back} back"
            );
        }
        // Ten back, rows 0 to 2 show lines 17 to 19: what is dragged over is what is copied.
        let sel = Selection {
            from: (0, 0),
            to: (2, 6),
        };
        assert_eq!(selected_text(&t, 5, 10, sel), "line 17\nline 18\nline 19");
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
    fn a_selection_inside_inverse_text_turns_inversion_off_so_its_edges_show() {
        // "ab" plain, "cdef" inverse (SGR 7); the selection covers "bcd".
        let t = term(6, 1, b"ab\x1b[7mcdef\x1b[0m");
        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        TerminalPane::new(&t)
            .selection(Some(Selection {
                from: (0, 1),
                to: (0, 3),
            }))
            .render(area, &mut buf);
        let inv = |x: u16| buf[(x, 0)].modifier.contains(Modifier::REVERSED);
        assert_eq!(
            (0..6).map(inv).collect::<Vec<_>>(),
            vec![false, true, false, false, true, true]
        );
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

    #[test]
    fn a_link_is_read_from_the_cells_as_drawn() {
        let t = term(
            40,
            3,
            "go https://a.test/x. now\r\n\x1b[8mhttps://hidden.test\x1b[0m \u{6f22}\u{5b57} javascript:x\r\nhttp://u:p@a.test"
                .as_bytes(),
        );
        assert_eq!(link_at(&t, 3, 0, 0, 5).as_deref(), Some("https://a.test/x"));
        assert_eq!(link_at(&t, 3, 0, 0, 1), None);
        assert_eq!(link_at(&t, 3, 0, 0, 20), None);
        // Credentials, other schemes and a click off the text are not links.
        assert_eq!(link_at(&t, 3, 0, 2, 5), None);
        assert_eq!(link_at(&t, 3, 0, 1, 30), None);
        assert_eq!(link_at(&t, 3, 0, 9, 0), None);
    }
}
