//! `agora pair open|close|status` (ADR-0065, `docs/specs/pairing.md` section 2): the owner's switch for new machines, on the machine
//! that runs the hub. It talks to the **running** hub as the owner (the token in `<hub dir>/token`, the same way as every owner
//! command that is not a database read), because the switch and the waiting requests live in the hub's memory.
//!
//! `open` is also the owner's screen for the moment: it stays in the foreground, shows each machine that asks to join with its
//! number, and asks whether the number on that machine is the same. The window cards (web, terminal) are a later step; until then
//! this is where the owner compares and answers. After one machine was approved it closes the switch again.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agora_i18n::key;
use agora_protocol::v1::{self, command_ok, command_request::Command};
use agora_tui::client::{Hub, PairNote, Session, Update};

use crate::hubcli::{hub_dir, own_hub_addr, DEFAULT_LISTEN};
use crate::{Opts, Say};

/// How long a command to the hub may take.
const WAIT: Duration = Duration::from_secs(15);

/// How often the open screen asks the hub whether the switch is still open.
const STATUS_EVERY: Duration = Duration::from_secs(10);

/// The most minutes the switch stays open (the hub's own window, section 2).
const MAX_MINUTES: u64 = 10;

/// The first key of an answer must come at least this long after the question was shown (ADR-0065 review): a key that was held, typed
/// ahead or sent by a program arrives sooner than someone can read two numbers.
const MIN_THINK: Duration = Duration::from_millis(700);

/// How many answers that came too fast are ignored (and asked again) before the request is denied.
const TOO_FAST_TRIES: u32 = 3;

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

/// The short fingerprint of this hub's certificate authority, the same a machine that found the hub by itself shows its person
/// (`agora join`): the person compares the two. From `<hub dir>/ca.pem`.
fn fingerprint_of(o: &Opts) -> Option<String> {
    let pem = std::fs::read_to_string(hub_dir(o).join("ca.pem")).ok()?;
    let fp = agora_hub::tls::leaf_hash(&pem).ok()?;
    Some(agora_hub::discovery::short_fingerprint(&fp))
}

fn say_fingerprint(say: &Say, fp: Option<&str>) {
    if let Some(fp) = fp {
        say.out(key!("pair-fingerprint"), &[("fingerprint", fp.into())]);
    }
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
    // Approving is for a person at a terminal: a pipe, a file or a script can feed any answer, so with no terminal on stdin this
    // refuses to start (the owner's windows are the other way to answer, later). Checked before anything is opened.
    if op == "open" && !std::io::stdin().is_terminal() {
        say.err(key!("pair-needs-terminal"), &[]);
        return ExitCode::from(2);
    }
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
                    say_fingerprint(say, fingerprint_of(o).as_deref());
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
        _ => open(say, session, &hub_dir(o), &addr, minutes, fingerprint_of(o)),
    }
}

/// What `agora pair open` tells the owner to type on the new machine (#943, RM-07): only an address the hub listens on **and** its
/// certificate names, because a machine refuses a hub whose certificate does not name the address it dialled (`pair.name_unverified`).
#[derive(Debug, PartialEq, Eq)]
enum JoinHint {
    /// Another machine can dial these (`host:port`, the first is the one to type), in the certificate's order.
    Network(Vec<String>),
    /// The hub listens on a loopback address only, so no other machine can reach it. `local` is what a second Agora on this same
    /// computer dials, when the certificate names one.
    ThisComputerOnly {
        listen: String,
        port: u16,
        local: Option<String>,
    },
    /// The hub listens beyond this computer, but its certificate names no address another machine could dial. `local` as above.
    NoAddress { local: Option<String> },
}

