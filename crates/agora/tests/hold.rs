//! ADR-0079 W2 step 1 (Unix): the terminal holder, `agora hold`, with `agora-fake-agent` and never a real agent.
//!
//! The "node" is either this test process or a copy of this test binary started as a helper (`AGORA_HOLD_HELPER`), so that one node can be
//! killed and another attach. The role lock (`run/node.lock`) names whichever process is the node, as the front door's lock does.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_node::holder_link::{
    socket_path, write_frame, AgentState, Client, LinkError, Refusal, Reply, Request, StartRequest,
    LINK_VERSION, LINK_VERSION_PIPES, MAX_AGENT_ID,
};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

/// The path a helper is handed, so that it never runs `cargo` itself (see [`fake_agent`]).
const FAKE_AGENT_ENV: &str = "AGORA_HOLD_FAKE_AGENT";

/// `agora-fake-agent`, built once by this test process. A helper (a copy of this binary) takes the path its parent hands it and never
/// builds: on macOS cargo copies a binary into place (never a hard link, cargo issue 10060), so the copy is never "the same file" as
/// its source and every `cargo build`, even one with nothing to do, removes `target/debug/agora-fake-agent` and copies it again. A
/// helper that built while another test's holder started an agent made that start fail (`ENOENT`, refused `Failed`).
fn fake_agent() -> String {
    if let Ok(path) = std::env::var(FAKE_AGENT_ENV) {
        return path;
    }
    let mut p = PathBuf::from(AGORA);
    p.pop();
    let target = p.parent().map(Path::to_path_buf).unwrap_or_default();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary: a binary left from an older checkout may lack what the tests need.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
                .arg(&target)
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    });
    p.to_string_lossy().into_owned()
}

fn temp_home(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "agora-hold-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("home");
    dir
}

/// A script that prints its frames (appending, never clearing) at the given delays, then ends.
fn script(dir: &Path, name: &str, frames: &[(&str, u64)]) -> String {
    let mut text = String::new();
    for (t, delay) in frames {
        text.push_str(&format!(
            "[[frame]]\ntext = {t:?}\ndelay_ms = {delay}\nclear = false\n"
        ));
    }
    let path = dir.join(format!("{name}.toml"));
    std::fs::write(&path, text).expect("script");
    path.to_string_lossy().into_owned()
}

fn start_request(agent: &str, script: &str) -> StartRequest {
    StartRequest {
        agent: agent.into(),
        argv: vec![fake_agent(), "--script".into(), script.into()],
        cwd: None,
        env: Vec::new(),
        env_remove: Vec::new(),
        cols: 80,
        rows: 24,
        cpu_seconds: None,
        memory_bytes: None,
    }
}

/// An `agora hold` on its own home, killed with everything it holds when this value is dropped.
///
/// The holder's stderr (where it says why it refused a start) is `<home>/hold.stderr`. A test that fails keeps its home and prints the
/// end of that file, so a refusal on a CI runner says why.
struct Rig {
    home: PathBuf,
    holder: Child,
}

/// The holder's stderr in a rig's home.
fn holder_log(home: &Path) -> PathBuf {
    home.join("hold.stderr")
}

/// The last `lines` lines of `path`, or why it could not be read. Never panics: it runs while a test unwinds.
fn tail(path: &Path, lines: usize) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let all: Vec<&str> = text.lines().collect();
            all[all.len().saturating_sub(lines)..].join("\n")
        }
        Err(e) => format!("({} not read: {e})", path.display()),
    }
}

