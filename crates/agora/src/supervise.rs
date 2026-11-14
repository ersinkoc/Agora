//! `agora supervise hub|node`: the supervisor the front door starts detached instead of the role itself (ADR-0062 item 8).
//!
//! It runs the role's own command line (the one `agora` ran before, from `machine.toml`) as a child bound to it
//! (`agora_platform::spawn_bound`), and starts it again after a crash by the rule of `agora_core::restart`: never after exit code 0,
//! never after an end soon after the start (a refusal), once per 30 s at most, giving up after 5 restarts in 10 minutes. Every
//! decision is a line in the role's log and in the owner-only `run/<role>.restarts`, which `agora status` reads.
//!
//! The supervisor holds the role's lock (`run/<role>.lock`), so two never run on one Agora folder: the front door takes the lock,
//! starts the supervisor and writes the supervisor's process id in it; a supervisor started any other way takes the lock itself.
//! `agora down` stops the supervisor and the child goes with it (Windows: the job ends it; Unix: the stop request is passed on, and a
//! child whose supervisor is killed ends by `agora_platform::end_with_supervisor`).
//!
//! What happens to agents is unchanged: a node child that crashes leaves its agents as any crashed node does (Unix: they keep
//! running in their own sessions and the restarted node finds them by the survivor check of ADR-0060; Windows: each agent is in a
//! job of the node that ends with it), and the restarted node follows the ledger and its resume setting (ADR-0060, default "ask").

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_core::restart::{Ending, Next, RestartRule, Restarts};
use agora_i18n::{key, Arg};

use crate::frontdoor::{self, Claim, LockInfo, Role};
use crate::{home, Say};

/// How long a supervisor waits for the front door that started it to write its process id in the lock.
const HANDOFF: Duration = Duration::from_secs(5);

/// The lines kept in `run/<role>.restarts`.
const RECORD_LINES: usize = 20;

/// How a child ended: an exit code, or (Unix) the signal that killed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct End {
    pub code: i32,
    /// 0 when it exited with `code`.
    pub signal: i32,
}

impl End {
    fn of(status: &std::process::ExitStatus) -> End {
        End {
            code: status.code().unwrap_or(-1),
            signal: agora_platform::exit_signal(status).unwrap_or(0),
        }
    }

    /// The child's end as the supervisor's own exit code: never 0 (a signal or an out-of-range code becomes 1).
    fn failure(self) -> i32 {
        if self.signal == 0 && (1..=255).contains(&self.code) {
            self.code
        } else {
            1
        }
    }
}

/// What the supervisor did, for its log and its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// The child could not be tied to the supervisor, so it is not started (it could outlive the supervisor).
    NotBound,
    /// A stop request reached the supervisor; the child was stopped with it.
    StopRequest,
    /// The child exited with code 0.
    Stopped,
    /// The child ended too soon after its start to be a crash.
    Refused { end: End, up_ms: u64 },
    /// The child crashed and starts again after `wait_ms`.
    Restarting {
        end: End,
        up_ms: u64,
        wait_ms: u64,
        count: usize,
    },
    /// The child crashed once more after the allowed restarts.
    GaveUp { end: End },
    /// Before a start: the role already answers here (started by hand), so starting another would fight over its socket or port.
    AlreadyRunning,
    /// Before a start: the lock no longer names this supervisor (`agora down` ran, or another took over).
    LostLock,
}

