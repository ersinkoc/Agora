//! ADR-0100 decisions 2 and 4: the terminal client's pane, fed what the node sends a subscription that asks for history (kind 4
//! frames, the kind 1 repaint, the kind 2 size) from a probe program on a real pseudo-terminal, holds exactly the node's history in
//! its scrollback (what the wheel reads); a later resize keeps it the node's, and a repaint fed again at its size never copies the
//! screen into it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use agora_node::terminal::{TerminalHandle, TerminalOptions};
use agora_platform::{spawn, PtyConfig};
use agora_tui::preview::{PaneFeed, HISTORY_LINES};
use agora_vt::{Size, Terminal};

const SIZE: Size = Size { cols: 60, rows: 10 };

/// The scrollback's lines as text, oldest first.
fn history(t: &dyn Terminal) -> Vec<String> {
    (1..=t.history_len())
        .rev()
        .map(|back| {
            let mut line = String::new();
            for col in 0..t.size().cols {
                match t.history_cell(back, col) {
                    Some(c) if !c.spacer => line.push(c.ch),
                    Some(_) => {}
                    None => break,
                }
            }
            line.trim_end().to_string()
        })
        .collect()
}

fn screen(t: &dyn Terminal) -> Vec<String> {
    (0..t.size().rows)
        .map(|r| t.line(r).trim_end().to_string())
        .collect()
}

/// A probe that prints 300 numbered lines, then waits.
fn probe() -> TerminalHandle {
    let mut cfg = PtyConfig::new(vec![
        "sh".into(),
        "-c".into(),
        "i=1; while [ $i -le 300 ]; do echo \"history line $i\"; i=$((i+1)); done; printf 'probe ready'; exec sleep 60".into(),
    ]);
    cfg.cols = SIZE.cols;
    cfg.rows = SIZE.rows;
    let opts = TerminalOptions {
        size: SIZE,
        ..TerminalOptions::default()
    };
    let h = TerminalHandle::start(spawn(&cfg).expect("spawn"), opts).expect("start");
    let end = Instant::now() + Duration::from_secs(20);
    let mut seq = 0;
    loop {
        let s = h.wait_change(seq, Duration::from_millis(100));
        if s.lines.iter().any(|l| l.contains("probe ready")) {
            break;
        }
        assert!(
            Instant::now() < end,
            "the probe never got ready: {:?}",
            s.lines
        );
        seq = s.seq;
    }
    h
}

#[test]
fn a_fresh_pane_holds_the_replayed_history_and_a_resize_refeed_does_not_repeat_it() {
    let h = probe();
    let feed_from = |lines: usize| {
        let (frames, snap, _sub) = h.subscribe_with_history(lines);
        let mut feed = PaneFeed::new();
        feed.screen(snap.to_ansi(), frames.concat());
        assert!(!feed.resized(snap.size.cols, snap.size.rows));
        (feed, snap)
    };
    let kept = h.inspect(history);
    assert!(
        kept.len() >= 290,
        "the node keeps the output: {}",
        kept.len()
    );
    assert_eq!(kept.first().map(String::as_str), Some("history line 1"));

    // A fresh pane asks for its own scrollback, and gets every line the node keeps, in order, with the screen drawn over them.
    let fresh = PaneFeed::new();
    assert_eq!(fresh.history_to_ask(), HISTORY_LINES);
    let (mut feed, snap) = feed_from(HISTORY_LINES as usize);
    assert_eq!(
        history(feed.screen_term()),
        kept,
        "the pane's scrollback is the node's"
    );
    assert_eq!(screen(feed.screen_term()), screen_of(&snap));
    // Once it shows a screen it keeps its own history: watching again asks for none.
    assert_eq!(feed.history_to_ask(), 0);

    // A later size (another window resized the terminal) only resizes the pane's emulator, as the node's own (#977): the history
    // stays the node's, never replayed again. A taller terminal pulls lines out of the history onto the screen in both.
    for size in [Size { cols: 60, rows: 14 }, Size { cols: 60, rows: 7 }] {
        h.resize(size).unwrap();
        assert!(!feed.resized(size.cols, size.rows));
        assert_eq!(
            history(feed.screen_term()),
            h.inspect(history),
            "after a resize to {size:?}"
        );
    }
    // A fresh screen without history (watching again) and its own size: the screen is fed, then fed again at its size, and neither
    // copies it into the history (the pane clears its screen first without moving it there).
    let kept = h.inspect(history);
    let (frames, snap, _sub) = h.subscribe_with_history(0);
    assert!(frames.is_empty());
    feed.screen(snap.to_ansi(), Vec::new());
    assert!(!feed.resized(snap.size.cols, snap.size.rows));
    assert_eq!(
        history(feed.screen_term()),
        kept,
        "after a repaint and its size"
    );
    assert_eq!(screen(feed.screen_term()), screen_of(&snap));
    h.kill().unwrap();
}

fn screen_of(snap: &agora_node::terminal::Snapshot) -> Vec<String> {
    let mut lines: Vec<String> = snap
        .lines
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    lines.resize(usize::from(snap.size.rows), String::new());
    lines
}
