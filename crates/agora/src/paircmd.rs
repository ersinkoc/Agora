//! `agora pair open|close|status` (ADR-0065, `docs/specs/pairing.md` section 2): the owner's switch for new machines, on the machine
//! that runs the hub. It talks to the **running** hub as the owner (the token in `<hub dir>/token`, the same way as every owner
//! command that is not a database read), because the switch and the waiting requests live in the hub's memory.
//!
//! `open` is also the owner's screen for the moment: it stays in the foreground, shows each machine that asks to join with its
//! number, and asks whether the number on that machine is the same. The window cards (web, terminal) are a later step; until then
//! this is where the owner compares and answers. After one machine was approved it closes the switch again.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_i18n::key;
use agora_protocol::v1::{self, command_ok, command_request::Command};
use agora_tui::client::{Hub, PairNote, Session};

use crate::hubcli::{hub_dir, own_hub_addr, DEFAULT_LISTEN};
use crate::{Opts, Say};

/// How long a command to the hub may take.
const WAIT: Duration = Duration::from_secs(15);

/// How often the open screen asks the hub whether the switch is still open.
const STATUS_EVERY: Duration = Duration::from_secs(10);

/// The most minutes the switch stays open (the hub's own window, section 2).
const MAX_MINUTES: u64 = 10;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The six digits as people read them, `482-193`.
fn grouped(number: &str) -> String {
    if number.len() == 6 && number.is_ascii() {
        format!("{}-{}", &number[..3], &number[3..])
    } else {
        number.to_string()
    }
}

fn fail_hub(say: &Say, reason: &str) -> ExitCode {
    say.err(key!("pair-hub-unreachable"), &[("reason", reason.into())]);
    ExitCode::from(1)
}

/// The running hub, as its owner.
fn connect(say: &Say, o: &Opts) -> Result<(Session, String), ExitCode> {
    let dir = hub_dir(o);
    let token = std::fs::read_to_string(dir.join("token")).map(|t| t.trim().to_string());
    let ca_pem = std::fs::read_to_string(dir.join("ca.pem"));
    let (Ok(token), Ok(ca_pem)) = (token, ca_pem) else {
        say.err(
            key!("pair-no-hub"),
            &[("dir", dir.display().to_string().into())],
        );
        return Err(ExitCode::from(1));
    };
    let addr = own_hub_addr(&dir).unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let hub = Hub {
        addr: addr.clone(),
        ca_pem,
    };
    Session::connect(&hub, &token)
        .map(|s| (s, addr))
        .map_err(|e| fail_hub(say, &format!("{e:?}")))
}

/// What the hub answers to `PairOpen` and `PairStatus`.
fn status_of(r: Result<v1::CommandOk, String>) -> Result<v1::PairStatusResult, String> {
    match r?.result {
        Some(command_ok::Result::PairStatus(s)) => Ok(s),
        _ => Err("protocol.invalid_message".into()),
    }
}

/// Minutes left, rounded up, so "closes in 0 minutes" is never said while it is still open.
fn minutes_left(closes_at_ms: u64) -> i64 {
    let left = closes_at_ms.saturating_sub(now_ms());
    i64::try_from(left.div_ceil(60_000)).unwrap_or(i64::MAX)
}

fn say_status(say: &Say, s: &v1::PairStatusResult) {
    if s.open {
        say.out(
            key!("pair-status-open"),
            &[("minutes", minutes_left(s.closes_at_ms).into())],
        );
    } else {
        say.out(key!("pair-status-closed"), &[]);
    }
    if s.pending > 0 {
        say.out(
            key!("pair-status-waiting"),
            &[("count", i64::from(s.pending).into())],
        );
    }
}

/// A refusal from the hub as one sentence (`Error pair.closed: ...`), the way the other commands print theirs.
fn refusal(say: &Say, e: &v1::ErrorInfo) {
    say.remote_error(e);
}

pub fn pair(say: &Say, o: &Opts, op: &str) -> ExitCode {
    // `--minutes` is a way to open for less than the hub's ten; anything else is a mistake.
    let minutes = match (op, o.minutes) {
        ("open", Some(m)) if (1..=MAX_MINUTES).contains(&m) => Some(m),
        ("open", None) => None,
        (_, None) if op != "open" => None,
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            say.err(
                key!("pair-minutes-usage"),
                &[("max", (MAX_MINUTES as i64).into())],
            );
            return ExitCode::from(2);
        }
    };
    let (mut session, addr) = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let cmd = |s: &mut Session, c: Command| s.command_info("", c, WAIT);
    match op {
        "status" => match cmd(&mut session, Command::PairStatus(v1::PairStatus {})) {
            Ok(ok) => match status_of(Ok(ok)) {
                Ok(s) => {
                    say_status(say, &s);
                    ExitCode::SUCCESS
                }
                Err(e) => fail_hub(say, &e),
            },
            Err(e) => {
                refusal(say, &e);
                ExitCode::from(1)
            }
        },
        "close" => match cmd(&mut session, Command::PairClose(v1::PairClose {})) {
            Ok(_) => {
                say.out(key!("pair-closed-now"), &[]);
                ExitCode::SUCCESS
            }
            Err(e) => {
                refusal(say, &e);
                ExitCode::from(1)
            }
        },
        _ => open(say, session, &addr, minutes),
    }
}

