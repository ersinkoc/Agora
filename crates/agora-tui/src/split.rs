//! The console as an editor-like screen: machines and their agents on the left, the selected agent's live terminal on the right,
//! a status line at the bottom. A pure function of the hub's snapshot and the selected agent's screen (render never mutates state,
//! AGENTS.md): it returns the bytes that paint one whole frame, and no terminal I/O happens here.

use std::fmt::Write as _;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{AgentSnapshot, AgentState, Snapshot};
use agora_vt::{Style, Terminal};

use crate::render::{plain, Text};

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
    /// ADR-0106: the hub said the person is a viewer; the keys that change something are not shown.
    pub viewer: bool,
}

/// The agents in the order the tree shows them: machines online first, then by name; agents by id within a machine.
pub fn tree_order(s: &Snapshot) -> Vec<String> {
    let mut nodes: Vec<_> = s.nodes.iter().collect();
    nodes.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| name(a).cmp(name(b))));
    let mut out = Vec::new();
    for n in nodes {
        let mut agents: Vec<&AgentSnapshot> =
            s.agents.iter().filter(|a| a.node_id == n.node_id).collect();
        agents.sort_by(|a, b| by_number(&a.agent_id).cmp(&by_number(&b.agent_id)));
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
    rest.sort_by(|a, b| by_number(&a.agent_id).cmp(&by_number(&b.agent_id)));
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
    let mut chars = text.chars().filter(|c| !c.is_control());
    let mut out: String = chars.by_ref().take(width).collect();
    let n = out.chars().count();
    if n < width {
        out.extend(std::iter::repeat_n(' ', width - n));
    } else if chars.next().is_some() && width > 0 {
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
    start_card_lines_for(a, width, t, false)
}

/// [`start_card_lines`] for a window that may only watch (`viewer`, ADR-0106): the same request without how to answer it (the hub
/// would refuse a viewer's answer and write the refusal down).
pub fn start_card_lines_for(
    a: &agora_protocol::v1::ApprovalSnapshot,
    width: usize,
    t: Text<'_>,
    viewer: bool,
) -> Option<Vec<String>> {
    let (line, settings, prompt) = crate::render::start_card(&a.action, &a.detail, t)?;
    let width = width.max(12);
    let mut lines = crate::render::wrap_row(&line, width, width);
    // ADR-0066: the model, effort and the like sit under the request line, before the prompt, in the lines y / n answers.
    for s in settings {
        lines.extend(
            crate::render::wrap_row(&s, width - 2, width - 2)
                .into_iter()
                .map(|w| format!("  {w}")),
        );
    }
    lines.push(String::new());
    for p in prompt {
        let wrapped = crate::render::wrap_row(&p, width - 2, width - 2);
        if wrapped.is_empty() {
            lines.push(String::new());
        }
        lines.extend(wrapped.into_iter().map(|w| format!("  {w}")));
    }
    if !viewer {
        lines.push(String::new());
        lines.push(t(key!("tui-split-answer"), &[]));
    }
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
    let lines = start_card_lines_for(a, usize::from(right).saturating_sub(3), t, v.viewer)?;
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
        // ADR-0106: a viewer's list watches; it does not say how to answer (the hub would refuse it).
        if !v.viewer {
            rows.push(Row {
                text: format!(" {}", t(key!("tui-split-answer"), &[])),
                paint: "\x1b[2m",
                selected: false,
            });
        }
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
        agents.sort_by(|a, b| by_number(&a.agent_id).cmp(&by_number(&b.agent_id)));
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
            // ADR-0059: an agent that acts without asking says so in the tree too (ADR-0101: else "asks less").
            let no_ask = crate::render::badge(a, t, "  ");
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
/// The name an agent is shown by everywhere (its kind and accent name, as in the tree); its local id when it is not in the snapshot.
pub fn agent_label(s: &Snapshot, agent_id: &str) -> String {
    s.agents
        .iter()
        .find(|a| a.agent_id == agent_id)
        .map_or_else(
            || local(agent_id).to_string(),
            |a| agent_name(&a.agent_id, &a.kind),
        )
}

/// An agent's tokens and, when the hub could price them, what they cost so far ("12.3k in · 4.1k out · 0.02 USD"). Nothing when it
/// reported neither.
fn usage_line(a: &AgentSnapshot, t: Text<'_>) -> Option<String> {
    let short = |n: u64| {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1e6)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1e3)
        } else {
            n.to_string()
        }
    };
    let mut parts = Vec::new();
    if a.input_tokens + a.output_tokens > 0 {
        parts.push(t(
            key!("tui-agent-tokens"),
            &[
                ("input", Arg::Str(short(a.input_tokens))),
                ("output", Arg::Str(short(a.output_tokens))),
            ],
        ));
    }
    if let Some(c) = a.cost_micros.filter(|c| *c > 0) {
        let amount = c as f64 / 1e6;
        let amount = if amount >= 0.01 {
            format!("{amount:.2}")
        } else {
            format!("{amount:.4}")
        };
        let currency = if a.currency.is_empty() {
            "USD".to_string()
        } else {
            plain(&a.currency)
        };
        parts.push(t(
            key!("tui-agent-cost"),
            &[
                ("amount", Arg::Str(amount)),
                ("currency", Arg::Str(currency)),
            ],
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The name of the project an agent works in, as the web shows it: the folder name of its project's checkout on its machine, else
/// of the folder it runs in, else the project's id; empty when it has none of them.
fn project_name(s: &Snapshot, a: &AgentSnapshot) -> String {
    let folder = |path: &str| {
        path.trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let checkout = s
        .checkouts
        .iter()
        .find(|c| {
            !a.project_id.is_empty() && c.project_id == a.project_id && c.node_id == a.node_id
        })
        .map(|c| folder(&c.path))
        .filter(|n| !n.is_empty());
    plain(
        &checkout
            .or_else(|| Some(folder(&a.folder)).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| project_from_id(&a.project_id)),
    )
}

/// A readable name from a project id when the machine has no checkout of it: the repository's name of
/// `github.com/owner/Agora#db6f…` ("Agora"), else `project` and the start of its hash, never the bare id.
pub(crate) fn project_from_id(id: &str) -> String {
    let (name, hash) = id.split_once('#').unwrap_or((id, ""));
    let name = name
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if !name.is_empty() {
        return name.to_string();
    }
    if hash.is_empty() {
        return String::new();
    }
    format!("project {}", hash.chars().take(7).collect::<String>())
}

/// The tree row that heads the old machines (ADR-0073); the palette offers to clean them up when it is there.
pub const OLD_MACHINES: &str = "#old-machines";

/// ADR-0073: a machine not seen for a day (offline, its last word a day ago or never), as the web decides it.
pub fn is_old(n: &agora_protocol::v1::NodeSnapshot, now_ms: u64) -> bool {
    !n.online && now_ms.saturating_sub(n.last_event_ms) >= 86_400_000
}

/// Whether the machine `node` is known to the hub and offline (ADR-0070: the hub owns presence).
fn machine_offline(s: &Snapshot, node: &str) -> bool {
    s.nodes.iter().any(|n| n.node_id == node && !n.online)
}

/// An agent's state in words, as its tree row says it: interrupted, finished (not running), offline (its machine is: what it last did
/// is not known now, so it never looks idle or waiting), else what it reported.
pub fn state_text(s: &Snapshot, a: &AgentSnapshot, t: Text<'_>) -> String {
    if a.interrupted && !a.running {
        t(key!("tui-interrupted"), &[])
    } else if !a.running {
        t(key!("tui-state-finished"), &[])
    } else if machine_offline(s, &a.node_id) {
        t(key!("tui-state-unreachable"), &[])
    } else {
        state_word(state_of(a), t)
    }
}

/// A machine's name as the tree shows it: its host name, else its id.
pub fn machine_name(n: &agora_protocol::v1::NodeSnapshot) -> &str {
    name(n)
}

pub fn app_rows(s: &Snapshot, now_ms: u64, t: Text<'_>) -> Vec<crate::app::Row> {
    let mut rows = Vec::new();
    let agent_row = |a: &AgentSnapshot, depth: usize| {
        let st = state_of(a);
        let lead = if depth == 0 {
            String::new()
        } else {
            format!("{}↳ ", "  ".repeat(depth - 1))
        };
        let no_ask = crate::render::badge(a, t, " ");
        let unreachable = a.running && machine_offline(s, &a.node_id);
        let state = state_text(s, a, t);
        let asks = s.approvals.iter().any(|x| x.agent_id == a.agent_id);
        // The request its tab's keys answer (its first), never another agent's; ADR-0101: whether it offers an answer the agent remembers
        // (Alt+A answers so in its tab).
        let own = s.approvals.iter().find(|x| x.agent_id == a.agent_id);
        let asks_always = own.is_some_and(|x| crate::client::always_offer(x).is_some());
        crate::app::Row {
            id: a.agent_id.clone(),
            label: format!(" {lead}{}{no_ask}", agent_name(&a.agent_id, &a.kind)),
            state,
            needs_you: asks || (a.running && !unreachable && st == AgentState::Blocked),
            terminal: s.terminals.iter().any(|x| x.agent_id == a.agent_id),
            header: false,
            group: false,
            request: asks,
            request_always: asks_always,
            approval: own.map(|x| x.approval_id.clone()).unwrap_or_default(),
        }
    };
    let mut nodes: Vec<_> = s.nodes.iter().collect();
    nodes.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| name(a).cmp(name(b))));
    // ADR-0073: machines not seen for a day are listed apart, last, under their own heading (as the web's Old machines).
    let (old, recent): (Vec<_>, Vec<_>) = nodes.into_iter().partition(|n| is_old(n, now_ms));
    let first_old = recent.len();
    for (i, n) in recent.into_iter().chain(old.iter().copied()).enumerate() {
        if i == first_old {
            rows.push(crate::app::Row {
                id: OLD_MACHINES.into(),
                label: t(
                    key!("tui-app-old-machines"),
                    &[(
                        "count",
                        Arg::Num(i64::try_from(old.len()).unwrap_or(i64::MAX)),
                    )],
                ),
                state: String::new(),
                needs_you: false,
                terminal: false,
                header: true,
                group: true,
                request: false,
                request_always: false,
                approval: String::new(),
            });
        }
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
            group: false,
            request: false,
            request_always: false,
            approval: String::new(),
        });
        let mut agents: Vec<&AgentSnapshot> =
            s.agents.iter().filter(|a| a.node_id == n.node_id).collect();
        agents.sort_by(|a, b| by_number(&a.agent_id).cmp(&by_number(&b.agent_id)));
        // As the web's tree: the machine's agents under their project's folder name, the folders in name order; agents of no
        // project and no folder stay right under the machine.
        let mut groups: Vec<(String, Vec<&AgentSnapshot>)> = Vec::new();
        for a in agents {
            let name = project_name(s, a);
            match groups.iter_mut().find(|(g, _)| *g == name) {
                Some((_, list)) => list.push(a),
                None => groups.push((name, vec![a])),
            }
        }
        groups.sort_by(|a, b| {
            (!a.0.is_empty())
                .cmp(&!b.0.is_empty())
                .then_with(|| a.0.cmp(&b.0))
        });
        for (name, list) in groups {
            if !name.is_empty() {
                rows.push(crate::app::Row {
                    id: format!("{}#{name}", n.node_id),
                    label: format!("  {name}/"),
                    state: String::new(),
                    needs_you: false,
                    terminal: false,
                    header: true,
                    group: true,
                    request: false,
                    request_always: false,
                    approval: String::new(),
                });
            }
            for (a, depth) in crate::render::helper_order(&list) {
                rows.push(agent_row(a, depth));
            }
        }
    }
    let mut rest: Vec<&AgentSnapshot> = s
        .agents
        .iter()
        .filter(|a| !s.nodes.iter().any(|n| n.node_id == a.node_id))
        .collect();
    rest.sort_by(|a, b| by_number(&a.agent_id).cmp(&by_number(&b.agent_id)));
    for a in rest {
        rows.push(agent_row(a, 0));
    }
    tell_apart(&mut rows);
    rows
}

