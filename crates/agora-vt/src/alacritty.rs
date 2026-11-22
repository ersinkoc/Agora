//! Backend on `alacritty_terminal` (ADR-0012).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AColor, CursorShape as AShape, CursorStyle, NamedColor, Processor,
};

use crate::{
    Cell, Color, Cursor, CursorShape, Modes, MouseEncoding, MouseTracking, Size, Style, Terminal,
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
    /// After `ESC [`: `>` makes it a kitty keyboard push (`CSI > flags u`), `<` a pop (`CSI < n u`), `?` a private mode list.
    Csi,
    /// Inside `ESC [ >`, reading the parameters.
    Push,
    /// Inside `ESC [ <`, reading the count (digits only: a pop in any other form is not counted, which only overstates the depth).
    Pop(u16),
    /// Inside `ESC [ ?`, reading the list the way `vte` keeps it (see [`ModeList`]).
    Modes(ModeList),
}

/// A `CSI ? Pm h/l` list as `vte` reads it, only to see whether it switches the alternate screen (1049, the one mode the backend
/// swaps its screens, and its keyboard stacks, for): at most 32 numbers (parameters and `:` sub-parameters together, each saturating
/// at 65535); a list with more is not acted on; the first number of each parameter is the mode.
#[derive(Clone, Copy, Default)]
struct ModeList {
    stored: u8,
    current: u16,
    /// The number being read is a sub-parameter (after `:`).
    sub: bool,
    /// A parameter's first number was 1049.
    screen: bool,
    ignored: bool,
}

impl ModeList {
    /// Ends the number being read at a `:`, `;` or the final byte.
    fn end_number(mut self, next_sub: bool) -> Self {
        if usize::from(self.stored) == VTE_MAX_PARAMS {
            self.ignored = true;
            return self;
        }
        if !self.sub && self.current == 1049 {
            self.screen = true;
        }
        self.stored += 1;
        self.current = 0;
        self.sub = next_sub;
        self
    }
}

/// The most numbers `vte` keeps in one sequence (`MAX_PARAMS`).
const VTE_MAX_PARAMS: usize = 32;

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
    /// Entries on the active screen's kitty keyboard stack, as the parser will have them once given the bytes scanned so far:
    /// pushes add one, pops remove theirs (floored at 0), and it swaps with `inactive_depth` when the program switches screens, as
    /// the backend swaps its two stacks. Never fewer than the backend holds: a sequence the scan cannot place is counted as a push
    /// or not as a pop.
    depth: usize,
    inactive_depth: usize,
    /// The program is on the alternate screen, as far as the scan has seen.
    alt: bool,
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
            depth: 0,
            inactive_depth: 0,
            alt: false,
        }
    }

    /// Empties both keyboard stacks once the active one is [`PUSH_LIMIT`] deep, keeping the current flags. A synchronized update may
    /// still hold some of the pushes, so it is ended first: every counted push is then applied, and the stacks never reach the
    /// backend's panic.
    fn bound_keyboard_stack(&mut self) {
        if self.depth < PUSH_LIMIT {
            return;
        }
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
        self.depth = 0;
        self.inactive_depth = 0;
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
            }
            let mut pushed = false;
            self.scan = match (self.scan, bytes[i]) {
                (_, 0x1b) => Scan::Escape,
                (Scan::Escape, b']') => Scan::Osc(0),
                (Scan::Escape, b'[') => Scan::Csi,
                // A full reset (RIS) empties both stacks and leaves the alternate screen.
                (Scan::Escape, b'c') => {
                    self.depth = 0;
                    self.inactive_depth = 0;
                    self.alt = false;
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
                (Scan::Csi, b'?') => Scan::Modes(ModeList::default()),
                // Executed or ignored inside a CSI sequence by `vte`, which stays in it.
                (Scan::Csi, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => Scan::Csi,
                (Scan::Push, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => Scan::Push,
                (Scan::Push, b'0'..=b'9' | b':' | b';') => Scan::Push,
                (Scan::Push, b'u') => {
                    pushed = true;
                    Scan::Text
                }
                (Scan::Pop(n), d @ b'0'..=b'9') => {
                    Scan::Pop(n.saturating_mul(10).saturating_add(u16::from(d - b'0')))
                }
                (Scan::Pop(n), b'u') => {
                    // `vte` reads 0 or no count as 1.
                    self.depth = self.depth.saturating_sub(usize::from(n.max(1)));
                    Scan::Text
                }
                (Scan::Modes(l), 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => Scan::Modes(l),
                (Scan::Modes(mut l), d @ b'0'..=b'9') => {
                    if usize::from(l.stored) == VTE_MAX_PARAMS {
                        l.ignored = true;
                    }
                    l.current = l
                        .current
                        .saturating_mul(10)
                        .saturating_add(u16::from(d - b'0'));
                    Scan::Modes(l)
                }
                (Scan::Modes(l), b':') => Scan::Modes(l.end_number(true)),
                (Scan::Modes(l), b';') => Scan::Modes(l.end_number(false)),
                (Scan::Modes(l), fin @ (b'h' | b'l')) => {
                    let l = l.end_number(false);
                    let to_alt = fin == b'h';
                    // The backend swaps only when the screen really changes.
                    if l.screen && !l.ignored && self.alt != to_alt {
                        std::mem::swap(&mut self.depth, &mut self.inactive_depth);
                        self.alt = to_alt;
                    }
                    Scan::Text
                }
                _ => Scan::Text,
            };
            i += 1;
            if pushed {
                self.depth += 1;
                if self.depth >= PUSH_LIMIT {
                    self.parser.advance(&mut self.term, &bytes[start..i]);
                    start = i;
                    self.bound_keyboard_stack();
                }
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
            assert!(t.depth < PUSH_LIMIT, "{what}");
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
            assert_eq!(t.depth, 0, "split {split}: the bound was reached");
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
            assert!(t.depth <= 2, "{what}: depth {}", t.depth);
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
        // A pop of many and a mode list that only names 1049 among others are read as the backend reads them.
        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        t.feed(b"\x1b[>1u\x1b[>1u\x1b[>1u\x1b[<2u\x1b[?25;1049h\x1b[>1u\x1b[?1049;7l");
        assert_eq!((t.depth, t.inactive_depth, t.alt), (1, 1, false));
        t.feed(b"\x1bc");
        assert_eq!(
            (t.depth, t.inactive_depth),
            (0, 0),
            "a full reset empties both"
        );
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
}
