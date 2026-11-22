//! History replay cost (ADR-0100 decision 2): what a window that asks for a full history costs the node, against the budgets in
//! `docs/design/performance.md` §1. Three figures, as the ADR names them:
//!
//! - the time the terminal's lock is held for the history of a full 2000-line, 200-column scrollback (the capture; formatting runs
//!   after the lock is released), plain and with a colour change every few characters (the byte cap's worst case);
//! - the bytes one attach sends (kind 4 history, kind 1 repaint);
//! - "terminal snapshot after switching ≤ 150 ms" with a full history: a real node on this machine, from the `TerminalSubscribe` sent
//!   on its control socket until the kind 2 size frame after the repaint arrived, with and without history, on one connection and on a
//!   new connection per subscription (as the node's uplink opens one for each subscription the hub routes).

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use agora_node::control::{
    Client, KIND_TERM_DATA, KIND_TERM_HISTORY, KIND_TERM_RESIZE, KIND_TERM_SNAPSHOT,
};
use agora_node::history::HistoryCapture;
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok, command_request::Command as Cmd};
use agora_vt::{AlacrittyTerminal, Size, Terminal};

/// The lines a window asks for and the node keeps.
const LINES: usize = 2000;
const COLS: u16 = 200;
const ROWS: u16 = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureResult {
    pub content: String,
    /// Scrollback lines the terminal keeps, and how many the replay carries (fewer when the byte cap is reached).
    pub kept_lines: usize,
    pub replayed_lines: usize,
    /// Held under the terminal's lock: copying the cells (p50, max over the runs).
    pub lock_p50_ms: f64,
    pub lock_max_ms: f64,
    /// After the lock: formatting the frames (p50).
    pub format_p50_ms: f64,
    pub bytes: usize,
    pub frames: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryReport {
    pub commit: String,
    pub capture: Vec<CaptureResult>,
    /// Subscribe → size frame after the repaint, through a real node's control socket, full history (p50, p95).
    pub switch_history_p50_ms: f64,
    pub switch_history_p95_ms: f64,
    /// The same without history.
    pub switch_plain_p50_ms: f64,
    pub switch_plain_p95_ms: f64,
    /// Both again (p95) with each subscription on a new connection, as the node's uplink makes one per routed subscription: adds the
    /// wait until the control socket accepts it.
    pub switch_history_new_connection_p95_ms: f64,
    pub switch_plain_new_connection_p95_ms: f64,
    /// Bytes of one attach with a full history: the kind 4 frames, their count, and the kind 1 repaint.
    pub attach_history_bytes: usize,
    pub attach_history_frames: usize,
    pub attach_repaint_bytes: usize,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn percentile(samples: &mut [Duration], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort();
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    ms(samples[i.min(samples.len() - 1)])
}

/// A 200×50 terminal with a full 2000-line scrollback of `line(i)`.
fn filled(line: impl Fn(usize) -> String) -> AlacrittyTerminal {
    let mut t = AlacrittyTerminal::with_scrollback(
        Size {
            cols: COLS,
            rows: ROWS,
        },
        LINES,
    );
    for i in 0..LINES + usize::from(ROWS) + 10 {
        t.feed(line(i).as_bytes());
        t.feed(b"\r\n");
    }
    t
}

fn capture(content: &str, t: &AlacrittyTerminal, runs: usize) -> CaptureResult {
    let mut lock = Vec::new();
    let mut format = Vec::new();
    let mut bytes = 0;
    let mut frames = 0;
    for _ in 0..runs {
        let t0 = Instant::now();
        let c = HistoryCapture::capture(t, LINES);
        lock.push(t0.elapsed());
        let t1 = Instant::now();
        let f = c.into_frames();
        format.push(t1.elapsed());
        bytes = f.iter().map(Vec::len).sum();
        frames = f.len();
    }
    // The lines the replay carries: what an empty window ends with.
    let mut w = AlacrittyTerminal::with_scrollback(t.size(), LINES);
    for f in agora_node::history::history_of(t, LINES) {
        w.feed(&f);
    }
    CaptureResult {
        content: content.into(),
        kept_lines: t.history_len(),
        replayed_lines: w.history_len(),
        lock_p50_ms: percentile(&mut lock.clone(), 0.5),
        lock_max_ms: lock.iter().copied().max().map_or(0.0, ms),
        format_p50_ms: percentile(&mut format, 0.5),
        bytes,
        frames,
    }
}

fn git(dir: &Path, args: &[&str]) -> std::io::Result<()> {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "bench")
        .env("GIT_AUTHOR_EMAIL", "bench@example.com")
        .env("GIT_COMMITTER_NAME", "bench")
        .env("GIT_COMMITTER_EMAIL", "bench@example.com")
        .status()?
        .success();
    ok.then_some(())
        .ok_or_else(|| std::io::Error::other(format!("git {args:?} failed")))
}

fn subscribe(terminal_id: &str, history_lines: u32) -> Cmd {
    Cmd::TerminalSubscribe(v1::TerminalSubscribe {
        terminal_id: terminal_id.into(),
        format: "terminal.raw".into(),
        history_lines,
    })
}

/// Subscribes on `stream` and reads until the size frame after the repaint: the time, the kind 4 bytes and frames, the kind 1 bytes.
fn attach(
    c: &mut Client,
    terminal_id: &str,
    stream: u32,
    lines: u32,
) -> std::io::Result<(Duration, usize, usize, usize)> {
    let t0 = Instant::now();
    c.call_on(stream, subscribe(terminal_id, lines))?
        .map_err(|e| std::io::Error::other(e.code))?;
    let (mut history, mut frames, mut repaint) = (0, 0, 0);
    let end = t0 + Duration::from_secs(10);
    while Instant::now() < end {
        let Some(f) = c.read_frame(Some(Duration::from_millis(100)))? else {
            continue;
        };
        if f.stream_id != stream {
            continue;
        }
        match f.kind {
            KIND_TERM_HISTORY => {
                history += f.payload.len();
                frames += 1;
            }
            KIND_TERM_SNAPSHOT => repaint = f.payload.len(),
            KIND_TERM_RESIZE if repaint > 0 => return Ok((t0.elapsed(), history, frames, repaint)),
            _ => {}
        }
    }
    Err(std::io::Error::other("no snapshot"))
}

/// The switch measurement through a real node on this machine.
fn switch(dir: &Path, runs: usize) -> std::io::Result<Switch> {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo)?;
    git(&repo, &["init", "-q", "-b", "main"])?;
    std::fs::write(repo.join("a.txt"), "x")?;
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-q", "-m", "init"])?;
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests)?;
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )?;
    let node = agora_node::daemon::Node::start(&agora_node::daemon::NodeConfig {
        resume_all: false,
        node_id: "bench".into(),
        socket: dir.join("n.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("worktrees"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: NodeLimits::default(),
    })
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut c = Client::connect(&dir.join("n.sock"))?;
    let started = c
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "shell".into(),
            ..Default::default()
        }))?
        .map_err(|e| std::io::Error::other(e.code))?;
    let Some(command_ok::Result::AgentStart(s)) = started.result else {
        return Err(std::io::Error::other("no agent"));
    };
    let tid = s.terminal_id;
    c.call(Cmd::TerminalResize(v1::TerminalResize {
        terminal_id: tid.clone(),
        cols: u32::from(COLS),
        rows: u32::from(ROWS),
    }))?
    .map_err(|e| std::io::Error::other(e.code))?;
    // Fill the scrollback: 2100 lines of 200 columns, each in a colour of its own.
    c.call_on(3, subscribe(&tid, 0))?
        .map_err(|e| std::io::Error::other(e.code))?;
    c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: tid.clone(),
        data: b"i=0; while [ $i -lt 2100 ]; do printf '\\033[3%dm%0200d\\033[0m\\n' $((i%8)) $i; i=$((i+1)); done; echo fill-$((6*7))-done\n"
            .to_vec(),
    }))?
    .map_err(|e| std::io::Error::other(e.code))?;
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(60);
    while Instant::now() < end && !String::from_utf8_lossy(&seen).contains("fill-42-done") {
        if let Some(f) = c.read_frame(Some(Duration::from_millis(100)))? {
            if f.stream_id == 3 && f.kind == KIND_TERM_DATA {
                seen.extend_from_slice(&f.payload);
                let keep = seen.len().saturating_sub(64);
                seen.drain(..keep);
            }
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    let mut m = Switch::default();
    // On one connection, a stream each (the earlier subscriptions stay open and idle): the node's own work.
    let mut stream = 5;
    for _ in 0..runs {
        let (d, h, f, r) = attach(&mut c, &tid, stream, LINES as u32)?;
        m.history.push(d);
        m.bytes = (h, f, r);
        let (d, ..) = attach(&mut c, &tid, stream + 1, 0)?;
        m.plain.push(d);
        stream += 2;
    }
    // On a connection of its own, as the node's uplink opens one for each subscription the hub routes: this adds the wait until the
    // control socket accepts it.
    let socket = dir.join("n.sock");
    for _ in 0..runs {
        let (d, ..) = attach(&mut Client::connect(&socket)?, &tid, 3, LINES as u32)?;
        m.history_new_connection.push(d);
        let (d, ..) = attach(&mut Client::connect(&socket)?, &tid, 3, 0)?;
        m.plain_new_connection.push(d);
    }
    drop(c);
    drop(node);
    Ok(m)
}

#[derive(Default)]
struct Switch {
    history: Vec<Duration>,
    plain: Vec<Duration>,
    history_new_connection: Vec<Duration>,
    plain_new_connection: Vec<Duration>,
    bytes: (usize, usize, usize),
}

/// Measure the history replay's cost.
pub fn run(commit: &str, runs: usize) -> std::io::Result<HistoryReport> {
    let plain = filled(|i| format!("{i:0>200}"));
    // A colour change every 4 characters: about 2.5 KB a line formatted, well past the byte cap for 2000 lines.
    let coloured = filled(|i| {
        let mut s = String::new();
        for c in 0..50 {
            s.push_str(&format!("\x1b[38;5;{}m{:0>4}", (i + c) % 256, c));
        }
        s
    });
    let capture = vec![
        capture("plain", &plain, runs),
        capture("colour every 4 characters", &coloured, runs),
    ];

    let mut report = HistoryReport {
        commit: commit.to_string(),
        capture,
        switch_history_p50_ms: 0.0,
        switch_history_p95_ms: 0.0,
        switch_plain_p50_ms: 0.0,
        switch_plain_p95_ms: 0.0,
        switch_history_new_connection_p95_ms: 0.0,
        switch_plain_new_connection_p95_ms: 0.0,
        attach_history_bytes: 0,
        attach_history_frames: 0,
        attach_repaint_bytes: 0,
    };
    if agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED {
        let dir = std::env::temp_dir().join(format!("agora-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let measured = switch(&dir, runs);
        let _ = std::fs::remove_dir_all(&dir);
        let mut m = measured?;
        let (h, f, r) = m.bytes;
        report.switch_history_p50_ms = percentile(&mut m.history, 0.5);
        report.switch_history_p95_ms = percentile(&mut m.history, 0.95);
        report.switch_plain_p50_ms = percentile(&mut m.plain, 0.5);
        report.switch_plain_p95_ms = percentile(&mut m.plain, 0.95);
        report.switch_history_new_connection_p95_ms =
            percentile(&mut m.history_new_connection, 0.95);
        report.switch_plain_new_connection_p95_ms = percentile(&mut m.plain_new_connection, 0.95);
        report.attach_history_bytes = h;
        report.attach_history_frames = f;
        report.attach_repaint_bytes = r;
    }
    Ok(report)
}
