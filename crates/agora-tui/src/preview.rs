//! The selected agent's terminal, watched through the hub and kept in a terminal emulator so the console can paint it inside its
//! right-hand panel with the agent's own colours. Watching never resizes the agent's terminal (other windows may be showing it);
//! the emulator follows the agent's size instead.

use std::time::Duration;

use agora_vt::{AlacrittyTerminal, Size, Terminal};

use crate::client::{Session, TermEvent};
use crate::termio::{ModeScanner, ProgramModes};

/// Columns assumed until the machine says the terminal's size (a node that does not send it after the snapshot).
const GUESS_COLS: u16 = 120;
/// Most stream items handled per call, so a flood of output never holds up the keyboard.
const BATCH: usize = 256;

pub struct Preview {
    /// The hub-wide id of the agent being watched.
    pub agent: String,
    stream: u32,
    term: AlacrittyTerminal,
    /// The last full repaint, applied again when the size it was meant for arrives after it.
    screen: Vec<u8>,
    output_since_screen: bool,
    /// The last repaint waits for its size: the machine sends the size a repaint was made at right after it (protocol.md, kind 2),
    /// and only that size paints the repaint again. A later size is the terminal resized since (this console sizes the agent's
    /// terminal to its pane right after it opens it). Painted at that size the repaint's rows, made at the old width, wrapped and
    /// scrolled its first rows away: a full-screen program that does not redraw on a resize showed an empty pane until the next
    /// refresh. The emulator here is the machine's own kind, so resizing what it holds gives the machine's screen at the new size.
    screen_unsized: bool,
    /// The mouse and focus modes the program switched on, read from the bytes that arrive (see `termio`), the repaint included: a
    /// current node's repaint sets or resets each mode it tracks (ADR-0100), so a mode switched on before this view opened is known.
    scanner: ModeScanner,
    /// When to ask the machine for a fresh repaint: after its terminal was resized while output had come since the last repaint.
    /// Reflowing that output here, size after size, mangled it until the tab was opened again (TUI review); the machine's own
    /// screen at the new size is the truth.
    repaint_at: Option<std::time::Instant>,
    /// Output was dropped while nothing read it (a question was open): watch again at the next pump, for the machine's own screen.
    behind: bool,
}

/// How long after a resize the fresh repaint is asked for: the agent has redrawn for its new size by then, and a burst of resizes
/// (a window being dragged) asks once.
const REPAINT_AFTER: Duration = Duration::from_millis(300);

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

impl Preview {
    /// Starts watching `agent`'s terminal (`<node>/<local id>`; its terminal is `t-<local id>`).
    pub fn open(session: &mut Session, agent: &str) -> Result<Self, String> {
        let (node, local) = agent.split_once('/').unwrap_or(("", agent));
        let stream = session.terminal_open(node, &format!("t-{local}"))?;
        Ok(Self {
            agent: agent.to_string(),
            stream,
            term: AlacrittyTerminal::new(Size {
                cols: GUESS_COLS,
                rows: 24,
            }),
            screen: Vec::new(),
            output_since_screen: false,
            screen_unsized: false,
            scanner: ModeScanner::default(),
            repaint_at: None,
            behind: false,
        })
    }

    /// Applies what arrived for this terminal, without waiting.
    pub fn pump(&mut self, session: &mut Session) -> Pumped {
        let mut seen = Pumped::Nothing;
        if self.behind
            || self
                .repaint_at
                .is_some_and(|at| at.elapsed() >= REPAINT_AFTER)
        {
            self.behind = false;
            self.repaint_at = None;
            // Watching again starts with the machine's full screen at its current size.
            let (node, local) = self.agent.split_once('/').unwrap_or(("", &self.agent));
            let (node, terminal) = (node.to_string(), format!("t-{local}"));
            session.terminal_close(self.stream);
            match session.terminal_open(&node, &terminal) {
                Ok(stream) => self.stream = stream,
                Err(_) => return Pumped::Ended,
            }
        }
        for _ in 0..BATCH {
            match session.terminal_next(self.stream, Duration::ZERO) {
                TermEvent::Quiet => break,
                TermEvent::Behind => {
                    self.behind = true;
                    break;
                }
                TermEvent::Data(b) => {
                    self.scanner.feed(&b);
                    self.term.feed(&b);
                    self.output_since_screen = true;
                    seen = Pumped::Changed;
                }
                TermEvent::Screen(b) => {
                    self.apply_screen(b);
                    seen = Pumped::Changed;
                }
                TermEvent::Resized(cols, rows) => {
                    self.apply_size(cols, rows);
                    seen = Pumped::Changed;
                }
                TermEvent::Exit(_) | TermEvent::Ended => return Pumped::Ended,
                TermEvent::Lost => return Pumped::Lost,
            }
        }
        seen
    }

