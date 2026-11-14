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
    /// Now, in milliseconds since the Unix epoch: what "last seen" of an offline machine counts from (ADR-0070).
    pub now_ms: u64,
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
        // Helpers right after the agent that started them (ADR-0057), the order the rows are drawn in.
        out.extend(
            crate::render::helper_order(&agents)
                .into_iter()
                .map(|(a, _)| a.agent_id.clone()),
        );
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

/// The lines of the `agent.start` request `a` as shown at `width` columns (ADR-0057): who asks to start what and where, its whole first
/// prompt indented, and how to answer. `None` for any other request.
pub fn start_card_lines(
    a: &agora_protocol::v1::ApprovalSnapshot,
    width: usize,
    t: Text<'_>,
) -> Option<Vec<String>> {
    let (line, prompt) = crate::render::start_card(&a.action, &a.detail, t)?;
    let width = width.max(12);
    let mut lines = crate::render::wrap_row(&line, width, width);
    lines.push(String::new());
    for p in prompt {
        let wrapped = crate::render::wrap_row(&p, width - 2, width - 2);
        if wrapped.is_empty() {
            lines.push(String::new());
        }
        lines.extend(wrapped.into_iter().map(|w| format!("  {w}")));
    }
    lines.push(String::new());
    lines.push(t(key!("tui-split-answer"), &[]));
    Some(lines)
}

/// The `agent.start` request y / n would answer in this view, and whether all of it fits the right panel. `None` when the request to
/// answer is not a start (or there is none). A start that does not fit must be read whole (the reader) before y may allow it.
pub fn start_card_fit<'a>(
    v: &View<'a>,
    t: Text<'_>,
) -> Option<(&'a agora_protocol::v1::ApprovalSnapshot, bool)> {
    let (cols, rows) = (v.cols.max(20), v.rows.max(5));
    let left = (cols / 3).clamp(18, 36);
    let right = cols.saturating_sub(left + 1);
    let room = usize::from((rows - 1).saturating_sub(1)).max(2);
    let a = crate::client::answered_approval(v.snapshot, Some(v.selected))?;
    let lines = start_card_lines(a, usize::from(right).saturating_sub(3), t)?;
    Some((a, lines.len() <= room))
}

