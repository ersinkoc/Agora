//! Backend on `alacritty_terminal` (ADR-0012).

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::{Cell, Cursor, Modes, Size, Terminal};

/// Scrolled-back lines kept per terminal. Memory is bounded by this (design/resource-limits.md).
const SCROLLBACK: usize = 2000;

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

pub struct AlacrittyTerminal {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    size: Size,
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
        }
    }
}

impl Terminal for AlacrittyTerminal {
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
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
        })
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
