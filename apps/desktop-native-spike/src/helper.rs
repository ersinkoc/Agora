//! The agents of the fixtures, run by the node as ordinary L1 agents (manifests written by the driver). Plain std, no GPUI.
//!
//! - `helper echo`: reads its input and does nothing with it; the PTY's own echo shows what is typed (F1).
//! - `helper flood <bytes/s> <secs>`: idle until a line arrives, then writes coloured text at `bytes/s` for `secs`, then idle again
//!   (F2, F3; the background terminals of F2 use a low rate).

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

fn text_block() -> Vec<u8> {
    let words = [
        "agent", "build", "cargo", "check", "render", "frame", "token", "mesh", "hub", "node",
        "pane", "state",
    ];
    let mut out = Vec::new();
    let mut n: usize = 0;
    while out.len() < 1 << 20 {
        let color = 31 + (n % 7);
        out.extend_from_slice(format!("\x1b[{color}m{n:08}\x1b[0m ").as_bytes());
        for k in 0..10 {
            out.extend_from_slice(words[(n * 7 + k * 3) % words.len()].as_bytes());
            out.push(b' ');
        }
        if n.is_multiple_of(5) {
            out.extend_from_slice(
                "\x1b[1mbold\x1b[0m \x1b[4munder\x1b[0m \x1b[7minverse\x1b[0m 日本語 ".as_bytes(),
            );
        }
        out.extend_from_slice(b"\r\n");
        n += 1;
    }
    out
}

pub fn main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("echo") => {
            print!("echo ready\r\n");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            while std::io::stdin().read_line(&mut line).is_ok_and(|n| n != 0) {
                line.clear();
            }
            0
        }
        Some("flood") => {
            let rate: u64 = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(10_000_000);
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(14);
            let block = text_block();
            print!("flood ready: {rate} B/s for {secs} s per line typed\r\n");
            let _ = std::io::stdout().flush();
            let stdin = std::io::stdin();
            let mut line = String::new();
            let mut pos = 0usize;
            while stdin.lock().read_line(&mut line).is_ok_and(|n| n != 0) {
                line.clear();
                let start = Instant::now();
                let tick = Duration::from_millis(5);
                let mut sent: u64 = 0;
                let mut out = std::io::stdout().lock();
                while start.elapsed() < Duration::from_secs(secs) {
                    let due = (start.elapsed().as_secs_f64() * rate as f64) as u64;
                    if due > sent {
                        let n = ((due - sent) as usize).min(256 * 1024);
                        let mut left = n;
                        while left > 0 {
                            let take = left.min(block.len() - pos);
                            if out.write_all(&block[pos..pos + take]).is_err() {
                                return 1;
                            }
                            pos = (pos + take) % block.len();
                            left -= take;
                        }
                        let _ = out.flush();
                        sent += n as u64;
                    }
                    std::thread::sleep(tick);
                }
                let _ =
                    out.write_all(format!("\x1b[0m\r\nflood done: {sent} bytes\r\n").as_bytes());
                let _ = out.flush();
            }
            0
        }
        Some("probe") => {
            // Unix only: waits for a line, puts its terminal in raw mode (`stty`, on the PTY it inherits), turns SGR mouse
            // reporting on, then prints every byte it receives in hex, one per line.
            print!("probe ready\r\n");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            let _ = std::process::Command::new("stty")
                .args(["raw", "-echo"])
                .stdin(std::process::Stdio::inherit())
                .status();
            print!("\x1b[?1000h\x1b[?1006hprobe armed\r\n");
            let _ = std::io::stdout().flush();
            let mut buf = [0u8; 256];
            loop {
                match std::io::Read::read(&mut std::io::stdin(), &mut buf) {
                    Ok(0) | Err(_) => return 0,
                    Ok(n) => {
                        let mut out = std::io::stdout().lock();
                        for b in &buf[..n] {
                            let _ = write!(out, "{b:02x}\r\n");
                        }
                        let _ = out.flush();
                    }
                }
            }
        }
        _ => 2,
    }
}
