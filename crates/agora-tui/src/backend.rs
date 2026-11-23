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

/// Everything the client switches on in the user's terminal, switched off again, then the cursor shown and the alternate screen left:
/// mouse reporting (1000, 1002, 1003, 1006), bracketed paste (2004), focus reports (1004). One sequence for every way out (quit, an
/// early exit, a panic, a stop request), so a terminal is never left reporting the mouse into the shell.
pub const RESET: &[u8] =
    b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?1004l\x1b[?25h\x1b[?1049l";

/// Whether a [`Screen`] is up (the panic hook writes [`RESET`] only then).
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The thread that made the screen: the only one whose panic puts the terminal back (a worker's panic leaves the UI as it is).
static OWNER: std::sync::Mutex<Option<std::thread::ThreadId>> = std::sync::Mutex::new(None);

/// Whether a panic on the calling thread must put the terminal back now: a screen is up and this is the thread that made it.
fn panic_resets_here() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Acquire)
        && OWNER
            .try_lock()
            .is_ok_and(|o| *o == Some(std::thread::current().id()))
}

/// Makes a panic put the terminal back before its message is printed (on the alternate screen the message would be lost, and the
/// shell would get mouse reports), then lets the earlier hook print it.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_resets_here() {
                let mut out = io::stdout();
                let _ = out.write_all(RESET);
                let _ = out.flush();
            }
            previous(info);
        }));
    });
}

/// The user's terminal while the client runs: the alternate screen, restored ([`RESET`]) when this is dropped: on a quit, on any early
/// return and on a panic unwind.
pub struct Screen<B: Backend> {
    term: Terminal<B>,
    restore: bool,
    /// Where [`RESET`] goes on drop; the real standard output when `None` (a fake in tests).
    sink: Option<Box<dyn Write + Send>>,
}

/// The user's real terminal as a [`Screen`].
pub type Window = Screen<CrosstermBackend<Stdout>>;

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
            Ok(term) => {
                install_panic_hook();
                if let Ok(mut owner) = OWNER.lock() {
                    *owner = Some(std::thread::current().id());
                }
                ACTIVE.store(true, std::sync::atomic::Ordering::Release);
                Ok(Self {
                    term,
                    restore: true,
                    sink: None,
                })
            }
            Err(e) => {
                let _ = io::stdout()
                    .execute(Show)
                    .and_then(|o| o.execute(LeaveAlternateScreen));
                Err(e)
            }
        }
    }
}

impl Screen<CrosstermBackend<Stdout>> {
    /// The real terminal's screen type, `cols` x `rows`, without entering the alternate screen or asking the terminal its size: for
    /// tests of the console's state that never draw (drawing on it would write to stdout).
    pub fn offscreen(cols: u16, rows: u16) -> io::Result<Self> {
        let options = ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, cols, rows)),
        };
        Ok(Self {
            term: Terminal::with_options(CrosstermBackend::new(io::stdout()), options)?,
            restore: false,
            sink: None,
        })
    }
}

impl Screen<TestBackend> {
    /// An in-memory screen of `cols` x `rows` for tests; nothing is written to the real terminal.
    pub fn test(cols: u16, rows: u16) -> io::Result<Self> {
        Ok(Self {
            term: Terminal::new(TestBackend::new(cols, rows))
                .map_err(|e| io::Error::other(e.to_string()))?,
            restore: false,
            sink: None,
        })
    }

