//! Draws a sample agent screen through the terminal pane on the real terminal, for the captures of ADR-0063 stage 1:
//! `cargo run -p agora-tui --example pane_demo` (`NO_COLOR=1` for the monochrome palette). Any key, or 10 s, ends it.

use std::io::Read;
use std::time::Duration;

use agora_tui::backend::Screen;
use agora_tui::pane::{ColorMode, TerminalPane};
use agora_vt::{AlacrittyTerminal, Size, Terminal};

const SAMPLE: &str = concat!(
    "\x1b[1mAgora terminal pane\x1b[0m  (agora-vt grid drawn by ratatui)\r\n",
    "\x1b[31mred \x1b[32mgreen \x1b[33myellow \x1b[34mblue \x1b[35mmagenta \x1b[36mcyan\x1b[0m\r\n",
    "\x1b[38;5;208m256: orange\x1b[0m  \x1b[38;2;120;200;255mtrue colour\x1b[0m  \x1b[48;2;60;60;90m background \x1b[0m\r\n",
    "\x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[7minverse\x1b[0m \x1b[9mstrike\x1b[0m\r\n",
    "wide: \u{6f22}\u{5b57} \u{3042}\u{3044} emoji-free  box: \u{250c}\u{2500}\u{2500}\u{2510}\r\n",
    "\x1b]0;a title the agent set\x07an OSC title above was interpreted, not printed\r\n",
    "$ ",
);

fn main() -> std::io::Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut vt = AlacrittyTerminal::new(Size { cols, rows });
    vt.feed(SAMPLE.as_bytes());
    let colors = ColorMode::from_env(
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("COLORTERM").ok().as_deref(),
    );
    let raw = crossterm::terminal::enable_raw_mode();
    // Whatever `show` returns, raw mode is switched off again before the error (if any) is reported.
    let shown = show(&vt, colors);
    if raw.is_ok() {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    shown
}

fn show(vt: &AlacrittyTerminal, colors: ColorMode) -> std::io::Result<()> {
    let mut screen = Screen::enter()?;
    screen.draw(|f| {
        let area = f.area();
        f.render_widget(TerminalPane::new(vt).colors(colors), area);
        if let Some(p) = TerminalPane::cursor_position(vt, area) {
            f.set_cursor_position(p);
        }
    })?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = [0u8; 1];
        let _ = std::io::stdin().read(&mut b);
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(10));
    Ok(())
}
