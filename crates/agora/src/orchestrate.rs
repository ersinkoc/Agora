//! Scripted orchestration from a shell: `agora agent prompt AGENT_ID TEXT --wait`, `agora agent wait AGENT_ID
//! --until STATE,...` and `agora events`.
//!
//! The node's control socket has no event stream of its own (only terminal, chat and preview streams), so these commands read the
//! one thing it already answers with every agent's state: `AgentList` (no new protocol message). They ask for it every [`POLL`]; a
//! state the agent held for less than that may not be seen, and `agora events` lines carry no sequence numbers. Exit codes (also in
//! `docs/getting-started.md`, "Script agents from a shell"): 0 idle or done (or the `--until` state reached, working included), [`EXIT_BLOCKED`] the agent
//! waits for the person, [`EXIT_EXITED`] its process ended (stopped, crashed, restarted, interrupted or no longer listed),
//! [`EXIT_TIMEOUT`] `--timeout` ran out, [`EXIT_REFUSED`] the prompt was not sent because the agent waits for the person; 1 an error
//! (no node, unknown agent, a refusal from the node), 2 a usage error.

use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_i18n::key;
use agora_node::control::Client;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use agora_protocol::ErrorCode;

use crate::{Opts, Say};

/// How often the agents are asked for. The node's own tick is 100 ms; each `AgentList` takes its supervisor's lock briefly.
pub(crate) const POLL: Duration = Duration::from_millis(200);
/// After a prompt, how long an unchanged state (still `idle`, as before the prompt) is taken for "the turn may not have shown yet"
/// rather than "the turn is over", and how long a queued prompt's agent must stay out of `working` before its turn counts as over.
pub(crate) const SETTLE: Duration = Duration::from_secs(5);

pub(crate) const EXIT_BLOCKED: u8 = 3;
pub(crate) const EXIT_EXITED: u8 = 4;
pub(crate) const EXIT_TIMEOUT: u8 = 5;
pub(crate) const EXIT_REFUSED: u8 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum State {
    Working,
    Blocked,
    Idle,
    Done,
    Unknown,
}

impl State {
    fn from_proto(s: i32) -> Self {
        match v1::AgentState::try_from(s) {
            Ok(v1::AgentState::Working) => Self::Working,
            Ok(v1::AgentState::Blocked) => Self::Blocked,
            Ok(v1::AgentState::Idle) => Self::Idle,
            Ok(v1::AgentState::Done) => Self::Done,
            _ => Self::Unknown,
        }
    }

    fn proto(self) -> i32 {
        (match self {
            Self::Working => v1::AgentState::Working,
            Self::Blocked => v1::AgentState::Blocked,
            Self::Idle => v1::AgentState::Idle,
            Self::Done => v1::AgentState::Done,
            Self::Unknown => v1::AgentState::Unspecified,
        }) as i32
    }

    /// The fixed token scripts read (`agora agent list` prints the same).
    fn word(self) -> &'static str {
        crate::state_word(self.proto())
    }

    /// The state in the person's language (the words the window uses).
    fn text_key(self) -> &'static str {
        match self {
            Self::Working => key!("agent-state-working"),
            Self::Blocked => key!("agent-state-blocked"),
            Self::Idle => key!("agent-state-idle"),
            Self::Done => key!("agent-state-done"),
            Self::Unknown => key!("agent-state-unknown"),
        }
    }
}

/// One agent as one `AgentList` answer shows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Row {
    id: String,
    kind: String,
    state: State,
    running: bool,
    restarts: u32,
    /// ADR-0107: listed as interrupted (its node restarted while it ran); its `state` is its last one.
    interrupted: bool,
}

/// The rows of one answer, in id order (`agent-2` before `agent-10`), the node's agents and the ones it found interrupted together.
fn rows(list: &v1::AgentListResult) -> Vec<Row> {
    let mut rows: Vec<Row> = list
        .agents
        .iter()
        .map(|a| Row {
            id: a.agent_id.clone(),
            kind: a.kind.clone(),
            state: State::from_proto(a.state),
            running: a.running,
            restarts: a.restarts,
            interrupted: false,
        })
        .chain(list.interrupted.iter().map(|i| Row {
            id: i.agent_id.clone(),
            kind: i.kind.clone(),
            state: State::from_proto(i.last_state),
            running: false,
            restarts: 0,
            interrupted: true,
        }))
        .collect();
    rows.sort_by_key(|r| {
        r.id.strip_prefix("agent-")
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });
    rows
}