impl Rig {
    fn new(tag: &str) -> Rig {
        let home = temp_home(tag);
        agora_platform::ensure_private_dir(&home.join("run")).expect("run folder");
        // Started as a careless starter would: with extra descriptors that are not close-on-exec (7 and 8 are files, 9 is a pipe: the
        // holder's stdin) and a variable of its own that no agent may see.
        let extra = home.join("extra");
        std::fs::write(&extra, b"x").expect("extra file");
        let log = std::fs::File::create(holder_log(&home)).expect("holder log");
        let holder = Command::new("sh")
            .args([
                "-c",
                "exec \"$0\" hold --home \"$1\" 7<\"$2\" 8>>\"$2\" 9<&0",
            ])
            .arg(AGORA)
            .arg(&home)
            .arg(&extra)
            .env("AGORA_HOLD_TEST_SECRET", "leaked")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("start agora hold");
        let rig = Rig { home, holder };
        let end = Instant::now() + Duration::from_secs(15);
        while !socket_path(&rig.home).exists() {
            assert!(Instant::now() < end, "the holder never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
        rig
    }

    fn socket(&self) -> PathBuf {
        socket_path(&self.home)
    }

    fn lock_text(&self, pid: u32, start: u64) {
        std::fs::write(
            self.home.join("run/node.lock"),
            format!("{pid} {start} test\n"),
        )
        .expect("lock");
    }

    /// The lock names `pid`, with its real start time.
    fn lock_for(&self, pid: u32) {
        let start = agora_platform::process_start_time(pid).expect("a running process");
        self.lock_text(pid, start);
    }

    fn lock_self(&self) {
        self.lock_for(std::process::id());
    }

    /// Connects, waiting out the moment before the holder has noticed that the node before this one is gone.
    fn connect(&self) -> Client {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match Client::connect(&self.socket()) {
                Ok(c) => return c,
                Err(LinkError::Refused(Refusal::Busy, _)) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => panic!("connect: {e}"),
            }
        }
    }

    fn holder_running(&mut self) -> bool {
        self.holder.try_wait().expect("try_wait").is_none()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // The agents get SIGHUP with the holder (Linux, ADR-0079 decision 8) and the master closing.
        let _ = self.holder.kill();
        let _ = self.holder.wait();
        if std::thread::panicking() {
            // Kept for a look (the agents ended with the holder); the end of the holder's stderr goes with the failure.
            eprintln!(
                "the holder's home is kept at {}; the end of its stderr:\n{}",
                self.home.display(),
                tail(&holder_log(&self.home), 40)
            );
            return;
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn alive(pid: u32) -> bool {
    agora_platform::process_start_time(pid).is_some()
}

/// Reads a terminal in the background so a test can wait with a timeout.
fn read_in_background(mut master: std::fs::File) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = master.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    rx
}

/// Collects `seen` (what came before) and then more from `rx` until it ends with `until`, or fails after `wait`.
fn collect_until(
    mut seen: Vec<u8>,
    rx: &mpsc::Receiver<Vec<u8>>,
    until: &str,
    wait: Duration,
) -> String {
    let end = Instant::now() + wait;
    while !String::from_utf8_lossy(&seen).contains(until) {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(more) => seen.extend(more),
            Err(_) => panic!(
                "waiting for {until:?}: only {:?} arrived",
                String::from_utf8_lossy(&seen)
            ),
        }
    }
    String::from_utf8_lossy(&seen).into_owned()
}

// ----- the helper processes -----

/// Not a test: a copy of this binary acting as another process. Without `AGORA_HOLD_HELPER` it does nothing.
#[test]
fn helper_entry() {
    let Ok(role) = std::env::var("AGORA_HOLD_HELPER") else {
        return;
    };
    let socket = PathBuf::from(std::env::var("AGORA_HOLD_SOCKET").expect("socket"));
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).expect("go");
    match role.as_str() {
        "node" => {
            let script = std::env::var("AGORA_HOLD_SCRIPT").expect("script");
            // A failure is printed as the helper's line (its stderr is not kept), so the test that waits for it says why.
            let started = Client::connect(&socket)
                .map_err(|e| format!("connect: {e}"))
                .and_then(|mut c| {
                    c.attach().map_err(|e| format!("attach: {e}"))?;
                    let (pid, terminal) = c
                        .start(&start_request("a1", &script))
                        .map_err(|e| format!("start: {e}"))?;
                    Ok((c, pid, terminal))
                });
            // The link and this node's copy of the terminal are kept until the helper is killed.
            let _held = match started {
                Ok((c, pid, terminal)) => {
                    println!("\nHELPER started {pid}");
                    (c, terminal)
                }
                Err(e) => {
                    println!("\nHELPER failed {e}");
                    std::io::stdout().flush().ok();
                    std::process::exit(1);
                }
            };
            std::io::stdout().flush().ok();
            // Keeps the link and the terminal until killed.
            let mut rest = String::new();
            while std::io::stdin()
                .lock()
                .read_line(&mut rest)
                .is_ok_and(|n| n > 0)
            {}
        }
        "intruder" => {
            match Client::connect(&socket) {
                Ok(_) => println!("\nHELPER connected"),
                Err(e) => println!("\nHELPER refused {e}"),
            }
            std::io::stdout().flush().ok();
        }
        "flood" => {
            // Connects and closes as fast as it can, until it is killed: a same-user process that is not the node.
            loop {
                let _ = agora_platform::local_connect(&socket);
            }
        }
        other => panic!("unknown helper {other}"),
    }
    std::process::exit(0);
}

fn helper(role: &str, rig: &Rig, script: Option<&str>) -> Child {
    let mut cmd = Command::new(std::env::current_exe().expect("exe"));
    cmd.args(["--exact", "helper_entry", "--nocapture", "--test-threads=1"])
        .env("AGORA_HOLD_HELPER", role)
        .env("AGORA_HOLD_SOCKET", rig.socket())
        // Built here, never by the helper (see `fake_agent`); a helper that tried to build anyway fails at once.
        .env(FAKE_AGENT_ENV, fake_agent())
        .env("CARGO", rig.home.join("no-cargo-in-a-helper"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(s) = script {
        cmd.env("AGORA_HOLD_SCRIPT", s);
    }
    cmd.spawn().expect("start helper")
}

fn say_go(child: &mut Child) {
    let stdin = child.stdin.as_mut().expect("stdin");
    stdin.write_all(b"go\n").expect("go");
    stdin.flush().expect("flush");
}

/// The first line the helper prints that starts with `HELPER`, after its start-up noise.
fn helper_line(child: &mut Child) -> String {
    let out = child.stdout.as_mut().expect("stdout");
    let mut reader = std::io::BufReader::new(out);
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).expect("line") > 0,
            "the helper ended"
        );
        if let Some(rest) = line.trim().strip_prefix("HELPER ") {
            assert!(!rest.starts_with("failed "), "the helper {rest}");
            return rest.to_string();
        }
    }
}