/// `host:port`, with an IPv6 address in brackets.
fn host_port(host: &str, port: u16) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The hint for a hub that listens on `listen` and whose certificate carries `names` (`agora_hub::ca::cert_names`: DNS names as
/// written, addresses in their shortest text). No I/O.
fn join_hint(listen: std::net::SocketAddr, names: &[String]) -> JoinHint {
    use std::net::IpAddr;
    let port = listen.port();
    let ip_of = |n: &str| n.parse::<IpAddr>().ok().map(|ip| ip.to_canonical());
    let listen_ip = listen.ip().to_canonical();
    let named = |want: IpAddr| names.iter().any(|n| ip_of(n) == Some(want));
    // What a client on this computer dials: the loopback address the hub listens on, or `localhost`, whichever the certificate names.
    let local = |loopback: Option<IpAddr>| {
        loopback
            .filter(|ip| named(*ip))
            .map(|ip| host_port(&ip.to_string(), port))
            .or_else(|| {
                names
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case("localhost"))
                    .then(|| host_port("localhost", port))
            })
    };
    if listen_ip.is_loopback() {
        return JoinHint::ThisComputerOnly {
            listen: listen.to_string(),
            port,
            local: local(Some(listen_ip)),
        };
    }
    // A name (not `localhost`) is offered as the owner or the host gave it; an address only when the hub listens on it: a specific
    // listen address is the only one, a wildcard one is every address the certificate names (IPv4 only for `0.0.0.0`).
    let dialable_ip = |ip: IpAddr| {
        !ip.is_loopback()
            && !ip.is_unspecified()
            && match listen.ip() {
                IpAddr::V4(l) if l.is_unspecified() => ip.is_ipv4(),
                IpAddr::V6(l) if l.is_unspecified() => true,
                _ => ip == listen_ip,
            }
    };
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let candidate = match ip_of(n) {
            Some(ip) if dialable_ip(ip) => host_port(&ip.to_string(), port),
            Some(_) => continue,
            None if n.eq_ignore_ascii_case("localhost") || n.is_empty() => continue,
            None => host_port(n, port),
        };
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    // A hub on one specific address: that address first, it is the one sure to reach it.
    if !listen_ip.is_unspecified() {
        let own = host_port(&listen_ip.to_string(), port);
        if let Some(i) = out.iter().position(|c| *c == own) {
            let first = out.remove(i);
            out.insert(0, first);
        }
    }
    if out.is_empty() {
        // A wildcard listen address also serves loopback; a specific one does not.
        let loopback_too = listen_ip.is_unspecified();
        return JoinHint::NoAddress {
            local: if loopback_too { local(None) } else { None },
        };
    }
    JoinHint::Network(out)
}

/// The sentences of a [`JoinHint`], in order.
fn hint_lines(say: &Say, hint: &JoinHint) -> Vec<String> {
    let local_line = |local: &Option<String>| {
        local.as_ref().map(|a| {
            say.text(
                key!("pair-open-local-join"),
                &[("address", a.as_str().into())],
            )
        })
    };
    match hint {
        JoinHint::Network(addresses) => {
            let Some((first, rest)) = addresses.split_first() else {
                return vec![say.text(key!("pair-open-no-address"), &[])];
            };
            let mut lines = vec![say.text(
                key!("pair-open-next"),
                &[("address", first.as_str().into())],
            )];
            if !rest.is_empty() {
                lines.push(say.text(
                    key!("pair-open-also"),
                    &[("addresses", rest.join(", ").into())],
                ));
            }
            lines
        }
        JoinHint::ThisComputerOnly {
            listen,
            port,
            local,
        } => {
            let mut lines = vec![say.text(
                key!("pair-open-local-only"),
                &[
                    ("listen", listen.as_str().into()),
                    ("port", port.to_string().into()),
                ],
            )];
            lines.extend(local_line(local));
            lines
        }
        JoinHint::NoAddress { local } => {
            let mut lines = vec![say.text(key!("pair-open-no-address"), &[])];
            lines.extend(local_line(local));
            lines
        }
    }
}

/// Where the hub listens, from `<hub dir>/listen` (written at every start); without it, the address this command reached it at.
fn listen_of(dir: &std::path::Path, reached: &str) -> Option<std::net::SocketAddr> {
    std::fs::read_to_string(dir.join(crate::hubcli::LISTEN_FILE))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .or_else(|| reached.parse().ok())
}

/// The names of the certificate the hub serves now, from `<hub dir>/server-cert.pem`: the hub writes it before it switches to a new
/// one, and `PairOpen` issues a new one first when a name is missing (spec section 15). Empty when it cannot be read.
fn served_names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("server-cert.pem"))
        .ok()
        .and_then(|pem| agora_hub::ca::cert_names(&pem).ok())
        .unwrap_or_default()
}

fn open(
    say: &Say,
    mut session: Session,
    dir: &std::path::Path,
    addr: &str,
    minutes: Option<u64>,
    fingerprint: Option<String>,
) -> ExitCode {
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
    // After `PairOpen`: it has made the certificate name every address the hub can be reached at now.
    let hint = listen_of(dir, addr).map_or(JoinHint::NoAddress { local: None }, |listen| {
        join_hint(listen, &served_names(dir))
    });
    for line in hint_lines(say, &hint) {
        let _ = writeln!(std::io::stdout(), "{line}");
    }
    say_fingerprint(say, fingerprint.as_deref());
    say.out(key!("pair-open-waiting"), &[]);
    let _ = std::io::stdout().flush();
    let until = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));
    watch(say, &mut session, until)
}