/// An agent id's order in the tree: by machine, then by its number (`agent-2` before `agent-10`, not after it as text).
pub(crate) fn by_number(id: &str) -> (&str, u64, &str) {
    let (machine, local) = id.split_once('/').unwrap_or(("", id));
    let digits = local.trim_start_matches(|c: char| !c.is_ascii_digit());
    (machine, digits.parse().unwrap_or(u64::MAX), local)
}

/// Agents that would read the same (dozens of `shell · rose` once there are many: eight accent names) get their number after the
/// name, so every row says which agent it is.
fn tell_apart(rows: &mut [crate::app::Row]) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in rows.iter().filter(|r| !r.header) {
        *seen.entry(r.label.trim().to_string()).or_default() += 1;
    }
    for r in rows.iter_mut().filter(|r| !r.header) {
        if seen.get(r.label.trim()).is_some_and(|n| *n > 1) {
            let number = local(&r.id).trim_start_matches("agent-");
            r.label = format!("{} {number}", r.label);
        }
    }
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
    details(s, selected, width, height, t, false, false)
}

/// The inspector of a window that may only watch (ADR-0106), beside the list (`in_tab == false`) or the agent's own tab: the same
/// request, without the keys that answer it (the hub would refuse a viewer's answer and write the refusal down).
pub fn viewer_details(
    s: &Snapshot,
    selected: &str,
    width: u16,
    height: u16,
    t: Text<'_>,
    in_tab: bool,
) -> Details {
    details(s, selected, width, height, t, in_tab, true)
}