// ----- the tests -----

#[test]
fn the_agent_outlives_its_node_and_a_new_node_gets_its_terminal_and_the_output_written_meanwhile() {
    let mut rig = Rig::new("survive");
    let long = script(
        &rig.home,
        "long",
        &[("first", 1500), ("second", 1500), ("third", 120_000)],
    );
    // Node A (another process) starts the agent and holds its terminal.
    let mut a = helper("node", &rig, Some(&long));
    rig.lock_for(a.id());
    say_go(&mut a);
    let started = helper_line(&mut a);
    let pid: u32 = started
        .strip_prefix("started ")
        .and_then(|p| p.parse().ok())
        .expect("a pid");
    let started_at = agora_platform::process_start_time(pid).expect("the agent runs");

    // Node A is killed. The holder and the agent are not touched.
    a.kill().expect("kill the node");
    a.wait().expect("wait");
    assert!(rig.holder_running());
    assert_eq!(agora_platform::process_start_time(pid), Some(started_at));

    // Node B takes the lock and attaches on a new connection.
    rig.lock_self();
    let mut b = rig.connect();
    let listed = b.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].pid, listed[0].state), (pid, AgentState::Running));
    let mut held = b.attach().expect("attach");
    assert_eq!(held.len(), 1);
    let h = held.remove(0);
    assert_eq!(h.info.info.pid, pid, "the same process, not a new one");
    let master = h.terminal.expect("the terminal").into_file().expect("file");
    let mut typed = master.try_clone().expect("clone");
    let rx = read_in_background(master);

    // The terminal answers: the line discipline echoes what is typed.
    typed.write_all(b"ping\n").expect("type");

    // What was written while no node was attached (the buffer) and what comes after (the terminal) are the whole output, once each.
    let all = collect_until(h.info.buffer, &rx, "third", Duration::from_secs(20));
    assert_eq!(
        all.replacen("ping\r\n", "", 1),
        "firstsecondthird",
        "got {all:?}"
    );
    assert!(
        all.contains("ping\r\n"),
        "the terminal did not answer: {all:?}"
    );
    assert_eq!(agora_platform::process_start_time(pid), Some(started_at));
}