/// Runs children made by `start` by the rule until one stops, refuses, or the supervisor gives up or is asked to stop. `blocked` is
/// asked before every start; an event from it ends the supervisor. Returns the exit code for the supervisor.
pub(crate) fn run(
    rule: &RestartRule,
    mut start: impl FnMut() -> std::io::Result<agora_platform::Bound>,
    mut blocked: impl FnMut() -> Option<Event>,
    mut note: impl FnMut(&Event),
) -> std::io::Result<i32> {
    let clock = Instant::now();
    let ms = |at: Instant| u64::try_from(at.duration_since(clock).as_millis()).unwrap_or(u64::MAX);
    let mut restarts = Restarts::default();
    loop {
        if agora_platform::stop_requested() {
            note(&Event::StopRequest);
            return Ok(0);
        }
        if let Some(e) = blocked() {
            note(&e);
            return Ok(0);
        }
        let mut b = start()?;
        if !b.bound {
            // It could outlive this supervisor, out of reach of `agora down`: not left running (checked before starting anything
            // too, by `children_can_be_bound`).
            let _ = b.child.kill();
            let _ = b.child.wait();
            note(&Event::NotBound);
            return Ok(1);
        }
        let began = ms(Instant::now());
        agora_platform::forward_stop_to(Some(b.child.id()));
        // A request caught between the start and the line above was not passed on: pass it on now, as the handler would have.
        if agora_platform::stop_requested() {
            let _ = agora_platform::terminate_process(b.child.id());
        }
        // Wait without reaping, so the pid a stop request is passed on to stays this child's until the target is cleared.
        let ended = agora_platform::wait_unreaped(&b.child);
        agora_platform::forward_stop_to(None);
        ended?;
        let status = b.child.wait()?;
        if agora_platform::stop_requested() {
            note(&Event::StopRequest);
            return Ok(0);
        }
        let end = End::of(&status);
        let now = ms(Instant::now());
        let up_ms = now.saturating_sub(began);
        let ending = if status.success() {
            Ending::Clean
        } else {
            Ending::Failed
        };
        match restarts.after(rule, ending, up_ms, now) {
            Next::Stop => {
                note(&Event::Stopped);
                return Ok(0);
            }
            Next::Refused => {
                note(&Event::Refused { end, up_ms });
                return Ok(end.failure());
            }
            Next::GiveUp => {
                note(&Event::GaveUp { end });
                return Ok(end.failure());
            }
            Next::Restart { wait_ms, count } => {
                note(&Event::Restarting {
                    end,
                    up_ms,
                    wait_ms,
                    count,
                });
                let end = Instant::now() + Duration::from_millis(wait_ms);
                while Instant::now() < end && !agora_platform::stop_requested() {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

/// Makes this process the holder of the role's lock. Started by the front door (`handoff`: the front door's pid and start time), it
/// waits only for that process to write this one's id, and refuses when the lock is anything else (missing: `agora down` freed it
/// meanwhile, and the role must stay stopped). Started any other way, it takes a missing or stale lock and refuses a live holder at
/// once. True when this process holds it.
fn take_lock(lock: &Path, handoff: Option<(u32, u64)>) -> std::io::Result<bool> {
    let me = LockInfo::of(std::process::id())
        .ok_or_else(|| std::io::Error::other("own start time unknown"))?;
    let end = Instant::now() + HANDOFF;
    loop {
        let held = frontdoor::read_lock(lock);
        if held
            .as_ref()
            .is_some_and(|l| l.pid == me.pid && l.start == me.start)
        {
            return Ok(true);
        }
        match handoff {
            Some(from) => match held {
                Some(l) if (l.pid, l.start) == from && l.alive() && Instant::now() < end => {}
                _ => return Ok(false),
            },
            None => {
                return match held {
                    Some(l) if l.alive() => Ok(false),
                    _ => Ok(matches!(frontdoor::claim(lock)?, Claim::Mine)),
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `--handoff PID:START`.
fn parse_handoff(text: &str) -> Option<(u32, u64)> {
    let (pid, start) = text.split_once(':')?;
    Some((pid.parse().ok()?, start.parse().ok()?))
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `run/<role>.restarts`.
pub(crate) fn record_path(home: &Path, role: Role) -> PathBuf {
    home.join("run").join(format!("{}.restarts", role.name()))
}

/// One line of the record: when (Unix milliseconds), what, the exit code, how long the child had run, the restart number, the signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub at: u64,
    pub kind: String,
    pub end: End,
    pub up_ms: u64,
    pub count: usize,
}

impl Record {
    fn line(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.at, self.kind, self.end.code, self.up_ms, self.count, self.end.signal
        )
    }

    fn parse(line: &str) -> Option<Record> {
        let mut w = line.split_whitespace();
        let at = w.next()?.parse().ok()?;
        let kind = w.next()?.to_string();
        let code = w.next()?.parse().ok()?;
        let up_ms = w.next()?.parse().ok()?;
        let count = w.next()?.parse().ok()?;
        let signal = w.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        Some(Record {
            at,
            kind,
            end: End { code, signal },
            up_ms,
            count,
        })
    }

    fn of(e: &Event, at: u64) -> Option<Record> {
        let clean = End { code: 0, signal: 0 };
        let (kind, end, up_ms, count) = match e {
            Event::Restarting {
                end, up_ms, count, ..
            } => ("restart", *end, *up_ms, *count),
            Event::Refused { end, up_ms } => ("refused", *end, *up_ms, 0),
            Event::GaveUp { end } => ("gave-up", *end, 0, 0),
            Event::Stopped | Event::StopRequest => ("stopped", clean, 0, 0),
            _ => return None,
        };
        Some(Record {
            at,
            kind: kind.into(),
            end,
            up_ms,
            count,
        })
    }
}

/// The record of the role's supervisor, oldest first.
pub(crate) fn read_record(home: &Path, role: Role) -> Vec<Record> {
    std::fs::read_to_string(record_path(home, role))
        .map(|t| t.lines().filter_map(Record::parse).collect())
        .unwrap_or_default()
}

fn append_record(path: &Path, r: &Record) {
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default();
    lines.push(r.line());
    let keep = lines.len().saturating_sub(RECORD_LINES);
    let text = lines[keep..].join("\n") + "\n";
    let _ = agora_platform::write_private(path, text.as_bytes());
}

/// "exit code 3" or "signal 9", for the messages below.
fn end_text(say: &Say, end: End) -> String {
    if end.signal != 0 {
        say.text(
            key!("supervise-end-signal"),
            &[("signal", i64::from(end.signal).into())],
        )
    } else {
        say.text(
            key!("supervise-end-code"),
            &[("code", i64::from(end.code).into())],
        )
    }
}

/// The log line of an event.
fn text(say: &Say, role: Role, rule: &RestartRule, e: &Event) -> String {
    let r: Arg = role.label(say).into();
    let secs = |ms: u64| i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    let minutes = secs(rule.window_ms) / 60;
    let max = i64::try_from(rule.max_restarts).unwrap_or(i64::MAX);
    match e {
        Event::NotBound => say.text(key!("supervise-not-bound"), &[("role", r)]),
        Event::StopRequest => say.text(key!("supervise-stop-request"), &[("role", r)]),
        Event::Stopped => say.text(key!("supervise-stopped"), &[("role", r)]),
        Event::Refused { end, up_ms } => say.text(
            key!("supervise-refused"),
            &[
                ("role", r),
                ("end", end_text(say, *end).into()),
                ("seconds", secs(*up_ms).into()),
            ],
        ),
        Event::Restarting {
            end,
            up_ms,
            wait_ms,
            count,
        } => say.text(
            key!("supervise-restarting"),
            &[
                ("role", r),
                ("end", end_text(say, *end).into()),
                ("seconds", secs(*up_ms).into()),
                ("wait", secs(*wait_ms).into()),
                ("count", i64::try_from(*count).unwrap_or(i64::MAX).into()),
                ("max", max.into()),
                ("minutes", minutes.into()),
            ],
        ),
        Event::GaveUp { end } => say.text(
            key!("supervise-gave-up"),
            &[
                ("role", r),
                ("end", end_text(say, *end).into()),
                ("max", max.into()),
                ("minutes", minutes.into()),
            ],
        ),
        Event::AlreadyRunning => say.text(key!("supervise-already-running"), &[("role", r)]),
        Event::LostLock => say.text(key!("supervise-lost-lock"), &[("role", r)]),
    }
}

/// `agora supervise hub|node [--handoff PID:START]`.
/// How `agora supervise` was started.
pub struct Start<'a> {
    /// `--handoff PID:START`: the front door that started it.
    pub handoff: Option<&'a str>,
    /// `--home`: the Agora folder, instead of `AGORA_HOME` or `~/.agora`.
    pub home: Option<PathBuf>,
    /// `--log`: write its notes and the child's output to the role's log itself.
    pub log: bool,
}

pub fn supervise(say: &Say, word: &str, start: &Start) -> ExitCode {
    let handoff = start.handoff;
    let Some(role) = Role::from_name(word) else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let handoff = match handoff.map(parse_handoff) {
        None => None,
        Some(Some(h)) => Some(h),
        Some(None) => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    // A service passes the folder; a relative one would depend on where the service manager runs it from.
    if start.home.as_ref().is_some_and(|h| !h.is_absolute()) {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    let h = start.home.clone().unwrap_or_else(home);
    let Some(m) = frontdoor::load(&h) else {
        say.err(key!("frontdoor-not-set-up"), &[]);
        return ExitCode::from(1);
    };
    let fail = |e: &dyn std::fmt::Display| {
        say.err(
            key!("frontdoor-failed"),
            &[("reason", e.to_string().into())],
        );
        ExitCode::from(1)
    };
    // Started by a service manager rather than the front door, the folders may not exist yet.
    for d in ["run", "logs"] {
        if let Err(e) = std::fs::create_dir_all(h.join(d)) {
            return fail(&e);
        }
    }
    let lock = role.lock(&h);
    match take_lock(&lock, handoff) {
        Ok(true) => {}
        Ok(false) => {
            match frontdoor::read_lock(&lock).filter(LockInfo::alive) {
                Some(l) => say.err(
                    key!("supervise-lock-held"),
                    &[
                        ("role", role.label(say).into()),
                        ("pid", i64::from(l.pid).into()),
                    ],
                ),
                None => say.err(
                    key!("supervise-handoff-lost"),
                    &[("role", role.label(say).into())],
                ),
            }
            return ExitCode::from(1);
        }
        Err(e) => return fail(&e),
    }
    if let Err(e) = agora_platform::catch_stop_requests() {
        return fail(&e);
    }
    if !agora_platform::children_can_be_bound() {
        say.err(
            key!("supervise-not-bound"),
            &[("role", role.label(say).into())],
        );
        return ExitCode::from(1);
    }
    // The command line is made once: every restart runs exactly what the front door asked for.
    let mut cmd = match frontdoor::command(&h, &m, role) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    // Where the notes go: the role's log opened here (`--log`), or this process's error output, which the front door already points
    // at that log.
    let mut out: Box<dyn Write> = if start.log {
        let log = role.log(&h);
        frontdoor::rotate(&log);
        let file = match agora_platform::open_append_private(&log) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                say.err(
                    key!("supervise-log-link"),
                    &[("log", log.display().to_string().into())],
                );
                return ExitCode::from(1);
            }
            Err(e) => return fail(&e),
        };
        match (file.try_clone(), file.try_clone()) {
            (Ok(o), Ok(e)) => {
                cmd.stdout(o).stderr(e);
            }
            (Err(e), _) | (_, Err(e)) => return fail(&e),
        }
        Box::new(file)
    } else {
        Box::new(std::io::stderr())
    };
    let record = record_path(&h, role);
    // A new supervisor starts a new history.
    let _ = std::fs::remove_file(&record);
    let rule = RestartRule::default();
    let me = LockInfo::of(std::process::id());
    let code = run(
        &rule,
        || agora_platform::spawn_bound(&mut cmd),
        || {
            let mine = frontdoor::read_lock(&lock).is_some_and(|l| {
                me.as_ref()
                    .is_some_and(|me| l.pid == me.pid && l.start == me.start)
            });
            if !mine {
                Some(Event::LostLock)
            } else if frontdoor::role_answers(&h, &m, role) {
                Some(Event::AlreadyRunning)
            } else {
                None
            }
        },
        |e| {
            let _ = writeln!(out, "{}", text(say, role, &rule, e));
            if let Some(r) = Record::of(e, unix_ms()) {
                append_record(&record, &r);
            }
        },
    );
    match code {
        Ok(c) => ExitCode::from(u8::try_from(c).unwrap_or(1)),
        Err(e) => fail(&e),
    }
}

/// The crash after which the role's supervisor is waiting to start it again, when that is where it is (the last thing recorded is a
/// restart). Only meaningful while the supervisor is alive and the role does not answer.
pub(crate) fn waiting_restart(home: &Path, role: Role) -> Option<Record> {
    read_record(home, role)
        .pop()
        .filter(|r| r.kind == "restart")
}

/// The header `agora status` shows instead of "running" while the supervisor (`pid`) waits to start the role again.
pub(crate) fn restarting_line(say: &Say, role: Role, pid: u32, r: &Record) -> String {
    let rule = RestartRule::default();
    say.text(
        key!("status-restarting"),
        &[
            ("role", role.label(say).into()),
            ("pid", i64::from(pid).into()),
            ("end", end_text(say, r.end).into()),
            ("count", i64::try_from(r.count).unwrap_or(i64::MAX).into()),
            (
                "max",
                i64::try_from(rule.max_restarts).unwrap_or(i64::MAX).into(),
            ),
        ],
    )
}

/// The lines `agora status` adds under a role about its supervisor's restarts, if any.
pub(crate) fn status_lines(
    say: &Say,
    home: &Path,
    role: Role,
    supervisor_alive: bool,
    answers: bool,
) -> Vec<String> {
    let rule = RestartRule::default();
    let records = read_record(home, role);
    let Some(last) = records.last() else {
        return Vec::new();
    };
    let now = unix_ms();
    let minutes = |at: u64| i64::try_from(now.saturating_sub(at) / 60_000).unwrap_or(i64::MAX);
    let max = i64::try_from(rule.max_restarts).unwrap_or(i64::MAX);
    let recent = records
        .iter()
        .filter(|r| r.kind == "restart" && now.saturating_sub(r.at) < rule.window_ms)
        .count();
    let role_arg = || -> Arg { role.label(say).into() };
    match last.kind.as_str() {
        // The header says it (`restarting_line`).
        "restart" if supervisor_alive && !answers => Vec::new(),
        "restart" | "stopped" if recent > 0 => {
            let crash = records.iter().rev().find(|r| r.kind == "restart");
            crash.map_or_else(Vec::new, |c| {
                vec![say.text(
                    key!("status-restarts"),
                    &[
                        ("count", i64::try_from(recent).unwrap_or(i64::MAX).into()),
                        (
                            "window",
                            i64::try_from(rule.window_ms / 60_000)
                                .unwrap_or(i64::MAX)
                                .into(),
                        ),
                        ("end", end_text(say, c.end).into()),
                        ("minutes", minutes(c.at).into()),
                    ],
                )]
            })
        }
        "gave-up" if !supervisor_alive && !answers => vec![say.text(
            key!("status-gave-up"),
            &[
                ("role", role_arg()),
                ("end", end_text(say, last.end).into()),
                ("max", max.into()),
                ("minutes", minutes(last.at).into()),
            ],
        )],
        "refused" if !supervisor_alive && !answers => vec![say.text(
            key!("status-refused"),
            &[
                ("role", role_arg()),
                ("end", end_text(say, last.end).into()),
                ("minutes", minutes(last.at).into()),
            ],
        )],
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The child the tests start: this test binary again, running only `child_helper`, which exits after a while with a code.
    fn child(after_ms: u64, code: i32) -> std::process::Command {
        let mut c = std::process::Command::new(std::env::current_exe().unwrap());
        c.args([
            "--exact",
            "supervise::tests::child_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("AGORA_TEST_CHILD", format!("{after_ms} {code}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        c
    }

    /// Not a test: the body of the child above (a plain run of the suite passes through it).
    #[test]
    fn child_helper() {
        let Ok(spec) = std::env::var("AGORA_TEST_CHILD") else {
            return;
        };
        let mut w = spec.split_whitespace();
        let after: u64 = w.next().unwrap().parse().unwrap();
        let code: i32 = w.next().unwrap().parse().unwrap();
        std::thread::sleep(Duration::from_millis(after));
        std::process::exit(code);
    }

    fn fast_rule() -> RestartRule {
        RestartRule {
            wait_ms: 0,
            refusal_ms: 300,
            max_restarts: 2,
            window_ms: 60_000,
        }
    }

    fn supervise_child(after_ms: u64, code: i32) -> (i32, usize, Vec<Event>) {
        let mut starts = 0;
        let mut events = Vec::new();
        let rc = run(
            &fast_rule(),
            || {
                starts += 1;
                agora_platform::spawn_bound(&mut child(after_ms, code))
            },
            || None,
            |e| events.push(e.clone()),
        )
        .unwrap();
        (rc, starts, events)
    }

    #[test]
    fn a_crashing_child_is_restarted_until_the_supervisor_gives_up() {
        let (rc, starts, events) = supervise_child(600, 3);
        assert_eq!(rc, 3);
        assert_eq!(starts, 3, "{events:?}");
        let kinds: Vec<_> = events
            .iter()
            .filter(|e| !matches!(e, Event::NotBound))
            .map(|e| match e {
                Event::Restarting { end, count, .. } => format!("restart {} {count}", end.code),
                Event::GaveUp { end } => format!("gave up {}", end.code),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(kinds, ["restart 3 1", "restart 3 2", "gave up 3"]);
    }

    #[test]
    fn a_child_exiting_with_code_0_is_not_restarted() {
        let (rc, starts, events) = supervise_child(600, 0);
        assert_eq!((rc, starts), (0, 1));
        assert_eq!(events.last(), Some(&Event::Stopped));
    }

    #[test]
    fn a_child_failing_at_once_is_a_refusal_and_is_not_restarted() {
        let (rc, starts, events) = supervise_child(0, 4);
        assert_eq!((rc, starts), (4, 1));
        assert!(
            matches!(
                events.last(),
                Some(Event::Refused {
                    end: End { code: 4, signal: 0 },
                    ..
                })
            ),
            "{events:?}"
        );
    }

    #[test]
    fn nothing_starts_when_the_role_already_runs_or_the_lock_is_gone() {
        for blocker in [Event::AlreadyRunning, Event::LostLock] {
            let mut starts = 0;
            let mut events = Vec::new();
            let rc = run(
                &fast_rule(),
                || {
                    starts += 1;
                    agora_platform::spawn_bound(&mut child(0, 0))
                },
                || Some(blocker.clone()),
                |e| events.push(e.clone()),
            )
            .unwrap();
            assert_eq!((rc, starts), (0, 0));
            assert_eq!(events, [blocker]);
        }
    }

    #[test]
    fn a_failure_code_is_never_0() {
        let code = |code| End { code, signal: 0 }.failure();
        assert_eq!(code(3), 3);
        assert_eq!(code(255), 255);
        assert_eq!(code(-1), 1);
        assert_eq!(code(0), 1);
        assert_eq!(code(-1073741510), 1);
        assert_eq!(
            End {
                code: -1,
                signal: 9
            }
            .failure(),
            1
        );
    }

    #[test]
    fn a_handoff_names_a_pid_and_a_start_time() {
        assert_eq!(parse_handoff("12:345"), Some((12, 345)));
        assert_eq!(parse_handoff("12"), None);
        assert_eq!(parse_handoff("x:1"), None);
    }

    /// Linux's M1 on #543: a supervisor started by the front door must not take a lock that is gone (freed by `agora down`) or held by
    /// someone other than that front door; one started without a handoff takes a missing lock but never a live holder's.
    #[test]
    fn a_handed_off_supervisor_takes_only_its_own_lock() {
        let dir = std::env::temp_dir().join(format!("agora-handoff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("node.lock");
        let me = LockInfo::of(std::process::id()).unwrap();
        let from = (me.pid, me.start);
        // Missing (freed by `agora down` meanwhile): refused, and still missing.
        assert!(!take_lock(&lock, Some((1, 1))).unwrap());
        assert!(!lock.exists());
        // Held by another live process: refused at once, with a handoff naming someone else or without one.
        let mut other = child(20_000, 0).spawn().unwrap();
        let held = LockInfo::of(other.id()).unwrap();
        std::fs::write(&lock, held.text()).unwrap();
        let started = Instant::now();
        assert!(!take_lock(&lock, Some((1, 1))).unwrap());
        assert!(!take_lock(&lock, None).unwrap());
        assert!(started.elapsed() < HANDOFF);
        // Held by the front door handing over, which then ends without writing this supervisor's id: refused once it is gone.
        let from_other = (held.pid, held.start);
        let _ = other.kill();
        let _ = other.wait();
        assert!(!take_lock(&lock, Some(from_other)).unwrap());
        // Without a handoff, a missing lock is taken.
        std::fs::remove_file(&lock).unwrap();
        assert!(take_lock(&lock, None).unwrap());
        assert_eq!(
            frontdoor::read_lock(&lock).map(|l| (l.pid, l.start)),
            Some(from)
        );
        // A lock naming this process is its own, with or without a handoff.
        assert!(take_lock(&lock, Some((1, 1))).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_record_keeps_the_last_lines_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("agora-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("node.restarts");
        for i in 0..25 {
            let e = Event::Restarting {
                end: End { code: i, signal: 0 },
                up_ms: 1000,
                wait_ms: 30_000,
                count: 1,
            };
            append_record(&path, &Record::of(&e, 1_000 + i as u64).unwrap());
        }
        let lines: Vec<Record> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter_map(Record::parse)
            .collect();
        assert_eq!(lines.len(), RECORD_LINES);
        assert_eq!(lines[0].end.code, 5);
        assert_eq!(lines.last().unwrap().kind, "restart");
        assert_eq!(Record::of(&Event::AlreadyRunning, 0), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
