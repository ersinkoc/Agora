//! Backend on `alacritty_terminal` (ADR-0012).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor, Processor};

use crate::{Cell, Color, Cursor, Modes, Size, Style, Terminal};

/// Scrolled-back lines kept per terminal. Memory is bounded by this (design/resource-limits.md).
const SCROLLBACK: usize = 2000;

/// Longest OSC payload, in bytes, the parser is given: the bound `vte` itself keeps when built without `std` (`MAX_OSC_RAW`).
/// Built with `std`, as `alacritty_terminal` does, it keeps a payload whole until its terminator, so an OSC that never ends
/// would grow the terminal's memory with every byte, and a long title would be copied up to 4096 times onto the title stack
/// (`CSI 22 t`). Bytes past the bound are left out; the sequence still ends at its terminator.
const MAX_OSC: usize = 1024;

/// What the OSC bound needs to know of the byte stream; it follows `vte`'s states for `ESC`, `ESC ]` and the OSC string.
#[derive(Clone, Copy, Default)]
enum Scan {
    #[default]
    Text,
    Escape,
    /// Inside an OSC, with this many payload bytes given to the parser.
    Osc(usize),
}

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
}

impl AlacrittyTerminal {
    pub fn new(size: Size) -> Self {
        Self::with_scrollback(size, SCROLLBACK)
    }

    /// Like [`new`](Self::new) with an explicit scrollback cap in lines. Scrollback is never read by
    /// detection; the cap only bounds memory.
    pub fn with_scrollback(size: Size, scrollback: usize) -> Self {
        let size = Size {
            cols: size.cols.max(1),
            rows: size.rows.max(1),
        };
        let listener = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let term = Term::new(config, &Dims(size), listener.clone());
        Self {
            term,
            parser: Processor::new(),
            listener,
            size,
            scan: Scan::Text,
        }
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
            self.scan = match (self.scan, bytes[i]) {
                (_, 0x1b) => Scan::Escape,
                (Scan::Escape, b']') => Scan::Osc(0),
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
                _ => Scan::Text,
            };
            i += 1;
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

    fn resize(&mut self, size: Size) {
        let size = Size {
            cols: size.cols.max(1),
            rows: size.rows.max(1),
        };
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
        Cursor {
            row: u16::try_from(p.line.0.max(0)).unwrap_or(0),
            col: u16::try_from(p.column.0).unwrap_or(0),
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
        }
    }

    fn modes(&self) -> Modes {
        let m = self.term.mode();
        Modes {
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            application_cursor: m.contains(TermMode::APP_CURSOR),
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