/// What the owner answered to a question.
pub(crate) enum Answer {
    Yes,
    No,
    /// The terminal cannot answer (it went away, or Ctrl-C).
    Gone,
}

/// Asks `question` and reads the answer from the terminal only (`agora_platform::read_confirmation`): what was typed or sent before
/// the question is discarded, and a first key that comes sooner than [`MIN_THINK`] is ignored; after [`TOO_FAST_TRIES`] of those the
/// answer is no. Only `y` (or the locale's yes word) then Enter is yes.
pub(crate) fn ask(say: &Say, question: &str) -> Answer {
    let yes = say.text(key!("cli-devices-yes"), &[]);
    for _ in 0..TOO_FAST_TRIES {
        let mut out = std::io::stdout();
        let _ = write!(out, "{question} ");
        let _ = out.flush();
        let read = agora_platform::read_confirmation(MIN_THINK);
        let _ = writeln!(out);
        match read {
            Ok(Some(line)) => {
                return if crate::trustcli::is_yes(&line, &yes) {
                    Answer::Yes
                } else {
                    Answer::No
                }
            }
            Ok(None) => say.out(key!("pair-too-fast"), &[]),
            Err(_) => return Answer::Gone,
        }
    }
    Answer::No
}

/// Like [`ask`] but returns the line that was typed (a number to choose with), under the same terminal-only discipline: typed-ahead
/// input is discarded, a first key that comes too fast is ignored (three tries), nothing is echoed. `None` when the terminal cannot
/// answer or the tries ran out.
pub(crate) fn ask_line(say: &Say, question: &str) -> Option<String> {
    for _ in 0..TOO_FAST_TRIES {
        let mut out = std::io::stdout();
        let _ = write!(out, "{question} ");
        let _ = out.flush();
        let read = agora_platform::read_confirmation(MIN_THINK);
        let _ = writeln!(out);
        match read {
            Ok(Some(line)) => return Some(line),
            Ok(None) => say.out(key!("pair-too-fast"), &[]),
            Err(_) => return None,
        }
    }
    None
}

/// The owner's screen: cards in, answers out, until a machine was approved, the time given ended, the switch closed or the terminal went.
fn watch(say: &Say, session: &mut Session, until: Option<Instant>) -> ExitCode {
    let mut waiting: std::collections::VecDeque<v1::PairRequested> = Default::default();
    let mut ended: std::collections::HashSet<Vec<u8>> = Default::default();
    let mut last_status = Instant::now();
    loop {
        // A connection that ended answers at once, so carrying on would spin and wait for a hub that is gone.
        if let Update::Closed = session.next(Duration::from_millis(250)) {
            return fail_hub(say, "client.unreachable");
        }
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
        while let Some(card) = waiting.pop_front() {
            if ended.contains(&card.pair_id) {
                continue;
            }
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
            match ask(say, &say.text(key!("pair-card-ask"), &[])) {
                Answer::Gone => {
                    say.out(key!("pair-input-ended"), &[]);
                    return ExitCode::SUCCESS;
                }
                Answer::No => {
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
                }
                Answer::Yes => match approve(say, session, &card) {
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
                },
            }
        }
    }
}

enum Approval {
    Done,
    Refused,
    Stop,
}

