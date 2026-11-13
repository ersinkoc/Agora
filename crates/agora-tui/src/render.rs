//! The console as lines of text: a pure function of the hub's snapshot (render never mutates state, AGENTS.md).
//! Machines, agents grouped by project with what each waits for, and the attention list at the top. No terminal I/O here.

use std::collections::BTreeMap;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{AgentSnapshot, AgentState, ChatEntry, ChatKind, ChatRole, Snapshot};

/// Looks a message up by id and fills its arguments (the binary passes its `Say`).
pub type Text<'a> = &'a dyn Fn(&str, &[(&str, Arg)]) -> String;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

fn state_of(a: &AgentSnapshot) -> AgentState {
    AgentState::try_from(a.state).unwrap_or(AgentState::Unspecified)
}

/// The state colors follow the design tokens: working accent, needs-you warm, idle and finished quiet.
fn paint(state: AgentState, on: bool) -> (&'static str, &'static str) {
    if !on {
        return ("", "");
    }
    let c = match state {
        AgentState::Working => "\x1b[36m",
        AgentState::Blocked => "\x1b[33;1m",
        AgentState::Done => "\x1b[32m",
        _ => "\x1b[2m",
    };
    (c, RESET)
}

fn state_text(state: AgentState, t: Text<'_>) -> String {
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

/// The part of a hub-wide id after `<node>/`.
fn local(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, rest)| rest)
}

