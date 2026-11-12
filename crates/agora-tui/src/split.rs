//! The console as an editor-like screen: machines and their agents on the left, the selected agent's live terminal on the right,
//! a status line at the bottom. A pure function of the hub's snapshot and the selected agent's screen (render never mutates state,
//! AGENTS.md): it returns the bytes that paint one whole frame, and no terminal I/O happens here.

use std::fmt::Write as _;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{AgentSnapshot, AgentState, Snapshot};
use agora_vt::{Style, Terminal};

use crate::render::Text;

/// What one frame shows.
pub struct View<'a> {
    pub snapshot: &'a Snapshot,
    /// The agent the keyboard points at (hub-wide id).
    pub selected: &'a str,
    /// The selected agent's screen, at the agent's own size; `None` while it is opening or when it has no terminal.
    pub screen: Option<&'a dyn Terminal>,
    /// The last thing the user did.
    pub notice: &'a str,
    pub color: bool,
    /// The size of the window this is drawn in.
    pub cols: u16,
    pub rows: u16,
}

/// The agents in the order the tree shows them: machines online first, then by name; agents by id within a machine.
pub fn tree_order(s: &Snapshot) -> Vec<String> {
    let mut nodes: Vec<_> = s.nodes.iter().collect();
    nodes.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| name(a).cmp(name(b))));
    let mut out = Vec::new();
    for n in nodes {
        let mut agents: Vec<&AgentSnapshot> =
            s.agents.iter().filter(|a| a.node_id == n.node_id).collect();
        agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        out.extend(agents.into_iter().map(|a| a.agent_id.clone()));
    }
    // Agents of a machine the hub does not list come last, so none is unreachable.
    let mut rest: Vec<&AgentSnapshot> = s
        .agents
        .iter()
        .filter(|a| !s.nodes.iter().any(|n| n.node_id == a.node_id))
        .collect();
    rest.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    out.extend(rest.into_iter().map(|a| a.agent_id.clone()));
    out
}

fn name(n: &agora_protocol::v1::NodeSnapshot) -> &str {
    if n.hostname.is_empty() {
        &n.node_id
    } else {
        &n.hostname
    }
}

fn state_of(a: &AgentSnapshot) -> AgentState {
    AgentState::try_from(a.state).unwrap_or(AgentState::Unspecified)
}

/// The part of a hub-wide id after `<node>/`.
fn local(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, l)| l)
}

/// A state as a mark and a colour (design tokens: working accent, needs-you warm, finished green, the rest quiet).
fn mark(state: AgentState, running: bool) -> (&'static str, &'static str) {
    if !running {
        return ("○", "\x1b[2m");
    }
    match state {
        AgentState::Working => ("●", "\x1b[36m"),
        AgentState::Blocked => ("◐", "\x1b[33;1m"),
        AgentState::Done => ("✓", "\x1b[32m"),
        AgentState::Idle => ("○", ""),
        AgentState::Unspecified => ("◌", "\x1b[2m"),
    }
}

fn state_word(state: AgentState, t: Text<'_>) -> String {
    t(
        match state {
            AgentState::Working => key!("tui-state-working"),
            AgentState::Blocked => key!("tui-state-needs-you"),
            AgentState::Idle => key!("tui-state-idle"),
            AgentState::Done => key!("tui-state-finished"),
            AgentState::Unspecified => key!("tui-state-unknown"),
        },
        &[],
    )
}

/// Text cut or padded to exactly `width` columns. Every character counts as one column; agent ids, kinds and host names are what
/// goes here, and a wide character only shortens the padding.
fn fit(text: &str, width: usize) -> String {
    let mut out: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(width)
        .collect();
    let n = out.chars().count();
    if n < width {
        out.extend(std::iter::repeat_n(' ', width - n));
    } else if text.chars().count() > width && width > 0 {
        out.pop();
        out.push('…');
    }
    out
}

