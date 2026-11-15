//! Terminal emulator trait and backend. See ADR-0012.
//!
//! [`Terminal`] is the only thing the rest of Agora depends on; [`AlacrittyTerminal`] is the first backend.
//! Another backend can replace it if it passes the shared [`conformance`] tests.
//!
//! Everything here reads the **live screen**: the rows currently being drawn by the program. Scrolled-back
//! history is never returned, so state detection cannot be fooled by old output (AGENTS.md).

mod alacritty;
pub mod conformance;

pub use alacritty::AlacrittyTerminal;

/// Terminal dimensions in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// One screen cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character, or a space for an empty cell.
    pub ch: char,
    /// The right half of a double-width character; `ch` is a space.
    pub spacer: bool,
    /// How the program asked for it to be drawn (SGR), for clients that paint the screen themselves.
    pub style: Style,
}

/// A colour as the program set it. Clients map [`Color::Default`] and the 16 palette entries to their own theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// The terminal's own foreground or background.
    #[default]
    Default,
    /// A 256-colour palette index (0-15 are the classic and bright colours).
    Indexed(u8),
    /// 24-bit colour.
    Rgb(u8, u8, u8),
}

/// SGR attributes of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    /// Foreground and background swapped (SGR 7); the colours above are as set, not swapped.
    pub inverse: bool,
    pub strike: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// Terminal modes clients and detection care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    /// The alternate screen is active (full-screen programs).
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    /// Arrow keys send application sequences.
    pub application_cursor: bool,
}

impl Style {
    /// The SGR sequence that draws this style from a reset state (`ESC [ 0 ; … m`): attributes, then the palette or 24-bit colours.
    pub fn sgr(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("\x1b[0");
        for (on, code) in [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.inverse, "7"),
            (self.hidden, "8"),
            (self.strike, "9"),
        ] {
            if on {
                out.push(';');
                out.push_str(code);
            }
        }
        let mut color = |c: Color, base: u8, bright: u8, ext: u8| match c {
            Color::Default => {}
            Color::Indexed(i) if i < 8 => {
                let _ = write!(out, ";{}", base + i);
            }
            Color::Indexed(i) if i < 16 => {
                let _ = write!(out, ";{}", bright + i - 8);
            }
            Color::Indexed(i) => {
                let _ = write!(out, ";{ext};5;{i}");
            }
            Color::Rgb(r, g, b) => {
                let _ = write!(out, ";{ext};2;{r};{g};{b}");
            }
        };
        color(self.fg, 30, 90, 38);
        color(self.bg, 40, 100, 48);
        out.push('m');
        out
    }
}

pub trait Terminal: Send {
    /// Process bytes written by the program. Never panics on malformed input.
    fn feed(&mut self, bytes: &[u8]);
    /// Change the screen size. A backend may keep a minimum size; [`size`](Terminal::size) reports the size it has.
    fn resize(&mut self, size: Size);
    fn size(&self) -> Size;
    /// The cell at `row`, `col` of the live screen (0-based, top-left origin); `None` if out of range.
    fn cell(&self, row: u16, col: u16) -> Option<Cell>;
    fn cursor(&self) -> Cursor;
    /// The style the program's next printed character gets (its current SGR state), so a repaint can hand it on.
    fn pen(&self) -> Style;
    fn modes(&self) -> Modes;
    /// Window title set by the program (OSC 0/2), if any.
    fn title(&self) -> Option<String>;

    /// Lines kept above the live screen (scrollback), 0 when the backend keeps none. For showing a user what scrolled away
    /// (the terminal client's wheel); detection never reads it (it reads the live bottom only).
    fn history_len(&self) -> usize {
        0
    }

    /// The cell at `col` of the scrollback line `back` rows above the live screen's top (1 = the line that scrolled off last,
    /// [`history_len`](Self::history_len) = the oldest kept); `None` out of range.
    fn history_cell(&self, back: usize, col: u16) -> Option<Cell> {
        let _ = (back, col);
        None
    }

    /// Text of a scrollback line with trailing blanks removed (see [`history_cell`](Self::history_cell)).
    fn history_line(&self, back: usize) -> String {
        let mut s = String::new();
        for col in 0..self.size().cols {
            match self.history_cell(back, col) {
                Some(Cell { spacer: true, .. }) | None => {}
                Some(c) => s.push(c.ch),
            }
        }
        s.truncate(s.trim_end().len());
        s
    }

    /// Text of one live-screen row with trailing blanks removed.
    fn line(&self, row: u16) -> String {
        let cols = self.size().cols;
        let mut s = String::new();
        for col in 0..cols {
            match self.cell(row, col) {
                Some(Cell { spacer: true, .. }) | None => {}
                Some(c) => s.push(c.ch),
            }
        }
        s.truncate(s.trim_end().len());
        s
    }

    /// The last `n` rows of the live screen, top to bottom (fewer if the screen is smaller).
    fn bottom_lines(&self, n: usize) -> Vec<String> {
        let rows = usize::from(self.size().rows);
        (rows.saturating_sub(n)..rows)
            .map(|r| self.line(r as u16))
            .collect()
    }

    /// The last `n` non-empty rows of the live screen, top to bottom.
    fn bottom_non_empty_lines(&self, n: usize) -> Vec<String> {
        let mut v: Vec<String> = (0..self.size().rows)
            .rev()
            .map(|r| self.line(r))
            .filter(|l| !l.is_empty())
            .take(n)
            .collect();
        v.reverse();
        v
    }
}