/// What a wait saw of its agent at one look.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Seen {
    Live(State),
    /// Its process ended (stopped, crashed, restarted since the wait began, interrupted) or it is no longer listed; the last state
    /// known.
    Exited(State),
}

fn seen(rows: &[Row], id: &str, restarts: u32, last: State) -> Seen {
    match rows.iter().find(|r| r.id == id) {
        Some(r) if r.running && !r.interrupted && r.restarts == restarts => Seen::Live(r.state),
        Some(r) => Seen::Exited(r.state),
        None => Seen::Exited(last),
    }
}

/// How a wait ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Outcome {
    State(State),
    Exited(State),
}

/// What a wait waits for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Target {
    /// `agent prompt --wait`: the turn the prompt started is over. `before` is the state the prompt found; `queued`, the node said
    /// the prompt waits behind a running turn.
    Turn { before: State, queued: bool },
    /// `agent wait --until`: one of these states. An agent whose process ends ends every wait.
    States(Vec<State>),
}

/// The wait's rule, apart from the socket and the clock so it can be tested.
pub(crate) struct Waiter {
    target: Target,
    since: Instant,
    seen_work: bool,
    /// Since when the agent has been out of `working` (and of `unknown`).
    quiet: Option<Instant>,
}

impl Waiter {
    pub(crate) fn new(target: Target, now: Instant) -> Self {
        Self {
            target,
            since: now,
            seen_work: false,
            quiet: None,
        }
    }

    /// One look at the agent; `Some` once the wait is over.
    ///
    /// For a prompt: `working` (or `unknown`, a screen the agent's description does not recognise) is never the end. `idle`, `done`
    /// or `blocked` is, once the turn was seen working, or when it differs from the state the prompt found, or (an agent whose turn
    /// was too quick to see, or that has not shown it yet) after [`SETTLE`]. A prompt sent while the agent was working (queued by
    /// the node, or typed into a running turn) has another turn before it, whose end is not its own: there `idle` and `done` count
    /// only once they lasted [`SETTLE`]; `blocked` needs the person whichever turn it is.
    pub(crate) fn step(&mut self, seen: Seen, now: Instant) -> Option<Outcome> {
        let s = match seen {
            Seen::Exited(s) => return Some(Outcome::Exited(s)),
            Seen::Live(s) => s,
        };
        match &self.target {
            Target::States(want) => want.contains(&s).then_some(Outcome::State(s)),
            Target::Turn { before, queued } => {
                if matches!(s, State::Working | State::Unknown) {
                    self.seen_work |= s == State::Working;
                    self.quiet = None;
                    return None;
                }
                let quiet = *self.quiet.get_or_insert(now);
                let settled = now.saturating_duration_since(quiet) >= SETTLE;
                let behind = *queued || *before == State::Working;
                let over = if behind {
                    settled || (s == State::Blocked && s != *before)
                } else {
                    self.seen_work
                        || s != *before
                        || now.saturating_duration_since(self.since) >= SETTLE
                };
                over.then_some(Outcome::State(s))
            }
        }
    }
}

/// `--until working,blocked,...`; `None` for a word that is not a state. Without `--until`: until the agent leaves `working`.
fn until(spec: Option<&str>) -> Option<Vec<State>> {
    let Some(spec) = spec else {
        return Some(vec![State::Idle, State::Done, State::Blocked]);
    };
    let mut want = Vec::new();
    for word in spec.split(',').map(str::trim) {
        match word {
            "working" => want.push(State::Working),
            "blocked" => want.push(State::Blocked),
            "idle" => want.push(State::Idle),
            "done" => want.push(State::Done),
            // Every wait ends when the agent's process does; naming it alone waits for nothing else.
            "exited" => {}
            _ => return None,
        }
    }
    Some(want)
}

