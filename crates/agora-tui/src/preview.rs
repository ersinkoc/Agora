//! The selected agent's terminal, watched through the hub and kept in a terminal emulator so the console can paint it inside its
//! right-hand panel with the agent's own colours. Watching never resizes the agent's terminal (other windows may be showing it);
//! the emulator follows the agent's size instead.

use std::time::{Duration, Instant};

use agora_vt::{AlacrittyTerminal, Size, Terminal};

use crate::client::{Session, TermEvent};
use crate::termio::{ModeScanner, ProgramModes};

/// Columns assumed until the machine says the terminal's size (a node that does not send it after the snapshot).
const GUESS_COLS: u16 = 120;
/// Most stream items handled per call, so a flood of output never holds up the keyboard.
const BATCH: usize = 256;
/// Most stream items a pane not in focus takes per turn (ADR-0068): with the node's 16 KiB chunks, 512 KiB, about 5 ms of feeding at
/// the pane's measured 100 MB/s (`examples/pane_timing.rs`).
pub const TILE_BATCH: usize = 32;

pub struct Preview {
    /// The hub-wide id of the agent being watched.
    pub agent: String,
    stream: u32,
    feed: PaneFeed,
    /// When to ask the machine for a fresh repaint: after its terminal was resized while output had come since the last repaint.
    /// Reflowing that output here, size after size, mangled it until the tab was opened again (TUI review); the machine's own
    /// screen at the new size is the truth.
    repaint_at: Option<Instant>,
    /// Output was dropped while nothing read it (a question was open): watch again at the next pump, for the machine's own screen.
    behind: bool,
}

/// How long after a resize the fresh repaint is asked for: the agent has redrawn for its new size by then, and a burst of resizes
/// (a window being dragged) asks once.
const REPAINT_AFTER: Duration = Duration::from_millis(300);

/// The history this view keeps, and so asks for on a fresh pane (ADR-0100 decision 2; the machine sends at most what it keeps).
pub const HISTORY_LINES: u32 = 2000;

/// How long a screen that came with history waits for the size that follows it (as the web window does, `SNAPSHOT_WAIT_MS`): a
/// machine that sends history always sends the size next, so this only bounds a frame that went missing.
const SIZE_WAIT: Duration = Duration::from_millis(150);

/// Clears the screen without moving its rows into the scrollback (origin mode off, so the cursor goes to the top row even inside a
/// scroll region the program left; the cursor home; erase below), so neither a resize nor a repaint's own clear (`CSI 2 J`, which
/// this emulator keeps in its history, as xterm.js does not) copies the screen into the history: the history stays the program's, as
/// the machine holds it. The repaint switches origin mode off itself (ADR-0100 decision 1).
const CLEAR_SCREEN_ONLY: &[u8] = b"\x1b[?6l\x1b[H\x1b[J";

/// What a call to [`Preview::pump`] saw.
#[derive(Debug, PartialEq, Eq)]
pub enum Pumped {
    Nothing,
    /// The screen changed: draw again.
    Changed,
    /// The program ended or the hub stopped relaying it.
    Ended,
    /// The link to the hub is gone.
    Lost,
}