    /// A test screen that behaves like the real one on drop, writing [`RESET`] to `sink`.
    #[cfg(test)]
    fn test_with_sink(cols: u16, rows: u16, sink: Box<dyn Write + Send>) -> io::Result<Self> {
        let mut s = Self::test(cols, rows)?;
        s.restore = true;
        s.sink = Some(sink);
        Ok(s)
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

    /// After something else wrote to the terminal (a question asked full-screen): the next frame is drawn whole.
    ///
    /// Not ratatui's `Terminal::clear`, which asks the terminal where its cursor is (`ESC [ 6 n`) and reads the answer from stdin, where
    /// the console's own key reader would take it: the screen is cleared without asking, and both of ratatui's buffers are emptied so
    /// the next frame writes every cell that is not blank.
    pub fn invalidate(&mut self) -> io::Result<()> {
        self.term
            .backend_mut()
            .clear()
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.term.swap_buffers();
        self.term.swap_buffers();
        Ok(())
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
            // (A screen with a fake sink is a test's: it does not own the real terminal's flag.)
            if self.sink.is_none() {
                ACTIVE.store(false, std::sync::atomic::Ordering::Release);
            }
            match self.sink.as_mut() {
                Some(sink) => {
                    let _ = sink.write_all(RESET);
                    let _ = sink.flush();
                }
                None => {
                    let mut out = io::stdout();
                    let _ = out.write_all(RESET);
                    let _ = out.flush();
                }
            }
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

    #[test]
    fn the_reset_switches_off_everything_the_client_switches_on() {
        let text = std::str::from_utf8(RESET).unwrap();
        for mode in ["1000", "1002", "1003", "1006", "2004", "1004"] {
            assert!(text.contains(&format!("\x1b[?{mode}l")), "{mode}");
        }
        // The cursor is shown and the alternate screen left, last.
        assert!(text.ends_with("\x1b[?25h\x1b[?1049l"));
    }

    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn only_a_panic_on_the_thread_that_made_the_screen_puts_the_terminal_back() {
        // One test owns the shared flags: nothing else in this crate's tests makes a real screen.
        ACTIVE.store(false, std::sync::atomic::Ordering::Release);
        *OWNER.lock().unwrap() = Some(std::thread::current().id());
        assert!(!panic_resets_here(), "no screen is up");
        ACTIVE.store(true, std::sync::atomic::Ordering::Release);
        assert!(panic_resets_here(), "the owner's panic");
        let elsewhere = std::thread::spawn(panic_resets_here).join().unwrap();
        assert!(
            !elsewhere,
            "a worker thread's panic leaves the screen as it is"
        );
        ACTIVE.store(false, std::sync::atomic::Ordering::Release);
        *OWNER.lock().unwrap() = None;
    }

    #[test]
    fn dropping_the_screen_writes_the_reset_once_and_only_then() {
        let sink = Sink::default();
        let s = Screen::test_with_sink(10, 2, Box::new(sink.clone())).unwrap();
        assert!(sink.0.lock().unwrap().is_empty());
        drop(s);
        assert_eq!(*sink.0.lock().unwrap(), RESET);
        // A screen that was never the real one writes nothing.
        let quiet = Sink::default();
        let mut s = Screen::test_with_sink(10, 2, Box::new(quiet.clone())).unwrap();
        s.restore = false;
        drop(s);
        assert!(quiet.0.lock().unwrap().is_empty());
    }

    #[test]
    fn dropping_during_a_panic_unwind_writes_the_reset() {
        let sink = Sink::default();
        let inner = sink.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _s = Screen::test_with_sink(10, 2, Box::new(inner)).unwrap();
            panic!("boom");
        }));
        assert!(r.is_err());
        assert_eq!(*sink.0.lock().unwrap(), RESET);
    }

    #[test]
    fn invalidate_makes_the_next_frame_write_every_cell_again() {
        let mut s = Screen::test(10, 2).unwrap();
        s.draw(|f| f.render_widget(ratatui::widgets::Paragraph::new("hello"), f.area()))
            .unwrap();
        s.invalidate().unwrap();
        // The test backend was cleared; the same frame drawn again writes "hello" again instead of skipping unchanged cells.
        assert_eq!(s.lines()[0], "");
        s.draw(|f| f.render_widget(ratatui::widgets::Paragraph::new("hello"), f.area()))
            .unwrap();
        assert_eq!(s.lines()[0], "hello");
    }
}
