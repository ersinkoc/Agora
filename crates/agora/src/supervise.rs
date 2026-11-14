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

/// What the supervisor did, for its log and its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// The child could not be tied to the supervisor (it may outlive it).
    NotBound,
    /// A stop request reached the supervisor; the child was stopped with it.
    StopRequest,
    /// The child exited with code 0.
    Stopped,
    /// The child ended too soon after its start to be a crash.
    Refused { code: i32, up_ms: u64 },
    /// The child crashed and starts again after `wait_ms`.
    Restarting {
        code: i32,
        up_ms: u64,
        wait_ms: u64,
        count: usize,
    },
    /// The child crashed once more after the allowed restarts.
    GaveUp { code: i32 },
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
            note(&Event::NotBound);
        }
        let began = ms(Instant::now());
        agora_platform::forward_stop_to(Some(b.child.id()));
        // A request caught between the start and the line above was not passed on: pass it on now.
        if agora_platform::stop_requested() {
            let _ = b.child.kill();
        }
        let status = b.child.wait();
        agora_platform::forward_stop_to(None);
        let status = status?;
        if agora_platform::stop_requested() {
            note(&Event::StopRequest);
            return Ok(0);
        }
        let code = status.code().unwrap_or(-1);
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
                note(&Event::Refused { code, up_ms });
                return Ok(failure(code));
            }
            Next::GiveUp => {
                note(&Event::GaveUp { code });
                return Ok(failure(code));
            }
            Next::Restart { wait_ms, count } => {
                note(&Event::Restarting {
                    code,
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

/// A failed child's code as the supervisor's own: never 0 (a signal or an out-of-range code becomes 1).
fn failure(code: i32) -> i32 {
    if (1..=255).contains(&code) {
        code
    } else {
        1
    }
}

/// Makes this process the holder of the role's lock: waits for the front door that started it to write its id, or takes a missing or
/// stale lock. False when another live process holds it.
fn take_lock(lock: &Path) -> std::io::Result<bool> {
    let me = LockInfo::of(std::process::id())
        .ok_or_else(|| std::io::Error::other("own start time unknown"))?;
    let end = Instant::now() + HANDOFF;
    loop {
        match frontdoor::read_lock(lock) {
            Some(l) if l.pid == me.pid && l.start == me.start => return Ok(true),
            Some(l) if l.alive() => {
                if Instant::now() > end {
                    return Ok(false);
                }
            }
            _ => {
                if let Claim::Mine = frontdoor::claim(lock)? {
                    return Ok(true);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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

/// One line of the record: when (Unix milliseconds), what, the exit code, how long the child had run, the restart number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub at: u64,
    pub kind: String,
    pub code: i32,
    pub up_ms: u64,
    pub count: usize,
}

impl Record {
    fn line(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.at, self.kind, self.code, self.up_ms, self.count
        )
    }

    fn parse(line: &str) -> Option<Record> {
        let mut w = line.split_whitespace();
        Some(Record {
            at: w.next()?.parse().ok()?,
            kind: w.next()?.to_string(),
            code: w.next()?.parse().ok()?,
            up_ms: w.next()?.parse().ok()?,
            count: w.next()?.parse().ok()?,
        })
    }

    fn of(e: &Event, at: u64) -> Option<Record> {
        let (kind, code, up_ms, count) = match e {
            Event::Restarting {
                code, up_ms, count, ..
            } => ("restart", *code, *up_ms, *count),
            Event::Refused { code, up_ms } => ("refused", *code, *up_ms, 0),
            Event::GaveUp { code } => ("gave-up", *code, 0, 0),
            Event::Stopped | Event::StopRequest => ("stopped", 0, 0, 0),
            _ => return None,
        };
        Some(Record {
            at,
            kind: kind.into(),
            code,
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
        Event::Refused { code, up_ms } => say.text(
            key!("supervise-refused"),
            &[
                ("role", r),
                ("code", i64::from(*code).into()),
                ("seconds", secs(*up_ms).into()),
            ],
        ),
        Event::Restarting {
            code,
            up_ms,
            wait_ms,
            count,
        } => say.text(
            key!("supervise-restarting"),
            &[
                ("role", r),
                ("code", i64::from(*code).into()),
                ("seconds", secs(*up_ms).into()),
                ("wait", secs(*wait_ms).into()),
                ("count", i64::try_from(*count).unwrap_or(i64::MAX).into()),
                ("max", max.into()),
                ("minutes", minutes.into()),
            ],
        ),
        Event::GaveUp { code } => say.text(
            key!("supervise-gave-up"),
            &[
                ("role", r),
                ("code", i64::from(*code).into()),
                ("max", max.into()),
                ("minutes", minutes.into()),
            ],
        ),
        Event::AlreadyRunning => say.text(key!("supervise-already-running"), &[("role", r)]),
        Event::LostLock => say.text(key!("supervise-lost-lock"), &[("role", r)]),
    }
}

/// `agora supervise hub|node`.
pub fn supervise(say: &Say, word: &str) -> ExitCode {
    let Some(role) = Role::from_name(word) else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let h = home();
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
    match take_lock(&lock) {
        Ok(true) => {}
        Ok(false) => {
            let pid = frontdoor::read_lock(&lock).map_or(0, |l| i64::from(l.pid));
            say.err(
                key!("supervise-lock-held"),
                &[("role", role.label(say).into()), ("pid", pid.into())],
            );
            return ExitCode::from(1);
        }
        Err(e) => return fail(&e),
    }
    if let Err(e) = agora_platform::catch_stop_requests() {
        return fail(&e);
    }
    // The command line is made once: every restart runs exactly what the front door asked for.
    let mut cmd = match frontdoor::command(&h, &m, role) {
        Ok(c) => c,
        Err(e) => return fail(&e),
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
            let _ = writeln!(std::io::stderr(), "{}", text(say, role, &rule, e));
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
    let minutes_ago = i64::try_from(now.saturating_sub(last.at) / 60_000).unwrap_or(i64::MAX);
    let max = i64::try_from(rule.max_restarts).unwrap_or(i64::MAX);
    let recent = records
        .iter()
        .filter(|r| r.kind == "restart" && now.saturating_sub(r.at) < rule.window_ms)
        .count();
    let role_arg = || -> Arg { role.label(say).into() };
    match last.kind.as_str() {
        "restart" if supervisor_alive && !answers => vec![say.text(
            key!("status-restarting"),
            &[
                ("code", i64::from(last.code).into()),
                (
                    "count",
                    i64::try_from(last.count).unwrap_or(i64::MAX).into(),
                ),
                ("max", max.into()),
            ],
        )],
        "restart" | "stopped" if recent > 0 => {
            let crash = records.iter().rev().find(|r| r.kind == "restart");
            crash.map_or_else(Vec::new, |c| {
                vec![say.text(
                    key!("status-restarts"),
                    &[
                        ("count", i64::try_from(recent).unwrap_or(i64::MAX).into()),
                        ("code", i64::from(c.code).into()),
                        (
                            "minutes",
                            i64::try_from(now.saturating_sub(c.at) / 60_000)
                                .unwrap_or(i64::MAX)
                                .into(),
                        ),
                    ],
                )]
            })
        }
        "gave-up" if !supervisor_alive && !answers => vec![say.text(
            key!("status-gave-up"),
            &[
                ("role", role_arg()),
                ("code", i64::from(last.code).into()),
                ("max", max.into()),
                ("minutes", minutes_ago.into()),
            ],
        )],
        "refused" if !supervisor_alive && !answers => vec![say.text(
            key!("status-refused"),
            &[
                ("role", role_arg()),
                ("code", i64::from(last.code).into()),
                ("minutes", minutes_ago.into()),
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
                Event::Restarting { code, count, .. } => format!("restart {code} {count}"),
                Event::GaveUp { code } => format!("gave up {code}"),
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
            matches!(events.last(), Some(Event::Refused { code: 4, .. })),
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
        assert_eq!(failure(3), 3);
        assert_eq!(failure(255), 255);
        assert_eq!(failure(-1), 1);
        assert_eq!(failure(0), 1);
        assert_eq!(failure(-1073741510), 1);
    }

    #[test]
    fn the_record_keeps_the_last_lines_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("agora-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("node.restarts");
        for i in 0..25 {
            let e = Event::Restarting {
                code: i,
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
        assert_eq!(lines[0].code, 5);
        assert_eq!(lines.last().unwrap().kind, "restart");
        assert_eq!(Record::of(&Event::AlreadyRunning, 0), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
