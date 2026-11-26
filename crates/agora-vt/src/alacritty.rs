//! Backend on `alacritty_terminal` (ADR-0012).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AColor, CursorShape as AShape, CursorStyle, Handler, NamedColor, NamedPrivateMode,
    PrivateMode, Processor,
};

use crate::{
    Cell, Color, Cursor, CursorShape, Margins, Modes, MouseEncoding, MouseTracking, Size, Style,
    Terminal,
};

/// Scrolled-back lines kept per terminal. Memory is bounded by this (design/resource-limits.md).
const SCROLLBACK: usize = 2000;

/// The fewest columns the screen has. A double-width character needs two cells: on a one-column grid `alacritty_terminal`
/// indexes past the row when one is printed and never finishes reflowing one on resize (F701).
const MIN_COLS: u16 = 2;

/// Longest OSC payload, in bytes, the parser is given: the bound `vte` itself keeps when built without `std` (`MAX_OSC_RAW`).
/// Built with `std`, as `alacritty_terminal` does, it keeps a payload whole until its terminator, so an OSC that never ends
/// would grow the terminal's memory with every byte, and a long title would be copied up to 4096 times onto the title stack
/// (`CSI 22 t`). Bytes past the bound are left out; the sequence still ends at its terminator.
const MAX_OSC: usize = 1024;

/// The backend's cursor style while the program has set none (see `with_scrollback`).
const DEFAULT_CURSOR: CursorStyle = CursorStyle {
    shape: AShape::HollowBlock,
    blinking: false,
};

/// What the OSC bound needs to know of the byte stream; it follows `vte`'s states for `ESC`, `ESC ]` and the OSC string.
#[derive(Clone, Copy, Default)]
enum Scan {
    #[default]
    Text,
    Escape,
    /// Inside an OSC, with this many payload bytes given to the parser.
    Osc(usize),
    /// After `ESC [`: `>` makes it a kitty keyboard push (`CSI > flags u`), `<` a pop (`CSI < n u`).
    Csi,
    /// Inside `ESC [ >`, reading the parameters.
    Push,
    /// Inside `ESC [ <`, reading the count (digits only: a pop in any other form is not counted, which only overstates the depth).
    Pop(u16),
}

/// Whether the private mode list at the start of `rest` (after `ESC [ ?`) may name 1049, the one mode the backend switches screens
/// for. Over-approximates (any number 1049, sub-parameters included) and says yes when the list runs past the chunk.
fn may_name_1049(rest: &[u8]) -> bool {
    let mut n: u32 = 0;
    for &b in rest {
        match b {
            b'0'..=b'9' => n = n.saturating_mul(10).saturating_add(u32::from(b - b'0')),
            b';' | b':' => {
                if n == 1049 {
                    return true;
                }
                n = 0;
            }
            // Executed or ignored inside the sequence by `vte`, which stays in it.
            0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff => {}
            _ => return n == 1049,
        }
    }
    true
}

/// A kitty keyboard stack change the scan found.
#[derive(Clone, Copy)]
enum KeyStack {
    Push,
    Pop(u16),
}

/// The depth a kitty keyboard stack may reach before the backend's stacks are emptied (see `depth`). `alacritty_terminal` 0.26 panics
/// on a push once a stack holds 4096 entries (it evicts from the title stack, which may be empty), so a program, or a file it
/// prints, could stop the terminal with 4096 `CSI > u`. Below that bound, and far above the depth any program uses.
const PUSH_LIMIT: usize = 1024;

#[derive(Clone, Default)]
struct Listener {
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let new = match event {
            Event::Title(t) => Some(Some(t)),
            Event::ResetTitle => Some(None),
            _ => None,
        };
        if let (Some(new), Ok(mut slot)) = (new, self.title.lock()) {
            *slot = new;
        }
    }
}

struct Dims(Size);

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        usize::from(self.0.rows)
    }
    fn screen_lines(&self) -> usize {
        usize::from(self.0.rows)
    }
    fn columns(&self) -> usize {
        usize::from(self.0.cols)
    }
}

