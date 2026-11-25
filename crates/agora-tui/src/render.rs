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

/// ADR-0101: "asks less" while a running agent holds at least one "always" answer (`AgentSnapshot.always_allowed`), never beside "does not
/// ask" (that one says more: an agent started so, or whose mode skips asking).
pub fn asks_less(a: &AgentSnapshot, t: Text<'_>) -> Option<String> {
    (a.running
        && !a.always_allowed.is_empty()
        && !crate::board::acts_freely(a, &crate::client::Configs::default()))
    .then(|| t(key!("tui-badge-asks-less"), &[]))
}

/// The agent's badge in a line: "does not ask" (ADR-0059), else "asks less" (ADR-0101), else nothing; `sep` goes before it.
pub fn badge(a: &AgentSnapshot, t: Text<'_>, sep: &str) -> String {
    if a.skips_permissions {
        format!("{sep}[{}]", t(key!("tui-badge-no-ask"), &[]))
    } else {
        asks_less(a, t).map_or_else(String::new, |l| format!("{sep}[{l}]"))
    }
}

/// ADR-0060: what an interrupted agent's row says: it stopped with its machine, and `u` resumes it (or `U` starts it again).
pub fn interrupted_text(a: &AgentSnapshot, t: Text<'_>) -> String {
    t(
        if a.still_running {
            key!("tui-interrupted-running")
        } else if a.resumable {
            key!("tui-interrupted-resume")
        } else {
            key!("tui-interrupted-again")
        },
        &[],
    )
}

/// ADR-0070: what a machine row says about its presence: "online", or "offline · last seen 3 min ago" from when the hub last heard
/// from it (`last_event_ms`), relative to `now_ms`.
pub fn presence_text(n: &agora_protocol::v1::NodeSnapshot, now_ms: u64, t: Text<'_>) -> String {
    if n.online {
        return t(key!("tui-online"), &[]);
    }
    let offline = t(key!("tui-offline"), &[]);
    if n.last_event_ms == 0 {
        return offline;
    }
    let s = now_ms.saturating_sub(n.last_event_ms) / 1000;
    let (unit, count) = match s {
        0..=59 => ("s", s),
        60..=3599 => ("min", s / 60),
        3600..=86_399 => ("h", s / 3600),
        _ => ("d", s / 86_400),
    };
    let seen = t(
        key!("tui-last-seen"),
        &[
            ("unit", Arg::Str(unit.into())),
            ("n", Arg::Num(i64::try_from(count).unwrap_or(i64::MAX))),
        ],
    );
    format!("{offline} · {seen}")
}

/// The part of a hub-wide id after `<node>/`.
fn local(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, rest)| rest)
}