/// One screen of lines for a snapshot. `color` off spells states out with a marker and no escape sequences (NO_COLOR).
/// `selected` is the agent the keyboard points at (Enter opens its terminal); `keys` says the console takes answers from the keyboard (a hint line appears while something waits); `notice` is the last
/// thing the user did, shown at the bottom; `cost_micro_usd` is the hub's estimate of what the agents cost (0 when unknown).
pub fn render(
    s: &Snapshot,
    color: bool,
    keys: bool,
    selected: &str,
    notice: &str,
    cost_micro_usd: u64,
    t: Text<'_>,
) -> Vec<String> {
    let (bold, dim, reset) = if color {
        (BOLD, DIM, RESET)
    } else {
        ("", "", "")
    };
    let mut out = Vec::new();
    let online = s.nodes.iter().filter(|n| n.online).count();
    let running = s.agents.iter().filter(|a| a.running).count();
    out.push(format!(
        "{bold}{}{reset}",
        t(
            key!("tui-header"),
            &[
                ("online", Arg::Num(online as i64)),
                ("total", Arg::Num(s.nodes.len() as i64)),
                ("agents", Arg::Num(running as i64)),
            ],
        )
    ));
    let (tokens_in, tokens_out) = s.agents.iter().fold((0u64, 0u64), |(i, o), a| {
        (i + a.input_tokens, o + a.output_tokens)
    });
    if tokens_in + tokens_out > 0 {
        let mut line = t(
            key!("tui-usage"),
            &[
                (
                    "input",
                    Arg::Num(i64::try_from(tokens_in).unwrap_or(i64::MAX)),
                ),
                (
                    "output",
                    Arg::Num(i64::try_from(tokens_out).unwrap_or(i64::MAX)),
                ),
            ],
        );
        // Only an estimate the hub could price; an agent without a price adds nothing (never a guess).
        if cost_micro_usd > 0 {
            let usd = format!(
                "{}.{:02}",
                cost_micro_usd / 1_000_000,
                cost_micro_usd % 1_000_000 / 10_000
            );
            line.push_str(" · ");
            line.push_str(&t(key!("tui-cost"), &[("usd", Arg::Str(usd))]));
        }
        out.push(format!("{dim}{line}{reset}"));
    }
    out.push(String::new());

    // What needs the user: approvals first, then agents that are waiting.
    out.push(format!("{bold}{}{reset}", t(key!("tui-attention"), &[])));
    let mut waiting = 0;
    for a in &s.approvals {
        waiting += 1;
        let (on, off) = paint(AgentState::Blocked, color);
        // ADR-0057: a request to start an agent shows what would start, where, and its whole first prompt.
        if let Some((line, prompt)) = start_card(&a.action, &a.detail, t) {
            out.push(format!("  {on}!{off} {line}"));
            for l in prompt {
                out.push(format!("      {dim}{l}{reset}"));
            }
            continue;
        }
        let line = t(
            key!("tui-asks"),
            &[
                ("agent", Arg::Str(local(&a.agent_id).to_string())),
                (
                    "action",
                    Arg::Str(format!("{} {}", a.action, a.detail).trim().to_string()),
                ),
            ],
        );
        out.push(format!("  {on}!{off} {line}"));
    }
    for a in s
        .agents
        .iter()
        .filter(|a| a.running && state_of(a) == AgentState::Blocked)
    {
        waiting += 1;
        let line = t(
            key!("tui-blocked"),
            &[("agent", Arg::Str(local(&a.agent_id).to_string()))],
        );
        let (on, off) = paint(AgentState::Blocked, color);
        out.push(format!("  {on}!{off} {line}"));
    }
    if waiting == 0 {
        out.push(format!(
            "  {dim}{}{reset}",
            t(key!("tui-nothing-waiting"), &[])
        ));
    } else if keys && !s.approvals.is_empty() {
        out.push(format!("  {dim}{}{reset}", t(key!("tui-hint-answer"), &[])));
    }
    out.push(String::new());

    // Agents grouped by project.
    if s.agents.is_empty() {
        out.push(format!("{dim}{}{reset}", t(key!("tui-no-agents"), &[])));
    }
    let mut by_project: BTreeMap<&str, Vec<&AgentSnapshot>> = BTreeMap::new();
    for a in &s.agents {
        by_project.entry(a.project_id.as_str()).or_default().push(a);
    }
    // A project is known by its id; its folder name on a machine is what people recognise.
    let folder = |project: &str| {
        s.checkouts
            .iter()
            .find(|c| c.project_id == project)
            .and_then(|c| {
                c.path
                    .trim_end_matches(['/', '\\'])
                    .rsplit(['/', '\\'])
                    .next()
            })
            .filter(|n| !n.is_empty())
            .map(str::to_string)
    };
    for (project, agents) in by_project {
        let name = if project.is_empty() {
            t(key!("tui-no-project"), &[])
        } else {
            folder(project).unwrap_or_else(|| project.to_string())
        };
        out.push(format!("{bold}{name}{reset}"));
        for (a, depth) in helper_order(&agents) {
            let st = state_of(a);
            let (on, off) = paint(st, color);
            let marker = if color { "●" } else { "*" };
            let pointer = if keys && a.agent_id == selected {
                ">"
            } else {
                " "
            };
            let word = state_text(st, t);
            // A helper sits under the agent that started it (ADR-0057) and says so.
            let indent = "  ".repeat(depth);
            let by = if a.parent_agent_id.is_empty() {
                String::new()
            } else {
                format!(
                    "  {}",
                    t(
                        key!("tui-started-by"),
                        &[("agent", Arg::Str(local(&a.parent_agent_id).to_string()))],
                    )
                )
            };
            out.push(format!(
                "{pointer} {indent}{on}{marker} {word:<10}{off} {}  {dim}{}  {}{by}{reset}",
                local(&a.agent_id),
                a.kind,
                a.node_id,
            ));
        }
    }
    out.push(String::new());

    out.push(format!("{bold}{}{reset}", t(key!("tui-machines"), &[])));
    for n in &s.nodes {
        let id = if n.hostname.is_empty() {
            &n.node_id
        } else {
            &n.hostname
        };
        let status = t(
            if n.online {
                key!("tui-online")
            } else {
                key!("tui-offline")
            },
            &[],
        );
        out.push(format!("  {id}  {dim}{status}{reset}"));
    }
    out.push(String::new());
    if keys && s.agents.iter().any(|x| x.agent_id == selected) {
        out.push(format!("{dim}{}{reset}", t(key!("tui-hint-open"), &[])));
    }
    if !notice.is_empty() {
        out.push(notice.to_string());
    }
    out.push(format!("{dim}{}{reset}", t(key!("tui-quit-hint"), &[])));
    out
}

/// What an `agent.start` request asks for (ADR-0057), from its detail; `None` for any other request or a detail that does not parse.
struct StartRequest {
    agent: String,
    agent_kind: String,
    node: String,
    kind: String,
    folder: String,
    prompt: String,
    inside: bool,
    worktree: bool,
    depth: i64,
}