/// The backend's colour as the program set it: the 16 named colours are palette entries 0-15, its dim variants the plain ones (dim is
/// carried by [`Style::dim`]), and the terminal's own foreground, background and cursor colours are [`Color::Default`].
fn color(c: AColor) -> Color {
    match c {
        AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(i) => Color::Indexed(i),
        AColor::Named(n) => {
            let i = n as usize;
            if i < 16 {
                Color::Indexed(i as u8)
            } else {
                match n {
                    NamedColor::DimBlack => Color::Indexed(0),
                    NamedColor::DimRed => Color::Indexed(1),
                    NamedColor::DimGreen => Color::Indexed(2),
                    NamedColor::DimYellow => Color::Indexed(3),
                    NamedColor::DimBlue => Color::Indexed(4),
                    NamedColor::DimMagenta => Color::Indexed(5),
                    NamedColor::DimCyan => Color::Indexed(6),
                    NamedColor::DimWhite => Color::Indexed(7),
                    _ => Color::Default,
                }
            }
        }
    }
}

/// A backend cell's (or the cursor template's) colours and flags as a [`Style`].
fn style(fg: AColor, bg: AColor, f: Flags) -> Style {
    Style {
        fg: color(fg),
        bg: color(bg),
        bold: f.contains(Flags::BOLD),
        dim: f.contains(Flags::DIM),
        italic: f.contains(Flags::ITALIC),
        underline: f.intersects(Flags::ALL_UNDERLINES),
        inverse: f.contains(Flags::INVERSE),
        strike: f.contains(Flags::STRIKEOUT),
        hidden: f.contains(Flags::HIDDEN),
    }
}

pub struct AlacrittyTerminal {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    size: Size,
    scan: Scan,
    /// The backend's settings, to empty the keyboard stacks with (see [`PUSH_LIMIT`]).
    config: Config,
    /// Entries on each screen's kitty keyboard stack (main, alternate), as the backend keeps one per screen: a push adds one and a
    /// pop removes its count (floored at 0) on the screen the backend itself is on when it applies them (read from the backend, so
    /// whatever switches its screen is followed exactly); a full reset empties both. Never fewer than the backend holds: a sequence
    /// the scan cannot place is counted as a push or not as a pop.
    depth: [usize; 2],
    /// The backend's screen (0 main, 1 alternate) when last read, and whether a sequence since could have switched it (a private
    /// mode list naming 1049, or a full reset): a push or pop reads the screen again only then, so a program that pushes and pops
    /// often does not split the parser's input at each change.
    screen: usize,
    screen_stale: bool,
}

impl AlacrittyTerminal {
    /// A terminal of `size`, at least 2 columns by 1 row ([`size`](Terminal::size) reports what it got).
    pub fn new(size: Size) -> Self {
        Self::with_scrollback(size, SCROLLBACK)
    }

    /// The size the screen really has for a requested one: at least [`MIN_COLS`] columns and one row.
    fn clamped(size: Size) -> Size {
        Size {
            cols: size.cols.max(MIN_COLS),
            rows: size.rows.max(1),
        }
    }

