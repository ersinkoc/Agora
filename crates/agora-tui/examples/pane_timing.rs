//! Timing of the terminal pane against docs/design/performance.md (ADR-0063 point 3): `cargo run --release -p agora-tui --example pane_timing`.
//!
//! - frame: a 200x60 screen full of 24-bit coloured text, one new line per frame, drawn through ratatui's diff (`TestBackend`);
//! - echo: one typed character fed to the screen and the frame drawn (the client's share of keystroke → echo, budget 16 ms);
//! - flood: 10 MB of coloured output fed in 64 KiB chunks with a frame drawn every 16 ms of wall time (budget: steady 60 fps,
//!   no step over 50 ms), reported as throughput and the longest frame.

#![allow(clippy::print_stdout)]

use std::time::{Duration, Instant};

use agora_tui::backend::Screen;
use agora_tui::pane::TerminalPane;
use agora_vt::{AlacrittyTerminal, Size, Terminal};

fn p(v: &mut [Duration], q: f64) -> Duration {
    v.sort();
    v[((v.len() as f64 - 1.0) * q) as usize]
}

fn coloured_line(i: usize) -> Vec<u8> {
    let mut s = String::new();
    for k in 0..20 {
        s.push_str(&format!(
            "\x1b[38;2;{};{};{}m{:<9}\x1b[0m ",
            (i * 7 + k * 13) % 256,
            (i * 3 + k * 29) % 256,
            (k * 11) % 256,
            format!("w{i}.{k}")
        ));
    }
    s.push_str("\r\n");
    s.into_bytes()
}

fn main() -> std::io::Result<()> {
    let (cols, rows) = (200u16, 60u16);
    let mut vt = AlacrittyTerminal::new(Size { cols, rows });
    let mut screen = Screen::test(cols, rows)?;
    for i in 0..rows as usize {
        vt.feed(&coloured_line(i));
    }

    // Frames.
    let mut frames = Vec::new();
    for i in 0..500 {
        vt.feed(&coloured_line(1000 + i));
        let t = Instant::now();
        screen.draw(|f| f.render_widget(TerminalPane::new(&vt), f.area()))?;
        frames.push(t.elapsed());
    }
    println!(
        "frame 200x60 full colour: p50 {:?}  p95 {:?}  max {:?}",
        p(&mut frames.clone(), 0.5),
        p(&mut frames.clone(), 0.95),
        frames.iter().max().copied().unwrap_or_default()
    );

    // Echo: one character typed at a prompt, then the frame.
    vt.feed(b"\x1b[2J\x1b[H$ ");
    let mut echo = Vec::new();
    for i in 0..500 {
        let t = Instant::now();
        vt.feed(&[b'a' + (i % 26) as u8]);
        screen.draw(|f| f.render_widget(TerminalPane::new(&vt), f.area()))?;
        echo.push(t.elapsed());
    }
    println!(
        "echo (feed one key + frame): p50 {:?}  p95 {:?}  (budget 16 ms)",
        p(&mut echo.clone(), 0.5),
        p(&mut echo.clone(), 0.95)
    );

    // Flood: 10 MB, a frame every 16 ms of wall time.
    let mut chunk = Vec::new();
    let mut n = 0;
    while chunk.len() < 64 * 1024 {
        chunk.extend_from_slice(&coloured_line(n));
        n += 1;
    }
    let total = 10 * 1024 * 1024;
    let (mut fed, mut drawn, mut longest, mut longest_step) =
        (0usize, 0usize, Duration::ZERO, Duration::ZERO);
    let start = Instant::now();
    let mut next_frame = start;
    // A step is one chunk fed plus the frame drawn after it, if one is due: the client's longest stall under a flood.
    while fed < total {
        let step = Instant::now();
        vt.feed(&chunk);
        fed += chunk.len();
        if Instant::now() >= next_frame {
            let t = Instant::now();
            screen.draw(|f| f.render_widget(TerminalPane::new(&vt), f.area()))?;
            longest = longest.max(t.elapsed());
            drawn += 1;
            next_frame += Duration::from_millis(16);
        }
        longest_step = longest_step.max(step.elapsed());
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "flood 10 MB: {:.1} MB/s fed, {drawn} frames in {secs:.2} s ({:.0} fps), longest frame {longest:?}, longest step (feed + frame) {longest_step:?} (budgets: 10 MB/s at 60 fps, no step over 50 ms)",
        total as f64 / 1e6 / secs,
        drawn as f64 / secs
    );
    Ok(())
}
