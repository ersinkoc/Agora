#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use agora_protocol::v1::{ErrorInfo, PlanStep, SessionOption};

fn t(id: &str, args: &[(&str, Arg)]) -> String {
    let a: Vec<String> = args
        .iter()
        .map(|(k, v)| match v {
            Arg::Str(s) => format!("{k}={s}"),
            Arg::Num(n) => format!("{k}={n}"),
        })
        .collect();
    if a.is_empty() {
        id.to_string()
    } else {
        format!("{id}[{}]", a.join(","))
    }
}

fn entry(id: u64, kind: ChatKind, text: &str) -> ChatEntry {
    ChatEntry {
        entry_id: id,
        role: ChatRole::Agent as i32,
        kind: kind as i32,
        text: text.into(),
        ..Default::default()
    }
}

fn select(
    id: &str,
    category: SessionConfigCategory,
    current: &str,
    choices: &[(&str, bool)],
) -> SessionConfigOption {
    SessionConfigOption {
        id: id.into(),
        name: format!("{id} name"),
        category: category as i32,
        r#type: SessionConfigType::Select as i32,
        current: current.into(),
        choices: choices
            .iter()
            .map(|(c, skips)| SessionOption {
                id: (*c).into(),
                name: String::new(),
                skips_permissions: *skips,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn a_later_version_replaces_the_entry_and_a_new_one_goes_in_order() {
    let mut entries = Vec::new();
    assert!(merge(
        &mut entries,
        vec![
            entry(1, ChatKind::Text, "hi"),
            entry(3, ChatKind::ToolCall, "{}")
        ],
        true
    ));
    let mut done = entry(3, ChatKind::ToolCall, "{}");
    done.status = ToolStatus::Completed as i32;
    assert!(merge(
        &mut entries,
        vec![done.clone(), entry(2, ChatKind::Text, "late")],
        false
    ));
    assert_eq!(
        entries.iter().map(|e| e.entry_id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(entries[2], done);
    // The same version again changes nothing.
    assert!(!merge(&mut entries, vec![done], false));
}

#[test]
fn a_snapshot_stands_for_everything_and_keeps_the_last_of_a_repeat() {
    let mut entries = vec![entry(9, ChatKind::Text, "old")];
    let mut second = entry(1, ChatKind::Text, "second");
    second.at_ms = 2;
    merge(
        &mut entries,
        vec![entry(1, ChatKind::Text, "first"), second.clone()],
        true,
    );
    assert_eq!(entries, vec![second]);
}

#[test]
fn the_conversation_keeps_at_most_the_nodes_ring() {
    let mut entries = Vec::new();
    let many: Vec<ChatEntry> = (1..=(MAX_ENTRIES as u64 + 20))
        .map(|i| entry(i, ChatKind::Text, "x"))
        .collect();
    merge(&mut entries, many, false);
    assert_eq!(entries.len(), MAX_ENTRIES);
    assert_eq!(entries[0].entry_id, 21);
}

#[test]
fn rich_is_what_only_a_rich_node_sends() {
    assert!(!is_rich(&[entry(1, ChatKind::Text, "a")], &[]));
    assert!(is_rich(&[entry(1, ChatKind::Thought, "a")], &[]));
    let mut call = entry(1, ChatKind::ToolCall, "{}");
    call.call_id = "c1".into();
    assert!(is_rich(&[call], &[]));
    assert!(is_rich(
        &[],
        &[select("e", SessionConfigCategory::ThoughtLevel, "", &[])]
    ));
}

#[test]
fn thinking_is_hidden_until_asked() {
    let e = entry(1, ChatKind::Thought, "secret plan\n");
    assert_eq!(
        body(&e, View::default(), &t).unwrap(),
        "tui-chat-thought-plain"
    );
    let rich = View {
        rich: true,
        ..View::default()
    };
    assert_eq!(body(&e, rich, &t).unwrap(), "tui-chat-thought");
    let shown = View {
        thoughts: true,
        ..View::default()
    };
    assert_eq!(
        body(&e, shown, &t).unwrap(),
        "tui-chat-thought-shown\nsecret plan"
    );
    // Drawn: one hidden line, never the text.
    let lines = crate::render::render_chat(&[e], "a", "", (80, 10), false, &t, false);
    assert!(lines.iter().any(|l| l.contains("tui-chat-thought")));
    assert!(!lines.iter().any(|l| l.contains("secret")));
}

#[test]
fn a_tool_call_says_its_status_and_duration_only_when_kept_rich() {
    let mut e = entry(
        1,
        ChatKind::ToolCall,
        r#"{"tool":"Run","arguments":{"command":"cargo test"}}"#,
    );
    e.status = ToolStatus::Completed as i32;
    e.started_ms = 1_000;
    e.ended_ms = 3_500;
    assert_eq!(body(&e, View::default(), &t), None);
    let rich = View {
        rich: true,
        ..View::default()
    };
    let said = body(&e, rich, &t).unwrap();
    assert!(
        said.ends_with("· tui-tool-status-done tui-tool-seconds[seconds=3]"),
        "{said}"
    );
    e.status = ToolStatus::InProgress as i32;
    e.ended_ms = 0;
    assert!(body(&e, rich, &t)
        .unwrap()
        .ends_with("· tui-tool-status-running"));
    let over = View {
        turn_over: true,
        ..rich
    };
    assert!(body(&e, over, &t)
        .unwrap()
        .ends_with("· tui-tool-status-stopped"));
    e.status = ToolStatus::Failed as i32;
    assert!(body(&e, over, &t)
        .unwrap()
        .ends_with("· tui-tool-status-failed"));
}

#[test]
fn a_plan_lists_its_steps_and_a_permission_its_outcome() {
    let mut plan = entry(1, ChatKind::Plan, "a\nb");
    plan.plan_steps = vec![
        PlanStep {
            text: "read".into(),
            status: ToolStatus::Completed as i32,
            ..Default::default()
        },
        PlanStep {
            text: "write\nit".into(),
            status: ToolStatus::InProgress as i32,
            ..Default::default()
        },
        PlanStep {
            text: "test".into(),
            ..Default::default()
        },
    ];
    let rich = View {
        rich: true,
        ..View::default()
    };
    assert_eq!(
        body(&plan, rich, &t).unwrap(),
        "tui-chat-plan\ntui-plan-step-done[text=read]\ntui-plan-step-now[text=write it]\ntui-plan-step-todo[text=test]"
    );
    let mut p = entry(2, ChatKind::Permission, "");
    p.permission_outcome = PermissionOutcome::Denied as i32;
    assert_eq!(body(&p, rich, &t).unwrap(), "tui-chat-permission-denied");
    // Kept as first sent, the outcome may be stale: not said.
    assert_eq!(
        body(&p, View::default(), &t).unwrap(),
        "tui-chat-permission"
    );
    p.permission_outcome = PermissionOutcome::Approved as i32;
    p.permission_always = true;
    assert_eq!(body(&p, rich, &t).unwrap(), "tui-chat-permission-always");
}

#[test]
fn a_prompt_that_waits_says_so() {
    let mut e = entry(1, ChatKind::Text, "next");
    e.role = ChatRole::User as i32;
    e.queued = true;
    let rich = View {
        rich: true,
        ..View::default()
    };
    assert_eq!(body(&e, rich, &t).unwrap(), "next\ntui-chat-queued");
    assert_eq!(waiting(&[e.clone()]), 1);
    e.queued = false;
    assert_eq!(body(&e, rich, &t), None);
}

#[test]
fn stop_and_thinking_are_read_and_other_lines_are_not() {
    assert_eq!(
        ask("/stop", &[], None, true),
        Ask::Stop { drop_queue: false }
    );
    assert_eq!(
        ask("  /stop all ", &[], None, true),
        Ask::Stop { drop_queue: true }
    );
    assert_eq!(ask("/thinking", &[], None, true), Ask::Thinking);
    assert_eq!(ask("/stop now please", &[], None, true), Ask::Other);
    assert_eq!(ask("/mode plan", &[], None, true), Ask::Other);
    assert_eq!(ask("stop", &[], None, true), Ask::Other);
    assert_eq!(ask("", &[], None, true), Ask::Other);
}

#[test]
fn effort_is_the_thought_level_option() {
    let options = vec![
        select("model", SessionConfigCategory::Model, "a", &[("a", false)]),
        select(
            "effort",
            SessionConfigCategory::ThoughtLevel,
            "low",
            &[("low", false), ("high", false)],
        ),
    ];
    assert_eq!(
        ask("/effort high", &options, None, true),
        Ask::Config(ConfigAsk::Set {
            config_id: "effort".into(),
            value: "high".into(),
            option: "effort name".into(),
            choice: "high".into(),
        })
    );
    assert_eq!(
        ask("/effort low", &options, None, true),
        Ask::Config(ConfigAsk::Current {
            option: "effort name".into(),
            choice: "low".into()
        })
    );
    assert_eq!(
        ask("/effort", &options, None, true),
        Ask::Config(ConfigAsk::UnknownValue {
            option: "effort name".into(),
            value: String::new(),
            offered: vec!["low".into(), "high".into()],
        })
    );
    assert_eq!(
        ask("/effort high", &options[..1], None, true),
        Ask::Config(ConfigAsk::NoEffort)
    );
    assert_eq!(
        effort_part(&options, &t),
        Some("tui-chat-effort[effort=low]".into())
    );
    assert_eq!(effort_part(&options[..1], &t), None);
}

#[test]
fn set_names_an_option_by_id_or_name_and_a_boolean_by_on_or_off() {
    let mut flag = SessionConfigOption {
        id: "web".into(),
        name: "Web search".into(),
        r#type: SessionConfigType::Boolean as i32,
        current: "false".into(),
        ..Default::default()
    };
    let options = vec![flag.clone()];
    assert!(matches!(
        ask("/set web on", &options, None, true),
        Ask::Config(ConfigAsk::Set { ref value, .. }) if value == "true"
    ));
    assert!(matches!(
        ask("/set web maybe", &options, None, true),
        Ask::Config(ConfigAsk::UnknownValue { ref offered, .. }) if offered == &vec!["on".to_string(), "off".to_string()]
    ));
    assert_eq!(
        ask("/set nope on", &options, None, true),
        Ask::Config(ConfigAsk::UnknownOption {
            id: "nope".into(),
            offered: vec!["web".into()]
        })
    );
    assert_eq!(ask("/set web on", &[], None, true), Ask::Other);
    // A boolean that makes the agent act without asking is confirmed by typing the same line again.
    flag.skips_permissions = true;
    let options = vec![flag];
    let line = "/set web on";
    let first = ask(line, &options, None, true);
    assert!(matches!(first, Ask::Config(ConfigAsk::Confirm { .. })));
    assert!(matches!(
        ask(line, &options, Some(line), true),
        Ask::Config(ConfigAsk::Set { .. })
    ));
    // Off never asks.
    let mut on = options;
    on[0].current = "true".into();
    assert!(matches!(
        ask("/set web off", &on, None, true),
        Ask::Config(ConfigAsk::Set { .. })
    ));
}

#[test]
fn a_choice_that_skips_permissions_is_confirmed_first() {
    let options = vec![select(
        "mode",
        SessionConfigCategory::Mode,
        "default",
        &[("default", false), ("yolo", true)],
    )];
    assert!(matches!(
        ask("/set mode yolo", &options, None, true),
        Ask::Config(ConfigAsk::Confirm { .. })
    ));
    assert!(matches!(
        ask("/set mode yolo", &options, Some("/set mode plan"), true),
        Ask::Config(ConfigAsk::Confirm { .. })
    ));
    assert!(matches!(
        ask("/set mode yolo", &options, Some("/set mode yolo"), true),
        Ask::Config(ConfigAsk::Set { .. })
    ));
}

#[test]
fn the_machines_answers_are_said_in_words() {
    assert_eq!(config_answer(Ok(()), &t), "tui-option-switched");
    let wait = ErrorInfo {
        code: "agent.approval_needed".into(),
        ..Default::default()
    };
    assert_eq!(config_answer(Err(&wait), &t), "tui-config-approval-needed");
    let no = ErrorInfo {
        code: "agent.config_unknown".into(),
        ..Default::default()
    };
    assert_eq!(
        config_answer(Err(&no), &t),
        "tui-send-failed[code=agent.config_unknown]"
    );
    assert_eq!(stop_answer(Ok(()), true, &t), "tui-turn-stopped-dropped");
    assert_eq!(
        stop_answer(Err("node.offline"), false, &t),
        "tui-turn-stop-failed[code=node.offline]"
    );
    assert_eq!(
        config_note(&ConfigAsk::NoEffort, &t),
        "tui-config-no-effort"
    );
}

#[test]
fn the_head_line_adds_the_effort_and_a_switch_shows_at_once() {
    let a = AgentSnapshot {
        current_mode: "plan".into(),
        ..Default::default()
    };
    let mut options = vec![select(
        "effort",
        SessionConfigCategory::ThoughtLevel,
        "low",
        &[("low", false), ("high", false)],
    )];
    switched(&mut options, "effort", "high");
    assert_eq!(
        session_line(&a, &options, &t).unwrap(),
        "tui-chat-mode[mode=plan] · tui-chat-effort[effort=high]"
    );
    assert_eq!(session_line(&AgentSnapshot::default(), &[], &t), None);
}

#[test]
fn a_rich_view_draws_the_rich_hint_and_statuses() {
    let mut e = entry(
        1,
        ChatKind::ToolCall,
        r#"{"tool":"Run","arguments":{"command":"ls"}}"#,
    );
    e.status = ToolStatus::Pending as i32;
    let view = View {
        rich: true,
        ..View::default()
    };
    let lines = crate::render::render_chat_view(&[e], "a", "", (100, 10), false, &t, false, view);
    assert!(
        lines.iter().any(|l| l.contains("tui-tool-status-pending")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("tui-chat-hint-rich")));
    let plain_view = crate::render::render_chat(&[], "a", "", (100, 10), false, &t, false);
    assert!(plain_view.iter().any(|l| l == "tui-chat-hint"));
}

/// Review of #1242, F1: an agent waiting for a permission answer is blocked, and one whose state is not known may still be in its turn
/// (the web: in a turn unless idle or finished); its call is not shown stopped. An idle, finished or ended agent's turn is over.
#[test]
fn a_call_waiting_for_a_permission_is_not_shown_stopped() {
    let call = ChatEntry {
        entry_id: 1,
        role: ChatRole::Agent as i32,
        kind: ChatKind::ToolCall as i32,
        text: r#"{"tool":"Run","arguments":{"command":"rm -rf build"}}"#.into(),
        call_id: "c1".into(),
        status: ToolStatus::Pending as i32,
        ..Default::default()
    };
    let agent = |state: AgentState, running: bool| AgentSnapshot {
        running,
        state: state as i32,
        ..Default::default()
    };
    for (a, over) in [
        (agent(AgentState::Blocked, true), false),
        (agent(AgentState::Unspecified, true), false),
        (agent(AgentState::Working, true), false),
        (agent(AgentState::Idle, true), true),
        (agent(AgentState::Done, true), true),
        (agent(AgentState::Working, false), true),
    ] {
        assert_eq!(turn_over(&a), over, "{a:?}");
        let view = View {
            rich: true,
            turn_over: turn_over(&a),
            ..View::default()
        };
        let said = body(&call, view, &t).unwrap();
        assert_eq!(said.contains("tui-tool-status-stopped"), over, "{said}");
    }
}

/// Review of #1242, F3: with no settings reported, `/effort` and `/set` may be the agent's own commands: they go as a request. An older
/// node (not rich) gets `/stop` as a request too, since it cannot end a turn.
#[test]
fn without_settings_or_the_rich_form_the_line_goes_to_the_agent() {
    assert_eq!(ask("/effort high", &[], None, true), Ask::Other);
    assert_eq!(ask("/set foo bar", &[], None, true), Ask::Other);
    assert_eq!(ask("/set", &[], None, true), Ask::Other);
    assert_eq!(ask("/stop", &[], None, false), Ask::Other);
    assert_eq!(ask("/stop all", &[], None, false), Ask::Other);
    assert_eq!(ask("/thinking", &[], None, false), Ask::Thinking);
}

/// Review of #1242, F4: the agent's names for an option and its choices reach one-line notes without a line break, cut to a length.
#[test]
fn agent_names_in_one_line_notes_keep_no_line_break() {
    let options = vec![SessionConfigOption {
        id: "effort".into(),
        name: "Effort\nFAKE: approved".into(),
        category: SessionConfigCategory::ThoughtLevel as i32,
        current: "low".into(),
        choices: vec![
            SessionOption {
                id: "low".into(),
                name: "Low\nx".into(),
                ..Default::default()
            },
            SessionOption {
                id: "x".repeat(200),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    for line in ["/effort low", "/effort nope", "/set nope 1"] {
        let Ask::Config(c) = ask(line, &options, None, true) else {
            panic!("{line}")
        };
        let note = config_note(&c, &t);
        assert!(!note.contains('\n'), "{note:?}");
        assert!(note.chars().count() < 400, "{note:?}");
    }
    let mut many = options[0].clone();
    many.choices = (0..40)
        .map(|i| SessionOption {
            id: format!("c{i}"),
            ..Default::default()
        })
        .collect();
    let Ask::Config(ConfigAsk::UnknownValue { offered, .. }) =
        ask("/effort z", &[many], None, true)
    else {
        panic!()
    };
    assert_eq!(offered.len(), LISTED_MAX + 1);
    assert_eq!(
        effort_part(&options, &t).unwrap(),
        "tui-chat-effort[effort=Low x]"
    );
}
