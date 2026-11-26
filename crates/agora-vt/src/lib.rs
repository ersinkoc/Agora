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
    /// The shape the program set (DECSCUSR), or [`CursorShape::Default`] when it never set one or set the default, so a window keeps
    /// the user's own cursor in that case (ADR-0100).
    pub shape: CursorShape,
    /// The cursor blinks (DECSCUSR's odd values, or mode 12).
    pub blink: bool,
}

/// The scroll region the program set (DECSTBM, `CSI top ; bottom r`): rows `top..=bottom` of the live screen, 0-based. The whole
/// screen when the program set none. Under origin mode (DECOM) cursor positions are relative to `top` (ADR-0100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margins {
    pub top: u16,
    pub bottom: u16,
}

/// The cursor's shape as the program set it (DECSCUSR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// The program never set one, or set the default (`CSI 0 SP q`): the window's own cursor.
    #[default]
    Default,
    Block,
    Underline,
    Bar,
}

/// Which pointer events the program asked to be told about (DECSET 1000, 1002, 1003; one at a time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseTracking {
    #[default]
    Off,
    /// 1000: presses, releases and the wheel.
    Click,
    /// 1002: and motion with a button down.
    Drag,
    /// 1003: and motion with no button.
    Motion,
}

/// How the program wants mouse reports written (DECSET 1005, 1006; none is the classic one-byte form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseEncoding {
    #[default]
    Default,
    /// 1005.
    Utf8,
    /// 1006.
    Sgr,
}

/// Terminal modes the program set: what clients, detection and the node's repaint (ADR-0100) need. Plain data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// The alternate screen is active (full-screen programs; 1049).
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    /// Arrow keys send application sequences (DECCKM).
    pub application_cursor: bool,
    /// The keypad sends application sequences (DECKPAM; DECKPNM is off).
    pub application_keypad: bool,
    pub mouse_tracking: MouseTracking,
    pub mouse_encoding: MouseEncoding,
    /// 1004: the program is told when it gains or loses the keyboard.
    pub focus_reporting: bool,
    /// 1007: the wheel sends arrow keys on the alternate screen.
    pub alternate_scroll: bool,
    /// DECAWM (on by default).
    pub autowrap: bool,
    /// DECOM: cursor positions are relative to the scroll region.
    pub origin: bool,
    /// IRM: printed characters push the rest of the row right.
    pub insert: bool,
    /// LNM: a line feed also returns the carriage.
    pub newline: bool,
    /// The kitty keyboard protocol's current flags (`CSI > / < / = u`), 0 when off.
    pub keyboard_flags: u8,
}

impl Default for Modes {
    /// Everything off but autowrap (xterm's power-on modes). Not every backend starts so: `AlacrittyTerminal` starts with
    /// alternate scroll (1007) on, so the repaint of a plain shell says `CSI ? 1007 h`.
    fn default() -> Self {
        Self {
            alt_screen: false,
            bracketed_paste: false,
            application_cursor: false,
            application_keypad: false,
            mouse_tracking: MouseTracking::Off,
            mouse_encoding: MouseEncoding::Default,
            focus_reporting: false,
            alternate_scroll: false,
            autowrap: true,
            origin: false,
            insert: false,
            newline: false,
            keyboard_flags: 0,
        }
    }
}

impl Style {
    /// The SGR sequence that draws this style from a reset state (`ESC [ 0 ; … m`): attributes, then the palette or 24-bit colours.
    pub fn sgr(&self) -> String {
        let mut out = String::new();
        self.append_sgr(&mut out);
        out
    }

    /// Append this style's SGR sequence to a repaint buffer.
    pub fn append_sgr(&self, out: &mut String) {
        use std::fmt::Write as _;
        out.push_str("\x1b[0");
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
    }
}

pub trait Terminal: Send {
    /// Process bytes written by the program. Never panics on malformed input.
    fn feed(&mut self, bytes: &[u8]);
    /// Change the screen size. A backend may keep a minimum size; [`size`](Terminal::size) reports the size it has.
    fn resize(&mut self, size: Size);
    /// The size [`resize`](Terminal::resize) would really give the screen for `size` (a backend may keep a minimum). Whoever also
    /// resizes something tied to the screen, such as the process's terminal, asks this first so both have the same size.
    fn clamp_size(&self, size: Size) -> Size {
        size
    }
    fn size(&self) -> Size;
    /// The cell at `row`, `col` of the live screen (0-based, top-left origin); `None` if out of range.
    fn cell(&self, row: u16, col: u16) -> Option<Cell>;
    fn cursor(&self) -> Cursor;
    /// The style the program's next printed character gets (its current SGR state), so a repaint can hand it on.
    fn pen(&self) -> Style;
    fn modes(&self) -> Modes;
    /// The scroll region (ADR-0100, M7.10), so a repaint can restore it and origin mode with it; `None` for a backend that cannot say
    /// (then a repaint leaves both alone). `&mut` because a backend may have to look for it: [`AlacrittyTerminal`] keeps it private
    /// and finds it by moving the cursor under origin mode, then puts the cursor and the mode back exactly as they were.
    fn margins(&mut self) -> Option<Margins> {
        None
    }
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

    /// Whether the scrollback line `back` rows above the live screen's top continues on the line below it (the program's text ran
    /// past the right edge and the terminal wrapped it), so a history replay (ADR-0100) writes it without a line break and the window
    /// can rewrap it on a resize. `false` out of range, and for a backend that does not know.
    fn history_wrapped(&self, back: usize) -> bool {
        let _ = back;
        false
    }

    /// Lines kept above the screen that is **not** active: the main screen's scrollback while the program is on the alternate screen
    /// (ADR-0100 decision 2). A history replay reads this, [`inactive_history_cell`](Self::inactive_history_cell) and
    /// [`inactive_history_wrapped`](Self::inactive_history_wrapped) while the alternate screen is active, since the alternate screen has
    /// no history of its own. 0 for a backend that cannot read the inactive screen ([`AlacrittyTerminal`]: `alacritty_terminal` keeps
    /// it private), so a program on the alternate screen gets no history replay there.
    fn inactive_history_len(&self) -> usize {
        0
    }

    /// Like [`history_cell`](Self::history_cell), on the inactive screen's scrollback (see
    /// [`inactive_history_len`](Self::inactive_history_len)).
    fn inactive_history_cell(&self, back: usize, col: u16) -> Option<Cell> {
        let _ = (back, col);
        None
    }

    /// Like [`history_wrapped`](Self::history_wrapped), on the inactive screen's scrollback (see
    /// [`inactive_history_len`](Self::inactive_history_len)).
    fn inactive_history_wrapped(&self, back: usize) -> bool {
        let _ = back;
        false
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

#[cfg(test)]
mod style_tests {
    use super::{Color, Style};

    #[test]
    fn append_sgr_preserves_the_prefix_and_emits_a_complete_style() {
        let style = Style {
            fg: Color::Rgb(24, 120, 250),
            bg: Color::Indexed(13),
            underline: true,
            inverse: true,
            ..Style::default()
        };
        let mut repaint = String::from("prefix");
        style.append_sgr(&mut repaint);
        Style::default().append_sgr(&mut repaint);

        assert_eq!(repaint, "prefix\x1b[0;4;7;38;2;24;120;250;105m\x1b[0m");
        assert_eq!(style.sgr(), "\x1b[0;4;7;38;2;24;120;250;105m");
    }
}