/// The inspector beside the agent's own tab, the keyboard in it: only that agent's request (never another agent's), answered
/// with Alt+Y / Alt+N (a plain y there is typing).
pub fn app_details_in_tab(
    s: &Snapshot,
    selected: &str,
    width: u16,
    height: u16,
    t: Text<'_>,
) -> Details {
    details(s, selected, width, height, t, true, false)
}

fn details(
    s: &Snapshot,
    selected: &str,
    width: u16,
    height: u16,
    t: Text<'_>,
    in_tab: bool,
    viewer: bool,
) -> Details {
    let room = usize::from(height);
    let answered = if in_tab {
        s.approvals.iter().find(|a| a.agent_id == selected)
    } else {
        crate::client::answered_approval(s, Some(selected))
    };
    if let Some(a) = answered {
        if let Some(card) = start_card_lines_for(a, usize::from(width).saturating_sub(1), t, viewer)
        {
            let mut lines = vec![t(key!("tui-attention"), &[])];
            lines.extend(card);
            let whole = width > 0 && lines.len() <= room;
            // Cut only what does not fit: a hidden inspector (no columns, the body's height) is never whole, yet nothing is cut.
            if lines.len() > room && room >= 2 {
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
            // Where it runs, in words (the machine's name and the folder), not its id.
            let machine = s
                .nodes
                .iter()
                .find(|n| n.node_id == a.node_id)
                .map_or(a.node_id.as_str(), name);
            lines.push(if a.folder.is_empty() {
                t(
                    key!("tui-agent-on"),
                    &[("machine", Arg::Str(plain(machine)))],
                )
            } else {
                t(
                    key!("tui-agent-on-in"),
                    &[
                        ("machine", Arg::Str(plain(machine))),
                        ("folder", Arg::Str(plain(&a.folder))),
                    ],
                )
            });
            lines.push(if !a.running {
                t(key!("tui-state-finished"), &[])
            } else if machine_offline(s, &a.node_id) {
                t(key!("tui-state-unreachable"), &[])
            } else {
                state_word(state_of(a), t)
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
            let badge = crate::render::badge(a, t, "");
            if !badge.is_empty() {
                lines.push(badge);
            }
            if let Some(l) = crate::render::session_line(a, t) {
                lines.push(l);
            }
            // What it changed in its worktree, and what it used: as the web's inspector shows them.
            if let Some(w) = s
                .worktrees
                .iter()
                .find(|w| !a.worktree_id.is_empty() && w.worktree_id == a.worktree_id)
                .filter(|w| w.files_changed > 0)
            {
                lines.push(t(
                    key!("tui-agent-changes"),
                    &[
                        ("files", Arg::Num(i64::from(w.files_changed))),
                        ("plus", Arg::Num(i64::from(w.insertions))),
                        ("minus", Arg::Num(i64::from(w.deletions))),
                    ],
                ));
            }
            if let Some(l) = usage_line(a, t) {
                lines.push(l);
            }
        }
    }
    if let Some(a) = answered {
        lines.push(String::new());
        lines.push(t(key!("tui-attention"), &[]));
        let what = format!("{}: {} {}", agent_label(s, &a.agent_id), a.action, a.detail);
        lines.extend(crate::render::wrap_row(
            what.trim(),
            usize::from(width).max(12),
            usize::from(width).max(12),
        ));
        // ADR-0101: the agent's own words for its always option and what "always" means, read before the keys that answer.
        for l in crate::render::always_lines(a, t) {
            lines.extend(crate::render::wrap_row(
                &l,
                usize::from(width).max(12),
                usize::from(width).max(12),
            ));
        }
        // ADR-0106: a viewer's inspector does not say how to answer (the hub would refuse it).
        if !viewer {
            lines.push(t(
                if in_tab {
                    // ADR-0101: Alt+A, when the request offers an answer the agent remembers.
                    crate::client::answer_hint(
                        a,
                        key!("tui-tab-answer"),
                        key!("tui-tab-answer-always"),
                        key!("tui-tab-answer-always-only"),
                    )
                } else {
                    key!("tui-split-answer")
                },
                &[],
            ));
        }
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
        .and_then(|a| start_card_lines_for(a, usize::from(right).saturating_sub(3), t, v.viewer))
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
            let no_ask = crate::render::badge(a, t, "  ");
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
        t(
            if v.viewer {
                key!("tui-split-keys-viewer")
            } else {
                key!("tui-split-keys")
            },
            &[],
        )
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
    fn a_viewers_status_line_shows_only_the_keys_that_read() {
        // ADR-0106: the hub said the person is a viewer.
        let s = snapshot();
        let v = View {
            snapshot: &s,
            selected: "",
            screen: None,
            notice: "",
            color: false,
            cols: 100,
            rows: 16,
            now_ms: 0,
            viewer: true,
        };
        let rows = plain_rows(&frame(&v, &text), 16);
        assert!(rows[15].contains("tui-split-keys-viewer"), "{rows:#?}");
        let v = View { viewer: false, ..v };
        let rows = plain_rows(&frame(&v, &text), 16);
        assert!(
            rows[15].contains("tui-split-keys") && !rows[15].contains("viewer"),
            "{rows:#?}"
        );
    }

    #[test]
    fn a_viewers_list_and_inspector_show_requests_without_the_keys_that_answer_them() {
        // ADR-0106 (#1140): the hub would refuse a viewer's answer and write the refusal down.
        let plain = snapshot();
        let mut start = snapshot();
        start.approvals = vec![ApprovalSnapshot {
            approval_id: "mac2/a9".into(),
            agent_id: "mac2/agent-1".into(),
            action: "agent.start".into(),
            detail: r#"{"node":"mac2","agent":"agent-1","agent_kind":"claude-code","kind":"codex","folder":"/w/proj","inside":true,"worktree":false,"depth":1,"prompt":"Fix the tests."}"#.into(),
            ..Default::default()
        }];
        let keys = |l: &str| l.contains("tui-split-answer") || l.contains("tui-tab-answer");
        for s in [&plain, &start] {
            // The split list: its tree and, for a start, the card on the right.
            let v = View {
                snapshot: s,
                selected: "mac2/agent-1",
                screen: None,
                notice: "",
                color: false,
                cols: 100,
                rows: 24,
                now_ms: 0,
                viewer: false,
            };
            let member = plain_rows(&frame(&v, &text), 24);
            assert!(member.iter().any(|r| keys(r)), "{member:#?}");
            let viewer = plain_rows(&frame(&View { viewer: true, ..v }, &text), 24);
            assert!(!viewer.iter().any(|r| keys(r)), "{viewer:#?}");
            assert!(
                viewer.iter().any(|r| r.contains("tui-attention")),
                "{viewer:#?}"
            );
            // The console's inspector, beside the list and in the agent's tab.
            for in_tab in [false, true] {
                let member = details(s, "mac2/agent-1", 60, 30, &text, in_tab, false);
                let viewer = viewer_details(s, "mac2/agent-1", 60, 30, &text, in_tab);
                assert!(member.lines.iter().any(|l| keys(l)), "{:#?}", member.lines);
                assert!(!viewer.lines.iter().any(|l| keys(l)), "{:#?}", viewer.lines);
                assert!(
                    viewer.lines.iter().any(|l| l == "tui-attention"),
                    "{:#?}",
                    viewer.lines
                );
            }
        }
        // The reader of a start request.
        let a = &start.approvals[0];
        assert!(start_card_lines(a, 80, &text)
            .unwrap()
            .iter()
            .any(|l| keys(l)));
        let read = start_card_lines_for(a, 80, &text, true).unwrap();
        assert!(!read.iter().any(|l| keys(l)), "{read:#?}");
        assert!(
            read.iter().any(|l| l.contains("Fix the tests.")),
            "{read:#?}"
        );
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
            viewer: false,
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
            viewer: false,
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
            viewer: false,
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
            viewer: false,
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
            viewer: false,
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
            viewer: false,
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
                // Where it runs in words, not its id.
                "tui-agent-on machine=mac2".into(),
                "tui-state-needs-you".into()
            ]
        );
        // The request names the agent as the tree does; at 40 columns it wraps after the action.
        let at = d.lines.iter().position(|l| l == "tui-attention").unwrap();
        assert_eq!(
            &d.lines[at + 1..at + 3],
            &[
                format!("{}: files.write", agent_name("mac2/agent-1", "claude-code")),
                "src/auth.rs".to_string()
            ],
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
    fn in_its_tab_the_inspector_shows_only_the_agents_own_request_and_the_alt_keys() {
        let s = snapshot();
        // mac2/agent-2 has no request; the list's inspector shows the first one (mac2/agent-1's), which y would answer.
        let list = app_details(&s, "mac2/agent-2", 60, 20, &text);
        assert!(
            list.lines.iter().any(|l| l.contains("files.write")),
            "{:#?}",
            list.lines
        );
        // Beside its own tab it shows none: another agent's request never sits in this agent's tab.
        let tab = app_details_in_tab(&s, "mac2/agent-2", 60, 20, &text);
        assert!(
            !tab.lines.iter().any(|l| l.contains("files.write")),
            "{:#?}",
            tab.lines
        );
        // Its own request is shown with the keys that answer it there.
        let own = app_details_in_tab(&s, "mac2/agent-1", 60, 20, &text);
        assert!(
            own.lines.iter().any(|l| l.contains("files.write")),
            "{:#?}",
            own.lines
        );
        assert_eq!(own.lines.last().map(String::as_str), Some("tui-tab-answer"));
    }

    /// ADR-0101: the tab's hint grows Alt+A only when the request offers it (and says Alt+Y is not there when it offers only that); the
    /// row knows it; the agent's line says "asks less" while it holds an always answer, never beside "does not ask".
    #[test]
    fn the_always_answer_shows_in_the_hint_the_row_and_the_agent_line() {
        use agora_protocol::v1::ApprovalScope;
        let mut s = snapshot();
        let row = |s: &Snapshot| {
            app_rows(s, 0, &text)
                .into_iter()
                .find(|r| r.id == "mac2/agent-1")
                .unwrap()
        };
        assert!(row(&s).request && !row(&s).request_always);
        s.approvals[0].approval_id = "mac2/a1".into();
        s.approvals[0].scopes = vec![ApprovalScope::Once as i32, ApprovalScope::Always as i32];
        s.approvals[0].always_label = "Always allow edits".into();
        s.approvals[0].always_option_id = "edits".into();
        assert!(row(&s).request_always);
        let own = app_details_in_tab(&s, "mac2/agent-1", 60, 20, &text);
        assert_eq!(
            own.lines.last().map(String::as_str),
            Some("tui-tab-answer-always-kept")
        );
        // The agent's own words and the warning are read before the keys, beside the tab and in the list's inspector.
        assert!(
            own.lines.iter().any(|l| l.starts_with("tui-always-label")),
            "{:#?}",
            own.lines
        );
        assert!(
            own.lines.iter().any(|l| l == "tui-always-warning-kept"),
            "{:#?}",
            own.lines
        );
        let list = app_details(&s, "mac2/agent-1", 60, 20, &text);
        assert!(
            list.lines.iter().any(|l| l == "tui-always-warning-kept"),
            "{:#?}",
            list.lines
        );
        s.approvals[0].scopes = vec![ApprovalScope::Always as i32];
        let own = app_details_in_tab(&s, "mac2/agent-1", 60, 20, &text);
        assert_eq!(
            own.lines.last().map(String::as_str),
            Some("tui-tab-answer-always-only-kept")
        );
        // The list's own hint is unchanged (no always from the list).
        let list = app_details(&s, "mac2/agent-1", 60, 20, &text);
        assert_eq!(
            list.lines.last().map(String::as_str),
            Some("tui-split-answer")
        );

        assert!(!row(&s).label.contains("tui-badge-asks-less"));
        s.agents[1].always_allowed = vec!["files.write".into()];
        assert!(
            row(&s).label.contains("[tui-badge-asks-less]"),
            "{}",
            row(&s).label
        );
        s.agents[1].skips_permissions = true;
        assert!(row(&s).label.contains("[tui-badge-no-ask]"));
        assert!(!row(&s).label.contains("tui-badge-asks-less"));
        s.agents[1].skips_permissions = false;
        s.agents[1].running = false;
        assert!(!row(&s).label.contains("tui-badge-asks-less"));
    }

    #[test]
    fn the_inspector_says_what_an_agent_changed_and_used() {
        let mut s = snapshot();
        s.worktrees = vec![agora_protocol::v1::WorktreeSnapshot {
            worktree_id: "w1".into(),
            files_changed: 3,
            insertions: 12,
            deletions: 4,
            ..Default::default()
        }];
        let a = &mut s.agents[0]; // mac2/agent-2
        a.worktree_id = "w1".into();
        a.input_tokens = 12_345;
        a.output_tokens = 678;
        a.cost_micros = Some(20_000);
        a.currency = "USD".into();
        let d = app_details(&s, "mac2/agent-2", 60, 20, &text);
        assert!(
            d.lines
                .contains(&"tui-agent-changes files=3 plus=12 minus=4".to_string()),
            "{:#?}",
            d.lines
        );
        assert!(
            d.lines.contains(
                &"tui-agent-tokens input=12.3k output=678 · tui-agent-cost amount=0.02 currency=USD"
                    .to_string()
            ),
            "{:#?}",
            d.lines
        );
        // Nothing reported, nothing said (never a guessed zero).
        let d = app_details(&s, "mac2/agent-1", 60, 20, &text);
        assert!(!d
            .lines
            .iter()
            .any(|l| l.contains("tui-agent-tokens") || l.contains("tui-agent-changes")));
    }

    #[test]
    fn agents_that_would_read_the_same_get_their_number() {
        let mut s = snapshot();
        let shell = |id: &str| AgentSnapshot {
            agent_id: id.into(),
            node_id: "mac2".into(),
            kind: "shell".into(),
            running: true,
            ..Default::default()
        };
        // Find two ids whose accent names collide, as dozens of shells do.
        let base = agent_name("mac2/agent-10", "shell");
        let twin = (11..400)
            .map(|i| format!("mac2/agent-{i}"))
            .find(|id| agent_name(id, "shell") == base)
            .unwrap();
        s.agents.push(shell("mac2/agent-10"));
        s.agents.push(shell(&twin));
        let rows = app_rows(&s, 0, &text);
        let labels: Vec<&str> = rows
            .iter()
            .filter(|r| r.id == "mac2/agent-10" || r.id == twin)
            .map(|r| r.label.trim())
            .collect();
        assert_eq!(labels.len(), 2);
        assert_ne!(labels[0], labels[1], "{labels:?}");
        assert!(
            labels.contains(&format!("{base} 10").as_str()),
            "{labels:?}"
        );
        // A name no other agent has stays as it is.
        let codex = rows.iter().find(|r| r.id == "mac2/agent-2").unwrap();
        assert_eq!(codex.label.trim(), agent_name("mac2/agent-2", "codex"));
    }

    #[test]
    fn agents_are_in_number_order() {
        let mut ids = vec!["m/agent-10", "m/agent-2", "m/agent-1", "a/agent-3"];
        ids.sort_by_key(|id| by_number(id));
        assert_eq!(ids, ["a/agent-3", "m/agent-1", "m/agent-2", "m/agent-10"]);
    }

    #[test]
    fn a_project_without_a_checkout_is_named_from_its_id_never_shown_bare() {
        assert_eq!(
            project_from_id("github.com/ersinkoc/Agora#db6f7d11b75c"),
            "Agora"
        );
        assert_eq!(project_from_id("#68539ee0a1b2"), "project 68539ee");
        assert_eq!(project_from_id("local/notes/"), "notes");
        assert_eq!(project_from_id(""), "");
    }

    #[test]
    fn a_machines_agents_sit_under_their_projects_folder_names() {
        let mut s = snapshot();
        s.checkouts = vec![
            agora_protocol::v1::CheckoutSnapshot {
                node_id: "mac2".into(),
                project_id: "p-web".into(),
                path: "/Users/me/code/web/".into(),
                ..Default::default()
            },
            agora_protocol::v1::CheckoutSnapshot {
                node_id: "mac2".into(),
                project_id: "p-api".into(),
                path: "/Users/me/code/api".into(),
                ..Default::default()
            },
        ];
        s.agents[0].project_id = "p-web".into(); // mac2/agent-2
        s.agents[1].project_id = "p-api".into(); // mac2/agent-1
                                                 // No project, a folder of its own (an agent started in a folder): grouped by that folder's name.
        s.agents.push(AgentSnapshot {
            agent_id: "mac2/agent-3".into(),
            node_id: "mac2".into(),
            kind: "shell".into(),
            folder: "/tmp/notes".into(),
            running: true,
            ..Default::default()
        });
        let rows = app_rows(&s, 0, &text);
        let mac2 = rows.iter().position(|r| r.id == "mac2").unwrap();
        let shown: Vec<(&str, bool)> = rows[mac2 + 1..]
            .iter()
            .take_while(|r| !r.header || r.group)
            .map(|r| (r.label.trim(), r.group))
            .collect();
        assert_eq!(
            shown,
            [
                ("api/", true),
                (agent_name("mac2/agent-1", "claude-code").as_str(), false),
                ("notes/", true),
                (agent_name("mac2/agent-3", "shell").as_str(), false),
                ("web/", true),
                (agent_name("mac2/agent-2", "codex").as_str(), false),
            ]
        );
        // A folder heading is a heading: never selected, never "needs you".
        assert!(rows
            .iter()
            .filter(|r| r.group)
            .all(|r| r.header && !r.needs_you));
    }

    #[test]
    fn machines_not_seen_for_a_day_are_listed_last_under_their_own_heading() {
        let mut s = snapshot();
        let now = 10 * 86_400_000;
        // "win" was offline 2 days, mac2 is online; a third machine went quiet an hour ago (not old yet).
        for n in &mut s.nodes {
            if n.node_id == "win" {
                n.last_event_ms = now - 2 * 86_400_000;
            }
        }
        s.nodes.push(NodeSnapshot {
            node_id: "pi".into(),
            hostname: "pi".into(),
            online: false,
            last_event_ms: now - 3_600_000,
            ..Default::default()
        });
        let rows = app_rows(&s, now, &text);
        let heads: Vec<&str> = rows
            .iter()
            .filter(|r| r.header && !r.group || r.id == OLD_MACHINES)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(heads, ["mac2", "pi", OLD_MACHINES, "win"]);
        let old = rows.iter().find(|r| r.id == OLD_MACHINES).unwrap();
        assert_eq!(old.label, "tui-app-old-machines count=1");
        // Its agents go with it, under it.
        let at = rows.iter().position(|r| r.id == OLD_MACHINES).unwrap();
        assert!(rows[at..].iter().any(|r| r.id == "win/agent-1"));
        // Nobody old: no heading.
        assert!(!app_rows(&snapshot(), 0, &text)
            .iter()
            .any(|r| r.id == OLD_MACHINES));
    }

    #[test]
    fn an_agent_of_an_offline_machine_never_looks_idle_or_waiting() {
        let mut s = snapshot();
        let node = s.agents[0].node_id.clone();
        let id = s.agents[0].agent_id.clone();
        s.agents[0].running = true;
        s.agents[0].state = AgentState::Blocked as i32;
        s.approvals.retain(|a| a.agent_id != id);
        for n in &mut s.nodes {
            n.online = n.node_id != node;
        }
        let rows = app_rows(&s, 0, &text);
        let row = rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row.state, "tui-state-unreachable");
        assert!(!row.needs_you);
        let d = app_details(&s, &id, 40, 20, &text);
        assert!(
            d.lines.contains(&"tui-state-unreachable".to_string()),
            "{:?}",
            d.lines
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
        // Hidden, the inspector is still as tall as the body (App::areas): never read, and nothing is cut or counted.
        let hidden = app_details(&s, "mac2/agent-2", 0, 29, &text);
        assert_eq!(hidden.start, Some(("mac2/a9".into(), false)));
        assert!(
            !hidden
                .lines
                .iter()
                .any(|l| l.starts_with("tui-split-card-more")),
            "{:#?}",
            hidden.lines
        );
    }

    #[test]
    fn agents_are_named_as_the_web_window_names_them() {
        // Names seen in the web window for these ids.
        assert_eq!(agent_name("mac/agent-1", "shell"), "shell · amber");
        assert_eq!(agent_name("laptop/agent-1", "shell"), "shell · mint");
        assert_eq!(agent_name("mac.lan/agent-1", "shell"), "shell · violet");
    }

    #[test]
    fn fit_does_not_truncate_when_only_control_characters_exceed_width() {
        assert_eq!(fit("abc\n", 3), "abc");
        assert_eq!(fit("\x07a\nb\x1bc", 3), "abc");
        assert_eq!(fit("abcd\n", 3), "ab…");
        assert_eq!(fit("a\n", 3), "a  ");
    }
}
