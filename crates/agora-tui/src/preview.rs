//! The selected agent's terminal, watched through the hub and kept in a terminal emulator so the console can paint it inside its
//! right-hand panel with the agent's own colours. Watching never resizes the agent's terminal (other windows may be showing it);
//! the emulator follows the agent's size instead.

use std::time::Duration;

use agora_vt::{AlacrittyTerminal, Size, Terminal};

use crate::client::{Session, TermEvent};

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
}

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
        })
    }

    /// Applies what arrived for this terminal, without waiting.
    pub fn pump(&mut self, session: &mut Session) -> Pumped {
        let mut seen = Pumped::Nothing;
        for _ in 0..BATCH {
            match session.terminal_next(self.stream, Duration::ZERO) {
                TermEvent::Quiet => break,
                TermEvent::Data(b) => {
                    self.term.feed(&b);
                    self.output_since_screen = true;
                    seen = Pumped::Changed;
                }
                TermEvent::Screen(b) => {
                    let rows = rows_in(&b);
                    if rows > 0 && rows != self.term.size().rows {
                        self.term.resize(Size {
                            cols: self.term.size().cols,
                            rows,
                        });
                    }
                    self.term.feed(&b);
                    self.screen = b;
                    self.output_since_screen = false;
                    seen = Pumped::Changed;
                }
                TermEvent::Resized(cols, rows) => {
                    self.term.resize(Size { cols, rows });
                    // The size that belongs to the repaint just applied: paint it again at that size.
                    if !self.output_since_screen && !self.screen.is_empty() {
                        let screen = self.screen.clone();
                        self.term.feed(&screen);
                    }
                    seen = Pumped::Changed;
                }
                TermEvent::Exit(_) | TermEvent::Ended => return Pumped::Ended,
                TermEvent::Lost => return Pumped::Lost,
            }
        }
        seen
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
    use super::rows_in;

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