/// The agents, or the exit code after saying why not.
fn agents(say: &Say, c: &mut Client, o: &Opts) -> Result<Vec<Row>, ExitCode> {
    match c.call(Cmd::AgentList(v1::AgentList::default())) {
        Ok(Ok(v1::CommandOk {
            result: Some(Ok_::AgentList(list)),
        })) => Ok(rows(&list)),
        Ok(Ok(_)) => Err(ExitCode::from(1)),
        Ok(Err(e)) if e.code == ErrorCode::AuthUnauthenticated.as_str() => {
            say.err(key!("cli-node-needs-token"), &[]);
            Err(ExitCode::from(1))
        }
        Ok(Err(e)) => {
            say.remote_error(&e);
            Err(ExitCode::from(1))
        }
        // The node went away (stopped mid-wait) or never answered.
        Err(e) => {
            crate::say_unreachable(say, &crate::socket_path(o), &e);
            Err(ExitCode::from(1))
        }
    }
}

fn not_found(say: &Say, id: &str) -> ExitCode {
    say.remote_error(
        &agora_protocol::Error::new(ErrorCode::AgentNotFound)
            .with("agent_id", id)
            .to_info(),
    );
    ExitCode::from(1)
}

/// One line on stdout; a reader that went away (`| head`) is not an error and never a panic. `false` when it could not be written.
fn line(text: &str) -> bool {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}").and_then(|()| out.flush()).is_ok()
}

/// `agora agent prompt AGENT_ID TEXT... [--wait [--timeout SECS]] [--force]`. A prompt to an agent that waits for the person
/// (`blocked`) is refused with `agent.blocked` unless `--force` (typing into a terminal agent's question is what `--force` is for).
/// The check is the node's agent list just before the prompt is sent, not one step with it.
pub(crate) fn agent_prompt(say: &Say, o: &Opts) -> ExitCode {
    let (Some(id), text) = (
        o.positional.first(),
        o.positional[1.min(o.positional.len())..].join(" "),
    ) else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    if text.is_empty() || (o.timeout.is_some() && !o.wait) {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    }
    let mut c = match crate::connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    // An agent the list does not show is left to the node, which says `agent.not_found` as it always did.
    let before = if o.force && !o.wait {
        None
    } else {
        match agents(say, &mut c, o) {
            Ok(rows) => rows.into_iter().find(|r| &r.id == id),
            Err(code) => return code,
        }
    };
    if !o.force
        && before
            .as_ref()
            .is_some_and(|r| r.running && r.state == State::Blocked)
    {
        say.remote_error(
            &agora_protocol::Error::new(ErrorCode::AgentBlocked)
                .with("agent_id", id.as_str())
                .to_info(),
        );
        say.err(key!("cli-agent-prompt-blocked-hint"), &[]);
        return ExitCode::from(EXIT_REFUSED);
    }
    let sent = crate::call(
        say,
        &mut c,
        Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: id.clone(),
            text,
        }),
    );
    let queued = match sent {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentPrompt(r)),
        }) => r.queued,
        Ok(_) => false,
        Err(code) => return code,
    };
    say.out(
        key!("cli-agent-prompted"),
        &[("agent_id", id.as_str().into())],
    );
    if !o.wait {
        return ExitCode::SUCCESS;
    }
    let (state, restarts) = before
        .as_ref()
        .map_or((State::Idle, 0), |r| (r.state, r.restarts));
    let target = Target::Turn {
        before: state,
        queued,
    };
    wait_on(say, &mut c, o, id, target, state, restarts)
}

/// `agora agent wait AGENT_ID [--until STATE[,STATE...]] [--timeout SECS]`: returns at once when the agent is in one of the states.
pub(crate) fn agent_wait(say: &Say, o: &Opts) -> ExitCode {
    let Some(id) = o.positional.first().filter(|_| o.positional.len() == 1) else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let Some(want) = until(o.until.as_deref()) else {
        say.err(key!("cli-agent-wait-bad-state"), &[]);
        return ExitCode::from(2);
    };
    let mut c = match crate::connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let first = match agents(say, &mut c, o) {
        Ok(rows) => rows.into_iter().find(|r| &r.id == id),
        Err(code) => return code,
    };
    let Some(first) = first else {
        return not_found(say, id);
    };
    wait_on(
        say,
        &mut c,
        o,
        id,
        Target::States(want),
        first.state,
        first.restarts,
    )
}