/// The host and port a new machine types: this computer's name and the hub's port.
fn join_address(addr: &str) -> String {
    let port = addr.rsplit_once(':').map_or(addr, |(_, p)| p);
    let host = agora_platform::hostname().unwrap_or_default();
    let host = host.trim();
    if host.is_empty() {
        format!("<this-computer>:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn open(say: &Say, mut session: Session, addr: &str, minutes: Option<u64>) -> ExitCode {
    let status = match session.command_info("", Command::PairOpen(v1::PairOpen {}), WAIT) {
        Ok(ok) => match status_of(Ok(ok)) {
            Ok(s) => s,
            Err(e) => return fail_hub(say, &e),
        },
        Err(e) => {
            refusal(say, &e);
            return ExitCode::from(1);
        }
    };
    let opened = minutes.map_or_else(
        || minutes_left(status.closes_at_ms),
        |m| i64::try_from(m).unwrap_or(1),
    );
    say.out(key!("pair-opened"), &[("minutes", opened.into())]);
    say.out(
        key!("pair-open-next"),
        &[("address", join_address(addr).into())],
    );
    // The answers come from this terminal, one line each; a reader thread keeps the loop from ever blocking on it. When input ends
    // (nobody is typing: a script, a closed terminal) the switch is left open for its time and the requests wait for a window.
    let (tx, answers) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        while stdin.lock().read_line(&mut line).is_ok_and(|n| n > 0) {
            if tx.send(std::mem::take(&mut line)).is_err() {
                break;
            }
        }
    });
    say.out(key!("pair-open-waiting"), &[]);
    let _ = std::io::stdout().flush();
    let until = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));
    watch(say, &mut session, &answers, until)
}

/// The owner's screen: cards in, answers out, until a machine was approved, the time given ended, the switch closed or the input did.
fn watch(
    say: &Say,
    session: &mut Session,
    answers: &mpsc::Receiver<String>,
    until: Option<Instant>,
) -> ExitCode {
    let yes = say.text(key!("cli-devices-yes"), &[]);
    let mut waiting: std::collections::VecDeque<v1::PairRequested> = Default::default();
    let mut ended: std::collections::HashSet<Vec<u8>> = Default::default();
    let mut last_status = Instant::now();
    loop {
        let _ = session.next(Duration::from_millis(250));
        for note in session.take_pair_notes() {
            match note {
                PairNote::Requested(c) if c.kind == v1::PairKind::Machine as i32 => {
                    if !waiting.iter().any(|w| w.pair_id == c.pair_id) {
                        waiting.push_back(c);
                    }
                }
                // A browser asks to sign in: that is for a window (a later step); it waits there.
                PairNote::Requested(_) => {}
                PairNote::Decided(d) => {
                    ended.insert(d.pair_id);
                }
            }
        }
        if until.is_some_and(|t| Instant::now() >= t) {
            return close_and_say(say, session, key!("pair-time-up"));
        }
        // The switch may have been closed from somewhere else, or ran out.
        if last_status.elapsed() >= STATUS_EVERY {
            last_status = Instant::now();
            let still_open = session
                .command_info("", Command::PairStatus(v1::PairStatus {}), WAIT)
                .map_err(|e| e.code)
                .and_then(|ok| status_of(Ok(ok)))
                .map_or(true, |s| s.open);
            if !still_open {
                say.out(key!("pair-closed-now"), &[]);
                return ExitCode::SUCCESS;
            }
        }
        // Lines typed while no card was shown answer nothing (a "y" typed ahead must never approve a machine that comes later);
        // and when the input is over with nothing waiting, there is nobody to ask.
        if waiting.is_empty() {
            loop {
                match answers.try_recv() {
                    Ok(_) => {}
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        say.out(key!("pair-input-ended"), &[]);
                        return ExitCode::SUCCESS;
                    }
                }
            }
        }
        while let Some(card) = waiting.pop_front() {
            if ended.contains(&card.pair_id) {
                continue;
            }
            while answers.try_recv().is_ok() {}
            let number = grouped(&card.number);
            say.out(
                key!("pair-card"),
                &[
                    ("name", card.name.as_str().into()),
                    ("os", card.os.as_str().into()),
                    ("address", card.address.as_str().into()),
                    ("number", number.as_str().into()),
                ],
            );
            let mut out = std::io::stdout();
            let _ = write!(out, "{} ", say.text(key!("pair-card-ask"), &[]));
            let _ = out.flush();
            let Some(answer) = wait_for_answer(session, answers, &card, &mut ended) else {
                // No answer can come (the input ended) or the machine gave up: say which.
                if ended.contains(&card.pair_id) {
                    say.out(
                        key!("pair-card-ended"),
                        &[("name", card.name.as_str().into())],
                    );
                    continue;
                }
                say.out(key!("pair-input-ended"), &[]);
                return ExitCode::SUCCESS;
            };
            if !crate::trustcli::is_yes(&answer, &yes) {
                match session.command_info(
                    "",
                    Command::PairDeny(v1::PairDeny {
                        pair_id: card.pair_id.clone(),
                    }),
                    WAIT,
                ) {
                    Ok(_) => say.out(key!("pair-denied-now"), &[]),
                    Err(e) => refusal(say, &e),
                }
                continue;
            }
            match approve(say, session, answers, &card, &yes) {
                Approval::Done => {
                    // One machine per opening: close the switch again, which is the safer thing to forget.
                    say.out(
                        key!("pair-approved"),
                        &[("name", card.name.as_str().into())],
                    );
                    return close_and_say(say, session, key!("pair-closed-after"));
                }
                Approval::Refused => {}
                Approval::Stop => return ExitCode::SUCCESS,
            }
        }
    }
}

