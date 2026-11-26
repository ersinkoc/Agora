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
    if a.is_empty() {
        id.to_string()
    } else {
        format!("{id}[{}]", a.join(","))
    }
}

/// One agent, as a snapshot says it.
#[derive(Clone, Copy)]
struct A {
    state: AgentState,
    running: bool,
    exit: Option<i32>,
    interrupted: bool,
}

const fn run(state: AgentState) -> A {
    A {
        state,
        running: true,
        exit: None,
        interrupted: false,
    }
}

const fn closed(exit: Option<i32>, interrupted: bool) -> A {
    A {
        state: AgentState::Working,
        running: false,
        exit,
        interrupted,
    }
}

fn agent(id: &str, a: A) -> AgentSnapshot {
    AgentSnapshot {
        agent_id: id.into(),
        node_id: id.split_once('/').map_or("", |(n, _)| n).into(),
        kind: "claude".into(),
        state: a.state as i32,
        running: a.running,
        exit_code: a.exit,
        interrupted: a.interrupted,
        ..Default::default()
    }
}

fn snap(agents: &[(&str, A)]) -> Snapshot {
    Snapshot {
        nodes: vec![NodeSnapshot {
            node_id: "m".into(),
            online: true,
            ..Default::default()
        }],
        agents: agents.iter().map(|(id, a)| agent(id, *a)).collect(),
        ..Default::default()
    }
}

fn ask(s: &mut Snapshot, id: &str, agent: &str) {
    s.approvals.push(ApprovalSnapshot {
        approval_id: id.into(),
        agent_id: agent.into(),
        ..Default::default()
    });
}

fn ev(kind: Kind, agent: &str) -> Event {
    Event {
        kind,
        agent: agent.into(),
    }
}

/// The changes from one look to the next.
fn step(before: &Snapshot, after: &Snapshot) -> Changes {
    let first = changes(None, before);
    changes(Some(&first.next), after)
}

#[test]
fn the_transition_table() {
    use AgentState::{Blocked, Done, Idle, Unspecified, Working};
    // (before, after, finished alert, resumed)
    let table: &[(&str, A, A, bool, bool)] = &[
        ("a turn ends", run(Working), run(Idle), true, false),
        ("a turn ends as done", run(Working), run(Done), true, false),
        (
            "exits with 0",
            run(Working),
            closed(Some(0), false),
            true,
            false,
        ),
        (
            "exits with an error",
            run(Working),
            closed(Some(1), false),
            false,
            false,
        ),
        (
            "crashes",
            run(Working),
            closed(Some(-11), false),
            false,
            false,
        ),
        (
            "stopped on request",
            run(Working),
            closed(Some(i32::MIN), false),
            false,
            false,
        ),
        (
            "no exit status known",
            run(Working),
            closed(None, false),
            false,
            false,
        ),
        (
            "interrupted with its machine",
            run(Working),
            closed(Some(0), true),
            false,
            false,
        ),
        (
            "blocked in its terminal",
            run(Working),
            run(Blocked),
            false,
            true,
        ),
        ("blocked, then idle", run(Blocked), run(Idle), false, false),
        ("idle, then done", run(Idle), run(Done), false, false),
        (
            "unknown, then idle",
            run(Unspecified),
            run(Idle),
            false,
            false,
        ),
        (
            "working, then unknown",
            run(Working),
            run(Unspecified),
            false,
            false,
        ),
        ("idle, then working", run(Idle), run(Working), false, true),
        ("done, then working", run(Done), run(Working), false, true),
        ("no change", run(Working), run(Working), false, false),
    ];
    for (what, before, after, finished, resumed) in table {
        let c = step(
            &snap(&[("m/agent-1", *before)]),
            &snap(&[("m/agent-1", *after)]),
        );
        assert_eq!(
            c.finished == vec!["m/agent-1".to_string()],
            *finished,
            "{what}: finished"
        );
        assert_eq!(
            c.events.contains(&ev(Kind::Finished, "m/agent-1")),
            *finished,
            "{what}: the finished alert"
        );
        assert_eq!(!c.resumed.is_empty(), *resumed, "{what}: resumed");
    }
}

