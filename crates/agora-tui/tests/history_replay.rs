//! ADR-0100 decisions 2 and 4: the terminal client's pane, fed what the node sends a subscription that asks for history (kind 4
//! frames, the kind 1 repaint, the kind 2 size) from a probe program on a real pseudo-terminal, holds exactly the node's history in
//! its scrollback (what the wheel reads); a later resize keeps it the node's, and a repaint fed again at its size never copies the
//! screen into it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use agora_node::terminal::{Output, OutputSubscription, TerminalHandle, TerminalOptions};
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

/// A probe that prints 300 numbered lines, then waits, redrawing its screen on every resize.
fn probe() -> TerminalHandle {
    let mut cfg = PtyConfig::new(vec![
        "sh".into(),
        "-c".into(),
        concat!(
            // On a resize it clears and redraws its screen, as the console host (ConPTY) does on Windows.
            "trap 'printf \"\\033[H\\033[2Jprobe redrawn\"' WINCH; ",
            "i=1; while [ $i -le 300 ]; do echo \"history line $i\"; i=$((i+1)); done; printf 'probe ready'; ",
            "while :; do sleep 0.1; done",
        )
        .into(),
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

/// Feeds the pane what the node sends after the screen, as `Preview::pump` does, until the node has been quiet for a moment: output,
/// and sizes (a size after output asks for a fresh screen, which the pane takes with the history it asks for, none once it has its
/// own). The node's own reaction to a resize reaches the pane this way too: on Windows the console host (ConPTY) repaints its screen
/// after a resize, which changes what the node's emulator holds (Windows CI, #964), and the pane gets the same bytes.
fn follow(h: &TerminalHandle, sub: &mut OutputSubscription, feed: &mut PaneFeed) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        match sub.recv_timeout(Duration::from_millis(500)) {
            Ok(Output::Data(b)) => feed.data(&b),
            Ok(Output::Resize(size)) => {
                if feed.resized(size.cols, size.rows) {
                    let (frames, snap, next) =
                        h.subscribe_with_history(feed.history_to_ask() as usize);
                    feed.screen(snap.to_ansi(), frames.concat());
                    assert!(!feed.resized(snap.size.cols, snap.size.rows));
                    *sub = next;
                }
            }
            Ok(other) => panic!("unexpected {other:?}"),
            Err(_) => return,
        }
    }
    panic!("the node never went quiet");
}

#[test]
fn a_fresh_pane_holds_the_replayed_history_and_a_resize_refeed_does_not_repeat_it() {
    let h = probe();
    let feed_from = |lines: usize| {
        let (frames, snap, sub) = h.subscribe_with_history(lines);
        let mut feed = PaneFeed::new();
        feed.screen(snap.to_ansi(), frames.concat());
        assert!(!feed.resized(snap.size.cols, snap.size.rows));
        (feed, snap, sub)
    };
    let kept = h.inspect(history);
    assert!(
        kept.len() >= 290,
        "the node keeps the output: {}",
        kept.len()
    );
    // The numbered lines are all there, in order. ConPTY (Windows) may leave blank rows before them, so the fixture skips leading
    // blank rows here (Codex, #964); the replay below is compared with the node's history exactly, blank rows included.
    let numbered: Vec<&str> = kept
        .iter()
        .map(String::as_str)
        .skip_while(|l| l.is_empty())
        .take(300)
        .collect();
    let want: Vec<String> = (1..=numbered.len())
        .map(|i| format!("history line {i}"))
        .collect();
    assert!(numbered.len() >= 290, "the numbered lines: {numbered:?}");
    assert_eq!(
        numbered,
        want.iter().map(String::as_str).collect::<Vec<_>>()
    );

    // A fresh pane asks for its own scrollback, and gets every line the node keeps, in order, with the screen drawn over them.
    let fresh = PaneFeed::new();
    assert_eq!(fresh.history_to_ask(), HISTORY_LINES);
    let (mut feed, snap, mut sub) = feed_from(HISTORY_LINES as usize);
    assert_eq!(
        history(feed.screen_term()),
        kept,
        "the pane's scrollback is the node's"
    );
    assert_eq!(screen(feed.screen_term()), screen_of(&snap));
    // Once it shows a screen it keeps its own history: watching again asks for none.
    assert_eq!(feed.history_to_ask(), 0);

    // A later size (another window resized the terminal) only resizes the pane's emulator, as the node's own (#977): the history
    // stays the node's, never replayed again. A taller terminal pulls lines out of the history onto the screen in both. The pane
    // follows the node's stream, as the client does, so whatever the node's terminal writes after a resize reaches both.
    for size in [Size { cols: 60, rows: 14 }, Size { cols: 60, rows: 7 }] {
        h.resize(size).unwrap();
        follow(&h, &mut sub, &mut feed);
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