/// Waits for the owner's line, while the hub's notes keep being read (a machine that gives up ends its card). `None` when the
/// input ended or the card ended before an answer came.
fn wait_for_answer(
    session: &mut Session,
    answers: &mpsc::Receiver<String>,
    card: &v1::PairRequested,
    ended: &mut std::collections::HashSet<Vec<u8>>,
) -> Option<String> {
    loop {
        match answers.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => return Some(line),
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let _ = session.next(Duration::from_millis(50));
        for note in session.take_pair_notes() {
            if let PairNote::Decided(d) = note {
                ended.insert(d.pair_id);
            }
        }
        if ended.contains(&card.pair_id) {
            return None;
        }
    }
}

enum Approval {
    Done,
    Refused,
    Stop,
}

/// Approves the card; if the name belongs to a machine that already joined, asks once more whether to replace it.
fn approve(
    say: &Say,
    session: &mut Session,
    answers: &mpsc::Receiver<String>,
    card: &v1::PairRequested,
    yes: &str,
) -> Approval {
    let send = |s: &mut Session, replace: bool| {
        s.command_info(
            "",
            Command::PairApprove(v1::PairApprove {
                pair_id: card.pair_id.clone(),
                replace,
            }),
            WAIT,
        )
    };
    match send(session, false) {
        Ok(_) => Approval::Done,
        Err(e) if e.code == "pair.state" && state_is(&e, "name_in_use") => {
            let mut out = std::io::stdout();
            let _ = write!(
                out,
                "{} ",
                say.text(
                    key!("pair-replace-ask"),
                    &[("name", card.name.as_str().into())]
                )
            );
            let _ = out.flush();
            let Ok(line) = answers.recv() else {
                say.out(key!("pair-input-ended"), &[]);
                return Approval::Stop;
            };
            if !crate::trustcli::is_yes(&line, yes) {
                // The request stays pending on the hub; this screen leaves it to its own time.
                say.out(key!("pair-left-waiting"), &[]);
                return Approval::Refused;
            }
            match send(session, true) {
                Ok(_) => Approval::Done,
                Err(e) => {
                    refusal(say, &e);
                    Approval::Refused
                }
            }
        }
        Err(e) => {
            refusal(say, &e);
            Approval::Refused
        }
    }
}

fn state_is(e: &v1::ErrorInfo, state: &str) -> bool {
    matches!(
        e.params.get("state").and_then(|v| v.value.as_ref()),
        Some(v1::param_value::Value::Str(s)) if s == state
    )
}

fn close_and_say(say: &Say, session: &mut Session, why: &'static str) -> ExitCode {
    match session.command_info("", Command::PairClose(v1::PairClose {}), WAIT) {
        Ok(_) => {
            say.out(why, &[]);
            ExitCode::SUCCESS
        }
        Err(e) => {
            refusal(say, &e);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_shown_in_two_groups() {
        assert_eq!(grouped("482193"), "482-193");
        assert_eq!(grouped("12"), "12");
    }

    #[test]
    fn the_join_address_has_the_hubs_port() {
        assert!(join_address("127.0.0.1:7443").ends_with(":7443"));
    }
}
