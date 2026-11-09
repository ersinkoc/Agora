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
/// thing the user did, shown at the bottom.
pub fn render(
    s: &Snapshot,
    color: bool,
    keys: bool,
    selected: &str,
    notice: &str,
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
        out.push(format!(
            "{dim}{}{reset}",
            t(
                key!("tui-usage"),
                &[
                    (
                        "input",
                        Arg::Num(i64::try_from(tokens_in).unwrap_or(i64::MAX))
                    ),
                    (
                        "output",
                        Arg::Num(i64::try_from(tokens_out).unwrap_or(i64::MAX))
                    ),
                ],
            )
        ));
    }
    out.push(String::new());

    // What needs the user: approvals first, then agents that are waiting.
    out.push(format!("{bold}{}{reset}", t(key!("tui-attention"), &[])));
    let mut waiting = 0;
    for a in &s.approvals {
        waiting += 1;
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
        let (on, off) = paint(AgentState::Blocked, color);
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
        for a in agents {
            let st = state_of(a);
            let (on, off) = paint(st, color);
            let marker = if color { "●" } else { "*" };
            let pointer = if keys && a.agent_id == selected {
                ">"
            } else {
                " "
            };
            let word = state_text(st, t);
            out.push(format!(
                "{pointer} {on}{marker} {word:<10}{off} {}  {dim}{}  {}{reset}",
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
        .collect()
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
            _ => plain(&e.text),
        };
        for (i, row) in body.split('\n').enumerate() {
            let prefix = if i == 0 {
                format!("{bold}{who}:{reset} ")
            } else {
                "  ".to_string()
            };
            // Hard wrap by characters; a wide character may overshoot a little, which is harmless.
            let width = cols.saturating_sub(4).max(10);
            let chars: Vec<char> = row.chars().collect();
            if chars.is_empty() {
                lines.push(prefix.trim_end().to_string());
            }
            for (j, chunk) in chars.chunks(width).enumerate() {
                let head = if j == 0 {
                    prefix.clone()
                } else {
                    "  ".to_string()
                };
                lines.push(format!("{head}{}", chunk.iter().collect::<String>()));
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
        let lines = render(&snapshot(), false, true, "", "", &t);
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
        let quiet = render(&snapshot(), false, false, "", "done!", &t).join("\n");
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
        let lines = render(&snap, false, true, "laptop/agent-1", "", &t);
        let row = lines
            .iter()
            .find(|l| l.contains("agent-1") && l.contains("codex"))
            .expect("row");
        assert!(row.starts_with('>'), "{row}");
        assert!(lines.join("\n").contains("tui-hint-open[]"));
        let other = render(&snap, false, true, "nobody/agent-9", "", &t).join("\n");
        assert!(!other.contains("tui-hint-open"), "{other}");
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
    fn the_header_adds_up_the_tokens_the_agents_used() {
        let mut snap = snapshot();
        snap.agents[0].input_tokens = 1000;
        snap.agents[1].output_tokens = 250;
        let text = render(&snap, false, false, "", "", &t).join("\n");
        assert!(text.contains("tui-usage[input=1000,output=250]"), "{text}");
    }

    #[test]
    fn without_color_there_are_no_escape_sequences() {
        let text = render(&snapshot(), false, true, "", "", &t).join("\n");
        assert!(!text.contains('\x1b'));
        let colored = render(&snapshot(), true, true, "", "", &t).join("\n");
        assert!(colored.contains('\x1b'));
    }

    #[test]
    fn an_empty_hub_says_nothing_waits_and_there_are_no_agents() {
        let text = render(&Snapshot::default(), false, true, "", "", &t).join("\n");
        assert!(
            text.contains("tui-nothing-waiting[]") && text.contains("tui-no-agents[]"),
            "{text}"
        );
    }
}