#[test]
fn the_holders_own_read_ends_before_the_handover() {
    let mut rig = Rig::new("handover");
    // "post" is written after the new node attached: the holder must not be the one that reads it.
    let long = script(&rig.home, "long", &[("pre", 4000), ("post", 120_000)]);
    let mut a = helper("node", &rig, Some(&long));
    rig.lock_for(a.id());
    say_go(&mut a);
    let _ = helper_line(&mut a);
    a.kill().expect("kill the node");
    a.wait().expect("wait");

    rig.lock_self();
    let mut b = rig.connect();
    let mut held = b.attach().expect("attach");
    let h = held.remove(0);
    let rx = read_in_background(h.terminal.expect("terminal").into_file().expect("file"));
    // Written 4 s after the start: it reaches the node's terminal, not the holder's buffer.
    let all = collect_until(h.info.buffer, &rx, "post", Duration::from_secs(20));
    assert_eq!(all, "prepost");
    assert!(rig.holder_running());
}

#[test]
fn an_agent_that_ends_while_no_node_is_attached_keeps_its_status_and_output_until_collected() {
    let mut rig = Rig::new("ended");
    let short = script(&rig.home, "short", &[("bye", 0)]);
    let mut a = helper("node", &rig, Some(&short));
    rig.lock_for(a.id());
    say_go(&mut a);
    let _ = helper_line(&mut a);
    a.kill().expect("kill the node");
    a.wait().expect("wait");
    // The agent ended on its own; the holder has no node and keeps running.
    std::thread::sleep(Duration::from_millis(600));
    assert!(rig.holder_running());

    rig.lock_self();
    let mut b = rig.connect();
    let listed = b.list().expect("list");
    assert_eq!(
        listed[0].state,
        AgentState::Exited {
            code: Some(0),
            signal: None
        }
    );
    let held = b.attach().expect("attach");
    assert_eq!(held.len(), 1);
    assert!(
        held[0].terminal.is_none(),
        "an ended agent has no terminal to take"
    );
    assert_eq!(held[0].info.buffer, b"bye");
    // Not collected yet: a node that took it and died finds it again.
    drop(b);
    let mut b = rig.connect();
    let again = b.attach().expect("attach again");
    assert_eq!(again[0].info.buffer, b"bye");
    assert!(
        rig.holder_running(),
        "the holder ends only after a node took everything"
    );

    b.release("a1").expect("release");
    let end = Instant::now() + Duration::from_secs(10);
    while rig.holder_running() {
        assert!(
            Instant::now() < end,
            "the holder did not end with nothing left"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!socket_path(&rig.home).exists(), "the endpoint is removed");
}

#[test]
fn a_second_node_connection_is_refused_while_one_is_attached() {
    let rig = Rig::new("busy");
    rig.lock_self();
    let mut first = rig.connect();
    match Client::connect(&rig.socket()) {
        Err(LinkError::Refused(Refusal::Busy, v)) => assert_eq!(v, LINK_VERSION),
        Err(e) => panic!("refused for another reason: {e}"),
        Ok(_) => panic!("a second node was let in"),
    }
    // The first is undisturbed.
    assert!(first.list().expect("list").is_empty());
}

#[test]
fn a_peer_that_is_not_the_node_holding_the_lock_cannot_use_the_endpoint() {
    let mut rig = Rig::new("peer");

    // No lock file at all.
    assert!(Client::connect(&rig.socket()).is_err());

    // The lock names another live process: this one is not the node.
    let mut other = Command::new("sleep").arg("60").spawn().expect("sleep");
    rig.lock_for(other.id());
    assert!(
        matches!(
            Client::connect(&rig.socket()),
            Err(LinkError::Closed | LinkError::Io(_))
        ),
        "a process that does not hold the lock got an answer"
    );

    // The lock names this process but a start time that is not its own: a reused pid is not the node.
    let start = agora_platform::process_start_time(std::process::id()).expect("start");
    rig.lock_text(std::process::id(), start + 1);
    assert!(Client::connect(&rig.socket()).is_err());

    // A confined-agent style peer: same user, another process, while the lock names this one.
    rig.lock_self();
    let mut intruder = helper("intruder", &rig, None);
    say_go(&mut intruder);
    let said = helper_line(&mut intruder);
    assert!(said.starts_with("refused"), "the intruder got in: {said}");
    let _ = intruder.wait();

    // None of that disturbed the holder: the real node is served at once.
    let mut c = Client::connect(&rig.socket()).expect("the node connects");
    assert!(c.list().expect("list").is_empty());
    assert!(rig.holder_running());
    let _ = other.kill();
    let _ = other.wait();
}

#[test]
fn a_version_mismatch_changes_nothing() {
    let rig = Rig::new("version");
    rig.lock_self();
    let long = script(&rig.home, "long", &[("x", 120_000)]);
    let pid = {
        let mut c = rig.connect();
        c.attach().expect("attach");
        let (pid, _terminal) = c.start(&start_request("a1", &long)).expect("start");
        pid
    };
    // Waits out the holder noticing that the first connection closed.
    std::thread::sleep(Duration::from_millis(400));

    match Client::connect_with_version(&rig.socket(), LINK_VERSION_PIPES + 1) {
        Err(LinkError::Refused(Refusal::VersionMismatch, v)) => assert_eq!(v, LINK_VERSION),
        other => panic!("expected a version refusal, got {:?}", other.map(|_| ())),
    }
    // The next node, of the right version, finds everything as it was and the slot free.
    let mut c = Client::connect(&rig.socket()).expect("connects at once");
    let listed = c.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].pid, listed[0].state), (pid, AgentState::Running));
    assert!(alive(pid));
}

