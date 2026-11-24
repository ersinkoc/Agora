//! The owner's window answers a new machine's request (ADR-0065): a real hub, the real `agora join` on the new machine, and an owner
//! driving the hub the way the web window does (the events stream for the cards, `PairStatus` / `PairOpen` / `PairApprove` as commands).
//! Approving through the owner commands completes the pairing; the number on the card is the number on the machine; a name that already
//! belongs to a machine is refused until the owner says Replace; and a connection that is not the owner's is told nothing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_protocol::v1;
use agora_protocol::v1::command_request::Command as Cmd;
use agora_tui::client::{Hub as HubAddr, PairNote, Session};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn agora(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(AGORA);
    c.args(args).env("AGORA_HOME", home).env("AGORA_LANG", "en");
    c
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-pairwin-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Hub {
    _proc: Proc,
    _out: BufReader<std::process::ChildStdout>,
    dir: PathBuf,
    port: String,
}

fn hub(root: &Path) -> Hub {
    let (dir, home) = (root.join("hub"), root.join("hub-home"));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = agora(
        &home,
        &[
            "hub",
            "run",
            "--listen",
            "127.0.0.1:0",
            "--name",
            "localhost",
            "--hub-dir",
        ],
    )
    .arg(&dir)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    out.read_line(&mut line).unwrap();
    let proc = Proc(child);
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    Hub {
        _proc: proc,
        _out: out,
        dir,
        port: addr.rsplit(':').next().unwrap().to_string(),
    }
}

impl Hub {
    /// A window of the owner: the events stream and the commands, with the owner's token.
    fn window(&self) -> Session {
        let token = std::fs::read_to_string(self.dir.join("token")).unwrap();
        let hub = HubAddr {
            addr: format!("localhost:{}", self.port),
            ca_pem: std::fs::read_to_string(self.dir.join("ca.pem")).unwrap(),
        };
        Session::connect(&hub, token.trim()).unwrap()
    }
}

/// `agora join` on a new machine, its output read as it comes.
struct Join {
    child: Proc,
    lines: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl Join {
    fn start(home: &Path, address: &str, id: &str, replace: bool) -> Self {
        let mut c = agora(home, &["join", address, "--id", id]);
        if replace {
            c.arg("--replace");
        }
        let mut child = c
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, lines) = mpsc::channel();
        for pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            });
        }
        Self {
            child: Proc(child),
            lines,
            seen: Vec::new(),
        }
    }

    fn wait_for(&mut self, needle: &str) -> String {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    self.seen.push(l.clone());
                    if l.contains(needle) {
                        return l;
                    }
                }
                Err(_) => panic!("never saw {needle:?} in {:#?}", self.seen),
            }
        }
    }

    fn finish(mut self) -> (bool, String) {
        let end = Instant::now() + Duration::from_secs(30);
        let ok = loop {
            if let Some(s) = self.child.0.try_wait().unwrap() {
                break s.success();
            }
            assert!(Instant::now() < end, "did not end: {:#?}", self.seen);
            std::thread::sleep(Duration::from_millis(50));
        };
        while let Ok(l) = self.lines.recv_timeout(Duration::from_millis(300)) {
            self.seen.push(l);
        }
        (ok, self.seen.join("\n"))
    }
}

/// `482-193` out of a line that says `Number 482-193.`.
fn number_in(line: &str) -> String {
    line.split("Number ")
        .nth(1)
        .expect(line)
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect()
}

/// Reads the window's stream until a card for `name` shows up (every other notice is kept in `rest`).
fn card_for(window: &mut Session, name: &str, rest: &mut Vec<PairNote>) -> v1::PairRequested {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        let _ = window.next(Duration::from_millis(250));
        for n in window.take_pair_notes() {
            match n {
                PairNote::Requested(c) if c.name == name => return c,
                other => rest.push(other),
            }
        }
    }
    panic!("no card for {name:?}; saw {rest:?}");
}