/// Approves the card; if the name belongs to a machine that already joined, asks once more whether to replace it.
fn approve(say: &Say, session: &mut Session, card: &v1::PairRequested) -> Approval {
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
            let question = say.text(
                key!("pair-replace-ask"),
                &[("name", card.name.as_str().into())],
            );
            match ask(say, &question) {
                Answer::Gone => {
                    say.out(key!("pair-input-ended"), &[]);
                    Approval::Stop
                }
                Answer::No => {
                    // The request stays pending on the hub; this screen leaves it to its own time.
                    say.out(key!("pair-left-waiting"), &[]);
                    Approval::Refused
                }
                Answer::Yes => match send(session, true) {
                    Ok(_) => Approval::Done,
                    Err(e) => {
                        refusal(say, &e);
                        Approval::Refused
                    }
                },
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_shown_in_two_groups() {
        assert_eq!(grouped("482193"), "482-193");
        assert_eq!(grouped("12"), "12");
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_string()).collect()
    }

    fn say() -> Say {
        Say(agora_i18n::Catalog::for_locale("en"))
    }

    /// #943 (RM-07): a hub on loopback only is reachable from this computer only; the hint says so, names the way to listen on the
    /// network with the hub's own port, and offers only the loopback address the certificate names.
    #[test]
    fn a_loopback_only_hub_offers_this_computer_only() {
        let hint = join_hint(
            "127.0.0.1:7443".parse().unwrap(),
            &names(&["localhost", "white", "127.0.0.1"]),
        );
        assert_eq!(
            hint,
            JoinHint::ThisComputerOnly {
                listen: "127.0.0.1:7443".into(),
                port: 7443,
                local: Some("127.0.0.1:7443".into()),
            }
        );
        let text = hint_lines(&say(), &hint).join("\n");
        assert!(
            text.contains("listens on this computer only (127.0.0.1:7443)"),
            "{text}"
        );
        assert!(text.contains("`agora --listen 0.0.0.0:7443`"), "{text}");
        assert!(text.contains("`agora join 127.0.0.1:7443`"), "{text}");
        assert!(!text.contains("agora join white"), "{text}");
        // A certificate that names only `localhost` (an older hub): that is what is offered.
        assert_eq!(
            join_hint("127.0.0.1:7443".parse().unwrap(), &names(&["localhost"])),
            JoinHint::ThisComputerOnly {
                listen: "127.0.0.1:7443".into(),
                port: 7443,
                local: Some("localhost:7443".into()),
            }
        );
        // IPv6 loopback, in brackets; a certificate that cannot be read offers nothing to dial.
        assert_eq!(
            join_hint("[::1]:7443".parse().unwrap(), &names(&["::1"])),
            JoinHint::ThisComputerOnly {
                listen: "[::1]:7443".into(),
                port: 7443,
                local: Some("[::1]:7443".into()),
            }
        );
        let none = join_hint("127.0.0.1:7443".parse().unwrap(), &[]);
        assert_eq!(
            none,
            JoinHint::ThisComputerOnly {
                listen: "127.0.0.1:7443".into(),
                port: 7443,
                local: None,
            }
        );
        assert!(!hint_lines(&say(), &none).join("\n").contains("agora join"));
    }

    /// #943: a hub on a LAN address offers that address (in its certificate), never one it does not listen on.
    #[test]
    fn a_hub_on_a_lan_address_offers_that_address() {
        let hint = join_hint(
            "192.168.1.20:7443".parse().unwrap(),
            // An address of an older certificate (10.0.0.5) that the hub no longer listens on is not offered.
            &names(&["localhost", "white", "10.0.0.5", "192.168.1.20"]),
        );
        assert_eq!(
            hint,
            JoinHint::Network(vec!["192.168.1.20:7443".into(), "white:7443".into()])
        );
        let text = hint_lines(&say(), &hint).join("\n");
        assert!(text.contains("`agora join 192.168.1.20:7443`"), "{text}");
        assert!(text.contains("white:7443"), "{text}");
        assert!(!text.contains("10.0.0.5"), "{text}");
        assert!(!text.contains("this computer only"), "{text}");
    }

    /// #943: a hub on every address offers its certificate's names and addresses (IPv4 only for `0.0.0.0`), never `localhost` or a
    /// loopback address; with none to offer it says so and how to name one.
    #[test]
    fn a_hub_on_every_address_offers_what_its_certificate_names() {
        let names = names(&[
            "agora.example",
            "localhost",
            "white",
            "127.0.0.1",
            "192.168.1.246",
            "fd00::5",
        ]);
        assert_eq!(
            join_hint("0.0.0.0:7443".parse().unwrap(), &names),
            JoinHint::Network(vec![
                "agora.example:7443".into(),
                "white:7443".into(),
                "192.168.1.246:7443".into(),
            ])
        );
        assert_eq!(
            join_hint("[::]:7443".parse().unwrap(), &names),
            JoinHint::Network(vec![
                "agora.example:7443".into(),
                "white:7443".into(),
                "192.168.1.246:7443".into(),
                "[fd00::5]:7443".into(),
            ])
        );
        let only_local = join_hint(
            "0.0.0.0:7443".parse().unwrap(),
            &self::names(&["localhost"]),
        );
        assert_eq!(
            only_local,
            JoinHint::NoAddress {
                local: Some("localhost:7443".into())
            }
        );
        let text = hint_lines(&say(), &only_local).join("\n");
        assert!(text.contains("--name ADDRESS"), "{text}");
        assert!(text.contains("`agora join localhost:7443`"), "{text}");
        // A hub on one LAN address does not answer on loopback.
        assert_eq!(
            join_hint(
                "192.168.1.20:7443".parse().unwrap(),
                &self::names(&["localhost"])
            ),
            JoinHint::NoAddress { local: None }
        );
    }
}