/// Kills the processes it holds when dropped, so a failed test leaves none.
struct Reaper(Vec<Child>);

impl Drop for Reaper {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[test]
fn a_flood_of_connections_cannot_starve_the_node() {
    let rig = Rig::new("flood");
    rig.lock_self();
    let mut node = rig.connect();
    node.attach().expect("attach");
    let mut flood = Reaper(Vec::new());
    for _ in 0..8 {
        let mut f = helper("flood", &rig, None);
        say_go(&mut f);
        flood.0.push(f);
    }
    std::thread::sleep(Duration::from_millis(500));
    // The node's requests are answered at once, whatever else connects: each of these took seconds when accepting had no bound.
    for _ in 0..5 {
        let began = Instant::now();
        node.list().expect("list");
        let took = began.elapsed();
        assert!(
            took < Duration::from_secs(2),
            "a list took {took:?} under a flood"
        );
    }
}

#[test]
fn a_node_that_keeps_its_terminal_and_drops_only_the_link_reattaches_at_once_and_loses_no_output() {
    let rig = Rig::new("livenode");
    rig.lock_self();
    // "a" at the start, "b" six seconds later.
    let long = script(&rig.home, "long", &[("a", 6000), ("b", 120_000)]);
    let mut c = rig.connect();
    c.attach().expect("attach");
    let (_pid, terminal) = c.start(&start_request("a1", &long)).expect("start");
    // The node keeps its master and reads it itself.
    let rx = read_in_background(terminal.into_file().expect("file"));
    drop(c);
    std::thread::sleep(Duration::from_millis(300));

    // Only the link dropped: the node process runs and still reads its terminal, so the holder reads none (a read of a terminal that
    // two processes read blocks, and ending it would never return). Reattaching is prompt.
    let began = Instant::now();
    let mut c = rig.connect();
    let held = c.attach().expect("attach again");
    let took = began.elapsed();
    assert!(took < Duration::from_secs(3), "reattaching took {took:?}");
    assert_eq!(held.len(), 1);
    assert!(
        held[0].info.buffer.is_empty(),
        "the holder read what the node reads"
    );
    assert!(held[0].terminal.is_some());
    // Everything the agent wrote reached the node's own read once.
    let all = collect_until(Vec::new(), &rx, "b", Duration::from_secs(20));
    assert_eq!(all, "ab");
}

#[test]
fn a_buffer_is_not_lost_when_the_link_breaks_during_attach() {
    let rig = Rig::new("midsend");
    // More than one socket buffer holds, so the send cannot finish into the void.
    let text: String = format!("{}END", "x".repeat(600_000));
    let big = script(&rig.home, "big", &[(&text, 120_000)]);
    let mut a = helper("node", &rig, Some(&big));
    rig.lock_for(a.id());
    say_go(&mut a);
    let _ = helper_line(&mut a);
    a.kill().expect("kill the node");
    a.wait().expect("wait");
    // The holder reads it all while no node is there.
    std::thread::sleep(Duration::from_millis(2500));

    rig.lock_self();
    // A node that asks to attach and goes away before reading anything.
    let mut raw = {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let mut s = agora_platform::local_connect(&rig.socket()).expect("connect");
            write_frame(
                &mut s,
                &Request::Hello {
                    version: LINK_VERSION,
                }
                .encode(),
                None,
            )
            .expect("hello");
            s.set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let (payload, _) = agora_node::holder_link::read_frame(&mut s, Duration::from_secs(5))
                .expect("frame")
                .expect("an answer");
            match Reply::decode(&payload).expect("reply") {
                Reply::HelloOk { .. } => break s,
                Reply::Refused {
                    code: Refusal::Busy,
                    ..
                } if Instant::now() < end => {}
                other => panic!("{other:?}"),
            }
        }
    };
    write_frame(&mut raw, &Request::Attach.encode(), None).expect("attach");
    drop(raw);