#[test]
fn the_first_look_and_a_new_agent_are_taken_as_they_are() {
    let mut s = snap(&[("m/agent-1", run(AgentState::Blocked))]);
    ask(&mut s, "r1", "m/agent-2");
    assert_eq!(changes(None, &s).events, vec![]);
    // An agent seen for the first time, already finished: no alert, no mark.
    let before = snap(&[]);
    let after = snap(&[("m/agent-1", run(AgentState::Idle))]);
    let c = step(&before, &after);
    assert!(c.events.is_empty() && c.finished.is_empty());
}

#[test]
fn needs_you_is_one_alert_per_agent_until_it_leaves_the_list() {
    let before = snap(&[("m/agent-1", run(AgentState::Working))]);
    let mut asking = before.clone();
    ask(&mut asking, "r1", "m/agent-1");
    let c = step(&before, &asking);
    assert_eq!(c.events, vec![ev(Kind::NeedsYou, "m/agent-1")]);
    // A second request of the same agent, or the agent blocked as well: nothing new.
    let mut more = asking.clone();
    ask(&mut more, "r2", "m/agent-1");
    more.agents[0].state = AgentState::Blocked as i32;
    assert_eq!(changes(Some(&c.next), &more).events, vec![]);
    // Off the list, then on it again: one alert again.
    let off = changes(Some(&c.next), &before);
    assert_eq!(off.events, vec![]);
    assert_eq!(
        changes(Some(&off.next), &asking).events,
        vec![ev(Kind::NeedsYou, "m/agent-1")]
    );
}

#[test]
fn a_blocked_agent_of_an_offline_machine_does_not_alert_and_a_request_of_no_agent_does() {
    let before = snap(&[("m/agent-1", run(AgentState::Working))]);
    let mut after = snap(&[("m/agent-1", run(AgentState::Blocked))]);
    after.nodes[0].online = false;
    assert!(step(&before, &after).events.is_empty());
    let mut request = before.clone();
    ask(&mut request, "r1", "");
    assert_eq!(step(&before, &request).events, vec![ev(Kind::NeedsYou, "")]);
}