/// One line of the left panel: its text and how it is painted.
struct Row {
    text: String,
    paint: &'static str,
    selected: bool,
}

fn tree(v: &View<'_>, t: Text<'_>) -> Vec<Row> {
    let s = v.snapshot;
    let mut rows = Vec::new();
    // What needs the user first: answers are given here (y / n), so they must not scroll away.
    if !s.approvals.is_empty() {
        rows.push(Row {
            text: t(key!("tui-attention"), &[]),
            paint: "\x1b[33;1m",
            selected: false,
        });
        let answered =
            crate::client::answered_approval(s, Some(v.selected)).map(|a| a.approval_id.as_str());
        for a in &s.approvals {
            let what = format!("{} {}", a.action, a.detail);
            // The one y / n answer is marked.
            let mark = if Some(a.approval_id.as_str()) == answered {
                "▶"
            } else {
                "!"
            };
            rows.push(Row {
                text: format!(" {mark} {}: {}", local(&a.agent_id), what.trim()),
                paint: "\x1b[33m",
                selected: false,
            });
        }
        rows.push(Row {
            text: format!(" {}", t(key!("tui-split-answer"), &[])),
            paint: "\x1b[2m",
            selected: false,
        });
        rows.push(Row {
            text: String::new(),
            paint: "",
            selected: false,
        });
    }
    let mut nodes: Vec<_> = s.nodes.iter().collect();
    nodes.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| name(a).cmp(name(b))));
    for n in nodes {
        let (dot, paint) = if n.online {
            ("●", "\x1b[32m")
        } else {
            ("○", "\x1b[2m")
        };
        let mem = if n.memory_used_bytes > 0 {
            format!("  {}", bytes(n.memory_used_bytes))
        } else {
            String::new()
        };
        rows.push(Row {
            text: format!("{dot} {}{mem}", name(n)),
            paint,
            selected: false,
        });
        let mut agents: Vec<&AgentSnapshot> =
            s.agents.iter().filter(|a| a.node_id == n.node_id).collect();
        agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        for a in agents {
            let st = state_of(a);
            let (m, paint) = mark(st, a.running);
            rows.push(Row {
                text: format!("  {m} {}  {}", local(&a.agent_id), a.kind),
                paint,
                selected: a.agent_id == v.selected,
            });
        }
    }
    if s.agents.is_empty() {
        rows.push(Row {
            text: String::new(),
            paint: "",
            selected: false,
        });
        rows.push(Row {
            text: t(key!("tui-no-agents"), &[]),
            paint: "\x1b[2m",
            selected: false,
        });
    }
    rows
}

/// Memory as people read it (1 decimal from GB down, whole MB below).
fn bytes(b: u64) -> String {
    const MB: u64 = 1 << 20;
    const GB: u64 = 1 << 30;
    if b >= GB {
        format!("{:.1} GB", b as f64 / GB as f64)
    } else {
        format!("{} MB", b.div_ceil(MB))
    }
}

/// One row of the agent's screen, cut to `width` columns, with its colours when `color` is on.
fn screen_row(term: &dyn Terminal, row: u16, width: u16, color: bool) -> String {
    let mut out = String::new();
    let mut current = Style::default();
    let mut used = 0u16;
    let cols = term.size().cols;
    for col in 0..cols.min(width) {
        let Some(cell) = term.cell(row, col) else {
            break;
        };
        if cell.spacer {
            continue;
        }
        // A wide character that would spill over the panel's edge becomes a space.
        let wide = term.cell(row, col + 1).is_some_and(|n| n.spacer);
        let w = if wide { 2 } else { 1 };
        if color && cell.style != current {
            out.push_str(&cell.style.sgr());
            current = cell.style;
        }
        if used + w > width {
            out.push(' ');
            used += 1;
            break;
        }
        out.push(if cell.ch.is_control() { ' ' } else { cell.ch });
        used += w;
    }
    if color && current != Style::default() {
        out.push_str("\x1b[0m");
    }
    if used < width {
        out.extend(std::iter::repeat_n(' ', usize::from(width - used)));
    }
    out
}

