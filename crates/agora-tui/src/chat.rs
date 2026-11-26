//! ADR-0090 step 7: a structured agent's conversation in the terminal client, as far as it fits a terminal. Pure: the entries a
//! client keeps (a later version of an entry replaces the one it has), how an entry reads (thinking hidden until asked, a tool call with
//! its status and duration, a plan with its steps, a permission record, a prompt that waits), and what a typed line asks for besides a
//! request: stop the turn (`/stop`, `/stop all`), show or hide the thinking (`/thinking`), set the effort (`/effort NAME`) or any other
//! setting the agent offers while it runs (`/set ID VALUE`). The caller sends what is asked (`client::Session::cancel_turn`,
//! `client::Session::set_config`); nothing here does I/O.
//!
//! Every text in an entry or a setting is the agent's (untrusted): it is made plain ([`crate::render::plain`]) and never acted on.

use agora_i18n::{key, Arg};
use agora_protocol::v1::{
    AgentSnapshot, AgentState, ChatEntry, ChatKind, ChatRole, PermissionOutcome,
    SessionConfigCategory, SessionConfigOption, SessionConfigType, ToolStatus,
};

use crate::render::{plain, Text};

/// The most entries kept per conversation, as the node's ring keeps (ADR-0038): older ones are dropped first.
pub const MAX_ENTRIES: usize = 500;

/// Takes what a chat stream sent: a snapshot (`replace`) stands for everything before it; an entry the client already has (same
/// `entry_id`) is replaced by its later version (a tool call or a plan changes in place, ADR-0090 decision 1); a new one goes in id
/// order. A node without `chat.rich` never sends a repeat, so for it this is what the client did before. Returns whether anything
/// changed.
pub fn merge(entries: &mut Vec<ChatEntry>, lines: Vec<ChatEntry>, replace: bool) -> bool {
    if replace {
        *entries = lines;
        entries.sort_by_key(|e| e.entry_id);
        entries.dedup_by(|later, earlier| {
            if later.entry_id == earlier.entry_id {
                std::mem::swap(later, earlier);
                true
            } else {
                false
            }
        });
        trim(entries);
        return true;
    }
    let mut changed = false;
    for e in lines {
        match entries.binary_search_by_key(&e.entry_id, |x| x.entry_id) {
            Ok(at) => {
                if entries[at] != e {
                    entries[at] = e;
                    changed = true;
                }
            }
            Err(at) => {
                entries.insert(at, e);
                changed = true;
            }
        }
    }
    trim(entries);
    changed
}

fn trim(entries: &mut Vec<ChatEntry>) {
    let over = entries.len().saturating_sub(MAX_ENTRIES);
    if over > 0 {
        entries.drain(..over);
    }
}

/// Whether the node speaks the rich form for this conversation, as far as a client can tell (it cannot read a node's capabilities,
/// as the web window cannot): an entry only such a node sends, or settings the agent reported.
pub fn is_rich(entries: &[ChatEntry], options: &[SessionConfigOption]) -> bool {
    !options.is_empty()
        || entries.iter().any(|e| {
            matches!(
                ChatKind::try_from(e.kind),
                Ok(ChatKind::Thought | ChatKind::Permission)
            ) || !e.call_id.is_empty()
                || e.status != ToolStatus::Unspecified as i32
                || !e.plan_steps.is_empty()
                || e.queued
        })
}

/// Whether the agent is in a turn now.
pub fn working(a: &AgentSnapshot) -> bool {
    a.running && a.state == AgentState::Working as i32
}

/// The person's prompts that wait behind the turn (ADR-0090 decision 5).
pub fn waiting(entries: &[ChatEntry]) -> usize {
    entries
        .iter()
        .filter(|e| e.queued && e.role == ChatRole::User as i32)
        .count()
}

/// How the conversation is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The agent's thinking is shown (`/thinking`); by default each thought is one line saying it is hidden (ADR-0090 security: shown
    /// only on the user's request).
    pub thoughts: bool,
    /// The entries are kept with [`merge`], so a tool call's status, a plan's steps and a permission's outcome are their latest:
    /// they are shown. A caller that keeps only the first version of an entry leaves this off, and those stay unsaid.
    pub rich: bool,
    /// The turn is over (the agent is not working): a call still waiting or running is shown stopped (#908; ACP leaves that to the
    /// client).
    pub turn_over: bool,
}