fn decided(window: &mut Session, id: &[u8]) -> v1::PairOutcome {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        let _ = window.next(Duration::from_millis(250));
        for n in window.take_pair_notes() {
            if let PairNote::Decided(d) = n {
                if d.pair_id == id {
                    return v1::PairOutcome::try_from(d.outcome).unwrap();
                }
            }
        }
    }
    panic!("no decision");
}

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn the_window_opens_the_switch_reads_the_card_and_approving_through_the_owner_command_joins_the_machine(
) {
    let root = tmp("flow");
    let hub = hub(&root);
    let mut window = hub.window();

    // The hub says this session is the owner's: `PairStatus` answers it. Closed to begin with; the window opens it.
    let status = |w: &mut Session| match w
        .command_info("", Cmd::PairStatus(v1::PairStatus {}), WAIT)
        .unwrap()
        .result
    {
        Some(v1::command_ok::Result::PairStatus(s)) => s,
        other => panic!("{other:?}"),
    };
    assert!(!status(&mut window).open);
    let opened = match window
        .command_info("", Cmd::PairOpen(v1::PairOpen {}), WAIT)
        .unwrap()
        .result
    {
        Some(v1::command_ok::Result::PairStatus(s)) => s,
        other => panic!("{other:?}"),
    };
    assert!(opened.open && opened.closes_at_ms > 0, "{opened:?}");

    // A new machine asks. Its number is the number on the card the window gets; name, system and address are on the card.
    let machine = root.join("machine");
    let mut join = Join::start(
        &machine,
        &format!("localhost:{}", hub.port),
        "laptop",
        false,
    );
    let on_machine = number_in(&join.wait_for("Number "));
    let mut rest = Vec::new();
    let card = card_for(&mut window, "laptop", &mut rest);
    assert_eq!(card.kind, v1::PairKind::Machine as i32);
    assert_eq!(
        format!("{}-{}", &card.number[..3], &card.number[3..]),
        on_machine
    );
    assert!(!card.os.is_empty() && !card.address.is_empty(), "{card:?}");
    assert!(card.expires_ms > 0);
    assert_eq!(card.pair_id.len(), 16);

    // A window that connects later gets the cards waiting now (a reconnect resnapshot), once each.
    let mut second = hub.window();
    let again = card_for(&mut second, "laptop", &mut Vec::new());
    assert_eq!(again.pair_id, card.pair_id);
    assert_eq!(again.number, card.number);

    // The owner compares and approves with the owner command, as the web window does.
    window
        .command_info(
            "",
            Cmd::PairApprove(v1::PairApprove {
                pair_id: card.pair_id.clone(),
                replace: false,
            }),
            WAIT,
        )
        .unwrap();
    let (ok, joined) = join.finish();
    assert!(ok, "{joined}");
    assert!(
        joined.contains("Joined. This machine is \"laptop\""),
        "{joined}"
    );
    assert_eq!(
        std::fs::read_to_string(machine.join("link").join("node-id")).unwrap(),
        "laptop"
    );
    // One machine per opening, whoever approved (#942, RM-07): the approval from the window closed the hub to new machines, as an
    // approval at `agora pair open` does.
    assert!(
        !status(&mut window).open,
        "the hub stayed open after the window approved"
    );
    // Both windows hear how it ended, and the card is gone.
    assert_eq!(
        decided(&mut window, &card.pair_id),
        v1::PairOutcome::Approved
    );
    assert_eq!(
        decided(&mut second, &card.pair_id),
        v1::PairOutcome::Approved
    );
    let mut third = hub.window();
    let _ = third.next(Duration::from_millis(500));
    assert!(
        third
            .take_pair_notes()
            .iter()
            .all(|n| !matches!(n, PairNote::Requested(c) if c.name == "laptop")),
        "a stale card came back"
    );

    // The window opens it again, and closes it with the owner command too.
    window
        .command_info("", Cmd::PairOpen(v1::PairOpen {}), WAIT)
        .unwrap();
    assert!(status(&mut window).open);
    window
        .command_info("", Cmd::PairClose(v1::PairClose {}), WAIT)
        .unwrap();
    assert!(!status(&mut window).open);
}