    // The next node still gets all of it.
    let mut b = rig.connect();
    let held = b.attach().expect("attach");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].info.buffer.len(), 600_003);
    assert!(held[0].info.buffer.ends_with(b"END"));
}

#[test]
fn an_agent_inherits_no_descriptor_of_the_holder() {
    let rig = Rig::new("cloexec");
    rig.lock_self();
    let mut c = rig.connect();
    c.attach().expect("attach");
    let request = StartRequest {
        agent: "a1".into(),
        argv: vec![fake_agent(), "--list-fds".into()],
        cols: 80,
        rows: 24,
        ..StartRequest::default()
    };
    let (_pid, terminal) = c.start(&request).expect("start");
    let rx = read_in_background(terminal.into_file().expect("file"));
    let all = collect_until(Vec::new(), &rx, "FDS-END", Duration::from_secs(20));
    // The holder was started with descriptors 7, 8 and 9 open (see `Rig::new`). The agent has its terminal (0, 1, 2) and at most the
    // one descriptor its own listing of the folder takes (`/proc/self/fd`, or `/dev/fd`).
    let mut seen = 0;
    let mut beyond = Vec::new();
    for line in all.lines().filter_map(|l| l.trim().strip_prefix("FD ")) {
        seen += 1;
        let (n, target) = line.split_once(' ').unwrap_or((line, ""));
        if n.parse::<u32>().is_ok_and(|n| n > 2) {
            beyond.push((n.to_string(), target.to_string()));
        }
    }
    assert!(seen >= 3, "{all:?}");
    assert!(
        beyond.len() <= 1
            && beyond
                .iter()
                .all(|(_, t)| t.is_empty() || t.starts_with("/proc/") || t.starts_with("/dev/fd")),
        "the agent holds descriptors of the holder: {beyond:?} in {all:?}"
    );
}