/// Looks at agent `id` every [`POLL`] until the wait is over or `--timeout` runs out, then prints `ID<TAB>STATE` (`exited` for an
/// agent whose process ended) on stdout, says why in words on stderr when it is not a plain end, and returns the exit code.
fn wait_on(
    say: &Say,
    c: &mut Client,
    o: &Opts,
    id: &str,
    target: Target,
    mut last: State,
    restarts: u32,
) -> ExitCode {
    let start = Instant::now();
    // A deadline past what the clock can hold is no deadline.
    let deadline = o
        .timeout
        .and_then(|s| start.checked_add(Duration::from_secs(s)));
    let mut waiter = Waiter::new(target, start);
    loop {
        let rows = match agents(say, c, o) {
            Ok(rows) => rows,
            Err(code) => return code,
        };
        let now = Instant::now();
        let look = seen(&rows, id, restarts, last);
        last = match look {
            Seen::Live(s) | Seen::Exited(s) => s,
        };
        if let Some(outcome) = waiter.step(look, now) {
            return finish(say, id, outcome);
        }
        if let Some(deadline) = deadline {
            if now >= deadline {
                line(&format!("{id}\t{}", last.word()));
                say.err(
                    key!("cli-agent-wait-timeout"),
                    &[
                        ("agent_id", id.into()),
                        ("state", say.text(last.text_key(), &[]).into()),
                        (
                            "seconds",
                            i64::try_from(o.timeout.unwrap_or(0))
                                .unwrap_or(i64::MAX)
                                .into(),
                        ),
                    ],
                );
                return ExitCode::from(EXIT_TIMEOUT);
            }
            std::thread::sleep(POLL.min(deadline.saturating_duration_since(now)));
        } else {
            std::thread::sleep(POLL);
        }
    }
}

fn finish(say: &Say, id: &str, outcome: Outcome) -> ExitCode {
    match outcome {
        Outcome::State(s) => {
            line(&format!("{id}\t{}", s.word()));
            if s == State::Blocked {
                say.err(key!("cli-agent-wait-blocked"), &[("agent_id", id.into())]);
                ExitCode::from(EXIT_BLOCKED)
            } else {
                ExitCode::SUCCESS
            }
        }
        Outcome::Exited(last) => {
            line(&format!("{id}\texited"));
            say.err(
                key!("cli-agent-wait-exited"),
                &[
                    ("agent_id", id.into()),
                    ("state", say.text(last.text_key(), &[]).into()),
                ],
            );
            ExitCode::from(EXIT_EXITED)
        }
    }
}

/// What changed for one agent between two looks ([`changes`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum What {
    /// There when `agora events` began.
    Present,
    /// Appeared since (started, or resumed under a new listing).
    Added,
    /// Its state changed (`previous` says from what).
    State,
    /// Its process ended.
    Exited,
    /// Running again (resumed, or started again).
    Running,
    /// Restarted by its node between two looks.
    Restarted,
    /// Listed as interrupted (ADR-0107).
    Interrupted,
    /// No longer listed.
    Removed,
}