/// The whole frame: every row is positioned and cleared to its end, so nothing of the previous frame is left.
pub fn frame(v: &View<'_>, t: Text<'_>) -> String {
    let (cols, rows) = (v.cols.max(20), v.rows.max(5));
    let (bold, dim, reset, rev) = if v.color {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m", "\x1b[7m")
    } else {
        ("", "", "", "")
    };
    let left = (cols / 3).clamp(18, 36);
    let right = cols.saturating_sub(left + 1);
    let body = rows - 1; // the last row is the status line
    let s = v.snapshot;
    let agent = s.agents.iter().find(|a| a.agent_id == v.selected);

    let tree = tree(v, t);
    // Keep the selected agent in view when the tree is taller than the panel.
    let at = tree.iter().position(|r| r.selected).unwrap_or(0);
    let first = at
        .saturating_sub(usize::from(body).saturating_sub(2))
        .min(tree.len().saturating_sub(usize::from(body)));

    // The right panel: a title row, then the agent's screen, bottom rows first when it is taller than the panel.
    let title = match agent {
        Some(a) => {
            let st = state_of(a);
            format!(
                " {}  {}  {}  {}",
                local(&a.agent_id),
                a.kind,
                a.node_id,
                state_word(st, t)
            )
        }
        None => format!(" {}", t(key!("tui-split-pick"), &[])),
    };
    // An agent without a terminal (a chat agent) is talked to in its conversation, which Enter opens.
    let empty_pane = if agent.is_some_and(|a| s.terminals.iter().any(|x| x.agent_id == a.agent_id))
    {
        key!("tui-split-no-screen")
    } else {
        key!("tui-split-chat")
    };
    let pane = body.saturating_sub(1);
    let screen_start = v.screen.map_or(0, |term| {
        let h = term.size().rows;
        if h <= pane {
            0
        } else {
            let below = term.cursor().row.saturating_add(1);
            below.saturating_sub(pane).min(h - pane)
        }
    });

    let mut out = String::from("\x1b[?25l");
    for r in 0..body {
        let _ = write!(out, "\x1b[{};1H", r + 1);
        match tree.get(first + usize::from(r)) {
            Some(row) => {
                let text = fit(&format!(" {}", row.text), usize::from(left));
                if row.selected && v.color {
                    let _ = write!(out, "{rev}{bold}{text}{reset}");
                } else if row.selected {
                    let _ = write!(out, ">{}", fit(&row.text, usize::from(left) - 1));
                } else if v.color && !row.paint.is_empty() {
                    let _ = write!(out, "{}{text}{reset}", row.paint);
                } else {
                    out.push_str(&text);
                }
            }
            None => out.push_str(&" ".repeat(usize::from(left))),
        }
        let _ = write!(out, "{dim}│{reset}");
        if r == 0 {
            let _ = write!(out, "{bold}{}{reset}", fit(&title, usize::from(right)));
        } else {
            match v.screen {
                Some(term) if r - 1 + screen_start < term.size().rows => {
                    out.push_str(&screen_row(term, r - 1 + screen_start, right, v.color));
                }
                Some(_) => out.push_str(&" ".repeat(usize::from(right))),
                None if r == 2 && agent.is_some() => {
                    let _ = write!(
                        out,
                        "{dim}{}{reset}",
                        fit(&format!(" {}", t(empty_pane, &[])), usize::from(right))
                    );
                }
                None => out.push_str(&" ".repeat(usize::from(right))),
            }
        }
        out.push_str("\x1b[K");
    }

    // The status line: machines, agents, what waits for the user, the keys; or the last thing done.
    let online = s.nodes.iter().filter(|n| n.online).count();
    let running = s.agents.iter().filter(|a| a.running).count();
    let waiting = s.approvals.len()
        + s.agents
            .iter()
            .filter(|a| a.running && state_of(a) == AgentState::Blocked)
            .count();
    let mut status = t(
        key!("tui-split-status"),
        &[
            ("online", Arg::Num(online as i64)),
            ("total", Arg::Num(s.nodes.len() as i64)),
            ("agents", Arg::Num(running as i64)),
            ("waiting", Arg::Num(waiting as i64)),
        ],
    );
    status.push_str("  ");
    status.push_str(&if v.notice.is_empty() {
        t(key!("tui-split-keys"), &[])
    } else {
        v.notice.to_string()
    });
    let _ = write!(
        out,
        "\x1b[{};1H{rev}{}{reset}\x1b[K",
        rows,
        fit(&format!(" {status}"), usize::from(cols))
    );
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{ApprovalSnapshot, NodeSnapshot};
    use agora_vt::{AlacrittyTerminal, Size};

    fn text(id: &str, args: &[(&str, Arg)]) -> String {
        let mut s = id.to_string();
        for (k, v) in args {
            let v = match v {
                Arg::Num(n) => n.to_string(),
                Arg::Str(s) => s.clone(),
                #[allow(unreachable_patterns)]
                _ => String::new(),
            };
            let _ = write!(s, " {k}={v}");
        }
        s
    }

    fn snapshot() -> Snapshot {
        let node = |id: &str, host: &str, online: bool| NodeSnapshot {
            node_id: id.into(),
            hostname: host.into(),
            online,
            memory_used_bytes: if online { 3 << 29 } else { 0 },
            ..Default::default()
        };
        let agent = |id: &str, node: &str, kind: &str, state: AgentState| AgentSnapshot {
            agent_id: id.into(),
            node_id: node.into(),
            kind: kind.into(),
            state: state as i32,
            running: true,
            ..Default::default()
        };
        Snapshot {
            nodes: vec![node("win", "WHITE", false), node("mac2", "mac2", true)],
            agents: vec![
                agent("mac2/agent-2", "mac2", "codex", AgentState::Working),
                agent("mac2/agent-1", "mac2", "claude-code", AgentState::Blocked),
                agent("win/agent-1", "win", "shell", AgentState::Idle),
            ],
            approvals: vec![ApprovalSnapshot {
                approval_id: "a1".into(),
                agent_id: "mac2/agent-1".into(),
                action: "files.write".into(),
                detail: "src/auth.rs".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// The frame with escapes removed, row by row.
    fn plain_rows(f: &str, rows: u16) -> Vec<String> {
        let mut grid = AlacrittyTerminal::new(Size { cols: 100, rows });
        grid.feed(f.as_bytes());
        (0..rows).map(|r| grid.line(r)).collect()
    }

    #[test]
    fn online_machines_come_first_and_every_agent_is_reachable() {
        assert_eq!(
            tree_order(&snapshot()),
            ["mac2/agent-1", "mac2/agent-2", "win/agent-1"]
        );
    }

    #[test]
    fn a_frame_has_the_tree_the_agents_screen_and_a_status_line() {
        let s = snapshot();
        let mut term = AlacrittyTerminal::new(Size { cols: 40, rows: 6 });
        term.feed(b"\x1b[1;32mok\x1b[0m line one\r\n\x1b[38;2;255;0;0mred\x1b[0m");
        let v = View {
            snapshot: &s,
            selected: "mac2/agent-1",
            screen: Some(&term),
            notice: "",
            color: true,
            cols: 100,
            rows: 12,
        };
        let f = frame(&v, &text);
        let rows = plain_rows(&f, 12);
        assert!(rows[0].contains("tui-attention"), "{rows:#?}");
        // Cut at the panel's edge, with an ellipsis; marked as the one y / n answer (the selected agent's).
        assert!(
            rows[1].contains("▶ agent-1: files.write src/aut…│"),
            "{rows:#?}"
        );
        let tree: Vec<&str> = rows
            .iter()
            .map(|r| r.split('│').next().unwrap_or_default())
            .filter(|r| r.contains("mac2") || r.contains("WHITE"))
            .collect();
        assert!(
            tree[0].contains("mac2") && tree[0].contains("1.5 GB"),
            "{rows:#?}"
        );
        assert!(
            rows[0].contains("│ agent-1  claude-code  mac2  tui-state-needs-you"),
            "{rows:#?}"
        );
        assert!(rows[1].contains("│ok line one"), "{rows:#?}");
        assert!(rows[2].contains("│red"), "{rows:#?}");
        assert!(
            rows[11].contains("tui-split-status online=1 total=2 agents=3 waiting=2"),
            "{rows:#?}"
        );
        // The agent's own colours reach the window: bold green, then 24-bit red.
        assert!(f.contains("\x1b[0;1;32mok"), "{f:?}");
        assert!(f.contains("\x1b[0;38;2;255;0;0mred"), "{f:?}");
        // The selected agent's row is drawn reversed.
        assert!(
            f.contains("\x1b[7m\x1b[1m   ◐ agent-1  claude-code"),
            "{f:?}"
        );
    }

    #[test]
    fn without_colour_there_are_no_escapes_but_positioning() {
        let s = snapshot();
        let v = View {
            snapshot: &s,
            selected: "mac2/agent-2",
            screen: None,
            notice: "done",
            color: false,
            cols: 80,
            rows: 10,
        };
        let f = frame(&v, &text);
        let sgr_free = f.replace("\x1b[?25l", "").replace("\x1b[K", "");
        assert!(
            !sgr_free.contains('m') || !sgr_free.contains("\x1b[0m"),
            "no SGR without colour"
        );
        assert!(!f.contains("\x1b[7m"));
        let rows = plain_rows(&f, 10);
        assert!(
            rows.iter().any(|r| r.starts_with(">  ● agent-2")),
            "{rows:#?}"
        );
        assert!(rows[9].ends_with("done"), "{rows:#?}");
    }

    #[test]
    fn a_tall_screen_shows_the_rows_around_its_cursor_and_a_wide_one_is_cut() {
        let s = snapshot();
        let mut term = AlacrittyTerminal::new(Size {
            cols: 200,
            rows: 30,
        });
        for i in 0..30 {
            term.feed(format!("line {i:02} {}\r\n", "x".repeat(150)).as_bytes());
        }
        term.feed(b"\x1b[30;1Hprompt$ ");
        let v = View {
            snapshot: &s,
            selected: "mac2/agent-1",
            screen: Some(&term),
            notice: "",
            color: true,
            cols: 90,
            rows: 12,
        };
        let rows = plain_rows(&frame(&v, &text), 12);
        // Ten panel rows (12 minus the status line and the title): the last ten of the agent's screen, the prompt at the bottom.
        assert!(rows[10].contains("│prompt$"), "{rows:#?}");
        assert!(rows[1].contains("│line 21"), "{rows:#?}");
        for r in &rows {
            assert!(r.chars().count() <= 90, "{r}");
        }
    }

    #[test]
    fn wide_characters_and_box_drawing_keep_their_columns() {
        let s = snapshot();
        let mut term = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
        term.feed("┌─┐ 漢字ok\r\n└─┘".as_bytes());
        let v = View {
            snapshot: &s,
            selected: "mac2/agent-1",
            screen: Some(&term),
            notice: "",
            color: false,
            cols: 60,
            rows: 8,
        };
        let rows = plain_rows(&frame(&v, &text), 8);
        assert!(rows[1].contains("│┌─┐ 漢字ok"), "{rows:#?}");
        assert!(rows[2].contains("│└─┘"), "{rows:#?}");
    }
}