/// ADR-0057: what an `agent.start` request shows before it is answered: one line saying who asks to start what and where, then its whole
/// first prompt, line by line. Everything comes from the signed request and every hidden or control character is made visible, since
/// the answer is signed over exactly that request. `None` for any other request.
pub(crate) fn start_card(action: &str, detail: &str, t: Text<'_>) -> Option<(String, Vec<String>)> {
    let start = start_request(action, detail)?;
    let line = t(
        if start.inside {
            key!("tui-asks-start")
        } else {
            key!("tui-asks-start-outside")
        },
        &[
            ("agent", Arg::Str(visible(&start.agent))),
            ("agent_kind", Arg::Str(visible(&start.agent_kind))),
            ("node", Arg::Str(visible(&start.node))),
            ("kind", Arg::Str(visible(&start.kind))),
            ("folder", Arg::Str(visible(&start.folder))),
            ("worktree", Arg::Str(start.worktree.to_string())),
            ("depth", Arg::Num(start.depth)),
        ],
    );
    let prompt = visible(&start.prompt).lines().map(str::to_string).collect();
    Some((line, prompt))
}

/// The kind an `agent.start` request asks for, made visible; `None` for any other request.
pub(crate) fn start_kind(action: &str, detail: &str) -> Option<String> {
    start_request(action, detail).map(|s| visible(&s.kind))
}

