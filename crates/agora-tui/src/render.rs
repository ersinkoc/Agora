//! The console as lines of text: a pure function of the hub's snapshot (render never mutates state, AGENTS.md).
//! Machines, agents grouped by project with what each waits for, and the attention list at the top. No terminal I/O here.

use std::collections::BTreeMap;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{AgentSnapshot, AgentState, Snapshot};

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
/// `keys` says the console takes answers from the keyboard (a hint line appears while something waits); `notice` is the last
/// thing the user did, shown at the bottom.
pub fn render(s: &Snapshot, color: bool, keys: bool, notice: &str, t: Text<'_>) -> Vec<String> {
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
    for (project, agents) in by_project {
        let name = if project.is_empty() {
            t(key!("tui-no-project"), &[])
        } else {
            project.to_string()
        };
        out.push(format!("{bold}{name}{reset}"));
        for a in agents {
            let st = state_of(a);
            let (on, off) = paint(st, color);
            let marker = if color { "●" } else { "*" };
            let word = state_text(st, t);
            out.push(format!(
                "  {on}{marker} {word:<10}{off} {}  {dim}{}  {}{reset}",
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
    if !notice.is_empty() {
        out.push(notice.to_string());
    }
    out.push(format!("{dim}{}{reset}", t(key!("tui-quit-hint"), &[])));
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
        let lines = render(&snapshot(), false, true, "", &t);
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
        let project = lines.iter().position(|l| l == "web").expect("project row");
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
        let quiet = render(&snapshot(), false, false, "done!", &t).join("\n");
        assert!(
            !quiet.contains("tui-hint-answer") && quiet.contains("done!"),
            "{quiet}"
        );
    }

    #[test]
    fn without_color_there_are_no_escape_sequences() {
        let text = render(&snapshot(), false, true, "", &t).join("\n");
        assert!(!text.contains('\x1b'));
        let colored = render(&snapshot(), true, true, "", &t).join("\n");
        assert!(colored.contains('\x1b'));
    }

    #[test]
    fn an_empty_hub_says_nothing_waits_and_there_are_no_agents() {
        let text = render(&Snapshot::default(), false, true, "", &t).join("\n");
        assert!(
            text.contains("tui-nothing-waiting[]") && text.contains("tui-no-agents[]"),
            "{text}"
        );
    }
}
