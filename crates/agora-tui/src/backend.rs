//! The screen the ratatui client draws on (ADR-0063): the user's terminal through crossterm, or ratatui's `TestBackend` in tests.
//!
//! Raw mode stays where it is today (`agora_platform::RawMode`, enabled by the caller); this only switches to the alternate screen,
//! hides the cursor while a frame is drawn and puts both back when it is dropped. The size and the output use crossterm's own
//! platform code, the exception ADR-0063 point 7 accepts for the vendored crate.

use std::io::{self, Stdout, Write};

use crossterm::cursor::{Hide, Show};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::{Backend, CrosstermBackend, TestBackend};
use ratatui::{Frame, Terminal};

/// The user's terminal while the client runs: the alternate screen, restored when this is dropped (also on a panic unwind).
pub struct Screen<B: Backend> {
    term: Terminal<B>,
    restore: bool,
}

impl Screen<CrosstermBackend<Stdout>> {
    /// Switches stdout to the alternate screen and hides the cursor. The caller has already put the terminal into raw mode.
    pub fn enter() -> io::Result<Self> {
        let mut out = io::stdout();
        out.execute(EnterAlternateScreen)?;
        // From here on a failure leaves the alternate screen again, as dropping a `Screen` would.
        let entered = out
            .execute(Hide)
            .map(|_| ())
            .and_then(|()| Terminal::new(CrosstermBackend::new(io::stdout())));
        match entered {
            Ok(term) => Ok(Self {
                term,
                restore: true,
            }),
            Err(e) => {
                let _ = io::stdout()
                    .execute(Show)
                    .and_then(|o| o.execute(LeaveAlternateScreen));
                Err(e)
            }
        }
    }
}

impl Screen<TestBackend> {
    /// An in-memory screen of `cols` x `rows` for tests; nothing is written to the real terminal.
    pub fn test(cols: u16, rows: u16) -> io::Result<Self> {
        Ok(Self {
            term: Terminal::new(TestBackend::new(cols, rows))
                .map_err(|e| io::Error::other(e.to_string()))?,
            restore: false,
        })
    }

    /// What is on the test screen, one string per row, trailing blanks removed.
    pub fn lines(&self) -> Vec<String> {
        let buf = self.term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }
}

impl<B: Backend> Screen<B> {
    /// Draws one frame. Only the cells that changed since the last frame are written (ratatui keeps the previous buffer).
    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> io::Result<()> {
        self.term
            .draw(render)
            .map(|_| ())
            .map_err(|e| io::Error::other(e.to_string()))
    }

    /// After the terminal was resized: the next frame is drawn whole at the new size.
    pub fn resized(&mut self) -> io::Result<()> {
        self.term
            .autoresize()
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

impl<B: Backend> Drop for Screen<B> {
    fn drop(&mut self) {
        if self.restore {
            let mut out = io::stdout();
            let _ = out.execute(Show);
            let _ = out.execute(LeaveAlternateScreen);
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::pane::TerminalPane;
    use agora_vt::{AlacrittyTerminal, Size, Terminal as _};

    #[test]
    fn a_frame_with_the_terminal_pane_lands_on_the_test_screen() {
        let mut vt = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        vt.feed(b"$ echo hi\r\nhi\r\n$ ");
        let mut screen = Screen::test(20, 3).unwrap();
        screen
            .draw(|f| {
                let area = f.area();
                f.render_widget(TerminalPane::new(&vt), area);
                if let Some(p) = TerminalPane::cursor_position(&vt, area) {
                    f.set_cursor_position(p);
                }
            })
            .unwrap();
        assert_eq!(screen.lines(), ["$ echo hi", "hi", "$"]);
    }
}