/// The full-screen reader of a start request that did not fit (ADR-0057): `lines` from `top`, a key line at the bottom. Returns the frame
/// and whether the last line is in view, which is what counts as read.
pub fn reader_frame(
    lines: &[String],
    top: usize,
    cols: u16,
    rows: u16,
    t: Text<'_>,
) -> (String, bool) {
    let rows = usize::from(rows.max(3));
    let room = rows - 1;
    let top = top.min(lines.len().saturating_sub(room));
    let mut out = String::from("\x1b[?25l\x1b[H\x1b[2J");
    for (i, l) in lines.iter().skip(top).take(room).enumerate() {
        let _ = write!(
            out,
            "\x1b[{};1H{}",
            i + 1,
            fit(&format!(" {l}"), usize::from(cols))
        );
    }
    let _ = write!(
        out,
        "\x1b[{rows};1H\x1b[2m{}\x1b[0m",
        fit(&t(key!("tui-reader-keys"), &[]), usize::from(cols))
    );
    (out, top + room >= lines.len())
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
            // ADR-0057: a request to start an agent is named in words here; its whole request is on the right while it is the one
            // y / n answers (`start_card`), never raw JSON cut to the panel's width.
            let what = match crate::render::start_kind(&a.action, &a.detail) {
                Some(kind) => t(key!("tui-split-asks-start"), &[("kind", Arg::Str(kind))]),
                None => format!("{} {}", a.action, a.detail),
            };
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
        // ADR-0070: an offline machine says when it was last heard from.
        let seen = if n.online {
            String::new()
        } else {
            format!("  {}", crate::render::presence_text(n, v.now_ms, t))
        };
        rows.push(Row {
            text: format!("{dot} {}{mem}{seen}", name(n)),
            paint,
            selected: false,
        });
        let mut agents: Vec<&AgentSnapshot> =
            s.agents.iter().filter(|a| a.node_id == n.node_id).collect();
        agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        // A helper is drawn under the agent that started it, one step in per level (ADR-0057).
        for (a, depth) in crate::render::helper_order(&agents) {
            let st = state_of(a);
            let (m, paint) = mark(st, a.running);
            let lead = if depth == 0 {
                String::new()
            } else {
                format!("{}↳ ", "  ".repeat(depth - 1))
            };
            let note = if a.interrupted && !a.running {
                format!("  · {}", t(key!("tui-interrupted"), &[]))
            } else {
                String::new()
            };
            // ADR-0059: an agent that acts without asking says so in the tree too.
            let no_ask = if a.skips_permissions {
                format!("  [{}]", t(key!("tui-badge-no-ask"), &[]))
            } else {
                String::new()
            };
            rows.push(Row {
                text: format!(
                    "  {lead}{m} {}  {}{note}{no_ask}",
                    local(&a.agent_id),
                    a.kind
                ),
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

/// The colour names the web window gives agents, in its order (`apps/web/src/net/view.ts`, `ACCENT_NAMES`).
const ACCENT_NAMES: [&str; 7] = ["amber", "teal", "violet", "rose", "slate", "sky", "mint"];

/// An agent's name as the web window shows it, "kind · colour", the colour picked from its hub-wide id with the web's hash (31 times the
/// previous value plus each UTF-16 unit, modulo 2^32), so both clients call the same agent the same thing.
pub fn agent_name(agent_id: &str, kind: &str) -> String {
    let h = agent_id
        .encode_utf16()
        .fold(0u32, |h, u| h.wrapping_mul(31).wrapping_add(u32::from(u)));
    let accent = ACCENT_NAMES[usize::try_from(h).unwrap_or(0) % ACCENT_NAMES.len()];
    format!("{kind} · {accent}")
}

/// ADR-0063: the tree as the app shell's rows: a heading per machine (online first, then by name), then its agents, helpers indented
/// under the agent that started them (ADR-0057). Agents of a machine the hub does not list come last.
pub fn app_rows(s: &Snapshot, now_ms: u64, t: Text<'_>) -> Vec<crate::app::Row> {
    let mut rows = Vec::new();
    let agent_row = |a: &AgentSnapshot, depth: usize| {
        let st = state_of(a);
        let lead = if depth == 0 {
            String::new()
        } else {
            format!("{}↳ ", "  ".repeat(depth - 1))
        };
        let no_ask = if a.skips_permissions {
            format!(" [{}]", t(key!("tui-badge-no-ask"), &[]))
        } else {
            String::new()
        };
        let state = if a.interrupted && !a.running {
            t(key!("tui-interrupted"), &[])
        } else if !a.running {
            t(key!("tui-state-finished"), &[])
        } else {
            state_word(st, t)
        };
        let asks = s.approvals.iter().any(|x| x.agent_id == a.agent_id);
        crate::app::Row {
            id: a.agent_id.clone(),
            label: format!(" {lead}{}{no_ask}", agent_name(&a.agent_id, &a.kind)),
            state,
            needs_you: asks || (a.running && st == AgentState::Blocked),
            terminal: s.terminals.iter().any(|x| x.agent_id == a.agent_id),
            header: false,
        }
    };
    let mut nodes: Vec<_> = s.nodes.iter().collect();
    nodes.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| name(a).cmp(name(b))));
    for n in nodes {
        rows.push(crate::app::Row {
            id: n.node_id.clone(),
            label: format!("{} {}", if n.online { "●" } else { "○" }, name(n)),
            // ADR-0070: an offline machine says when it was last heard from; an online one, its memory in use.
            state: if !n.online {
                crate::render::presence_text(n, now_ms, t)
            } else if n.memory_used_bytes > 0 {
                bytes(n.memory_used_bytes)
            } else {
                String::new()
            },
            needs_you: false,
            terminal: false,
            header: true,
        });
        let mut agents: Vec<&AgentSnapshot> =
            s.agents.iter().filter(|a| a.node_id == n.node_id).collect();
        agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        for (a, depth) in crate::render::helper_order(&agents) {
            rows.push(agent_row(a, depth));
        }
    }
    let mut rest: Vec<&AgentSnapshot> = s
        .agents
        .iter()
        .filter(|a| !s.nodes.iter().any(|n| n.node_id == a.node_id))
        .collect();
    rest.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    for a in rest {
        rows.push(agent_row(a, 0));
    }
    rows
}

/// What the app shell's inspector shows for the selected agent ([`app_details`]).
pub struct Details {
    pub lines: Vec<String>,
    /// The `agent.start` request y / n would answer, by approval id, and whether all of it is on screen in the inspector (ADR-0057: y
    /// allows a start only once its whole request was in view, here or in the reader).
    pub start: Option<(String, bool)>,
}

/// ADR-0063: the inspector's lines for `selected` in a panel of `width` x `height` (zero when the inspector is hidden). When the
/// request y / n would answer asks to start an agent, the panel shows that request alone, like the old right panel; otherwise the
/// agent's facts, its session line, and the request it waits on.
pub fn app_details(s: &Snapshot, selected: &str, width: u16, height: u16, t: Text<'_>) -> Details {
    let room = usize::from(height);
    let answered = crate::client::answered_approval(s, Some(selected));
    if let Some(a) = answered {
        if let Some(card) = start_card_lines(a, usize::from(width).saturating_sub(1), t) {
            let mut lines = vec![t(key!("tui-attention"), &[])];
            lines.extend(card);
            let whole = width > 0 && lines.len() <= room;
            if !whole && room >= 2 {
                let hidden = lines.len() - (room - 1);
                lines.truncate(room - 1);
                lines.push(t(
                    key!("tui-split-card-more"),
                    &[("count", Arg::Num(i64::try_from(hidden).unwrap_or(i64::MAX)))],
                ));
            }
            return Details {
                lines,
                start: Some((a.approval_id.clone(), whole)),
            };
        }
    }
    let mut lines = Vec::new();
    match s.agents.iter().find(|a| a.agent_id == selected) {
        None => lines.push(t(key!("tui-split-pick"), &[])),
        Some(a) => {
            lines.push(agent_name(&a.agent_id, &a.kind));
            lines.push(a.agent_id.clone());
            lines.push(if a.running {
                state_word(state_of(a), t)
            } else {
                t(key!("tui-state-finished"), &[])
            });
            if !a.parent_agent_id.is_empty() {
                lines.push(t(
                    key!("tui-started-by"),
                    &[("agent", Arg::Str(local(&a.parent_agent_id).to_string()))],
                ));
            }
            if a.interrupted && !a.running {
                lines.push(crate::render::interrupted_text(a, t));
            }
            if a.skips_permissions {
                lines.push(format!("[{}]", t(key!("tui-badge-no-ask"), &[])));
            }
            if let Some(l) = crate::render::session_line(a, t) {
                lines.push(l);
            }
            if a.running && !s.terminals.iter().any(|x| x.agent_id == a.agent_id) {
                lines.push(String::new());
                lines.push(t(key!("tui-split-chat"), &[]));
            }
        }
    }
    if let Some(a) = answered {
        lines.push(String::new());
        lines.push(t(key!("tui-attention"), &[]));
        let what = format!("{}: {} {}", local(&a.agent_id), a.action, a.detail);
        lines.extend(crate::render::wrap_row(
            what.trim(),
            usize::from(width).max(12),
            usize::from(width).max(12),
        ));
        lines.push(t(key!("tui-split-answer"), &[]));
    }
    Details { lines, start: None }
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
    // ADR-0057: when the request y / n would answer asks to start an agent, the right panel shows it instead of a screen. One that
    // does not fit says how much is hidden; y refuses it until it was read whole (`start_card_fit`, the reader).
    let card: Option<Vec<String>> = crate::client::answered_approval(s, Some(v.selected))
        .and_then(|a| start_card_lines(a, usize::from(right).saturating_sub(3), t))
        .map(|mut lines| {
            let room = usize::from(body.saturating_sub(1)).max(2);
            if lines.len() > room {
                let hidden = lines.len() - (room - 1);
                lines.truncate(room - 1);
                lines.push(t(
                    key!("tui-split-card-more"),
                    &[("count", Arg::Num(i64::try_from(hidden).unwrap_or(i64::MAX)))],
                ));
            }
            lines
        });

    let tree = tree(v, t);
    // Keep the selected agent in view when the tree is taller than the panel.
    let at = tree.iter().position(|r| r.selected).unwrap_or(0);
    let first = at
        .saturating_sub(usize::from(body).saturating_sub(2))
        .min(tree.len().saturating_sub(usize::from(body)));

    // The right panel: a title row, then the agent's screen, bottom rows first when it is taller than the panel.
    let title = match (&card, agent) {
        (Some(_), _) => format!(" {}", t(key!("tui-attention"), &[])),
        (None, Some(a)) => {
            let st = state_of(a);
            let by = if a.parent_agent_id.is_empty() {
                String::new()
            } else {
                format!(
                    "  · {}",
                    t(
                        key!("tui-started-by"),
                        &[("agent", Arg::Str(local(&a.parent_agent_id).to_string()))]
                    )
                )
            };
            let by = if a.interrupted && !a.running {
                format!("{by}  · {}", crate::render::interrupted_text(a, t))
            } else {
                by
            };
            let no_ask = if a.skips_permissions {
                format!("  [{}]", t(key!("tui-badge-no-ask"), &[]))
            } else {
                String::new()
            };
            format!(
                " {}  {}  {}  {}{by}{no_ask}",
                local(&a.agent_id),
                a.kind,
                a.node_id,
                state_word(st, t)
            )
        }
        (None, None) => format!(" {}", t(key!("tui-split-pick"), &[])),
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
        } else if let Some(card) = &card {
            let line = card.get(usize::from(r - 1)).map_or("", String::as_str);
            out.push_str(&fit(&format!(" {line}"), usize::from(right)));
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
    fn a_request_to_start_an_agent_is_shown_whole_on_the_right_while_it_is_the_one_answered() {
        // Seen on macOS: the split view listed an `agent.start` request as raw JSON cut to the panel ("agent.start {\"agent\":\"…"),
        // so y signed a start whose prompt and folder were never in view.
        let mut s = snapshot();
        s.approvals = vec![ApprovalSnapshot {
            approval_id: "mac2/a9".into(),
            agent_id: "mac2/agent-1".into(),
            action: "agent.start".into(),
            detail: r#"{"node":"mac2","agent":"agent-1","agent_kind":"claude-code","kind":"codex","folder":"/w/proj","inside":true,"worktree":false,"depth":1,"prompt":"Fix the tests.\nThen open a PR, do not \u001b[2Jmerge."}"#.into(),
            ..Default::default()
        }];
        let v = View {
            snapshot: &s,
            selected: "mac2/agent-2",
            screen: None,
            notice: "",
            color: false,
            cols: 100,
            rows: 16,
            now_ms: 0,
        };
        let rows = plain_rows(&frame(&v, &text), 16);
        let left: Vec<&str> = rows
            .iter()
            .map(|r| r.split('│').next().unwrap_or_default())
            .collect();
        let right: String = rows
            .iter()
            .map(|r| r.split_once('│').map_or("", |(_, r)| r).trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            left[1].contains("▶ agent-1: tui-split-asks-sta"),
            "{rows:#?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains("agent.start {")),
            "{rows:#?}"
        );
        assert!(right.contains("tui-attention"), "{right}");
        assert!(
            right.contains("folder=/w/proj") && right.contains("kind=codex"),
            "{right}"
        );
        assert!(
            right.contains("  Fix the tests.") && right.contains("  Then open a PR"),
            "{right}"
        );
        // The escape in the prompt is shown, not obeyed.
        assert!(
            right.contains("\\u{1b}") || right.contains("\\x1b") || right.contains("␛"),
            "{right}"
        );
        assert!(right.contains("tui-split-answer"), "{right}");
        // Any other request keeps the screen side as it was.
        let mut other = s.clone();
        other.approvals[0].action = "files.write".into();
        let v = View {
            snapshot: &other,
            ..v
        };
        let rows = plain_rows(&frame(&v, &text), 16);
        assert!(
            !rows[0].split_once('│').unwrap().1.contains("tui-attention"),
            "{rows:#?}"
        );
    }

    #[test]
    fn a_helper_is_drawn_and_reached_right_under_the_agent_that_started_it() {
        let mut s = snapshot();
        s.approvals.clear();
        s.agents.push(AgentSnapshot {
            agent_id: "mac2/agent-3".into(),
            node_id: "mac2".into(),
            kind: "codex".into(),
            parent_agent_id: "mac2/agent-1".into(),
            running: true,
            ..Default::default()
        });
        // Arrow keys follow the rows: the helper comes right after its parent, before agent-2.
        assert_eq!(
            tree_order(&s),
            [
                "mac2/agent-1",
                "mac2/agent-3",
                "mac2/agent-2",
                "win/agent-1"
            ]
        );
        let v = View {
            snapshot: &s,
            selected: "mac2/agent-3",
            screen: None,
            notice: "",
            color: false,
            cols: 100,
            rows: 10,
            now_ms: 0,
        };
        let rows = plain_rows(&frame(&v, &text), 10);
        let at = |id: &str| {
            rows.iter()
                .position(|r| r.split('│').next().unwrap_or("").contains(id))
        };
        let (parent, helper) = (at("agent-1  claude").unwrap(), at("agent-3").unwrap());
        assert_eq!(helper, parent + 1, "{rows:#?}");
        assert!(rows[helper].contains("↳"), "{rows:#?}");
        assert!(rows[0].contains("· tui-started-by agent="), "{rows:#?}");
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
            now_ms: 0,
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
            now_ms: 0,
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
            now_ms: 0,
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
            now_ms: 0,
        };
        let rows = plain_rows(&frame(&v, &text), 8);
        assert!(rows[1].contains("│┌─┐ 漢字ok"), "{rows:#?}");
        assert!(rows[2].contains("│└─┘"), "{rows:#?}");
    }

    #[test]
    fn app_rows_put_each_machine_over_its_agents_and_mark_what_needs_you() {
        let mut s = snapshot();
        s.terminals = vec![agora_protocol::v1::TerminalSnapshot {
            agent_id: "mac2/agent-2".into(),
            ..Default::default()
        }];
        let rows = app_rows(&s, 0, &text);
        let ids: Vec<(&str, bool)> = rows.iter().map(|r| (r.id.as_str(), r.header)).collect();
        assert_eq!(
            ids,
            vec![
                ("mac2", true),
                ("mac2/agent-1", false),
                ("mac2/agent-2", false),
                ("win", true),
                ("win/agent-1", false)
            ]
        );
        assert!(
            rows[0].label.starts_with("● mac2") && rows[0].state == "1.5 GB",
            "{:?}",
            rows[0]
        );
        assert!(rows[3].label.starts_with("○ WHITE"));
        // ADR-0070: the offline machine's heading says when it was last heard from, not its memory.
        assert!(rows[3].state.starts_with("tui-"), "{:?}", rows[3]);
        // agent-1 waits on a request (and is blocked); agent-2 works and has a terminal; agent-1 has none (a chat agent here).
        assert!(rows[1].needs_you && !rows[1].terminal);
        assert!(!rows[2].needs_you && rows[2].terminal);
        assert_eq!(rows[2].state, "tui-state-working");
        assert_eq!(
            rows[1].label,
            format!(" {}", agent_name("mac2/agent-1", "claude-code"))
        );
    }

    #[test]
    fn app_details_show_the_agent_and_the_request_it_waits_on() {
        let s = snapshot();
        let d = app_details(&s, "mac2/agent-1", 40, 20, &text);
        assert_eq!(d.start, None);
        assert_eq!(
            &d.lines[..3],
            &[
                agent_name("mac2/agent-1", "claude-code"),
                "mac2/agent-1".into(),
                "tui-state-needs-you".into()
            ]
        );
        assert!(
            d.lines
                .iter()
                .any(|l| l == "agent-1: files.write src/auth.rs"),
            "{:#?}",
            d.lines
        );
        assert_eq!(d.lines.last().map(String::as_str), Some("tui-split-answer"));
        assert_eq!(
            app_details(&s, "", 40, 20, &text).lines[0],
            "tui-split-pick"
        );
    }

    #[test]
    fn a_start_request_in_the_inspector_counts_as_read_only_when_all_of_it_fits() {
        let mut s = snapshot();
        s.approvals = vec![ApprovalSnapshot {
            approval_id: "mac2/a9".into(),
            agent_id: "mac2/agent-1".into(),
            action: "agent.start".into(),
            detail: r#"{"node":"mac2","agent":"agent-1","agent_kind":"claude-code","kind":"codex","folder":"/w/proj","inside":true,"worktree":false,"depth":1,"prompt":"Fix the tests.\nThen open a PR."}"#.into(),
            ..Default::default()
        }];
        let tall = app_details(&s, "mac2/agent-2", 40, 30, &text);
        assert_eq!(tall.start, Some(("mac2/a9".into(), true)));
        assert!(
            tall.lines.iter().any(|l| l.contains("Fix the tests.")),
            "{:#?}",
            tall.lines
        );
        // Too short: the rest is counted, and y must wait for the reader.
        let short = app_details(&s, "mac2/agent-2", 40, 4, &text);
        assert_eq!(short.start, Some(("mac2/a9".into(), false)));
        assert_eq!(short.lines.len(), 4);
        assert!(
            short.lines[3].starts_with("tui-split-card-more"),
            "{:#?}",
            short.lines
        );
        // A hidden inspector shows nothing of it: never read here.
        assert_eq!(
            app_details(&s, "mac2/agent-2", 0, 0, &text).start,
            Some(("mac2/a9".into(), false))
        );
    }

    #[test]
    fn agents_are_named_as_the_web_window_names_them() {
        // Names seen in the web window for these ids.
        assert_eq!(agent_name("mac/agent-1", "shell"), "shell · amber");
        assert_eq!(agent_name("laptop/agent-1", "shell"), "shell · mint");
        assert_eq!(agent_name("mac.lan/agent-1", "shell"), "shell · violet");
    }
}