/// An entry in words, without its speaker; rows separated by `\n`. `None`: the caller's own reading (text, a diff, a notice).
pub fn body(e: &ChatEntry, view: View, t: Text<'_>) -> Option<String> {
    match ChatKind::try_from(e.kind).unwrap_or(ChatKind::Unspecified) {
        ChatKind::Thought => Some(if view.thoughts {
            let text = plain(&e.text);
            format!(
                "{}\n{}",
                t(key!("tui-chat-thought-shown"), &[]),
                text.trim_end()
            )
        } else if view.rich {
            t(key!("tui-chat-thought"), &[])
        } else {
            // A caller that keeps entries as first sent does not read `/thinking` either: no hint to a command it lacks.
            t(key!("tui-chat-thought-plain"), &[])
        }),
        ChatKind::ToolCall if view.rich => {
            let line = match crate::tool::call(&e.text) {
                Some(c) => crate::render::tool_line(&c, t),
                None => t(key!("tui-chat-tool"), &[("text", Arg::Str(plain(&e.text)))])
                    .trim_end()
                    .to_string(),
            };
            match tool_status(e, view.turn_over, t) {
                Some(s) => Some(format!("{line} · {s}")),
                None => Some(line),
            }
        }
        ChatKind::Plan if view.rich && !e.plan_steps.is_empty() => {
            let mut rows = vec![t(key!("tui-chat-plan"), &[])];
            for s in e.plan_steps.iter().take(64) {
                let text = Arg::Str(plain(&s.text).replace('\n', " "));
                rows.push(t(
                    match ToolStatus::try_from(s.status).unwrap_or(ToolStatus::Unspecified) {
                        ToolStatus::Completed => key!("tui-plan-step-done"),
                        ToolStatus::InProgress => key!("tui-plan-step-now"),
                        _ => key!("tui-plan-step-todo"),
                    },
                    &[("text", text)],
                ));
            }
            Some(rows.join("\n"))
        }
        ChatKind::Permission => Some(t(
            if !view.rich {
                key!("tui-chat-permission")
            } else {
                match PermissionOutcome::try_from(e.permission_outcome)
                    .unwrap_or(PermissionOutcome::Unspecified)
                {
                    PermissionOutcome::Approved if e.permission_always => {
                        key!("tui-chat-permission-always")
                    }
                    PermissionOutcome::Approved => key!("tui-chat-permission-approved"),
                    PermissionOutcome::Denied => key!("tui-chat-permission-denied"),
                    PermissionOutcome::Cancelled => key!("tui-chat-permission-cancelled"),
                    PermissionOutcome::Pending => key!("tui-chat-permission-pending"),
                    PermissionOutcome::Unspecified => key!("tui-chat-permission"),
                }
            },
            &[],
        )),
        ChatKind::Text if view.rich && e.queued && e.role == ChatRole::User as i32 => {
            Some(format!(
                "{}\n{}",
                plain(&e.text).trim_end(),
                t(key!("tui-chat-queued"), &[])
            ))
        }
        _ => None,
    }
}

/// A tool call's status in words, with its duration once it ended. `None` when the node said nothing about it.
fn tool_status(e: &ChatEntry, turn_over: bool, t: Text<'_>) -> Option<String> {
    let status = ToolStatus::try_from(e.status).unwrap_or(ToolStatus::Unspecified);
    let seconds = || {
        e.ended_ms
            .checked_sub(e.started_ms)
            .filter(|_| e.started_ms > 0 && e.ended_ms > 0)
            .map(|ms| i64::try_from(ms.div_ceil(1000)).unwrap_or(i64::MAX))
    };
    let ended = |id: &str| match seconds() {
        Some(s) => format!(
            "{} {}",
            t(id, &[]),
            t(key!("tui-tool-seconds"), &[("seconds", Arg::Num(s))])
        ),
        None => t(id, &[]),
    };
    Some(match status {
        ToolStatus::Unspecified => return None,
        ToolStatus::Pending | ToolStatus::InProgress if turn_over => {
            t(key!("tui-tool-status-stopped"), &[])
        }
        ToolStatus::Pending => t(key!("tui-tool-status-pending"), &[]),
        ToolStatus::InProgress => t(key!("tui-tool-status-running"), &[]),
        ToolStatus::Completed => ended(key!("tui-tool-status-done")),
        ToolStatus::Failed => ended(key!("tui-tool-status-failed")),
    })
}

/// What a typed line asks for besides a request or a mode or model switch (those are [`crate::render::chat_command`]'s).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// None of these: the caller reads the line as before.
    Other,
    /// `/stop`: end the turn and keep the prompts that wait; `/stop all`: drop them too.
    Stop { drop_queue: bool },
    /// `/thinking`: show or hide the agent's thinking.
    Thinking,
    /// `/effort ...` or `/set ...`.
    Config(ConfigAsk),
}