fn start_request(action: &str, detail: &str) -> Option<StartRequest> {
    if action != "agent.start" {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(detail).ok()?;
    let s = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let flag = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    Some(StartRequest {
        agent: s("agent"),
        agent_kind: s("agent_kind"),
        node: s("node"),
        kind: s("kind"),
        folder: s("folder"),
        prompt: s("prompt"),
        inside: flag("inside"),
        worktree: flag("worktree"),
        depth: v
            .get("depth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1),
    })
}

/// `agents` with each helper right after the agent that started it, and how deep it is. A helper whose parent is not in the list (another
/// project, or gone) stays where it is, at depth 0.
pub(crate) fn helper_order<'a>(agents: &[&'a AgentSnapshot]) -> Vec<(&'a AgentSnapshot, usize)> {
    let present = |id: &str| agents.iter().any(|a| a.agent_id == id);
    let mut out = Vec::new();
    fn add<'a>(
        a: &'a AgentSnapshot,
        depth: usize,
        agents: &[&'a AgentSnapshot],
        out: &mut Vec<(&'a AgentSnapshot, usize)>,
    ) {
        if depth > 8 || out.iter().any(|(x, _)| x.agent_id == a.agent_id) {
            return;
        }
        out.push((a, depth));
        for c in agents.iter().filter(|c| c.parent_agent_id == a.agent_id) {
            add(c, depth + 1, agents, out);
        }
    }
    for a in agents {
        if a.parent_agent_id.is_empty() || !present(&a.parent_agent_id) {
            add(a, 0, agents, &mut out);
        }
    }
    // A loop of parents (never made by a node) would leave agents out: they are listed anyway.
    for a in agents {
        if !out.iter().any(|(x, _)| x.agent_id == a.agent_id) {
            out.push((a, 0));
        }
    }
    out
}

/// `agora_core::text::visible`, with each line break kept (a prompt's lines are shown as lines).
fn visible(text: &str) -> String {
    agora_core::text::visible(text)
}

/// Text an agent or a person wrote is shown as plain text: control characters (escape sequences that could move the cursor, change the
/// title or paste into the prompt) and bidirectional overrides are dropped, a tab becomes spaces.
pub fn plain(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            '\n' => Some('\n'),
            c if c.is_control() => None,
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => None,
            c => Some(c),
        })
        // Other invisible characters (zero-width, tags, fillers) are shown as escapes rather than passed through unseen.
        .map(|c| {
            if agora_core::text::is_hidden(c) {
                format!("\\u{{{:x}}}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// ADR-0058: a notice in the conversation, in words. The code picks the message; parameters are the agent's text, shown plain.
pub fn notice_text(e: &ChatEntry, t: Text<'_>) -> String {
    let p = |k: &str| Arg::Str(plain(e.params.get(k).map_or("", String::as_str)));
    match e.code.as_str() {
        "adapter.turn_failed" => t(
            key!("tui-notice-turn_failed"),
            &[
                ("rpc_code", p("rpc_code")),
                ("stderr_tail", p("stderr_tail")),
            ],
        ),
        "adapter.turn_stopped" => t(
            key!("tui-notice-turn_stopped"),
            &[("stop_reason", p("stop_reason"))],
        ),
        "adapter.turn_empty" => t(
            key!("tui-notice-turn_empty"),
            &[("stderr_tail", p("stderr_tail"))],
        ),
        "adapter.agent_exited" => t(
            key!("tui-notice-agent_exited"),
            &[
                ("stage", p("stage")),
                (
                    "exit_code",
                    Arg::Str(plain(e.params.get("exit_code").map_or("?", String::as_str))),
                ),
                ("stderr_tail", p("stderr_tail")),
            ],
        ),
        "adapter.agent_refused" => t(
            key!("tui-notice-agent_refused"),
            &[
                ("stage", p("stage")),
                ("rpc_code", p("rpc_code")),
                ("detail", p("detail")),
            ],
        ),
        other => t(
            key!("tui-notice-other"),
            &[("code", Arg::Str(plain(other)))],
        ),
    }
}

/// ADR-0058: the line above a chat agent's conversation: its mode, model and how full its context is, when it reported them.
/// Records in `a` a switch the agent accepted (ADR-0058), so the line [`session_line`] makes shows it at once instead of what it ran
/// with when the conversation was opened.
pub fn switched(a: &mut AgentSnapshot, model: bool, id: &str) {
    if model {
        a.current_model = id.to_string();
    } else {
        a.current_mode = id.to_string();
    }
}

pub fn session_line(a: &AgentSnapshot, t: Text<'_>) -> Option<String> {
    let name = |list: &[agora_protocol::v1::SessionOption], id: &str| {
        list.iter()
            .find(|o| o.id == id)
            .map_or(id, |o| {
                if o.name.is_empty() {
                    id
                } else {
                    o.name.as_str()
                }
            })
            .to_string()
    };
    let mut parts = Vec::new();
    if !a.current_mode.is_empty() {
        parts.push(t(
            key!("tui-chat-mode"),
            &[("mode", Arg::Str(plain(&name(&a.modes, &a.current_mode))))],
        ));
    }
    if !a.current_model.is_empty() {
        parts.push(t(
            key!("tui-chat-model"),
            &[("model", Arg::Str(plain(&name(&a.models, &a.current_model))))],
        ));
    }
    if let Some(pct) = a
        .context_tokens
        .saturating_mul(100)
        .checked_div(a.context_window)
    {
        parts.push(t(
            key!("tui-chat-context"),
            &[(
                "percent",
                Arg::Num(i64::try_from(pct.min(100)).unwrap_or(100)),
            )],
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// ADR-0058: what a typed chat line asks for when it starts with `/mode ` or `/model `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand {
    /// Not a switch: send the line as a request.
    Prompt,
    /// Switch to this id (`model` false for a mode).
    Switch { model: bool, id: String },
    /// The agent did not offer this id; the ids it offers.
    Unknown { id: String, offered: Vec<String> },
    /// A mode in which the agent acts without asking: the same line again confirms it.
    Confirm { id: String },
}

/// Reads a typed line. `confirming` is the line that asked for confirmation last time, if any: typing it again switches.
pub fn chat_command(
    line: &str,
    agent: Option<&AgentSnapshot>,
    confirming: Option<&str>,
) -> ChatCommand {
    let (model, id) = match (line.strip_prefix("/mode "), line.strip_prefix("/model ")) {
        (Some(id), _) => (false, id.trim()),
        (_, Some(id)) => (true, id.trim()),
        _ => return ChatCommand::Prompt,
    };
    let list = agent.map_or(
        &[][..],
        |a| if model { &a.models[..] } else { &a.modes[..] },
    );
    let Some(option) = list
        .iter()
        .find(|o| o.id == id || (!o.name.is_empty() && o.name == id))
    else {
        return ChatCommand::Unknown {
            id: id.to_string(),
            offered: list.iter().map(|o| o.id.clone()).collect(),
        };
    };
    if !model && option.skips_permissions && confirming != Some(line) {
        return ChatCommand::Confirm {
            id: option.id.clone(),
        };
    }
    ChatCommand::Switch {
        model,
        id: option.id.clone(),
    }
}

/// `row` cut into pieces of at most `first` characters for the first and `rest` for the others, at the last space that fits when
/// there is one (the space itself is dropped), else in the middle of the word. Counted in characters: a wide character may overshoot
/// a little, which is harmless.
pub(crate) fn wrap_row(row: &str, first: usize, rest: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut left: Vec<char> = row.chars().collect();
    while !left.is_empty() {
        let width = if out.is_empty() { first } else { rest };
        if left.len() <= width {
            out.push(left.iter().collect());
            break;
        }
        let cut = left[..=width]
            .iter()
            .rposition(|c| *c == ' ')
            .filter(|&at| at > 0);
        let (take, skip) = cut.map_or((width, width), |at| (at, at + 1));
        out.push(left[..take].iter().collect());
        left.drain(..skip);
    }
    out
}

/// The conversation of an agent that has no terminal, as the last lines that fit `rows` x `cols`, then the line being typed.
pub fn render_chat(
    entries: &[ChatEntry],
    agent: &str,
    typed: &str,
    (cols, rows): (usize, usize),
    color: bool,
    t: Text<'_>,
) -> Vec<String> {
    let (bold, dim, reset) = if color {
        (BOLD, DIM, RESET)
    } else {
        ("", "", "")
    };
    let mut lines: Vec<String> = Vec::new();
    for e in entries {
        let who = if e.role == ChatRole::User as i32 {
            t(key!("tui-chat-you"), &[])
        } else {
            t(
                key!("tui-chat-agent"),
                &[("agent", Arg::Str(agent.to_string()))],
            )
        };
        let body = match ChatKind::try_from(e.kind).unwrap_or(ChatKind::Unspecified) {
            ChatKind::ToolCall => t(key!("tui-chat-tool"), &[("text", Arg::Str(plain(&e.text)))]),
            ChatKind::Plan => format!("{}\n{}", t(key!("tui-chat-plan"), &[]), plain(&e.text)),
            ChatKind::Diff => t(key!("tui-chat-diff"), &[("path", Arg::Str(plain(&e.path)))]),
            ChatKind::Notice => notice_text(e, t),
            _ => plain(&e.text),
        };
        for (i, row) in body.split('\n').enumerate() {
            // The first line starts with the speaker, the others are indented; each line is cut to what is left of the width
            // after its own start (one column spare, so a full line never makes the terminal wrap it on its own).
            let (prefix, prefix_width) = if i == 0 {
                (format!("{bold}{who}:{reset} "), who.chars().count() + 2)
            } else {
                ("  ".to_string(), 2)
            };
            let first = cols.saturating_sub(prefix_width + 1).max(10);
            let rest = cols.saturating_sub(3).max(10);
            let pieces = wrap_row(row, first, rest);
            if pieces.is_empty() {
                lines.push(prefix.trim_end().to_string());
            }
            for (j, piece) in pieces.into_iter().enumerate() {
                let head = if j == 0 { prefix.as_str() } else { "  " };
                lines.push(format!("{head}{piece}"));
            }
        }
    }
    let room = rows.saturating_sub(3).max(1);
    let skip = lines.len().saturating_sub(room);
    let mut out: Vec<String> = lines.into_iter().skip(skip).collect();
    out.push(format!("{dim}{}{reset}", t(key!("tui-chat-hint"), &[])));
    out.push(format!(
        "{} {}",
        t(key!("tui-chat-input"), &[]),
        plain(typed)
    ));
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use agora_protocol::v1::{ApprovalSnapshot, NodeSnapshot};

    fn t(id: &str, args: &[(&str, Arg)]) -> String {
        let a: Vec<String> = args
            .iter()
            .map(|(k, v)| match v {
                Arg::Str(s) => format!("{k}={s}"),
                Arg::Num(n) => format!("{k}={n}"),
            })
            .collect();
        format!("{id}[{}]", a.join(","))
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            nodes: vec![
                NodeSnapshot {
                    node_id: "laptop".into(),
                    hostname: "laptop".into(),
                    online: true,
                    ..Default::default()
                },
                NodeSnapshot {
                    node_id: "vps".into(),
                    online: false,
                    ..Default::default()
                },
            ],
            agents: vec![
                AgentSnapshot {
                    agent_id: "laptop/agent-1".into(),
                    node_id: "laptop".into(),
                    project_id: "web".into(),
                    kind: "codex".into(),
                    state: AgentState::Blocked as i32,
                    running: true,
                    ..Default::default()
                },
                AgentSnapshot {
                    agent_id: "vps/agent-2".into(),
                    node_id: "vps".into(),
                    project_id: "web".into(),
                    kind: "claude-code".into(),
                    state: AgentState::Working as i32,
                    running: true,
                    ..Default::default()
                },
            ],
            checkouts: vec![agora_protocol::v1::CheckoutSnapshot {
                checkout_id: "laptop/c".into(),
                node_id: "laptop".into(),
                project_id: "web".into(),
                path: "/home/me/code/website/".into(),
            }],
            approvals: vec![ApprovalSnapshot {
                approval_id: "a1".into(),
                agent_id: "laptop/agent-1".into(),
                action: "shell".into(),
                detail: "rm -r build".into(),
                expires_at_ms: 0,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_screen_says_what_waits_for_the_user_and_groups_agents_by_project() {
        let lines = render(&snapshot(), false, true, "", "", 0, &t);
        let text = lines.join("\n");
        assert!(
            text.contains("tui-header[online=1,total=2,agents=2]"),
            "{text}"
        );
        assert!(
            text.contains("tui-asks[agent=agent-1,action=shell rm -r build]"),
            "{text}"
        );
        assert!(text.contains("tui-blocked[agent=agent-1]"), "{text}");
        let project = lines
            .iter()
            .position(|l| l == "website")
            .expect("project row");
        assert!(
            lines[project + 1].contains("agent-1")
                && lines[project + 1].contains("tui-state-needs-you")
        );
        assert!(
            lines[project + 2].contains("agent-2")
                && lines[project + 2].contains("tui-state-working")
        );
        assert!(text.contains("vps") && text.contains("tui-offline[]"));
        assert!(text.contains("tui-hint-answer[]"), "{text}");
        let quiet = render(&snapshot(), false, false, "", "done!", 0, &t).join("\n");
        assert!(
            !quiet.contains("tui-hint-answer") && quiet.contains("done!"),
            "{quiet}"
        );
    }

    #[test]
    fn the_pointer_marks_the_selected_agent_and_offers_its_terminal() {
        let mut snap = snapshot();
        snap.terminals.push(agora_protocol::v1::TerminalSnapshot {
            terminal_id: "laptop/t-agent-1".into(),
            node_id: "laptop".into(),
            agent_id: "laptop/agent-1".into(),
            title: String::new(),
        });
        let lines = render(&snap, false, true, "laptop/agent-1", "", 0, &t);
        let row = lines
            .iter()
            .find(|l| l.contains("agent-1") && l.contains("codex"))
            .expect("row");
        assert!(row.starts_with('>'), "{row}");
        assert!(lines.join("\n").contains("tui-hint-open[]"));
        let other = render(&snap, false, true, "nobody/agent-9", "", 0, &t).join("\n");
        assert!(!other.contains("tui-hint-open"), "{other}");
    }

    #[test]
    fn a_switch_shows_on_the_session_line_at_once() {
        let opt = |id: &str, name: &str| agora_protocol::v1::SessionOption {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let mut a = AgentSnapshot {
            modes: vec![opt("build", "Build"), opt("plan", "Plan")],
            models: vec![opt("m1", "One"), opt("m2", "Two")],
            current_mode: "build".into(),
            current_model: "m1".into(),
            ..Default::default()
        };
        switched(&mut a, false, "plan");
        switched(&mut a, true, "m2");
        let line = session_line(&a, &t).unwrap();
        assert!(
            line.contains("mode=Plan") && line.contains("model=Two"),
            "{line}"
        );
    }

    #[test]
    fn long_chat_lines_fit_the_width_and_break_between_words() {
        // Seen on macOS: a long notice overflowed its first line (the speaker was not counted), the terminal wrapped it on its own
        // and "invalid" came out as "inva" / "lid".
        let body = "ERROR service=llm provider rejected: invalid x-api-key (auth) at ~/.local/share/fake/auth.json";
        let entries = vec![ChatEntry {
            role: ChatRole::Agent as i32,
            kind: ChatKind::Text as i32,
            text: body.into(),
            ..Default::default()
        }];
        let cols = 40;
        let lines = render_chat(&entries, "agent-1", "", (cols, 24), false, &t);
        let said: Vec<&String> = lines
            .iter()
            .take_while(|l| !l.starts_with("tui-chat-hint"))
            .collect();
        assert!(said.iter().all(|l| l.chars().count() < cols), "{said:#?}");
        assert!(
            said[0].starts_with("tui-chat-agent[agent=agent-1]: "),
            "{said:#?}"
        );
        assert!(said[1..].iter().all(|l| l.starts_with("  ")), "{said:#?}");
        let words: Vec<&str> = said
            .iter()
            .enumerate()
            .flat_map(|(i, l)| {
                let l = if i == 0 {
                    l.split_once(": ").map_or("", |(_, r)| r)
                } else {
                    l.trim_start()
                };
                l.split(' ')
            })
            .collect();
        assert_eq!(
            words.join(" "),
            body,
            "nothing lost, no word cut: {said:#?}"
        );
        // A word longer than the line is still cut, and nothing is lost.
        assert_eq!(wrap_row(&"x".repeat(25), 10, 10).concat(), "x".repeat(25));
        assert_eq!(wrap_row("aa bb", 10, 10), ["aa bb"]);
        assert_eq!(wrap_row("aaaa bbbb", 5, 5), ["aaaa", "bbbb"]);
        assert!(wrap_row("", 5, 5).is_empty());
    }

    #[test]
    fn the_chat_shows_the_last_lines_and_drops_what_an_agent_could_use_to_drive_the_terminal() {
        let entry = |role: ChatRole, kind: ChatKind, text: &str| ChatEntry {
            role: role as i32,
            kind: kind as i32,
            text: text.into(),
            ..Default::default()
        };
        let entries = vec![
            entry(ChatRole::User, ChatKind::Text, "hello"),
            entry(
                ChatRole::Agent,
                ChatKind::Text,
                "hi\x1b]0;owned\x07 there\u{202e}x",
            ),
            entry(ChatRole::Agent, ChatKind::ToolCall, "{\"tool\":\"ls\"}"),
        ];
        let lines = render_chat(&entries, "agent-1", "ty\x1bped", (80, 24), false, &t);
        let text = lines.join("\n");
        assert!(
            !text.contains('\x1b') && !text.contains('\x07') && !text.contains('\u{202e}'),
            "{text:?}"
        );
        assert!(text.contains("tui-chat-you[]: hello"), "{text}");
        assert!(text.contains("hi]0;owned there"), "{text}");
        assert!(text.contains("tui-chat-tool[text="), "{text}");
        assert!(lines.last().is_some_and(|l| l.contains("typed")));
        // ADR-0058: a notice is said from its code; its parameters are plain text; an unknown code is named.
        let mut notice = entry(ChatRole::Agent, ChatKind::Notice, "");
        notice.code = "adapter.agent_exited".into();
        notice
            .params
            .insert("stderr_tail".into(), "boom\x1b[2J".into());
        let mut other = entry(ChatRole::Agent, ChatKind::Notice, "");
        other.code = "future.thing".into();
        // Wide enough that the notice is on one line (wrapping is checked on its own).
        let said = render_chat(&[notice, other], "a", "", (160, 24), false, &t).join("\n");
        assert!(
            said.contains("tui-notice-agent_exited[stage=,exit_code=?,stderr_tail=boom[2J]"),
            "{said}"
        );
        assert!(
            said.contains("tui-notice-other[code=future.thing]"),
            "{said}"
        );
        let a = AgentSnapshot {
            modes: vec![agora_protocol::v1::SessionOption {
                id: "plan".into(),
                name: "Plan".into(),
                ..Default::default()
            }],
            current_mode: "plan".into(),
            context_tokens: 50,
            context_window: 200,
            ..Default::default()
        };
        assert_eq!(
            session_line(&a, &t).as_deref(),
            Some("tui-chat-mode[mode=Plan] · tui-chat-context[percent=25]")
        );
        assert!(session_line(&AgentSnapshot::default(), &t).is_none());
        // Only what fits is kept.
        let many: Vec<ChatEntry> = (0..50)
            .map(|i| entry(ChatRole::Agent, ChatKind::Text, &format!("line {i}")))
            .collect();
        let short = render_chat(&many, "a", "", (80, 10), false, &t);
        assert_eq!(short.len(), 7 + 2);
        assert!(
            short.iter().any(|l| l.contains("line 49"))
                && !short.iter().any(|l| l.contains("line 0"))
        );
    }

    #[test]
    fn a_typed_switch_needs_an_offered_id_and_a_mode_that_does_not_ask_needs_it_twice() {
        let opt = |id: &str, skips: bool| agora_protocol::v1::SessionOption {
            id: id.into(),
            name: id.to_uppercase(),
            skips_permissions: skips,
            ..Default::default()
        };
        let a = AgentSnapshot {
            modes: vec![opt("plan", false), opt("yolo", true)],
            models: vec![opt("m1", false)],
            ..Default::default()
        };
        assert_eq!(chat_command("fix it", Some(&a), None), ChatCommand::Prompt);
        assert_eq!(
            chat_command("/mode PLAN", Some(&a), None),
            ChatCommand::Switch {
                model: false,
                id: "plan".into()
            }
        );
        assert_eq!(
            chat_command("/model m1", Some(&a), None),
            ChatCommand::Switch {
                model: true,
                id: "m1".into()
            }
        );
        assert_eq!(
            chat_command("/model m9", Some(&a), None),
            ChatCommand::Unknown {
                id: "m9".into(),
                offered: vec!["m1".into()]
            }
        );
        assert_eq!(
            chat_command("/mode yolo", Some(&a), None),
            ChatCommand::Confirm { id: "yolo".into() }
        );
        assert_eq!(
            chat_command("/mode yolo", Some(&a), Some("/mode yolo")),
            ChatCommand::Switch {
                model: false,
                id: "yolo".into()
            }
        );
        assert!(matches!(
            chat_command("/mode plan", None, None),
            ChatCommand::Unknown { .. }
        ));
    }

    #[test]
    fn helpers_sit_under_their_parent_and_a_start_request_shows_its_prompt() {
        let mut snap = snapshot();
        snap.agents.insert(
            0,
            AgentSnapshot {
                agent_id: "laptop/agent-3".into(),
                node_id: "laptop".into(),
                project_id: "web".into(),
                kind: "codex".into(),
                running: true,
                parent_agent_id: "laptop/agent-1".into(),
                ..Default::default()
            },
        );
        snap.approvals[0].action = "agent.start".into();
        snap.approvals[0].detail =
            r#"{"agent":"agent-1","agent_kind":"codex","node":"laptop","kind":"claude-code","folder":"/w","inside":true,"worktree":true,"depth":1,"prompt":"line one\nline \u001b[2Jtwo\u200b"}"#
                .into();
        let lines = render(&snap, false, false, "", "", 0, &t);
        let text = lines.join("\n");
        assert!(
            text.contains("tui-asks-start[agent=agent-1,agent_kind=codex,node=laptop,kind=claude-code,folder=/w,worktree=true,depth=1]"),
            "{text}"
        );
        assert!(
            text.contains("line one")
                && text.contains("line \\u{1b}[2Jtwo\\u{200b}")
                && !text.contains('\u{1b}'),
            "{text}"
        );
        let parent = lines
            .iter()
            .position(|l| l.contains("agent-1  codex"))
            .expect("parent row");
        assert!(
            lines[parent + 1].contains("agent-3")
                && lines[parent + 1].contains("tui-started-by[agent=agent-1]"),
            "{lines:?}"
        );
    }

    #[test]
    fn the_tui_hides_nothing_on_the_shared_list() {
        // The one list (crates/agora-core/src/hidden-characters.txt, pinned there and in the web client): every range's ends show as
        // escapes, in a start request and in plain text.
        for (lo, hi) in agora_core::text::HIDDEN {
            for c in [*lo, *hi].into_iter().filter_map(char::from_u32) {
                let s = c.to_string();
                let want = format!("\\u{{{:x}}}", u32::from(c));
                assert_eq!(visible(&s), want);
                // plain() drops the bidirectional controls outright and escapes the rest.
                assert!(plain(&s) == want || plain(&s).is_empty(), "{want}");
            }
        }
    }

    #[test]
    fn the_header_adds_up_the_tokens_the_agents_used() {
        let mut snap = snapshot();
        snap.agents[0].input_tokens = 1000;
        snap.agents[1].output_tokens = 250;
        let text = render(&snap, false, false, "", "", 0, &t).join("\n");
        assert!(text.contains("tui-usage[input=1000,output=250]"), "{text}");
        assert!(!text.contains("tui-cost"), "{text}");
        let priced = render(&snap, false, false, "", "", 1_234_567, &t).join("\n");
        assert!(priced.contains("tui-cost[usd=1.23]"), "{priced}");
    }

    #[test]
    fn without_color_there_are_no_escape_sequences() {
        let text = render(&snapshot(), false, true, "", "", 0, &t).join("\n");
        assert!(!text.contains('\x1b'));
        let colored = render(&snapshot(), true, true, "", "", 0, &t).join("\n");
        assert!(colored.contains('\x1b'));
    }

    #[test]
    fn an_empty_hub_says_nothing_waits_and_there_are_no_agents() {
        let text = render(&Snapshot::default(), false, true, "", "", 0, &t).join("\n");
        assert!(
            text.contains("tui-nothing-waiting[]") && text.contains("tui-no-agents[]"),
            "{text}"
        );
    }
}