    /// Like [`new`](Self::new) with an explicit scrollback cap in lines. Scrollback is never read by
    /// detection; the cap only bounds memory.
    pub fn with_scrollback(size: Size, scrollback: usize) -> Self {
        let size = Self::clamped(size);
        let listener = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
            // ADR-0100: the emulator follows the program's kitty keyboard flags (`CSI > / < / = u`) so the repaint can hand them on.
            // A query's reply (`CSI ? u`) is dropped like every other reply the backend would write (`Listener`).
            kitty_keyboard: true,
            // A shape no program can set (DECSCUSR only gives block, underline and bar): while the backend reports it, the
            // program never set a shape or set the default, and the cursor is `CursorShape::Default`.
            default_cursor_style: DEFAULT_CURSOR,
            ..Config::default()
        };
        let term = Term::new(config.clone(), &Dims(size), listener.clone());
        Self {
            term,
            parser: Processor::new(),
            listener,
            size,
            scan: Scan::Text,
            config,
            depth: [0; 2],
            screen: 0,
            screen_stale: true,
        }
    }

    /// The screen (0 main, 1 alternate) the backend applies a kitty keyboard push or pop to, the parser having been given every byte
    /// through it. A synchronized update may still hold it (and the screen switches before it), so that update is ended first and
    /// the backend's screen is then the one the change applied to. Rare: once per chunk with kitty changes, and again only after a
    /// possible screen switch (see `advance_bounded`).
    fn read_screen(&mut self) -> usize {
        if self.parser.sync_timeout().sync_timeout().is_some() {
            self.parser.stop_sync(&mut self.term);
        }
        usize::from(self.term.mode().contains(TermMode::ALT_SCREEN))
    }

    /// A kitty push or pop ending just before `bytes[end]`: counted on the backend's screen, read again (after giving the parser
    /// everything through the change, from `start`) only when a sequence since the last read may have switched it.
    fn key_change(&mut self, change: KeyStack, bytes: &[u8], start: &mut usize, end: usize) {
        if self.screen_stale {
            self.parser.advance(&mut self.term, &bytes[*start..end]);
            *start = end;
            self.screen = self.read_screen();
            self.screen_stale = false;
        }
        if self.count_key_stack(change, self.screen) {
            if *start < end {
                self.parser.advance(&mut self.term, &bytes[*start..end]);
                *start = end;
            }
            self.bound_keyboard_stack();
        }
    }

    /// Counts a push or pop on `screen`; returns whether the stacks must be emptied now (the parser must have been given the push).
    fn count_key_stack(&mut self, change: KeyStack, screen: usize) -> bool {
        match change {
            KeyStack::Push => {
                self.depth[screen] += 1;
                self.depth[screen] >= PUSH_LIMIT
            }
            // `vte` reads 0 or no count as 1.
            KeyStack::Pop(n) => {
                self.depth[screen] = self.depth[screen].saturating_sub(usize::from(n.max(1)));
                false
            }
        }
    }

    /// Empties both keyboard stacks once one is [`PUSH_LIMIT`] deep, keeping the current flags. A synchronized update may still hold
    /// some of the pushes, so it is ended first: every counted push is then applied, and the stacks never reach the backend's panic.
    fn bound_keyboard_stack(&mut self) {
        self.parser.stop_sync(&mut self.term);
        let flags = self.modes().keyboard_flags;
        // Switching the backend's kitty tracking off and on again empties both stacks (`Term::set_options`).
        let mut off = self.config.clone();
        off.kitty_keyboard = false;
        self.term.set_options(off);
        self.term.set_options(self.config.clone());
        // `CSI = flags ; 1 u`: the flags again, without a stack entry (at most "\x1b[=255;1u", 11 bytes).
        let mut buf = [0u8; 16];
        let mut rest = &mut buf[..];
        let _ = std::io::Write::write_fmt(&mut rest, format_args!("\x1b[={flags};1u"));
        let n = 16 - rest.len();
        self.parser.advance(&mut self.term, &buf[..n]);
        self.depth = [0; 2];
    }

    /// Gives `bytes` to the parser, leaving out OSC payload bytes past [`MAX_OSC`].
    fn advance_bounded(&mut self, bytes: &[u8]) {
        let mut start = 0;
        let mut i = 0;
        while i < bytes.len() {
            if let Scan::Text = self.scan {
                // Only `ESC` can begin an OSC (a C1 0x9D is not one to `vte`).
                match bytes[i..].iter().position(|&b| b == 0x1b) {
                    Some(n) => i += n,
                    None => break,
                }
                // Most escapes are a CSI that is no kitty keyboard push or pop (colours, cursor moves, modes): its third byte already
                // takes the scan back to `Text` (see the `Scan::Csi` arms), so step over all three at once.
                if bytes.get(i + 1) == Some(&b'[') {
                    match bytes.get(i + 2) {
                        // A private mode list: if it may name 1049 (or runs past this chunk), the backend may switch screens.
                        Some(&b'?') => {
                            if may_name_1049(&bytes[i + 3..]) {
                                self.screen_stale = true;
                            }
                            i += 3;
                            continue;
                        }
                        Some(&(0x20..=0x3b | 0x3d | 0x40..=0x7e)) => {
                            i += 3;
                            continue;
                        }
                        // A kitty push or pop written plainly (digits only, all in this chunk) is read here at once; any other form
                        // goes through the scan below, which reads it the same way.
                        Some(&marker @ (b'>' | b'<')) => {
                            let mut j = i + 3;
                            let mut n: u16 = 0;
                            while let Some(&d @ b'0'..=b'9') = bytes.get(j) {
                                n = n.saturating_mul(10).saturating_add(u16::from(d - b'0'));
                                j += 1;
                            }
                            if bytes.get(j) == Some(&b'u') {
                                i = j + 1;
                                let change = if marker == b'>' {
                                    KeyStack::Push
                                } else {
                                    KeyStack::Pop(n)
                                };
                                self.key_change(change, bytes, &mut start, i);
                                continue;
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut change = None;
            self.scan = match (self.scan, bytes[i]) {
                (_, 0x1b) => Scan::Escape,
                (Scan::Escape, b']') => Scan::Osc(0),
                (Scan::Escape, b'[') => Scan::Csi,
                // A full reset (RIS) empties both stacks and leaves the alternate screen.
                (Scan::Escape, b'c') => {
                    self.depth = [0; 2];
                    self.screen_stale = true;
                    Scan::Text
                }
                // `vte` stays in its escape state for these.
                (Scan::Escape, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => Scan::Escape,
                (Scan::Osc(_), 0x07 | 0x18 | 0x1a) => Scan::Text,
                // Ignored inside an OSC: not kept.
                (Scan::Osc(n), 0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f) => Scan::Osc(n),
                (Scan::Osc(n), _) if n < MAX_OSC => Scan::Osc(n + 1),
                (Scan::Osc(n), _) => {
                    if start < i {
                        self.parser.advance(&mut self.term, &bytes[start..i]);
                    }
                    start = i + 1;
                    Scan::Osc(n)
                }
                (Scan::Csi, b'>') => Scan::Push,
                (Scan::Csi, b'<') => Scan::Pop(0),
                // A private mode list the fast path above did not see (split by the chunk, or controls before it): it may switch.
                (Scan::Csi, b'?') => {
                    self.screen_stale = true;
                    Scan::Text
                }
                // Executed or ignored inside a CSI sequence by `vte`, which stays in it.
                (Scan::Csi, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => Scan::Csi,
                (Scan::Push, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => Scan::Push,
                (Scan::Push, b'0'..=b'9' | b':' | b';') => Scan::Push,
                (Scan::Push, b'u') => {
                    change = Some(KeyStack::Push);
                    Scan::Text
                }
                (Scan::Pop(n), d @ b'0'..=b'9') => {
                    Scan::Pop(n.saturating_mul(10).saturating_add(u16::from(d - b'0')))
                }
                (Scan::Pop(n), b'u') => {
                    change = Some(KeyStack::Pop(n));
                    Scan::Text
                }
                _ => Scan::Text,
            };
            i += 1;
            if let Some(change) = change {
                self.key_change(change, bytes, &mut start, i);
            }
        }
        if start < bytes.len() {
            self.parser.advance(&mut self.term, &bytes[start..]);
        }
    }
}

impl Terminal for AlacrittyTerminal {
    fn feed(&mut self, bytes: &[u8]) {
        // A synchronized update (`CSI ? 2026 h`) the program never ended is applied once its timeout has passed, as a
        // terminal window does; the parser only buffers it and leaves the expiry to its owner (F702).
        if let Some(deadline) = self.parser.sync_timeout().sync_timeout() {
            if Instant::now() >= deadline {
                self.parser.stop_sync(&mut self.term);
            }
        }
        self.advance_bounded(bytes);
    }

    fn clamp_size(&self, size: Size) -> Size {
        Self::clamped(size)
    }

    fn resize(&mut self, size: Size) {
        let size = Self::clamped(size);
        self.term.resize(Dims(size));
        self.size = size;
    }

    fn size(&self) -> Size {
        self.size
    }

    fn cell(&self, row: u16, col: u16) -> Option<Cell> {
        if row >= self.size.rows || col >= self.size.cols {
            return None;
        }
        // Indexing by `Line(n)` addresses the live screen; the scrolled viewport (display offset) does not apply.
        let c = &self.term.grid()[Line(i32::from(row))][Column(usize::from(col))];
        let spacer = c.flags.contains(Flags::WIDE_CHAR_SPACER);
        Some(Cell {
            ch: if spacer || c.c == '\0' { ' ' } else { c.c },
            spacer,
            style: style(c.fg, c.bg, c.flags),
        })
    }

    fn history_len(&self) -> usize {
        self.term.grid().history_size()
    }

    fn history_cell(&self, back: usize, col: u16) -> Option<Cell> {
        if back == 0 || back > self.history_len() || col >= self.size.cols {
            return None;
        }
        // Negative lines are the scrollback above the live screen's top (`Line(-1)` scrolled off last).
        let line = Line(-i32::try_from(back).ok()?);
        let c = &self.term.grid()[line][Column(usize::from(col))];
        let spacer = c.flags.contains(Flags::WIDE_CHAR_SPACER);
        Some(Cell {
            ch: if spacer || c.c == '\0' { ' ' } else { c.c },
            spacer,
            style: style(c.fg, c.bg, c.flags),
        })
    }

    fn history_wrapped(&self, back: usize) -> bool {
        if back == 0 || back > self.history_len() {
            return false;
        }
        let Ok(back) = i32::try_from(back) else {
            return false;
        };
        // The backend marks a wrapped row on its last cell.
        let last = Column(usize::from(self.size.cols).saturating_sub(1));
        self.term.grid()[Line(-back)][last]
            .flags
            .contains(Flags::WRAPLINE)
    }

    // The inactive screen's history (`inactive_history_len` and the rest) keeps the trait's defaults: `alacritty_terminal` 0.26 keeps
    // the inactive grid private, and switching screens to peek would clear the alternate one (ADR-0100, Context).

    /// Like the default, but keeps the zero-width characters (combining accents, variation selectors, joiners) the
    /// backend stores on each cell, which [`Cell`] cannot carry (F703).
    fn line(&self, row: u16) -> String {
        let mut s = String::new();
        if row >= self.size.rows {
            return s;
        }
        let line = &self.term.grid()[Line(i32::from(row))];
        for col in 0..usize::from(self.size.cols) {
            let c = &line[Column(col)];
            if c.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            s.push(if c.c == '\0' { ' ' } else { c.c });
            if let Some(zw) = c.zerowidth() {
                s.extend(zw);
            }
        }
        s.truncate(s.trim_end().len());
        s
    }

    fn pen(&self) -> Style {
        let t = &self.term.grid().cursor.template;
        style(t.fg, t.bg, t.flags)
    }

    fn cursor(&self) -> Cursor {
        let p = self.term.grid().cursor.point;
        let style = self.term.cursor_style();
        Cursor {
            row: u16::try_from(p.line.0.max(0)).unwrap_or(0),
            col: u16::try_from(p.column.0).unwrap_or(0),
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
            shape: match style.shape {
                AShape::Block => CursorShape::Block,
                AShape::Underline => CursorShape::Underline,
                AShape::Beam => CursorShape::Bar,
                // The sentinel default (`DEFAULT_CURSOR`); `Hidden` is never set by a program either.
                AShape::HollowBlock | AShape::Hidden => CursorShape::Default,
            },
            blink: style.blinking,
        }
    }

    /// `alacritty_terminal` 0.26 keeps the scroll region private, and following DECSTBM in the byte stream would put a scanner on the
    /// feed path (ADR-0100 decision 1, alternatives). So it is read here, once per repaint, from the backend's own cursor addressing:
    /// under origin mode a cursor position is clamped to the region, so row 0 lands on its top and the last row on its bottom. The
    /// cursor (position, pen, character sets, pending wrap) and the origin mode are put back exactly as they were; only the backend's
    /// damage tracking, which nothing here reads, sees the move.
    fn margins(&mut self) -> Option<Margins> {
        let saved = self.term.grid().cursor.clone();
        let origin = self.term.mode().contains(TermMode::ORIGIN);
        let mode = PrivateMode::Named(NamedPrivateMode::Origin);
        if !origin {
            self.term.set_private_mode(mode);
        }
        self.term.goto(0, 0);
        let top = self.term.grid().cursor.point.line.0;
        // The row count: past every region, and no overflow in the backend's line arithmetic.
        self.term.goto(i32::from(self.size.rows), 0);
        let bottom = self.term.grid().cursor.point.line.0;
        if !origin {
            self.term.unset_private_mode(mode);
        }
        self.term.grid_mut().cursor = saved;
        Some(Margins {
            top: u16::try_from(top).ok()?,
            bottom: u16::try_from(bottom).ok()?,
        })
    }

    fn modes(&self) -> Modes {
        let m = *self.term.mode();
        // One at a time in the backend (setting one clears the others); the widest wins should that ever change.
        let mouse_tracking = if m.contains(TermMode::MOUSE_MOTION) {
            MouseTracking::Motion
        } else if m.contains(TermMode::MOUSE_DRAG) {
            MouseTracking::Drag
        } else if m.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseTracking::Click
        } else {
            MouseTracking::Off
        };
        let mouse_encoding = if m.contains(TermMode::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else if m.contains(TermMode::UTF8_MOUSE) {
            MouseEncoding::Utf8
        } else {
            MouseEncoding::Default
        };
        // The kitty flags are bits 18 to 22 of the backend's mode, in the protocol's order (1, 2, 4, 8, 16).
        let keyboard_flags = [
            TermMode::DISAMBIGUATE_ESC_CODES,
            TermMode::REPORT_EVENT_TYPES,
            TermMode::REPORT_ALTERNATE_KEYS,
            TermMode::REPORT_ALL_KEYS_AS_ESC,
            TermMode::REPORT_ASSOCIATED_TEXT,
        ]
        .iter()
        .enumerate()
        .filter(|(_, f)| m.contains(**f))
        .fold(0u8, |acc, (i, _)| acc | (1 << i));
        Modes {
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            application_cursor: m.contains(TermMode::APP_CURSOR),
            application_keypad: m.contains(TermMode::APP_KEYPAD),
            mouse_tracking,
            mouse_encoding,
            focus_reporting: m.contains(TermMode::FOCUS_IN_OUT),
            alternate_scroll: m.contains(TermMode::ALTERNATE_SCROLL),
            autowrap: m.contains(TermMode::LINE_WRAP),
            origin: m.contains(TermMode::ORIGIN),
            insert: m.contains(TermMode::INSERT),
            newline: m.contains(TermMode::LINE_FEED_NEW_LINE),
            keyboard_flags,
        }
    }

    fn title(&self) -> Option<String> {
        self.listener.title.lock().ok().and_then(|t| t.clone())
    }
}

#[cfg(test)]
mod osc_tests {
    use super::*;

    /// An OSC payload is kept up to `MAX_OSC` bytes, however long the program makes it and however it is split (F704).
    #[test]
    fn a_long_osc_is_cut_at_its_bound() {
        let long = "t".repeat(3 * MAX_OSC);
        for split in [1, 7, 4096] {
            let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 2 });
            let bytes = format!("\x1b]0;{long}\x07$ ok");
            for c in bytes.as_bytes().chunks(split) {
                t.feed(c);
            }
            let title = t.title().unwrap_or_default();
            assert_eq!(title.len(), MAX_OSC - 2, "split {split}");
            assert_eq!(t.line(0), "$ ok", "split {split}");
        }
        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 2 });
        t.feed(format!("\x1b]0;{}\x07", "u".repeat(MAX_OSC - 2)).as_bytes());
        assert_eq!(
            t.title().map(|s| s.len()),
            Some(MAX_OSC - 2),
            "a title at the bound is whole"
        );
        t.feed(&[&b"\x1b\x80]0;"[..], long.as_bytes(), b"\x1b\\"].concat());
        assert_eq!(
            t.title(),
            Some("t".repeat(MAX_OSC - 2)),
            "ESC <ignored byte> ] is an OSC too"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kitty keyboard pushes past the backend's stack bound never panic it, in one chunk, byte by byte, inside a synchronized
    /// update, or with controls inside the sequence (`vte` still pushes then), and the current flags survive the bound.
    #[test]
    fn a_flood_of_keyboard_pushes_is_bounded() {
        let cases: [(&str, String, usize); 4] = [
            ("plain", "\x1b[>1u".repeat(5000) + "\x1b[>5u", 1 << 20),
            ("byte by byte", "\x1b[>1u".repeat(5000) + "\x1b[>5u", 1),
            (
                "synchronized",
                format!("\x1b[?2026h{}\x1b[>5u", "\x1b[>1u".repeat(5000)),
                4096,
            ),
            (
                "controls inside",
                "\x1b[>\x07\u{e9}1u".repeat(5000) + "\x1b[>5u",
                777,
            ),
        ];
        for (what, bytes, split) in cases {
            let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
            for c in bytes.as_bytes().chunks(split) {
                t.feed(c);
            }
            t.parser.stop_sync(&mut t.term);
            assert_eq!(t.modes().keyboard_flags, 5, "{what}");
            assert!(t.depth.iter().all(|&d| d < PUSH_LIMIT), "{what}");
            // The stack still works after the bound: one pop goes back to the push before.
            t.feed(b"\x1b[<u");
            assert_eq!(t.modes().keyboard_flags & !5, 0, "{what}");
        }
        // Ending exactly on the bound: the push that reaches it empties the stacks, and its flags (not 0) stay on.
        for split in [1 << 20, 1, 5] {
            let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
            let bytes = "\x1b[>1u".repeat(PUSH_LIMIT - 1) + "\x1b[>6u";
            for c in bytes.as_bytes().chunks(split) {
                t.feed(c);
            }
            assert_eq!(t.depth, [0, 0], "split {split}: the bound was reached");
            assert_eq!(t.modes().keyboard_flags, 6, "split {split}");
            // The stacks are empty now: a pop leaves no flags (the entries below were dropped with the bound).
            t.feed(b"\x1b[<u");
            assert_eq!(t.modes().keyboard_flags, 0, "split {split}");
        }
        // Pushes and pops in balance never reach the bound, however many, on either screen, and the entries below survive.
        for (what, screen) in [("main", ""), ("alternate", "\x1b[?1049h")] {
            let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
            let bytes = format!(
                "\x1b[>1u{screen}\x1b[>2u{}",
                "\x1b[>5u\x1b[<u".repeat(3 * PUSH_LIMIT)
            );
            for c in bytes.as_bytes().chunks(7) {
                t.feed(c);
            }
            assert_eq!(
                t.modes().keyboard_flags,
                2,
                "{what}: the entry below the balanced pairs"
            );
            assert!(
                t.depth.iter().all(|&d| d <= 2),
                "{what}: depth {:?}",
                t.depth
            );
            t.feed(b"\x1b[<u");
            let below = if screen.is_empty() { 1 } else { 0 };
            assert_eq!(t.modes().keyboard_flags, below, "{what}");
            t.feed(b"\x1b[?1049l");
            assert_eq!(
                t.modes().keyboard_flags,
                1,
                "{what}: the main screen's stack is intact"
            );
        }
        // A pop of many, and the screen read from the backend whatever switched it: 1049 in a list, inside a synchronized update.
        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        t.feed(b"\x1b[>1u\x1b[>1u\x1b[>1u\x1b[<2u\x1b[?25;1049h\x1b[>1u\x1b[?1049;7l");
        assert_eq!(t.depth, [1, 1]);
        t.feed(b"\x1b[?2026h\x1b[?1049h\x1b[>1u\x1b[>1u");
        assert_eq!(
            t.depth,
            [1, 3],
            "the pushes inside the update count on the screen it switched to"
        );
        t.feed(b"\x1b[?2026l");
        assert_eq!(t.modes().keyboard_flags, 1);
        t.feed(b"\x1b[?1049l\x1b[<u");
        assert_eq!(t.depth, [0, 3]);
        t.feed(b"\x1bc");
        assert_eq!(t.depth, [0, 0], "a full reset empties both");
        // 1049 in a list with a control inside it, and a list split between two chunks, still switch the counted screen.
        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        t.feed(b"\x1b[>1u\x1b[?10\x0749h\x1b[>1u");
        assert_eq!(t.depth, [1, 1]);
        t.feed(b"\x1b[?10");
        t.feed(b"49l\x1b[>1u");
        assert_eq!(t.depth, [2, 1]);
        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        t.feed(b"\x1b[>1u\x1b[>3u");
        assert_eq!(t.modes().keyboard_flags, 3);
        t.feed(b"\x1b[<u");
        assert_eq!(
            t.modes().keyboard_flags,
            1,
            "a pop restores the flags below"
        );
        t.feed(b"\x1b[=8;1u");
        assert_eq!(t.modes().keyboard_flags, 8, "set without a push");
    }

    /// This backend knows which scrollback rows it wrapped (ADR-0100 decision 2), and while the alternate screen is active it shows no
    /// history at all: the active grid is the alternate one, and the main screen's (the inactive one) cannot be read.
    #[test]
    fn wrapped_history_rows_are_marked_and_the_alternate_screen_has_none() {
        let mut t = AlacrittyTerminal::new(Size { cols: 10, rows: 3 });
        t.feed(b"0123456789abcd\r\nend\r\n1\r\n2\r\n3\r\n");
        let n = t.history_len();
        assert_eq!(n, 4);
        assert!(t.history_wrapped(4), "0123456789 continues on the next row");
        assert!(!t.history_wrapped(3) && !t.history_wrapped(2) && !t.history_wrapped(1));
        t.feed(b"\x1b[?1049h");
        assert_eq!(t.history_len(), 0);
        assert_eq!(t.history_cell(1, 0), None);
        assert!(!t.history_wrapped(1));
        assert_eq!(t.inactive_history_len(), 0, "not readable in this backend");
        t.feed(b"\x1b[?1049l");
        assert_eq!(t.history_len(), n);
    }

    /// A synchronized update the program never ends is applied once its timeout has passed (F702).
    #[test]
    fn unfinished_synchronized_update_expires() {
        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        t.feed(b"\x1b[?2026hframe");
        assert_eq!(t.line(0), "", "held while the update may still end");
        std::thread::sleep(std::time::Duration::from_millis(300));
        t.feed(b"\r\n$ ");
        assert_eq!(t.bottom_non_empty_lines(2), vec!["frame", "$"]);
    }

    fn margins(top: u16, bottom: u16) -> Option<Margins> {
        Some(Margins { top, bottom })
    }

    /// M7.10: the scroll region as the program set it, the whole screen when it set none, again after a resize or a full reset (the
    /// backend resets it then), the same on the alternate screen.
    #[test]
    fn the_margins_are_the_programs_scroll_region() {
        let size = Size { cols: 10, rows: 8 };
        let mut t = AlacrittyTerminal::new(size);
        assert_eq!(t.margins(), margins(0, 7));
        t.feed(b"\x1b[3;6r");
        assert_eq!(t.margins(), margins(2, 5));
        t.feed(b"\x1b[?1049h");
        assert_eq!(t.margins(), margins(2, 5));
        t.feed(b"\x1b[2r");
        assert_eq!(t.margins(), margins(1, 7));
        // Refused by the backend (top not above bottom): unchanged.
        t.feed(b"\x1b[5;5r");
        assert_eq!(t.margins(), margins(1, 7));
        t.feed(b"\x1b[r");
        assert_eq!(t.margins(), margins(0, 7));
        t.feed(b"\x1b[2;4r");
        t.resize(Size { cols: 10, rows: 6 });
        assert_eq!(t.margins(), margins(0, 5));
        t.feed(b"\x1b[2;4r\x1bc");
        assert_eq!(t.margins(), margins(0, 5));
    }

    /// Looking for the margins changes nothing the program or a window could see: the cursor (with a pending wrap), the pen, origin
    /// mode, and what the next bytes do, also between the halves of a sequence and inside a synchronized update.
    #[test]
    fn looking_for_the_margins_changes_nothing() {
        let size = Size { cols: 10, rows: 8 };
        let cases: [(&[u8], &[u8]); 5] = [
            // A pending wrap at the last column: the next character wraps.
            (b"\x1b[3;6r\x1b[31m0123456789", b"next"),
            // Origin mode on, the cursor inside the region.
            (
                b"\x1b[3;6r\x1b[?6h\x1b[2;4Hab",
                b"\x1b[1;1Hc\x1b[9;1Hd\n\n\n\ne",
            ),
            // Origin mode on, the cursor below the region (DECRC).
            (b"\x1b[8;2H\x1b7\x1b[3;6r\x1b[?6h\x1b8", b"x\x1b[1;1Hy"),
            // Between the halves of a DECSTBM.
            (b"\x1b[4;", b"7r\x1b[?6h\x1b[2;2Hz"),
            // Inside a synchronized update (the parser holds the bytes; the backend has not applied them yet).
            (b"\x1b[?2026h\x1b[2;5r\x1b[?6h", b"\x1b[?2026l\x1b[1;1Hw"),
        ];
        for (before, after) in cases {
            let mut probed = AlacrittyTerminal::new(size);
            let mut twin = AlacrittyTerminal::new(size);
            probed.feed(before);
            twin.feed(before);
            let _ = probed.margins();
            let _ = probed.margins();
            let what = String::from_utf8_lossy(before);
            assert_eq!(probed.cursor(), twin.cursor(), "{what}");
            assert_eq!(probed.modes(), twin.modes(), "{what}");
            assert_eq!(probed.pen(), twin.pen(), "{what}");
            probed.feed(after);
            twin.feed(after);
            assert_eq!(probed.cursor(), twin.cursor(), "{what}");
            assert_eq!(probed.modes(), twin.modes(), "{what}");
            assert_eq!(probed.margins(), twin.margins(), "{what}");
            for row in 0..size.rows {
                assert_eq!(probed.line(row), twin.line(row), "{what}, row {row}");
            }
        }
    }
}