/// What `/effort` or `/set` asks for, read against the agent's settings ([`crate::client::Configs::options`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigAsk {
    /// Send `AgentSetConfig { config_id, value }`; `option` and `choice` are the agent's words for them (plain), for the note.
    Set {
        config_id: String,
        value: String,
        option: String,
        choice: String,
    },
    /// The value makes the agent act without asking: the same line again sends it (and the machine then asks for the owner's
    /// signed approval, ADR-0090 Amendment A).
    Confirm { option: String, choice: String },
    /// The setting is on that value already.
    Current { option: String, choice: String },
    /// The agent has no setting by that id or name; the ids it has.
    UnknownOption { id: String, offered: Vec<String> },
    /// The setting has no such value (or none was typed); the values it takes.
    UnknownValue {
        option: String,
        value: String,
        offered: Vec<String>,
    },
    /// The agent offers no setting while it runs.
    NoOptions,
    /// `/effort` for an agent that offers no effort while it runs: it is a start setting there (ADR-0066).
    NoEffort,
}

/// Reads a typed line. `options` are the agent's settings as it last reported them; `confirming` is the line that asked to confirm
/// last time, if any: typing it again sends it.
pub fn ask(line: &str, options: &[SessionConfigOption], confirming: Option<&str>) -> Ask {
    let line = line.trim();
    let mut words = line.split_whitespace();
    let Some(first) = words.next() else {
        return Ask::Other;
    };
    let rest: Vec<&str> = words.collect();
    match (first, rest.as_slice()) {
        ("/stop", []) => Ask::Stop { drop_queue: false },
        ("/stop", ["all"]) => Ask::Stop { drop_queue: true },
        ("/thinking", []) => Ask::Thinking,
        ("/effort", value) => {
            let effort = options
                .iter()
                .find(|o| o.category == SessionConfigCategory::ThoughtLevel as i32);
            Ask::Config(match effort {
                None => ConfigAsk::NoEffort,
                Some(o) => choose(o, &value.join(" "), line, confirming),
            })
        }
        ("/set", [id, value @ ..]) => Ask::Config(if options.is_empty() {
            ConfigAsk::NoOptions
        } else {
            match options
                .iter()
                .find(|o| o.id == *id)
                .or_else(|| options.iter().find(|o| !o.name.is_empty() && o.name == *id))
            {
                Some(o) => choose(o, &value.join(" "), line, confirming),
                None => ConfigAsk::UnknownOption {
                    id: plain(id),
                    offered: options.iter().map(|o| plain(&o.id)).collect(),
                },
            }
        }),
        ("/set", []) => Ask::Config(if options.is_empty() {
            ConfigAsk::NoOptions
        } else {
            ConfigAsk::UnknownOption {
                id: String::new(),
                offered: options.iter().map(|o| plain(&o.id)).collect(),
            }
        }),
        _ => Ask::Other,
    }
}

/// The agent's word for an option (its name, else its id).
fn option_name(o: &SessionConfigOption) -> String {
    plain(if o.name.is_empty() { &o.id } else { &o.name })
}

/// The value `typed` names for `o`: a choice by id or name, or for a boolean on / off (true / false).
fn choose(o: &SessionConfigOption, typed: &str, line: &str, confirming: Option<&str>) -> ConfigAsk {
    let typed = typed.trim();
    let option = option_name(o);
    let boolean = o.r#type == SessionConfigType::Boolean as i32;
    let picked: Option<(String, String, bool)> = if boolean {
        let value = match typed.to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" => Some("true"),
            "off" | "false" | "no" => Some("false"),
            _ => None,
        };
        value.map(|v| {
            (
                v.to_string(),
                v.to_string(),
                o.skips_permissions && v == "true",
            )
        })
    } else {
        o.choices
            .iter()
            .find(|c| c.id == typed)
            .or_else(|| {
                o.choices
                    .iter()
                    .find(|c| !c.name.is_empty() && c.name == typed)
            })
            .map(|c| {
                (
                    c.id.clone(),
                    plain(if c.name.is_empty() { &c.id } else { &c.name }),
                    c.skips_permissions,
                )
            })
    };
    let Some((value, choice, skips)) = picked else {
        return ConfigAsk::UnknownValue {
            option,
            value: plain(typed),
            offered: if boolean {
                vec!["on".into(), "off".into()]
            } else {
                o.choices.iter().map(|c| plain(&c.id)).collect()
            },
        };
    };
    if value == o.current {
        return ConfigAsk::Current { option, choice };
    }
    if skips && confirming != Some(line) {
        return ConfigAsk::Confirm { option, choice };
    }
    ConfigAsk::Set {
        config_id: o.id.clone(),
        value,
        option,
        choice,
    }
}