/// One screen of lines for a snapshot. `color` off spells states out with a marker and no escape sequences (NO_COLOR).
/// `selected` is the agent the keyboard points at (Enter opens its terminal); `keys` says the console takes answers from the keyboard (a hint line appears while something waits); `notice` is the last
/// thing the user did, shown at the bottom; `cost_micro_usd` is the hub's estimate of what the agents cost (0 when unknown);
/// `now_ms` is the time "last seen" counts from.
#[allow(clippy::too_many_arguments)]
pub fn render(
    s: &Snapshot,
    color: bool,
    keys: bool,
    selected: &str,
    notice: &str,
    cost_micro_usd: u64,
    now_ms: u64,
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
        if let Some((line, settings, prompt)) = start_card(&a.action, &a.detail, t) {
            out.push(format!("  {on}!{off} {line}"));
            for l in settings {
                out.push(format!("      {l}"));
            }
            for l in prompt {
                out.push(format!("      {dim}{l}{reset}"));
            }
            continue;
        }
        let line = t(
            key!("tui-asks"),
            &[
                ("agent", Arg::Str(visible(local(&a.agent_id)))),
                (
                    "action",
                    Arg::Str(visible(format!("{} {}", a.action, a.detail).trim())),
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
            &[("agent", Arg::Str(visible(local(&a.agent_id))))],
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
        out.push(format!("{bold}{}{reset}", visible(&name)));
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
                        &[("agent", Arg::Str(visible(local(&a.parent_agent_id))))],
                    )
                )
            };
            // ADR-0060: stopped with its machine; the keys that bring it back.
            let by = if a.interrupted && !a.running {
                format!("{by}  · {}", interrupted_text(a, t))
            } else {
                by
            };
            // ADR-0059: an agent started so that it acts without asking says so on its row, for its whole life (ADR-0101: else "asks
            // less" while it holds an "always" answer).
            let no_ask = badge(a, t, "  ");
            out.push(format!(
                "{pointer} {indent}{on}{marker} {word:<10}{off} {}  {dim}{}  {}{by}{reset}{no_ask}",
                visible(local(&a.agent_id)),
                visible(&a.kind),
                visible(&a.node_id),
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
        let status = presence_text(n, now_ms, t);
        out.push(format!("  {}  {dim}{status}{reset}", visible(id)));
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
pub(crate) fn start_card(
    action: &str,
    detail: &str,
    t: Text<'_>,
) -> Option<(String, Vec<String>, Vec<String>)> {
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
    // ADR-0066: every launch setting, in the order the request lists them and the signature covers them, named in this terminal's words.
    let settings = crate::client::start_setting_rows(detail)
        .iter()
        .map(|(label, id, value)| {
            t(
                key!("tui-start-setting"),
                &[
                    ("name", Arg::Str(setting_name(label, id, t))),
                    ("value", Arg::Str(visible(value))),
                ],
            )
        })
        .collect::<Vec<_>>();
    // ADR-0059 decision 5: the launch options, and what "acts without asking" means, in the same lines y / n answers.
    let mut settings = settings;
    let options = crate::client::start_option_rows(detail);
    if !options.is_empty() {
        let names: Vec<String> = options
            .iter()
            .map(|(label, id, _)| option_name(label, id, t))
            .collect();
        settings.push(t(
            key!("tui-start-setting"),
            &[
                ("name", Arg::Str(t(key!("tui-start-how-it-acts"), &[]))),
                ("value", Arg::Str(names.join(", "))),
            ],
        ));
        if options.iter().any(|(_, _, skips)| *skips) {
            settings.push(t(key!("tui-start-does-not-ask"), &[]));
        }
    }
    let prompt = visible(&start.prompt).lines().map(str::to_string).collect();
    Some((line, settings, prompt))
}

/// ADR-0059 decision 5: the names of the launch options of an `agent.start` request that make the helper act without asking, in this
/// terminal's words; `None` when it has none (or it is not a start). Allowing such a request asks for the confirmation first.
pub fn start_skipping_options(action: &str, detail: &str, t: Text<'_>) -> Option<String> {
    start_request(action, detail)?;
    let names: Vec<String> = crate::client::start_option_rows(detail)
        .iter()
        .filter(|(_, _, skips)| *skips)
        .map(|(label, id, _)| option_name(label, id, t))
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// ADR-0059: an option's name in this terminal's words: the machine's label when it is one of this terminal's option texts
/// (`option-skip-permissions`), else the option's id. Never the requesting agent's words.
fn option_name(label: &str, id: &str, t: Text<'_>) -> String {
    let own = label.starts_with("option-")
        && label.len() <= 40
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if own {
        let said = t(label, &[]);
        if said != label {
            return said;
        }
    }
    visible(id)
}

/// ADR-0066: a setting's name in this terminal's words: the machine's label when it is one of this terminal's setting texts
/// (`setting-model`, `setting-effort`), else the setting's id. Never the requesting agent's words.
fn setting_name(label: &str, id: &str, t: Text<'_>) -> String {
    let own = label.starts_with("setting-")
        && label.len() <= 40
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if own {
        let said = t(label, &[]);
        if said != label {
            return said;
        }
    }
    visible(id)
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

/// ADR-0101: the most characters of the agent's own name for its "always" option a line shows (as the web's `plainText(…, 64)`).
pub const ALWAYS_LABEL_MAX: usize = 64;

/// ADR-0101: "In its own words: “…”", the agent's own name for the always option `a` offers, as one short plain line of agent text
/// (control and direction characters dropped, line breaks and runs of spaces made one space, at most [`ALWAYS_LABEL_MAX`] characters).
/// `None` when the request offers no "always" or the agent gave it no name.
pub fn always_label_line(a: &agora_protocol::v1::ApprovalSnapshot, t: Text<'_>) -> Option<String> {
    let offer = crate::client::always_offer(a)?;
    let one = plain(&offer.label)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if one.is_empty() {
        return None;
    }
    let label = if one.chars().count() > ALWAYS_LABEL_MAX {
        let mut cut: String = one.chars().take(ALWAYS_LABEL_MAX - 1).collect();
        cut.push('…');
        cut
    } else {
        one
    };
    Some(t(key!("tui-always-label"), &[("label", Arg::Str(label))]))
}

/// ADR-0101: what is read before answering a request that offers an answer the agent remembers: the agent's own name for it (when it
/// gave one, [`always_label_line`]) and the warning that the agent decides what it covers and how long it keeps it, possibly longer than
/// this session (the approved words; `always_text` picks them). Empty when the request offers no "always".
pub fn always_lines(a: &agora_protocol::v1::ApprovalSnapshot, t: Text<'_>) -> Vec<String> {
    if crate::client::always_offer(a).is_none() {
        return Vec::new();
    }
    let mut out: Vec<String> = always_label_line(a, t).into_iter().collect();
    out.push(t(
        crate::client::always_text(key!("tui-always-warning"), false),
        &[],
    ));
    out
}

/// ADR-0058: a notice in the conversation, in words. The code picks the message; parameters are the agent's text, shown plain.
pub fn notice_text(e: &ChatEntry, t: Text<'_>) -> String {
    let p = |k: &str| Arg::Str(plain(e.params.get(k).map_or("", String::as_str)));
    let text = match e.code.as_str() {
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
        "adapter.output_dropped" => t(key!("tui-notice-output_dropped"), &[("bytes", p("bytes"))]),
        // ADR-0110 decision 6: the prompt before it was not confirmed by the agent in time.
        "adapter.no_answer" => t(
            key!("tui-notice-no_answer"),
            &[("waited_ms", p("waited_ms"))],
        ),
        // #902: the node's own notices about an adapter that ended on its own.
        "adapter.restarting" => t(key!("tui-notice-restarting"), &[("attempt", p("attempt"))]),
        // The exit code only when the node knows it (#902): an unknown one is left out, never said as `?` or as 1.
        "adapter.gave_up" => match e.params.get("exit_code") {
            Some(_) => t(
                key!("tui-notice-gave_up"),
                &[("restarts", p("restarts")), ("exit_code", p("exit_code"))],
            ),
            None => t(
                key!("tui-notice-gave_up-no_code"),
                &[("restarts", p("restarts"))],
            ),
        },
        // ADR-0089: the node could not write this conversation to its disk.
        "chat.history_not_saved" => t(key!("tui-notice-history_not_saved"), &[]),
        other => t(
            key!("tui-notice-other"),
            &[("code", Arg::Str(plain(other)))],
        ),
    };
    // Some sentences end in the agent's text (`stderr_tail`), which may be missing or end in a line break: the notice ends at its
    // words, without a space or an empty row after them.
    text.trim_end().to_string()
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
pub fn wrap_row(row: &str, first: usize, rest: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut left: Vec<char> = row.chars().collect();
    while !left.is_empty() {
        let width = (if out.is_empty() { first } else { rest }).max(1);
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

/// A tool call as one line: what kind of step it was, then its command, search, address or file (the agent's own title when it named
/// nothing else). Every part is the agent's text and is made plain.
pub fn tool_line(c: &crate::tool::Call, t: Text<'_>) -> String {
    use crate::tool::Action;
    let word = t(
        match c.action {
            Action::Read => key!("tui-tool-read"),
            Action::Edit => key!("tui-tool-edit"),
            Action::Run => key!("tui-tool-run"),
            Action::Search => key!("tui-tool-search"),
            Action::Fetch => key!("tui-tool-fetch"),
            Action::Delete => key!("tui-tool-delete"),
            Action::Move => key!("tui-tool-move"),
            Action::Think => key!("tui-tool-think"),
            Action::Other => key!("tui-tool-other"),
        },
        &[],
    );
    let what = if c.subject.is_empty() {
        &c.title
    } else {
        &c.subject
    };
    let what = plain(what).replace('\n', " ");
    if what.is_empty() {
        word
    } else {
        format!("{word} {what}")
    }
}

/// The conversation of an agent that has no terminal, as the last lines that fit `rows` x `cols`, then the line being typed.
/// `watching` (a viewer's window, ADR-0106): no line to type on; the hint says the window watches (the hub would refuse a request, a
/// mode or a model switch, and write the refusal down).
pub fn render_chat(
    entries: &[ChatEntry],
    agent: &str,
    typed: &str,
    (cols, rows): (usize, usize),
    color: bool,
    t: Text<'_>,
    watching: bool,
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
            // A tool call in words, as the web window says it ("Command cargo test"); text that is not a call is shown as it came.
            ChatKind::ToolCall => match crate::tool::call(&e.text) {
                Some(c) => tool_line(&c, t),
                // The text may end in a line break, which would add an empty row.
                None => t(key!("tui-chat-tool"), &[("text", Arg::Str(plain(&e.text)))])
                    .trim_end()
                    .to_string(),
            },
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
    // The hint wraps to the width (a narrow pane showed only its start); the conversation gets what is left.
    let hint_width = cols.saturating_sub(1).max(10);
    let hint_id = if watching {
        key!("tui-chat-watching")
    } else {
        key!("tui-chat-hint")
    };
    let hint = wrap_row(&t(hint_id, &[]), hint_width, hint_width);
    let input = usize::from(!watching);
    let room = rows.saturating_sub(1 + input + hint.len()).max(1);
    let skip = lines.len().saturating_sub(room);
    let mut out: Vec<String> = lines.into_iter().skip(skip).collect();
    for h in hint {
        out.push(format!("{dim}{h}{reset}"));
    }
    if !watching {
        out.push(format!(
            "{} {}",
            t(key!("tui-chat-input"), &[]),
            plain(typed)
        ));
    }
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

    /// ADR-0101: the request box quotes the agent's own name for its always option as one short plain line, only when offered.
    #[test]
    fn the_always_label_is_one_short_plain_line_of_agent_text() {
        use agora_protocol::v1::ApprovalScope;
        let ask = |label: &str, scopes: &[ApprovalScope]| ApprovalSnapshot {
            approval_id: "laptop/ap-1".into(),
            agent_id: "laptop/agent-1".into(),
            action: "run_command".into(),
            scopes: scopes.iter().map(|s| *s as i32).collect(),
            always_label: label.into(),
            always_option_id: "opt".into(),
            ..Default::default()
        };
        let both = [ApprovalScope::Once, ApprovalScope::Always];
        assert_eq!(
            always_label_line(&ask("Always allow git status", &both), &t).as_deref(),
            Some("tui-always-label[label=Always allow git status]")
        );
        // Control characters, an escape sequence, a direction override and line breaks never reach the screen; spaces collapse.
        assert_eq!(
            always_label_line(
                &ask("\u{1b}[2Jallow\n\n  \u{202e}all\ttabs\u{7}", &both),
                &t
            )
            .as_deref(),
            Some("tui-always-label[label=[2Jallow all tabs]")
        );
        // Cut at 64 characters, an ellipsis included.
        let long = always_label_line(&ask(&"é".repeat(100), &both), &t).unwrap();
        let label = long
            .trim_start_matches("tui-always-label[label=")
            .trim_end_matches(']');
        assert_eq!(label.chars().count(), ALWAYS_LABEL_MAX);
        assert!(label.ends_with('…'));
        // No name, only spaces, or no "always" offered: no line.
        assert_eq!(always_label_line(&ask("", &both), &t), None);
        assert_eq!(always_label_line(&ask(" \n\t ", &both), &t), None);
        assert_eq!(
            always_label_line(&ask("Always", &[ApprovalScope::Once]), &t),
            None
        );
    }

    /// ADR-0070: an offline machine's row says when it was last heard from, in the largest whole unit.
    #[test]
    fn an_offline_machine_says_when_it_was_last_seen() {
        let node = |online: bool, last: u64| NodeSnapshot {
            node_id: "vps".into(),
            online,
            last_event_ms: last,
            ..Default::default()
        };
        let now = 10_000_000;
        assert_eq!(presence_text(&node(true, 1), now, &t), "tui-online[]");
        assert_eq!(presence_text(&node(false, 0), now, &t), "tui-offline[]");
        assert_eq!(
            presence_text(&node(false, now - 42_000), now, &t),
            "tui-offline[] · tui-last-seen[unit=s,n=42]"
        );
        assert_eq!(
            presence_text(&node(false, now - 3 * 60_000 - 5_000), now, &t),
            "tui-offline[] · tui-last-seen[unit=min,n=3]"
        );
        assert_eq!(
            presence_text(&node(false, now - 2 * 3_600_000), now, &t),
            "tui-offline[] · tui-last-seen[unit=h,n=2]"
        );
        let mut s = snapshot();
        s.nodes[1].last_event_ms = 1_000;
        let text = render(&s, false, false, "", "", 0, 61_000, &t).join("\n");
        assert!(
            text.contains("vps  tui-offline[] · tui-last-seen[unit=min,n=1]"),
            "{text}"
        );
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
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_screen_says_what_waits_for_the_user_and_groups_agents_by_project() {
        let lines = render(&snapshot(), false, true, "", "", 0, 0, &t);
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
        let quiet = render(&snapshot(), false, false, "", "done!", 0, 0, &t).join("\n");
        assert!(
            !quiet.contains("tui-hint-answer") && quiet.contains("done!"),
            "{quiet}"
        );
    }

    #[test]
    fn an_agent_that_stopped_with_its_machine_says_how_to_bring_it_back() {
        let mut snap = snapshot();
        for (id, resumable) in [("laptop/agent-3", true), ("laptop/agent-4", false)] {
            snap.agents.push(AgentSnapshot {
                agent_id: id.into(),
                node_id: "laptop".into(),
                project_id: "web".into(),
                kind: "claude-code".into(),
                state: AgentState::Done as i32,
                interrupted: true,
                resumable,
                ..Default::default()
            });
        }
        let text = render(&snap, false, true, "", "", 0, 0, &t).join("\n");
        let row = |id: &str| text.lines().find(|l| l.contains(id)).unwrap().to_string();
        assert!(
            row("agent-3").contains("tui-interrupted-resume[]"),
            "{text}"
        );
        assert!(row("agent-4").contains("tui-interrupted-again[]"), "{text}");
        assert!(!row("agent-1").contains("tui-interrupted"), "{text}");
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
        let lines = render(&snap, false, true, "laptop/agent-1", "", 0, 0, &t);
        let row = lines
            .iter()
            .find(|l| l.contains("agent-1") && l.contains("codex"))
            .expect("row");
        assert!(row.starts_with('>'), "{row}");
        assert!(lines.join("\n").contains("tui-hint-open[]"));
        let other = render(&snap, false, true, "nobody/agent-9", "", 0, 0, &t).join("\n");
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
        let lines = render_chat(&entries, "agent-1", "", (cols, 24), false, &t, false);
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
    fn a_viewers_chat_has_no_line_to_type_on_and_says_it_watches() {
        // ADR-0106 (#1140): the hub would refuse a viewer's request and mode or model switch, and write the refusal down.
        let entries = vec![ChatEntry {
            role: ChatRole::Agent as i32,
            kind: ChatKind::Text as i32,
            text: "done".into(),
            ..Default::default()
        }];
        let member = render_chat(&entries, "a", "", (80, 10), false, &t, false);
        assert_eq!(member.last().map(String::as_str), Some("tui-chat-input[] "));
        assert!(member.iter().any(|l| l.contains("tui-chat-hint[]")));
        let viewer = render_chat(&entries, "a", "", (80, 10), false, &t, true);
        assert!(
            viewer.iter().all(|l| !l.contains("tui-chat-input")),
            "{viewer:?}"
        );
        assert!(
            viewer.iter().all(|l| !l.contains("tui-chat-hint")),
            "{viewer:?}"
        );
        assert_eq!(
            viewer.last().map(String::as_str),
            Some("tui-chat-watching[]")
        );
        assert!(viewer.iter().any(|l| l.contains("done")));
    }

    #[test]
    fn a_tool_line_ending_in_a_line_break_adds_no_empty_row() {
        let en = agora_i18n::Catalog::english();
        let t = |id: &str, args: &[(&str, Arg)]| en.render(id, args).unwrap_or_default();
        let tool = |text: &str| ChatEntry {
            role: ChatRole::Agent as i32,
            kind: ChatKind::ToolCall as i32,
            text: text.into(),
            ..Default::default()
        };
        let plain_text = render_chat(&[tool("make")], "a", "", (80, 24), false, &t, false);
        let with_break = render_chat(&[tool("make\n")], "a", "", (80, 24), false, &t, false);
        assert!(
            plain_text.iter().any(|l| l.ends_with("ran make")),
            "{plain_text:?}"
        );
        assert_eq!(with_break, plain_text);
    }

    #[test]
    fn a_notice_without_the_agents_text_ends_at_its_words() {
        // The sentence ends in the agent's text, which can be missing or end in a line break.
        let en = agora_i18n::Catalog::english();
        let t = |id: &str, args: &[(&str, Arg)]| en.render(id, args).unwrap_or_default();
        let mut e = ChatEntry {
            kind: ChatKind::Notice as i32,
            code: "adapter.turn_empty".into(),
            ..Default::default()
        };
        let said = notice_text(&e, &t);
        assert!(said.ends_with("request)"), "{said:?}");
        e.params.insert("stderr_tail".into(), "boom\n".into());
        let said = notice_text(&e, &t);
        assert!(said.ends_with("request) boom"), "{said:?}");
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
        let lines = render_chat(&entries, "agent-1", "ty\x1bped", (80, 24), false, &t, false);
        let text = lines.join("\n");
        assert!(
            !text.contains('\x1b') && !text.contains('\x07') && !text.contains('\u{202e}'),
            "{text:?}"
        );
        assert!(text.contains("tui-chat-you[]: hello"), "{text}");
        assert!(text.contains("hi]0;owned there"), "{text}");
        assert!(text.contains("tui-tool-other[] ls"), "{text}");
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
        let said = render_chat(&[notice, other], "a", "", (160, 24), false, &t, false).join("\n");
        assert!(
            said.contains("tui-notice-agent_exited[stage=,exit_code=?,stderr_tail=boom[2J]"),
            "{said}"
        );
        assert!(
            said.contains("tui-notice-other[code=future.thing]"),
            "{said}"
        );
        // ADR-0110 decision 6: the node's mark on a prompt its adapter did not confirm in time.
        let mut unconfirmed = entry(ChatRole::Agent, ChatKind::Notice, "");
        unconfirmed.code = "adapter.no_answer".into();
        unconfirmed
            .params
            .insert("waited_ms".into(), "10000".into());
        let said = render_chat(&[unconfirmed], "a", "", (160, 24), false, &t, false).join("\n");
        assert!(
            said.contains("tui-notice-no_answer[waited_ms=10000]"),
            "{said}"
        );
        // #902: the node's own notices about an adapter that ended on its own, with their numbers.
        let mut restarting = entry(ChatRole::Agent, ChatKind::Notice, "");
        restarting.code = "adapter.restarting".into();
        restarting.params.insert("attempt".into(), "2".into());
        let mut gave_up = entry(ChatRole::Agent, ChatKind::Notice, "");
        gave_up.code = "adapter.gave_up".into();
        gave_up.params.insert("restarts".into(), "5".into());
        let unknown = gave_up.clone();
        gave_up.params.insert("exit_code".into(), "3".into());
        let said = render_chat(
            &[restarting, gave_up, unknown],
            "a",
            "",
            (160, 24),
            false,
            &t,
            false,
        )
        .join("\n");
        assert!(said.contains("tui-notice-restarting[attempt=2]"), "{said}");
        assert!(
            said.contains("tui-notice-gave_up[restarts=5,exit_code=3]"),
            "{said}"
        );
        assert!(
            said.contains("tui-notice-gave_up-no_code[restarts=5]"),
            "{said}"
        );
        // ADR-0089: the node could not save the conversation.
        let mut not_saved = entry(ChatRole::Agent, ChatKind::Notice, "");
        not_saved.code = "chat.history_not_saved".into();
        let said = render_chat(&[not_saved], "a", "", (160, 24), false, &t, false).join("\n");
        assert!(said.contains("tui-notice-history_not_saved"), "{said}");
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
        let short = render_chat(&many, "a", "", (80, 10), false, &t, false);
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
    fn an_agent_that_does_not_ask_says_so_on_its_row() {
        let mut snap = snapshot();
        snap.agents.insert(
            0,
            AgentSnapshot {
                agent_id: "laptop/agent-9".into(),
                node_id: "laptop".into(),
                kind: "codex".into(),
                running: true,
                launch_options: vec!["skip-permissions".into()],
                skips_permissions: true,
                ..Default::default()
            },
        );
        let lines = render(&snap, false, false, "", "", 0, 0, &t);
        let row = lines.iter().find(|l| l.contains("agent-9")).expect("row");
        assert!(row.contains("[tui-badge-no-ask[]]"), "{row}");
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.contains("tui-badge-no-ask"))
                .count(),
            1,
            "{lines:?}"
        );
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
        let lines = render(&snap, false, false, "", "", 0, 0, &t);
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
        let text = render(&snap, false, false, "", "", 0, 0, &t).join("\n");
        assert!(text.contains("tui-usage[input=1000,output=250]"), "{text}");
        assert!(!text.contains("tui-cost"), "{text}");
        let priced = render(&snap, false, false, "", "", 1_234_567, 0, &t).join("\n");
        assert!(priced.contains("tui-cost[usd=1.23]"), "{priced}");
    }

    #[test]
    fn without_color_there_are_no_escape_sequences() {
        let text = render(&snapshot(), false, true, "", "", 0, 0, &t).join("\n");
        assert!(!text.contains('\x1b'));
        let colored = render(&snapshot(), true, true, "", "", 0, 0, &t).join("\n");
        assert!(colored.contains('\x1b'));
    }

    #[test]
    fn an_empty_hub_says_nothing_waits_and_there_are_no_agents() {
        let text = render(&Snapshot::default(), false, true, "", "", 0, 0, &t).join("\n");
        assert!(
            text.contains("tui-nothing-waiting[]") && text.contains("tui-no-agents[]"),
            "{text}"
        );
    }

    #[test]
    fn a_tool_call_reads_as_words_and_its_text_cannot_drive_the_terminal() {
        let t = |id: &str, _: &[(&str, Arg)]| id.to_string();
        let line = |text: &str| {
            let e = ChatEntry {
                role: ChatRole::Agent as i32,
                kind: ChatKind::ToolCall as i32,
                text: text.into(),
                ..Default::default()
            };
            render_chat(&[e], "agent-1", "", (100, 10), false, &t, false).join("\n")
        };
        assert!(
            line(r#"{"tool":"execute","arguments":{"command":"cargo test"}}"#)
                .contains("tui-tool-run cargo test")
        );
        assert!(
            line(r#"{"tool":"Read src/a.rs","arguments":{"file_path":"/w/src/a.rs"}}"#)
                .contains("tui-tool-read /w/src/a.rs")
        );
        // An agent's command with escapes in it is drawn as plain text.
        let hostile = line(
            "{\"tool\":\"execute\",\"arguments\":{\"command\":\"echo \\u001b]0;owned\\u0007 x\"}}",
        );
        assert!(
            !hostile.contains('\x1b') && !hostile.contains('\x07'),
            "{hostile:?}"
        );
        assert!(hostile.contains("tui-tool-run echo"), "{hostile}");
        // Text that is not a call is said as it came.
        assert!(line("not json").contains("tui-chat-tool"));
    }

    #[test]
    fn wrap_row_with_zero_width_terminates_without_looping() {
        assert_eq!(wrap_row("ab", 0, 0), vec!["a", "b"]);
        assert_eq!(wrap_row("ab", 1, 0), vec!["a", "b"]);
        assert!(wrap_row("", 0, 0).is_empty());
    }

    #[test]
    fn approval_rows_make_untrusted_text_visible() {
        let mut snap = snapshot();
        snap.approvals[0].agent_id = "laptop/a\u{3164}gent".into();
        snap.approvals[0].action = "sh\u{2800}ell".into();
        snap.approvals[0].detail = "safe\u{034f} detail".into();
        let text = render(&snap, false, false, "", "", 0, 0, &t).join("\n");
        assert!(text.contains("website"), "control missing: {text:?}");
        assert!(!text.chars().any(agora_core::text::is_hidden), "{text:?}");
        assert!(
            text.contains("a\\u{3164}gent") && text.contains("sh\\u{2800}ell"),
            "{text:?}"
        );
    }

    #[test]
    fn inventory_rows_make_snapshot_labels_visible() {
        let mut snap = snapshot();
        snap.checkouts[0].path = "/work/pro\u{2800}ject".into();
        snap.agents[0].agent_id = "laptop/a\u{3164}gent".into();
        snap.agents[0].kind = "co\u{034f}dex".into();
        snap.agents[0].node_id = "lap\u{ffa0}top".into();
        snap.nodes[0].hostname = "ma\u{202e}chine".into();
        let text = render(&snap, false, false, "", "", 0, 0, &t).join("\n");
        assert!(text.contains("claude-code"), "control missing: {text:?}");
        assert!(!text.chars().any(agora_core::text::is_hidden), "{text:?}");
        for shown in [
            "pro\\u{2800}ject",
            "a\\u{3164}gent",
            "co\\u{34f}dex",
            "lap\\u{ffa0}top",
            "ma\\u{202e}chine",
        ] {
            assert!(text.contains(shown), "missing {shown:?}: {text:?}");
        }
    }

    #[test]
    fn verifies_approval_visibility_across_fields_and_unicode() {
        let mut snap = snapshot();
        snap.approvals[0].agent_id = "laptop/Grü\u{034f}ße".into();
        snap.approvals[0].action = "ça\u{3164}lış".into();
        snap.approvals[0].detail = "漢\u{2800}字 😀".into();
        let text = render(&snap, false, false, "", "", 0, 0, &t).join("\n");
        assert!(!text.chars().any(agora_core::text::is_hidden), "{text:?}");
        for shown in ["Grü\\u{34f}ße", "ça\\u{3164}lış", "漢\\u{2800}字 😀"] {
            assert!(text.contains(shown), "missing {shown:?}: {text:?}");
        }
    }

    #[test]
    fn verifies_inventory_visibility_across_sources_and_unicode() {
        let mut snap = snapshot();
        snap.checkouts[0].path = "/w/Grü\u{034f}ße".into();
        snap.agents[0].kind = "ça\u{3164}lış".into();
        snap.nodes[0].hostname = "漢\u{2800}字-😀".into();
        let text = render(&snap, false, false, "", "", 0, 0, &t).join("\n");
        assert!(!text.chars().any(agora_core::text::is_hidden), "{text:?}");
        for shown in ["Grü\\u{34f}ße", "ça\\u{3164}lış", "漢\\u{2800}字-😀"] {
            assert!(text.contains(shown), "missing {shown:?}: {text:?}");
        }
    }
}