    /// A full repaint of the machine's screen.
    fn apply_screen(&mut self, b: Vec<u8>) {
        let rows = rows_in(&b);
        if rows > 0 && rows != self.term.size().rows {
            self.term.resize(Size {
                cols: self.term.size().cols,
                rows,
            });
        }
        self.scanner.feed(&b);
        self.term.feed(&b);
        self.screen = b;
        self.output_since_screen = false;
        self.screen_unsized = true;
    }

    /// The terminal's size: the one the last repaint was made at, or a resize since.
    fn apply_size(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size { cols, rows });
        let own = std::mem::take(&mut self.screen_unsized);
        if self.output_since_screen {
            // Output came after the repaint: the machine is asked for its screen at the new size (see `repaint_at`).
            self.repaint_at = Some(std::time::Instant::now());
        } else if own {
            // The size that belongs to the repaint just applied: paint it again at that size.
            let screen = self.screen.clone();
            self.term.feed(&screen);
        }
        // Otherwise the terminal was resized since its repaint was painted at its own size: resizing the emulator was all of it
        // (see `screen_unsized`).
    }

    /// The mouse and focus modes the program has switched on, as far as this view has seen.
    pub fn program_modes(&self) -> ProgramModes {
        self.scanner.modes()
    }

    pub fn screen(&self) -> &dyn Terminal {
        &self.term
    }

    /// Stops watching.
    pub fn close(self, session: &mut Session) {
        session.terminal_close(self.stream);
    }
}

#[cfg(test)]
mod tests {
    use super::{rows_in, Preview, GUESS_COLS};
    use crate::termio::ModeScanner;
    use agora_node::terminal::repaint_of;
    use agora_vt::{AlacrittyTerminal, Size, Terminal};

    /// A view as `Preview::open` makes it, without a hub.
    fn view() -> Preview {
        Preview {
            agent: "laptop/agent-1".to_string(),
            stream: 1,
            term: AlacrittyTerminal::new(Size {
                cols: GUESS_COLS,
                rows: 24,
            }),
            screen: Vec::new(),
            output_since_screen: false,
            screen_unsized: false,
            scanner: ModeScanner::default(),
            repaint_at: None,
            behind: false,
        }
    }

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
    fn late_attach(program: &[u8]) -> (Vec<String>, Preview) {
        let mut machine = node(WIDE, program);
        let mut v = view();
        v.apply_screen(repaint_of(&machine));
        v.apply_size(WIDE.cols, WIDE.rows);
        assert_eq!(lines(&v.term), lines(&machine), "painted at its own size");
        machine.resize(PANE);
        v.apply_size(PANE.cols, PANE.rows);
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
        assert_eq!(lines(&v.term), machine);
        assert_eq!(lines(&v.term)[6].trim_end(), "PROBE READY");
        assert_eq!(v.repaint_at, None, "nothing to ask the machine for");
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
        assert_eq!(lines(&v.term), machine);
        assert!(lines(&v.term).iter().any(|l| l.trim_end() == "PROBE READY"));
    }

    #[test]
    fn output_after_the_repaint_asks_the_machine_again_on_a_resize() {
        let machine = node(WIDE, b"$ ");
        let mut v = view();
        v.apply_screen(repaint_of(&machine));
        v.apply_size(WIDE.cols, WIDE.rows);
        v.term.feed(b"more");
        v.output_since_screen = true;
        v.apply_size(PANE.cols, PANE.rows);
        assert!(v.repaint_at.is_some());
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