impl What {
    fn word(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Added => "added",
            Self::State => "state",
            Self::Exited => "exited",
            Self::Running => "running",
            Self::Restarted => "restarted",
            Self::Interrupted => "interrupted",
            Self::Removed => "removed",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Change {
    what: What,
    row: Row,
    previous: Option<State>,
}

/// The changes from `old` (`None`: the first look) to `new`, in `new`'s order and then the agents no longer listed.
pub(crate) fn changes(old: Option<&[Row]>, new: &[Row]) -> Vec<Change> {
    let one = |what, row: &Row, previous| Change {
        what,
        row: row.clone(),
        previous,
    };
    let Some(old) = old else {
        return new.iter().map(|r| one(What::Present, r, None)).collect();
    };
    let mut out = Vec::new();
    for r in new {
        let Some(was) = old.iter().find(|o| o.id == r.id) else {
            out.push(one(What::Added, r, None));
            continue;
        };
        if r.interrupted && !was.interrupted {
            out.push(one(What::Interrupted, r, None));
        } else if was.running && !r.running {
            out.push(one(What::Exited, r, None));
        } else if !was.running && r.running {
            out.push(one(What::Running, r, None));
        } else if r.running && r.restarts > was.restarts {
            out.push(one(What::Restarted, r, None));
        }
        if r.state != was.state {
            out.push(one(What::State, r, Some(was.state)));
        }
    }
    for was in old {
        if !new.iter().any(|r| r.id == was.id) {
            out.push(one(What::Removed, was, None));
        }
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// One `--json` line: fixed tokens only.
fn json_line(c: &Change, time_ms: u64) -> String {
    let mut v = serde_json::json!({
        "event": c.what.word(),
        "agent_id": c.row.id,
        "kind": c.row.kind,
        "state": c.row.state.word(),
        "running": c.row.running,
        "restarts": c.row.restarts,
        "time_ms": time_ms,
    });
    if let Some(p) = c.previous {
        v["previous"] = p.word().into();
    }
    v.to_string()
}

fn text_line(say: &Say, c: &Change) -> String {
    let state = say.text(c.row.state.text_key(), &[]);
    let args = [
        ("agent_id", c.row.id.as_str().into()),
        ("kind", c.row.kind.as_str().into()),
        ("state", state.into()),
        (
            "previous",
            c.previous
                .map_or(String::new(), |p| say.text(p.text_key(), &[]))
                .into(),
        ),
        ("restarts", i64::from(c.row.restarts).into()),
    ];
    let id = match c.what {
        What::Present => key!("cli-event-present"),
        What::Added => key!("cli-event-added"),
        What::State => key!("cli-event-state"),
        What::Exited => key!("cli-event-exited"),
        What::Running => key!("cli-event-running"),
        What::Restarted => key!("cli-event-restarted"),
        What::Interrupted => key!("cli-event-interrupted"),
        What::Removed => key!("cli-event-removed"),
    };
    say.text(id, &args)
}

/// `agora events [--agent ID] [--json]`: a line for every agent on this node first (`present`), then one for each change, until
/// interrupted, the node goes away (exit 1) or the reader of stdout does (exit 0, noticed at the next line). Only the last answer is
/// kept, so memory does not grow with time.
pub(crate) fn events(say: &Say, o: &Opts) -> ExitCode {
    if !o.positional.is_empty() {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    }
    let mut c = match crate::connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let mut last: Option<Vec<Row>> = None;
    loop {
        let now = match agents(say, &mut c, o) {
            Ok(rows) => rows,
            Err(code) => return code,
        };
        let time_ms = now_ms();
        for change in changes(last.as_deref(), &now) {
            if o.agent.as_ref().is_some_and(|a| *a != change.row.id) {
                continue;
            }
            let text = if o.json {
                json_line(&change, time_ms)
            } else {
                text_line(say, &change)
            };
            if !line(&text) {
                return ExitCode::SUCCESS;
            }
        }
        last = Some(now);
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn row(id: &str, state: State, running: bool) -> Row {
        Row {
            id: id.into(),
            kind: "k".into(),
            state,
            running,
            restarts: 0,
            interrupted: false,
        }
    }

    fn turn(before: State, queued: bool) -> (Waiter, Instant) {
        let t0 = Instant::now();
        (Waiter::new(Target::Turn { before, queued }, t0), t0)
    }

    #[test]
    fn a_prompt_wait_ends_when_the_turn_seen_working_is_over() {
        let (mut w, t0) = turn(State::Idle, false);
        let ms = |n| t0 + Duration::from_millis(n);
        // The state before the turn shows: not yet the end.
        assert_eq!(w.step(Seen::Live(State::Idle), ms(10)), None);
        assert_eq!(w.step(Seen::Live(State::Working), ms(200)), None);
        assert_eq!(
            w.step(Seen::Live(State::Idle), ms(400)),
            Some(Outcome::State(State::Idle))
        );
    }

    #[test]
    fn a_prompt_wait_takes_an_unchanged_idle_only_after_it_settled() {
        let (mut w, t0) = turn(State::Idle, false);
        assert_eq!(w.step(Seen::Live(State::Idle), t0), None);
        assert_eq!(
            w.step(
                Seen::Live(State::Idle),
                t0 + SETTLE - Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(
            w.step(Seen::Live(State::Idle), t0 + SETTLE),
            Some(Outcome::State(State::Idle))
        );
    }

    #[test]
    fn a_prompt_wait_ends_at_once_on_a_new_state_or_an_ended_process() {
        let (mut w, t0) = turn(State::Idle, false);
        assert_eq!(
            w.step(Seen::Live(State::Blocked), t0),
            Some(Outcome::State(State::Blocked))
        );
        let (mut w, t0) = turn(State::Idle, false);
        assert_eq!(
            w.step(Seen::Live(State::Done), t0),
            Some(Outcome::State(State::Done))
        );
        let (mut w, t0) = turn(State::Idle, false);
        assert_eq!(
            w.step(Seen::Exited(State::Working), t0),
            Some(Outcome::Exited(State::Working))
        );
    }

    #[test]
    fn a_forced_prompt_into_a_blocked_agent_waits_for_what_follows() {
        let (mut w, t0) = turn(State::Blocked, false);
        assert_eq!(w.step(Seen::Live(State::Blocked), t0), None);
        assert_eq!(
            w.step(Seen::Live(State::Working), t0 + Duration::from_millis(300)),
            None
        );
        assert_eq!(
            w.step(Seen::Live(State::Idle), t0 + Duration::from_millis(600)),
            Some(Outcome::State(State::Idle))
        );
    }

    #[test]
    fn unknown_and_working_never_end_a_prompt_wait() {
        let (mut w, t0) = turn(State::Unknown, false);
        for n in 0..100 {
            let at = t0 + SETTLE * n;
            assert_eq!(w.step(Seen::Live(State::Unknown), at), None);
            assert_eq!(w.step(Seen::Live(State::Working), at), None);
        }
    }

    #[test]
    fn a_queued_prompt_counts_an_idle_only_once_it_lasted() {
        let (mut w, t0) = turn(State::Working, true);
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(w.step(Seen::Live(State::Working), ms(0)), None);
        // Between the turn ahead and the queued one.
        assert_eq!(w.step(Seen::Live(State::Idle), ms(200)), None);
        assert_eq!(w.step(Seen::Live(State::Working), ms(400)), None);
        let quiet = ms(600);
        assert_eq!(w.step(Seen::Live(State::Idle), quiet), None);
        assert_eq!(
            w.step(Seen::Live(State::Idle), quiet + SETTLE),
            Some(Outcome::State(State::Idle))
        );
        let (mut w, t0) = turn(State::Working, true);
        assert_eq!(
            w.step(Seen::Live(State::Blocked), t0),
            Some(Outcome::State(State::Blocked))
        );
    }

    #[test]
    fn a_prompt_typed_into_a_running_turn_does_not_end_at_that_turns_idle() {
        // A terminal agent, or an adapter that does not report turns: the node does not queue, the agent was working.
        let (mut w, t0) = turn(State::Working, false);
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(w.step(Seen::Live(State::Working), ms(0)), None);
        assert_eq!(w.step(Seen::Live(State::Idle), ms(200)), None);
        assert_eq!(w.step(Seen::Live(State::Working), ms(400)), None);
        assert_eq!(w.step(Seen::Live(State::Idle), ms(600)), None);
        assert_eq!(
            w.step(Seen::Live(State::Idle), ms(600) + SETTLE),
            Some(Outcome::State(State::Idle))
        );
    }

    #[test]
    fn until_waits_for_the_named_states_and_an_ended_process() {
        let t0 = Instant::now();
        let mut w = Waiter::new(Target::States(until(Some("blocked")).unwrap()), t0);
        assert_eq!(w.step(Seen::Live(State::Working), t0), None);
        assert_eq!(w.step(Seen::Live(State::Idle), t0), None);
        assert_eq!(
            w.step(Seen::Live(State::Blocked), t0),
            Some(Outcome::State(State::Blocked))
        );
        let mut w = Waiter::new(Target::States(until(Some("exited")).unwrap()), t0);
        assert_eq!(w.step(Seen::Live(State::Idle), t0), None);
        assert_eq!(
            w.step(Seen::Exited(State::Idle), t0),
            Some(Outcome::Exited(State::Idle))
        );
    }

    #[test]
    fn until_reads_states_and_refuses_other_words() {
        assert_eq!(
            until(Some("idle, done,working")),
            Some(vec![State::Idle, State::Done, State::Working])
        );
        assert_eq!(
            until(None),
            Some(vec![State::Idle, State::Done, State::Blocked])
        );
        assert_eq!(until(Some("idle,asleep")), None);
        assert_eq!(until(Some("")), None);
        assert_eq!(until(Some("unknown")), None);
    }

    #[test]
    fn an_agent_that_stopped_restarted_or_left_the_list_has_exited() {
        let rows = vec![
            row("agent-1", State::Idle, true),
            row("agent-2", State::Done, false),
        ];
        assert_eq!(
            seen(&rows, "agent-1", 0, State::Idle),
            Seen::Live(State::Idle)
        );
        assert_eq!(
            seen(&rows, "agent-2", 0, State::Idle),
            Seen::Exited(State::Done)
        );
        assert_eq!(
            seen(&rows, "agent-1", 1, State::Idle),
            Seen::Exited(State::Idle)
        );
        assert_eq!(
            seen(&rows, "agent-3", 0, State::Working),
            Seen::Exited(State::Working)
        );
        let mut interrupted = row("agent-1", State::Working, false);
        interrupted.interrupted = true;
        assert_eq!(
            seen(&[interrupted], "agent-1", 0, State::Idle),
            Seen::Exited(State::Working)
        );
    }

    #[test]
    fn changes_name_what_changed_once() {
        let a = row("agent-1", State::Idle, true);
        let b = row("agent-2", State::Working, true);
        let first = changes(None, &[a.clone(), b.clone()]);
        assert_eq!(
            first.iter().map(|c| c.what).collect::<Vec<_>>(),
            [What::Present, What::Present]
        );
        // Nothing changed: nothing said.
        assert!(changes(Some(&[a.clone(), b.clone()]), &[a.clone(), b.clone()]).is_empty());

        let a2 = row("agent-1", State::Working, true);
        let mut b2 = row("agent-2", State::Idle, false);
        let c = row("agent-3", State::Idle, true);
        let got = changes(Some(&[a.clone(), b.clone()]), &[a2, b2.clone(), c]);
        let got: Vec<(What, &str, Option<State>)> = got
            .iter()
            .map(|c| (c.what, c.row.id.as_str(), c.previous))
            .collect();
        assert_eq!(
            got,
            [
                (What::State, "agent-1", Some(State::Idle)),
                (What::Exited, "agent-2", None),
                (What::State, "agent-2", Some(State::Working)),
                (What::Added, "agent-3", None),
            ]
        );

        b2.running = true;
        b2.restarts = 1;
        let restarted = changes(Some(std::slice::from_ref(&b)), std::slice::from_ref(&b2));
        assert_eq!(restarted[0].what, What::Restarted);
        let gone = changes(Some(&[a, b2]), &[]);
        assert_eq!(
            gone.iter().map(|c| c.what).collect::<Vec<_>>(),
            [What::Removed, What::Removed]
        );
    }

    #[test]
    fn a_json_line_has_fixed_tokens() {
        let c = Change {
            what: What::State,
            row: row("agent-1", State::Blocked, true),
            previous: Some(State::Working),
        };
        let v: serde_json::Value = serde_json::from_str(&json_line(&c, 7)).unwrap();
        assert_eq!(v["event"], "state");
        assert_eq!(v["agent_id"], "agent-1");
        assert_eq!(v["state"], "blocked");
        assert_eq!(v["previous"], "working");
        assert_eq!(v["running"], true);
        assert_eq!(v["time_ms"], 7);
    }
}