#[test]
fn an_agent_does_not_see_the_holders_own_environment() {
    let rig = Rig::new("env");
    rig.lock_self();
    let mut c = rig.connect();
    c.attach().expect("attach");
    let request = StartRequest {
        agent: "a1".into(),
        argv: vec![fake_agent(), "--print-env".into()],
        env: vec![("VISIBLE".into(), "1".into())],
        cols: 80,
        rows: 24,
        ..StartRequest::default()
    };
    let (_pid, terminal) = c.start(&request).expect("start");
    let rx = read_in_background(terminal.into_file().expect("file"));
    let all = collect_until(Vec::new(), &rx, "ENV-END", Duration::from_secs(20));
    assert!(all.contains("ENV VISIBLE=1"), "{all:?}");
    assert!(
        !all.contains("AGORA_HOLD_TEST_SECRET"),
        "the holder's own variable reached the agent: {all:?}"
    );
    // Nothing else of the holder's environment either: its HOME, PATH and the rest are the node's to send.
    assert!(!all.contains("ENV HOME="), "{all:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn the_holder_closes_what_its_starter_handed_it() {
    let rig = Rig::new("inherited");
    let proc = format!("/proc/{}/fd", rig.holder.id());
    for n in [7, 8, 9] {
        assert!(
            !Path::new(&format!("{proc}/{n}")).exists(),
            "descriptor {n} of the starter is still open in the holder"
        );
    }
}

#[test]
fn a_holder_that_leads_a_process_group_refuses_to_start() {
    use std::os::unix::process::CommandExt;
    // `setsid` fails in a group leader: the holder would stay in its starter's session and end with it, so it must not run.
    let home = temp_home("leader");
    let status = Command::new(AGORA)
        .args(["hold", "--home"])
        .arg(&home)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run agora hold");
    assert_eq!(status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_node_that_stops_reading_cannot_hang_the_holder() {
    let rig = Rig::new("stuck");
    // More than a socket holds, so the holder's send to a node that never reads cannot finish.
    let text = "y".repeat(1_100_000);
    let big = script(&rig.home, "big", &[(&text, 120_000)]);
    let mut a = helper("node", &rig, Some(&big));
    rig.lock_for(a.id());
    say_go(&mut a);
    let _ = helper_line(&mut a);
    a.kill().expect("kill the node");
    a.wait().expect("wait");
    std::thread::sleep(Duration::from_millis(3000));

    rig.lock_self();
    let mut stuck = agora_platform::local_connect(&rig.socket()).expect("connect");
    write_frame(
        &mut stuck,
        &Request::Hello {
            version: LINK_VERSION,
        }
        .encode(),
        None,
    )
    .expect("hello");
    stuck
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let _ =
        agora_node::holder_link::read_frame(&mut stuck, Duration::from_secs(5)).expect("answer");
    write_frame(&mut stuck, &Request::Attach.encode(), None).expect("attach");
    // It reads nothing and stays connected. After the holder's write timeout it lets this node go, and the next one is served.
    let began = Instant::now();
    let mut c = rig.connect();
    assert!(c.list().expect("list").len() == 1);
    assert!(began.elapsed() < Duration::from_secs(10));
    drop(stuck);
}

#[test]
fn the_buffer_keeps_exactly_the_last_mebibyte_and_says_it_dropped() {
    let rig = Rig::new("bound");
    // 1.5 MB of a pattern a miscounted bound cannot match by luck.
    let total = 1_500_000usize;
    let text: String = (0..total)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    let flood = script(&rig.home, "flood", &[(&text, 120_000)]);
    let mut a = helper("node", &rig, Some(&flood));
    rig.lock_for(a.id());
    say_go(&mut a);
    let _ = helper_line(&mut a);
    a.kill().expect("kill the node");
    a.wait().expect("wait");
    std::thread::sleep(Duration::from_millis(3000));

    rig.lock_self();
    let mut b = rig.connect();
    let held = b.attach().expect("attach");
    let buffer = &held[0].info.buffer;
    assert_eq!(buffer.len(), 1 << 20);
    assert!(held[0].info.buffer_dropped);
    assert_eq!(buffer.as_slice(), &text.as_bytes()[total - (1 << 20)..]);
}

/// #929: an agent id is at most `MAX_AGENT_ID` bytes. A longer one (here one byte longer, and one of 3 MiB, which still fits a `Start`
/// frame but would have made every later `Attach` reply larger than a frame) is refused on `Start` and nothing starts; one at the bound
/// starts, is listed, and is handed over by `Attach`.
#[test]
fn an_agent_id_longer_than_the_bound_is_refused_and_one_at_the_bound_is_listed_and_handed_over() {
    let rig = Rig::new("idlen");
    rig.lock_self();
    let idle = script(&rig.home, "idle", &[("up", 0), ("bye", 600_000)]);
    let mut node = rig.connect();
    assert!(node.attach().expect("attach").is_empty());
    for long in ["a".repeat(MAX_AGENT_ID + 1), "a".repeat(3 << 20)] {
        match node.start(&start_request(&long, &idle)) {
            Err(LinkError::Refused(Refusal::BadRequest, _)) => {}
            Err(e) => panic!("refused for another reason: {e}"),
            Ok((pid, _)) => {
                let _ = node.stop(&long, true);
                panic!("an id of {} bytes was started (pid {pid})", long.len());
            }
        }
    }
    assert!(node.list().expect("list").is_empty(), "nothing started");

    let at = "b".repeat(MAX_AGENT_ID);
    let (pid, master) = node.start(&start_request(&at, &idle)).expect("start");
    let listed = node.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].agent.as_str(), listed[0].pid),
        (at.as_str(), pid)
    );
    // The link drops (the node keeps its terminal) and a node attaches again: the agent comes back under its id.
    drop(node);
    let mut node = rig.connect();
    let held = node.attach().expect("attach");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].info.info.agent, at);
    assert!(held[0].terminal.is_some());
    node.stop(&at, true).expect("stop");
    drop(master);
}