#[test]
fn a_name_that_already_belongs_to_a_machine_is_refused_until_the_owner_says_replace() {
    let root = tmp("replace");
    let hub = hub(&root);
    let mut window = hub.window();
    window
        .command_info("", Cmd::PairOpen(v1::PairOpen {}), WAIT)
        .unwrap();
    let approve = |w: &mut Session, id: &[u8], replace: bool| {
        w.command_info(
            "",
            Cmd::PairApprove(v1::PairApprove {
                pair_id: id.to_vec(),
                replace,
            }),
            WAIT,
        )
    };

    // The first machine called "laptop" joins.
    let first = root.join("first");
    let mut j = Join::start(&first, &format!("localhost:{}", hub.port), "laptop", false);
    j.wait_for("Number ");
    let card = card_for(&mut window, "laptop", &mut Vec::new());
    approve(&mut window, &card.pair_id, false).unwrap();
    assert!(j.finish().0);
    // The approval closed the hub to new machines; the owner opens it again for the next one.
    window
        .command_info("", Cmd::PairOpen(v1::PairOpen {}), WAIT)
        .unwrap();

    // Another machine asks for the same name: the hub refuses the approval and keeps the request, saying why in a parameter the card
    // reads (`pair.state`, `state = name_in_use`).
    let second = root.join("second");
    let mut j = Join::start(&second, &format!("localhost:{}", hub.port), "laptop", false);
    j.wait_for("Number ");
    let card = card_for(&mut window, "laptop", &mut Vec::new());
    let refused = approve(&mut window, &card.pair_id, false).unwrap_err();
    assert_eq!(refused.code, "pair.state");
    assert_eq!(
        refused.params.get("state").and_then(|v| v.value.clone()),
        Some(v1::param_value::Value::Str("name_in_use".into()))
    );
    // Still pending: the same card can be approved with Replace.
    approve(&mut window, &card.pair_id, true).unwrap();
    let (ok, text) = j.finish();
    assert!(ok, "{text}");
    assert_eq!(
        decided(&mut window, &card.pair_id),
        v1::PairOutcome::Approved
    );
}

#[test]
fn a_deny_from_the_window_ends_the_request_and_gives_the_machine_nothing() {
    let root = tmp("deny");
    let hub = hub(&root);
    let mut window = hub.window();
    window
        .command_info("", Cmd::PairOpen(v1::PairOpen {}), WAIT)
        .unwrap();
    let machine = root.join("machine");
    let mut j = Join::start(&machine, &format!("localhost:{}", hub.port), "desk", false);
    j.wait_for("Number ");
    let card = card_for(&mut window, "desk", &mut Vec::new());
    window
        .command_info(
            "",
            Cmd::PairDeny(v1::PairDeny {
                pair_id: card.pair_id.clone(),
            }),
            WAIT,
        )
        .unwrap();
    let (ok, _) = j.finish();
    assert!(!ok);
    assert!(!machine.join("link").join("node-id").exists());
    assert_eq!(decided(&mut window, &card.pair_id), v1::PairOutcome::Denied);
}

#[test]
fn a_session_that_is_not_the_owners_is_refused_and_is_told_no_card() {
    let root = tmp("member");
    let hub = hub(&root);
    let mut owner = hub.window();
    owner
        .command_info("", Cmd::PairOpen(v1::PairOpen {}), WAIT)
        .unwrap();

    // A team member signs in with the code `agora hub user add` prints.
    let out = agora(
        &root.join("hub-home"),
        &[
            "hub",
            "user",
            "add",
            "Ayse",
            "--role",
            "member",
            "--hub-dir",
        ],
    )
    .arg(&hub.dir)
    .output()
    .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let code = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
        .find(|w| w.len() == 9 && w.as_bytes()[4] == b'-')
        .unwrap_or_else(|| panic!("no sign-in code in {text:?}"))
        .to_string();
    let addr = HubAddr {
        addr: format!("localhost:{}", hub.port),
        ca_pem: std::fs::read_to_string(hub.dir.join("ca.pem")).unwrap(),
    };
    let token = agora_tui::client::sign_in(&addr, &code).unwrap();
    let mut member = Session::connect(&addr, &token).unwrap();

    // The hub answers every owner command with `auth.forbidden`, which is how a window knows not to show cards.
    for c in [
        Cmd::PairStatus(v1::PairStatus {}),
        Cmd::PairOpen(v1::PairOpen {}),
        Cmd::PairApprove(v1::PairApprove {
            pair_id: vec![1; 16],
            replace: false,
        }),
        Cmd::PairDeny(v1::PairDeny {
            pair_id: vec![1; 16],
        }),
    ] {
        let e = member.command_info("", c, WAIT).unwrap_err();
        assert_eq!(e.code, "auth.forbidden");
    }

    // A request arrives: the owner is told, the member is not.
    let machine = root.join("machine");
    let mut join = Join::start(&machine, &format!("localhost:{}", hub.port), "desk", false);
    join.wait_for("Number ");
    let card = card_for(&mut owner, "desk", &mut Vec::new());
    let _ = member.next(Duration::from_millis(1500));
    assert!(
        member.take_pair_notes().is_empty(),
        "a member was sent a card"
    );
    owner
        .command_info(
            "",
            Cmd::PairDeny(v1::PairDeny {
                pair_id: card.pair_id,
            }),
            WAIT,
        )
        .unwrap();
}