/// The note a [`ConfigAsk`] that sends nothing leaves under the conversation (`Set` is said by the caller once the machine answered).
pub fn config_note(a: &ConfigAsk, t: Text<'_>) -> String {
    match a {
        ConfigAsk::Set { .. } => String::new(),
        ConfigAsk::Confirm { option, choice } => t(
            key!("tui-config-confirm"),
            &[
                ("option", Arg::Str(option.clone())),
                ("value", Arg::Str(choice.clone())),
            ],
        ),
        ConfigAsk::Current { option, choice } => t(
            key!("tui-config-current"),
            &[
                ("option", Arg::Str(option.clone())),
                ("value", Arg::Str(choice.clone())),
            ],
        ),
        ConfigAsk::UnknownOption { id, offered } => t(
            key!("tui-config-unknown"),
            &[
                ("id", Arg::Str(id.clone())),
                ("ids", Arg::Str(offered.join(", "))),
            ],
        ),
        ConfigAsk::UnknownValue {
            option, offered, ..
        } => t(
            key!("tui-config-values"),
            &[
                ("option", Arg::Str(option.clone())),
                ("values", Arg::Str(offered.join(", "))),
            ],
        ),
        ConfigAsk::NoOptions => t(key!("tui-config-none"), &[]),
        ConfigAsk::NoEffort => t(key!("tui-config-no-effort"), &[]),
    }
}

/// What the machine's answer to `AgentSetConfig` says: switched, or waiting for the owner's signed approval (Amendment A, the request
/// shows like any other), or the stable code of a refusal.
pub fn config_answer(answer: Result<(), &agora_protocol::v1::ErrorInfo>, t: Text<'_>) -> String {
    match answer {
        Ok(()) => t(key!("tui-option-switched"), &[]),
        Err(e) if e.code == "agent.approval_needed" => t(key!("tui-config-approval-needed"), &[]),
        Err(e) => t(
            key!("tui-send-failed"),
            &[("code", Arg::Str(plain(&e.code)))],
        ),
    }
}

/// What the machine's answer to `AgentCancelTurn` says.
pub fn stop_answer(answer: Result<(), &str>, drop_queue: bool, t: Text<'_>) -> String {
    match answer {
        Ok(()) if drop_queue => t(key!("tui-turn-stopped-dropped"), &[]),
        Ok(()) => t(key!("tui-turn-stopped"), &[]),
        Err(code) => t(
            key!("tui-turn-stop-failed"),
            &[("code", Arg::Str(plain(code)))],
        ),
    }
}

/// Records in `options` a setting the agent accepted, so the line above the conversation shows it before the agent's next report.
pub fn switched(options: &mut [SessionConfigOption], config_id: &str, value: &str) {
    if let Some(o) = options.iter_mut().find(|o| o.id == config_id) {
        o.current = value.to_string();
    }
}

/// The effort part of the line above the conversation (`effort high`), when the agent offers its effort while it runs.
pub fn effort_part(options: &[SessionConfigOption], t: Text<'_>) -> Option<String> {
    let o = options
        .iter()
        .find(|o| o.category == SessionConfigCategory::ThoughtLevel as i32)?;
    if o.current.is_empty() {
        return None;
    }
    let shown = o
        .choices
        .iter()
        .find(|c| c.id == o.current)
        .map_or(o.current.as_str(), |c| {
            if c.name.is_empty() {
                c.id.as_str()
            } else {
                c.name.as_str()
            }
        });
    Some(t(
        key!("tui-chat-effort"),
        &[("effort", Arg::Str(plain(shown)))],
    ))
}

/// The line above a structured agent's conversation: [`crate::render::session_line`] (mode, model, context) and the effort.
pub fn session_line(
    a: &AgentSnapshot,
    options: &[SessionConfigOption],
    t: Text<'_>,
) -> Option<String> {
    let parts: Vec<String> = crate::render::session_line(a, t)
        .into_iter()
        .chain(effort_part(options, t))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The hint under the conversation of a node that speaks the rich form: it names `/stop`, `/effort` and `/set` too.
pub fn hint_key(rich: bool) -> &'static str {
    if rich {
        key!("tui-chat-hint-rich")
    } else {
        key!("tui-chat-hint")
    }
}

#[cfg(test)]
mod tests;