/// A start the holder refuses with `Failed` says why on its stderr (`run/hold.log` when a node started it): the refusal on the link is
/// frozen and cannot carry it. Here the program is not there: the kind of error and whether the program exists now are in the log, and
/// the holder serves on.
#[test]
fn a_start_refused_because_the_program_is_missing_says_why_on_the_holders_stderr() {
    let rig = Rig::new("missing");
    rig.lock_self();
    let mut c = rig.connect();
    c.attach().expect("attach");
    let missing = rig.home.join("no-such-program");
    let request = StartRequest {
        agent: "a1".into(),
        argv: vec![
            missing.to_string_lossy().into_owned(),
            "--secret-arg".into(),
        ],
        cols: 80,
        rows: 24,
        ..StartRequest::default()
    };
    match c.start(&request) {
        Err(LinkError::Refused(Refusal::Failed, _)) => {}
        Err(e) => panic!("refused for another reason: {e}"),
        Ok((pid, _)) => panic!("a missing program started (pid {pid})"),
    }
    let log = std::fs::read_to_string(holder_log(&rig.home)).expect("the holder's stderr");
    let line = log
        .lines()
        .find(|l| l.contains("\"a1\" refused (Failed)"))
        .unwrap_or_else(|| panic!("no reason logged: {log:?}"));
    assert!(line.contains("NotFound"), "{line}");
    assert!(
        line.contains(&format!(
            "{:?} exists now: false",
            missing.to_string_lossy()
        )),
        "{line}"
    );
    assert!(
        !line.contains("--secret-arg"),
        "the arguments were logged: {line}"
    );
    // The holder serves on.
    assert!(c.list().expect("list").is_empty());
}