#[test]
fn an_agent_that_left_the_list_is_gone() {
    let c = step(
        &snap(&[
            ("m/agent-1", run(AgentState::Idle)),
            ("m/agent-2", run(AgentState::Idle)),
        ]),
        &snap(&[("m/agent-2", run(AgentState::Idle))]),
    );
    assert_eq!(c.gone, vec!["m/agent-1".to_string()]);
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn v(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_mark_lasts_until_the_agent_is_seen_works_again_or_goes() {
    // Finished while not on screen: marked; finished while on screen: not.
    let m = next_unseen(&set(&[]), &v(&["a", "b"]), &[], &[], &v(&["b"]));
    assert_eq!(m, set(&["a"]));
    // Another look with nothing new keeps it.
    assert_eq!(next_unseen(&m, &[], &[], &[], &[]), set(&["a"]));
    // Seen.
    assert_eq!(next_unseen(&m, &[], &[], &[], &v(&["a"])), set(&[]));
    // At work again, or waiting on the person.
    assert_eq!(next_unseen(&m, &[], &v(&["a"]), &[], &[]), set(&[]));
    // Gone from the list.
    assert_eq!(next_unseen(&m, &[], &[], &v(&["a"]), &[]), set(&[]));
}

#[test]
fn the_gate_gives_one_alert_per_gap_the_most_urgent_of_a_burst() {
    let g = Gate::default();
    let (play, g) = gate(
        &g,
        vec![
            ev(Kind::Finished, "a"),
            ev(Kind::NeedsYou, "b"),
            ev(Kind::Finished, "c"),
        ],
        1000,
    );
    assert_eq!(play, Some(ev(Kind::NeedsYou, "b")));
    // Within the gap: it waits.
    let (play, g) = gate(&g, vec![ev(Kind::Finished, "d")], 1000 + GAP_MS - 1);
    assert_eq!(play, None);
    assert_eq!(g.pending, vec![ev(Kind::Finished, "d")]);
    // Flushing too early gives nothing; at the end of the gap the one that waited.
    assert_eq!(flush(&g, 1000 + GAP_MS - 1, |_| true).0, None);
    let (play, g2) = flush(&g, 1000 + GAP_MS, |_| true);
    assert_eq!(play, Some(ev(Kind::Finished, "d")));
    assert!(g2.pending.is_empty());
    assert_eq!(g2.last_ms, Some(1000 + GAP_MS));
    // What is no longer true when the gap ends is dropped, and starts no gap.
    let (play, g3) = flush(&g, 1000 + GAP_MS, |_| false);
    assert_eq!(play, None);
    assert!(g3.pending.is_empty());
    assert_eq!(g3.last_ms, Some(1000));
}

#[test]
fn many_finished_never_push_a_needs_you_out_of_the_burst() {
    let (_, g) = gate(&Gate::default(), vec![ev(Kind::Finished, "x")], 0);
    let mut burst = vec![ev(Kind::NeedsYou, "asks")];
    burst.extend((0..100).map(|i| ev(Kind::Finished, &format!("f{i}"))));
    let (play, g) = gate(&g, burst, 10);
    assert_eq!(play, None);
    assert!(g.pending.len() <= 2 * PENDING_MAX);
    assert_eq!(
        flush(&g, GAP_MS, |_| true).0,
        Some(ev(Kind::NeedsYou, "asks"))
    );
}

#[test]
fn alerts_never_for_the_agent_in_focus_and_a_late_alert_is_checked_again() {
    let mut alerts = Alerts::new(Prefs::default());
    let mut unseen = BTreeSet::new();
    let working = snap(&[
        ("m/agent-1", run(AgentState::Working)),
        ("m/agent-2", run(AgentState::Working)),
    ]);
    assert_eq!(alerts.look(&working, &mut unseen, &[], None, 0), None);
    // agent-1 is in focus (and on screen): no alert, no mark; agent-2 finishes elsewhere: an alert and a mark.
    let idle = snap(&[
        ("m/agent-1", run(AgentState::Idle)),
        ("m/agent-2", run(AgentState::Idle)),
    ]);
    let on = v(&["m/agent-1"]);
    assert_eq!(
        alerts.look(&idle, &mut unseen, &on, Some("m/agent-1"), 100),
        Some(ev(Kind::Finished, "m/agent-2"))
    );
    assert_eq!(unseen, set(&["m/agent-2"]));
    // Within the gap agent-2 asks, then works again before the gap ends: the alert that waited is not given.
    let mut asking = idle.clone();
    ask(&mut asking, "r1", "m/agent-2");
    assert_eq!(
        alerts.look(&asking, &mut unseen, &on, Some("m/agent-1"), 200),
        None
    );
    assert!(alerts.waiting());
    // Waiting on the person takes the mark away.
    assert_eq!(unseen, set(&[]));
    let back = snap(&[
        ("m/agent-1", run(AgentState::Idle)),
        ("m/agent-2", run(AgentState::Working)),
    ]);
    assert_eq!(
        alerts.look(&back, &mut unseen, &on, Some("m/agent-1"), 300),
        None
    );
    assert_eq!(alerts.due(100 + GAP_MS, Some("m/agent-1")), None);
    assert!(!alerts.waiting());
}

#[test]
fn a_late_alert_still_true_is_given_unless_its_agent_is_now_in_focus() {
    let working = snap(&[
        ("m/agent-1", run(AgentState::Working)),
        ("m/agent-2", run(AgentState::Working)),
    ]);
    let one = snap(&[
        ("m/agent-1", run(AgentState::Idle)),
        ("m/agent-2", run(AgentState::Working)),
    ]);
    let both = snap(&[
        ("m/agent-1", run(AgentState::Idle)),
        ("m/agent-2", run(AgentState::Idle)),
    ]);
    for (viewing, expect) in [
        (None, Some(ev(Kind::Finished, "m/agent-2"))),
        (Some("m/agent-2"), None),
    ] {
        let mut alerts = Alerts::new(Prefs::default());
        let mut unseen = BTreeSet::new();
        alerts.look(&working, &mut unseen, &[], None, 0);
        assert!(alerts.look(&one, &mut unseen, &[], None, 10).is_some());
        assert_eq!(alerts.look(&both, &mut unseen, &[], None, 20), None);
        assert_eq!(alerts.due(10 + GAP_MS - 1, viewing), None);
        assert_eq!(alerts.due(10 + GAP_MS, viewing), expect);
    }
}

#[test]
fn no_alert_when_both_are_off_but_the_mark_still_follows() {
    let mut alerts = Alerts::new(Prefs {
        sound: false,
        desktop: Desktop::Off,
    });
    let mut unseen = BTreeSet::new();
    alerts.look(
        &snap(&[("m/agent-1", run(AgentState::Working))]),
        &mut unseen,
        &[],
        None,
        0,
    );
    let done = snap(&[("m/agent-1", run(AgentState::Idle))]);
    assert_eq!(alerts.look(&done, &mut unseen, &[], None, 10), None);
    assert_eq!(unseen, set(&["m/agent-1"]));
}

#[test]
fn settings_read_back_and_anything_unknown_is_the_default() {
    assert_eq!(Prefs::parse(""), Prefs::default());
    assert!(Prefs::default().sound);
    assert_eq!(Prefs::default().desktop, Desktop::Off);
    let p = Prefs::parse("# mine\nsound = off\ndesktop_notify = on  # the usual one\n");
    assert_eq!(
        p,
        Prefs {
            sound: false,
            desktop: Desktop::Osc9
        }
    );
    assert_eq!(
        Prefs::parse("desktop_notify = \"osc777\"").desktop,
        Desktop::Osc777
    );
    assert_eq!(
        Prefs::parse("sound = loud\ndesktop_notify = maybe\nvolume = 3"),
        Prefs::default()
    );
    for p in [
        Prefs::default(),
        Prefs {
            sound: false,
            desktop: Desktop::Osc777,
        },
        Prefs {
            sound: true,
            desktop: Desktop::Osc9,
        },
    ] {
        assert_eq!(Prefs::parse(&p.text()), p);
    }
}

#[test]
fn the_settings_file_round_trips_and_a_large_one_is_not_read() {
    let d = std::env::temp_dir().join(format!("agora-tui-alerts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let file = d.join("sub").join(FILE);
    assert_eq!(load(&file), Prefs::default());
    let p = Prefs {
        sound: false,
        desktop: Desktop::Osc777,
    };
    save(&file, &p).unwrap();
    assert_eq!(load(&file), p);
    let mut big = "sound = off\n".to_string();
    big.push_str(&"#".repeat(usize::try_from(MAX_FILE_BYTES).unwrap()));
    std::fs::write(&file, big).unwrap();
    assert_eq!(load(&file), Prefs::default());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_agent_name_cannot_break_out_of_the_notification() {
    let hostile = "evil\x07\x1b]0;pwned\x1b\\\u{9c}\u{202e}gnp.exe\u{200b};rm -rf\u{2066}\nx";
    let s = sanitize(hostile, NAME_MAX);
    for bad in [
        '\x07', '\x1b', '\u{9c}', '\u{202e}', '\u{200b}', '\u{2066}', ';', '\n',
    ] {
        assert!(!s.contains(bad), "{bad:?} in {s:?}");
    }
    assert_eq!(s, "evil]0pwned\\gnp.exerm -rf x");
    // Cut to the cap, with an ellipsis.
    let long = sanitize(&"a".repeat(100), NAME_MAX);
    assert_eq!(long.chars().count(), NAME_MAX);
    assert!(long.ends_with('…'));
    // The fixed phrase, with the name made safe and cut.
    let (title, body) = message(Kind::Finished, hostile, &t);
    assert_eq!(title, "tui-alert-title");
    assert_eq!(body, format!("tui-alert-finished[agent={s}]"));
    assert_eq!(
        message(Kind::NeedsYou, "\x1b\x07", &t).1,
        "tui-alert-needs-you-unnamed"
    );
}

#[test]
fn the_bytes_of_an_alert() {
    let bell = Prefs::default();
    let osc9 = Prefs {
        sound: false,
        desktop: Desktop::Osc9,
    };
    let osc777 = Prefs {
        sound: true,
        desktop: Desktop::Osc777,
    };
    assert_eq!(alert_bytes(&bell, true, "Agora", "x finished"), b"\x07");
    assert_eq!(
        alert_bytes(&osc9, true, "Agora", "x finished"),
        b"\x1b]9;x finished\x1b\\"
    );
    assert_eq!(
        alert_bytes(&osc777, true, "Agora", "x finished"),
        b"\x07\x1b]777;notify;Agora;x finished\x1b\\"
    );
    // Not a terminal: nothing at all, whatever is on.
    for p in [bell, osc9, osc777] {
        assert!(alert_bytes(&p, false, "Agora", "x finished").is_empty());
    }
    // Whatever the caller passed, the payload holds no control character and no separator of its own.
    let out = alert_bytes(&osc777, true, "A;\x1b]9;t\x07", "b\x1b\\;\x07c\u{202e}");
    assert_eq!(out, b"\x07\x1b]777;notify;A]9t;b\\c\x1b\\");
}