/// The number of rows a repaint positions (`ESC [ n ; 1 H` for each row of the screen).
fn rows_in(screen: &[u8]) -> u16 {
    let mut rows = 0u16;
    let mut i = 0;
    while let Some(at) = screen[i..].windows(2).position(|w| w == b"\x1b[") {
        let start = i + at + 2;
        let digits = screen[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if screen[start + digits..].starts_with(b";1H") {
            if let Ok(n) = std::str::from_utf8(&screen[start..start + digits])
                .unwrap_or("0")
                .parse::<u16>()
            {
                rows = rows.max(n);
            }
        }
        i = start;
    }
    rows
}

/// What a pane holds of the agent's terminal, and how each thing the machine sends changes it: its emulator (whose scrollback the
/// wheel reads), the program's mouse and focus modes, and the last screen. No connection, so tests feed it the machine's frames.
///
/// History (kind 4, ADR-0100) is made at the size of the screen that follows it, and that size comes right after the screen, so a
/// screen that came with history is held until its size and then both are fed at that size, once. Only the screen (kind 1) is kept
/// as the "last screen" fed again when its size arrives later; history never is.
pub struct PaneFeed {
    term: AlacrittyTerminal,
    /// The last full repaint, applied again when the size it was meant for arrives after it.
    screen: Vec<u8>,
    output_since_screen: bool,
    /// The last repaint waits for its size: the machine sends the size a repaint was made at right after it (protocol.md, kind 2),
    /// and only that size paints the repaint again. A later size is the terminal resized since (this console sizes the agent's
    /// terminal to its pane right after it opens it). Painted at that size the repaint's rows, made at the old width, wrapped and
    /// scrolled its first rows away: a full-screen program that does not redraw on a resize showed an empty pane until the next
    /// refresh. The emulator here is the machine's own kind, so resizing what it holds gives the machine's screen at the new size
    /// (#977). A screen that came with history is held until its size instead, so it is painted at that size once.
    screen_unsized: bool,
    /// The mouse and focus modes the program switched on, read from the bytes that arrive (see `termio`), the repaint included: a
    /// current node's repaint sets or resets each mode it tracks (ADR-0100), so a mode switched on before this view opened is known.
    scanner: ModeScanner,
    /// A screen and the history that came before it, waiting for the size they were made at, and since when.
    held: Option<(Vec<u8>, Vec<u8>, Instant)>,
    /// A screen has been fed: the emulator has history of its own, so watching again asks for none.
    painted: bool,
}

impl Default for PaneFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneFeed {
    pub fn new() -> Self {
        Self {
            term: AlacrittyTerminal::with_scrollback(
                Size {
                    cols: GUESS_COLS,
                    rows: 24,
                },
                HISTORY_LINES as usize,
            ),
            screen: Vec::new(),
            output_since_screen: false,
            screen_unsized: false,
            scanner: ModeScanner::default(),
            held: None,
            painted: false,
        }
    }

    /// The history to ask for when watching (again): this view's own while it has shown no screen (a fresh pane), 0 after (it keeps
    /// its own; a replay would repeat it).
    pub fn history_to_ask(&self) -> u32 {
        if self.painted {
            0
        } else {
            HISTORY_LINES
        }
    }

    /// Output the program wrote.
    pub fn data(&mut self, bytes: &[u8]) {
        self.release();
        self.scanner.feed(bytes);
        self.term.feed(bytes);
        self.output_since_screen = true;
    }

    /// A repaint of the whole screen, with the history that came before it (empty when none did).
    pub fn screen(&mut self, screen: Vec<u8>, history: Vec<u8>) {
        self.release();
        if history.is_empty() {
            let rows = rows_in(&screen);
            let size = (rows > 0 && rows != self.term.size().rows).then(|| Size {
                cols: self.term.size().cols,
                rows,
            });
            self.paint(size, &[], screen);
            self.screen_unsized = true;
        } else {
            self.held = Some((screen, history, Instant::now()));
        }
    }

    /// The machine's terminal has this size: after a screen, the size it was painted at. Returns true when the machine should be
    /// asked for a fresh screen (output came since the last one, so the screen here cannot be put right by feeding it again).
    pub fn resized(&mut self, cols: u16, rows: u16) -> bool {
        let size = Size { cols, rows };
        if let Some((screen, history, _)) = self.held.take() {
            // The screen and its history, at the size they were made at: painted once (never again on a later size).
            self.paint(Some(size), &history, screen);
            return false;
        }
        let own = std::mem::take(&mut self.screen_unsized);
        if self.output_since_screen {
            // Output came after the repaint: the machine is asked for its screen at the new size (`Preview::pump`).
            self.term.resize(size);
            return true;
        }
        if own && !self.screen.is_empty() {
            // The size that belongs to the repaint just applied: paint it again at that size (the screen only, never history).
            let screen = std::mem::take(&mut self.screen);
            self.paint(Some(size), &[], screen);
            return false;
        }
        // The terminal was resized since its repaint was painted at its own size: resizing the emulator is all of it
        // (`screen_unsized`).
        self.term.resize(size);
        false
    }

    /// Feeds a held screen whose size has not come in time; true when it did.
    pub fn tick(&mut self) -> bool {
        if self
            .held
            .as_ref()
            .is_some_and(|h| h.2.elapsed() >= SIZE_WAIT)
        {
            self.release();
            return true;
        }
        false
    }

    /// Feeds a held screen and its history at the size the emulator has (something else came before the size).
    fn release(&mut self) {
        if let Some((screen, history, _)) = self.held.take() {
            let rows = rows_in(&screen);
            let size = (rows > 0).then(|| Size {
                cols: self.term.size().cols,
                rows,
            });
            self.paint(size, &history, screen);
            self.screen_unsized = true;
        }
    }

    /// Clears the screen (never into the history), takes `size`, then feeds the history and the screen.
    fn paint(&mut self, size: Option<Size>, history: &[u8], screen: Vec<u8>) {
        if self.painted {
            self.term.feed(CLEAR_SCREEN_ONLY);
        }
        if let Some(size) = size.filter(|s| *s != self.term.size()) {
            self.term.resize(size);
        }
        self.term.feed(history);
        self.scanner.feed(&screen);
        self.term.feed(&screen);
        self.screen = screen;
        self.output_since_screen = false;
        self.painted = true;
    }

    /// The mouse and focus modes the program has switched on, as far as this view has seen.
    pub fn program_modes(&self) -> ProgramModes {
        self.scanner.modes()
    }

    pub fn screen_term(&self) -> &dyn Terminal {
        &self.term
    }
}

impl Preview {
    /// Starts watching `agent`'s terminal (`<node>/<local id>`; its terminal is `t-<local id>`), with its history: the pane is fresh.
    pub fn open(session: &mut Session, agent: &str) -> Result<Self, String> {
        let feed = PaneFeed::new();
        let (node, local) = agent.split_once('/').unwrap_or(("", agent));
        let stream = session.terminal_open_with_history(
            node,
            &format!("t-{local}"),
            feed.history_to_ask(),
        )?;
        Ok(Self {
            agent: agent.to_string(),
            stream,
            feed,
            repaint_at: None,
            behind: false,
        })
    }

    /// Applies what arrived for this terminal, without waiting.
    pub fn pump(&mut self, session: &mut Session) -> Pumped {
        self.pump_up_to(session, BATCH)
    }

    /// [`Preview::pump`] taking at most `most` stream items: the other panes of a tiled layout take fewer per turn than the pane in
    /// focus ([`TILE_BATCH`]), so several flooding panes never hold up the keyboard or the pane being typed in (ADR-0068 decision 7).
    pub fn pump_up_to(&mut self, session: &mut Session, most: usize) -> Pumped {
        let mut seen = Pumped::Nothing;
        if self.behind
            || self
                .repaint_at
                .is_some_and(|at| at.elapsed() >= REPAINT_AFTER)
        {
            self.behind = false;
            self.repaint_at = None;
            // Watching again starts with the machine's full screen at its current size, and no history once this view has its own.
            let (node, local) = self.agent.split_once('/').unwrap_or(("", &self.agent));
            let (node, terminal) = (node.to_string(), format!("t-{local}"));
            session.terminal_close(self.stream);
            match session.terminal_open_with_history(&node, &terminal, self.feed.history_to_ask()) {
                Ok(stream) => self.stream = stream,
                Err(_) => return Pumped::Ended,
            }
        }
        if self.feed.tick() {
            seen = Pumped::Changed;
        }
        for _ in 0..most {
            match session.terminal_next(self.stream, Duration::ZERO) {
                TermEvent::Quiet => break,
                TermEvent::Behind => {
                    self.behind = true;
                    break;
                }
                TermEvent::Data(b) => {
                    self.feed.data(&b);
                    seen = Pumped::Changed;
                }
                TermEvent::Screen(b) => {
                    let history = session.terminal_history(self.stream);
                    self.feed.screen(b, history);
                    seen = Pumped::Changed;
                }
                TermEvent::Resized(cols, rows) => {
                    // With output since the last screen, the machine is asked for its screen at the new size (see `repaint_at`).
                    if self.feed.resized(cols, rows) {
                        self.repaint_at = Some(Instant::now());
                    }
                    seen = Pumped::Changed;
                }
                TermEvent::Exit(_) | TermEvent::Ended => return Pumped::Ended,
                TermEvent::Lost => return Pumped::Lost,
            }
        }
        seen
    }

    /// The mouse and focus modes the program has switched on, as far as this view has seen.
    pub fn program_modes(&self) -> ProgramModes {
        self.feed.program_modes()
    }

    pub fn screen(&self) -> &dyn Terminal {
        self.feed.screen_term()
    }

    /// Stops watching.
    pub fn close(self, session: &mut Session) {
        session.terminal_close(self.stream);
    }
}

#[cfg(test)]
mod tests {
    use super::{rows_in, PaneFeed, HISTORY_LINES};
    use agora_vt::{Size, Terminal};

    /// A node terminal of `size` that printed `lines` numbered lines and then `last`: its history replay and its repaint.
    fn node(size: Size, lines: usize, last: &str) -> (Vec<u8>, Vec<u8>) {
        let mut t = agora_vt::AlacrittyTerminal::new(size);
        for i in 1..=lines {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.feed(last.as_bytes());
        let mut history = Vec::new();
        // The replay's shape (docs/specs/protocol.md, kind 4), for these plain lines.
        history.extend_from_slice(
            b"\x1b[?1049l\x1b[r\x1b[?6l\x1b[4l\x1b[?7h\x1b[0m\x1b(B\x0f\x1b[2J\x1b[3J\x1b[H",
        );
        let kept = t.history_len();
        for back in (1..=kept).rev() {
            let text: String = (0..size.cols)
                .filter_map(|c| t.history_cell(back, c))
                .map(|c| c.ch)
                .collect();
            history.extend_from_slice(format!("{}\r\n", text.trim_end()).as_bytes());
        }
        history.extend_from_slice(
            format!(
                "\x1b[{};1H{}",
                size.rows,
                "\n".repeat(kept.min(usize::from(size.rows) - 1))
            )
            .as_bytes(),
        );
        (history, plain_repaint(&t))
    }

    /// The screen as plain rows at absolute positions (enough for these tests; the node's repaint is tested in `tests/`).
    fn plain_repaint(t: &dyn Terminal) -> Vec<u8> {
        let mut out = b"\x1b[0m\x1b[2J".to_vec();
        for r in 0..t.size().rows {
            out.extend_from_slice(
                format!("\x1b[{};1H{}\x1b[K", r + 1, t.line(r).trim_end()).as_bytes(),
            );
        }
        out
    }

    #[test]
    fn a_screen_with_history_waits_for_its_size_and_is_fed_once_at_it() {
        let size = Size { cols: 40, rows: 5 };
        let (history, screen) = node(size, 12, "ready");
        let mut feed = PaneFeed::new();
        assert_eq!(feed.history_to_ask(), HISTORY_LINES);
        feed.screen(screen, history);
        // Held: nothing fed until the size that follows it.
        assert_eq!(feed.screen_term().history_len(), 0);
        assert_eq!(feed.screen_term().size().cols, 120);
        assert!(!feed.resized(size.cols, size.rows));
        let t = feed.screen_term();
        assert_eq!(t.size(), size);
        assert_eq!(
            t.history_len(),
            8,
            "line 1 to line 8 scrolled off a 5-row screen"
        );
        assert_eq!(t.line(4), "ready");
        assert_eq!(feed.history_to_ask(), 0);
    }

    // Review of #964: a program left origin mode on inside a scroll region; clearing the screen before the next repaint must still
    // start at the top row, or the rows above the region go into the history.
    #[test]
    fn a_new_screen_never_moves_rows_above_a_scroll_region_into_the_history() {
        let size = Size { cols: 40, rows: 5 };
        let (_, screen) = node(size, 2, "ready");
        let mut feed = PaneFeed::new();
        feed.screen(screen.clone(), Vec::new());
        assert!(!feed.resized(size.cols, size.rows));
        feed.data(b"\x1b[2;4r\x1b[?6h");
        let before = feed.screen_term().history_len();
        feed.screen(screen, Vec::new());
        assert_eq!(feed.screen_term().history_len(), before);
        assert_eq!(feed.screen_term().line(0), "line 1");
    }

    #[test]
    fn a_held_screen_is_fed_when_something_else_comes_first_or_its_size_never_does() {
        let size = Size { cols: 40, rows: 5 };
        let (history, screen) = node(size, 12, "ready");
        let mut feed = PaneFeed::new();
        feed.screen(screen.clone(), history.clone());
        feed.data(b"!");
        assert_eq!(feed.screen_term().history_len(), 8);
        assert_eq!(feed.screen_term().line(4), "ready!");

        let mut feed = PaneFeed::new();
        feed.screen(screen, history);
        assert!(!feed.tick(), "not before the wait");
        std::thread::sleep(super::SIZE_WAIT);
        assert!(feed.tick());
        assert_eq!(feed.screen_term().history_len(), 8);
    }

    // #977, kept with the history replay: a size after the repaint's own size only resizes the emulator.
    mod sizes {
        use super::super::PaneFeed;
        use agora_node::terminal::repaint_of;
        use agora_vt::{AlacrittyTerminal, Size, Terminal};

        fn lines(t: &dyn Terminal) -> Vec<String> {
            (0..t.size().rows).map(|r| t.line(r)).collect()
        }

        /// The node's terminal at `size` after the program wrote `bytes`.
        fn node(size: Size, bytes: &[u8]) -> AlacrittyTerminal {
            let mut t = AlacrittyTerminal::new(size);
            t.feed(bytes);
            t
        }

        const WIDE: Size = Size {
            cols: 158,
            rows: 40,
        };
        const PANE: Size = Size { cols: 60, rows: 38 };

        /// A late attach as the terminal client makes one: the repaint at the size another window gave the terminal, that size, then
        /// the size of this console's pane (it sizes the terminal as it opens it). The view shows what the node shows at the new size.
        fn late_attach(program: &[u8]) -> (Vec<String>, PaneFeed) {
            let mut machine = node(WIDE, program);
            let mut v = PaneFeed::new();
            v.screen(repaint_of(&machine), Vec::new());
            assert!(!v.resized(WIDE.cols, WIDE.rows));
            assert_eq!(
                lines(v.screen_term()),
                lines(&machine),
                "painted at its own size"
            );
            machine.resize(PANE);
            assert!(
                !v.resized(PANE.cols, PANE.rows),
                "nothing to ask the machine for"
            );
            (lines(&machine), v)
        }

        #[test]
        fn a_full_screen_program_stays_in_view_when_the_pane_resizes_its_terminal() {
            // The repaint check's probe (scripts/terminal-repaint/probe.py --alt): it draws once and never redraws on a resize. The
            // repaint painted again at 60 columns wrapped its 158-column rows and scrolled the whole screen away (CI, #953, #960).
            let mut program = b"\x1b[?1049h\x1b[H\x1b[2J".to_vec();
            for r in 1..=5 {
                program.extend_from_slice(format!("\x1b[{r};1HALT SCREEN row {r}").as_bytes());
            }
            program.extend_from_slice(b"\x1b[7;1HPROBE READY");
            let (machine, v) = late_attach(&program);
            assert_eq!(lines(v.screen_term()), machine);
            assert_eq!(lines(v.screen_term())[6].trim_end(), "PROBE READY");
        }

        #[test]
        fn a_shell_reflows_as_the_machine_does_when_the_pane_resizes_its_terminal() {
            let mut program = Vec::new();
            for n in 0..12 {
                program.extend_from_slice(
                    format!("$ python3 probe.py /tmp/agt-{n}/p{n}.log --ctl /tmp/agt-{n}/p{n}.ctl --modes 1000,1006\r\nline {n}\r\n")
                        .as_bytes(),
                );
            }
            program.extend_from_slice(b"\r\nPROBE READY\r\n$ ");
            let (machine, v) = late_attach(&program);
            assert_eq!(lines(v.screen_term()), machine);
            assert!(lines(v.screen_term())
                .iter()
                .any(|l| l.trim_end() == "PROBE READY"));
        }

        #[test]
        fn output_after_the_repaint_asks_the_machine_again_on_a_resize() {
            let machine = node(WIDE, b"$ ");
            let mut v = PaneFeed::new();
            v.screen(repaint_of(&machine), Vec::new());
            assert!(!v.resized(WIDE.cols, WIDE.rows));
            v.data(b"more");
            assert!(v.resized(PANE.cols, PANE.rows));
        }
    }

    #[test]
    fn a_repaint_says_how_many_rows_the_screen_has() {
        assert_eq!(
            rows_in(
                b"\x1b[0m\x1b[2J\x1b[1;1Ha\x1b[K\x1b[2;1H\x1b[K\x1b[3;1Hc\x1b[K\x1b[2;5H\x1b[?25h"
            ),
            3
        );
        assert_eq!(rows_in(b"plain"), 0);
        assert_eq!(
            rows_in(b"\x1b[31mred\x1b[12;40H"),
            0,
            "a cursor position is not a row"
        );
    }
}
